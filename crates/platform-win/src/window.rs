//! Finding, describing and validating other applications' windows.
//!
//! The filter is deliberately conservative: a window we cannot describe, reach
//! or raise is not offered to the wheel at all.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{BOOL, PWSTR};
use windows::Win32::Foundation::{CloseHandle, FILETIME, HWND, LPARAM, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP,
};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, GetProcessTimes, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow;
use windows::Win32::UI::Shell::IVirtualDesktopManager;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, CreateWindowExW, DestroyWindow, EnumWindows, GetClassLongPtrW, GetClassNameW,
    GetForegroundWindow, GetIconInfo, GetTopWindow, GetWindow, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, IsZoomed,
    SendMessageTimeoutW, SetForegroundWindow, SetWindowPos, ShowWindow, SwitchToThisWindow,
    GCLP_HICON, GCLP_HICONSM, GWL_EXSTYLE, GW_HWNDNEXT, GW_HWNDPREV, GW_OWNER, HICON,
    HWND_NOTOPMOST, HWND_TOP, ICONINFO, ICON_BIG, ICON_SMALL, ICON_SMALL2, SMTO_ABORTIFHUNG,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE, WM_GETICON, WS_EX_APPWINDOW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use crate::Rect;

/// How long [`icon_of`] waits for an application that is not answering. Long
/// enough for a busy-but-alive program to get its turn, short enough that the
/// settings page is back on screen before anybody wonders about it.
const ICON_TIMEOUT_MS: u32 = 250;

/// Classes that are never windows the user thinks of as theirs.
const IGNORED_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "DV2ControlHost",
    "SysShadow",
    "Windows.UI.Core.CoreWindow",
];

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

pub fn hwnd_of(raw: isize) -> HWND {
    HWND(raw as *mut core::ffi::c_void)
}

pub fn raw_of(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

/// One window, described well enough to name it and reach it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    /// Window title, as shown on the wheel.
    pub title: String,
    pub class: String,
    /// Lower-cased executable name, e.g. `chrome.exe`. This is the identity
    /// used for the accent colour and for grouping.
    pub exe: String,
    pub exe_path: String,
    pub rect: Rect,
    pub maximized: bool,
    /// Creation time of the owning process, in 100 ns units. Together with the
    /// pid this identifies the *process instance*, not just the pid, so a
    /// recycled pid cannot be mistaken for the window this record describes.
    pub process_start: u64,
    /// Whether the window is on the virtual desktop the user is looking at. A
    /// window on another desktop is still listed - it can be switched to - but it
    /// cannot be previewed without moving it, so the dial leaves it where it is.
    pub on_current_desktop: bool,
    /// Whether the owning process runs at a higher integrity level than us.
    /// Such a window is still listed - the user expects Task Manager to be on
    /// the wheel - but it cannot be raised or switched to from here (UIPI).
    pub elevated: bool,
    /// `false` only when the window exists but this process may not touch it.
    /// The dial offers it, says why, and does not try an operation Windows will
    /// refuse anyway.
    pub accessible: bool,
}

impl WindowInfo {
    /// The line under the title: the application the window belongs to.
    pub fn app_name(&self) -> String {
        let stem = self.exe.strip_suffix(".exe").unwrap_or(&self.exe);
        let mut chars = stem.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => "Window".to_string(),
        }
    }
}

pub fn title(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if n <= 0 {
        String::new()
    } else {
        from_wide(&buf[..n as usize])
    }
}

pub fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    if n <= 0 {
        String::new()
    } else {
        from_wide(&buf[..n as usize])
    }
}

pub fn pid_of(hwnd: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
    }
    pid
}

/// The window's own small icon, borrowed from the process that owns the window.
///
/// `WM_GETICON` answers for anything built with a manifest; an older program
/// keeps its small icon in the class record, which is what `GCLP_HICONSM` reads.
/// The ask goes out with a timeout because the settings page calls this while it
/// is on screen: one application that has stopped pumping messages must not take
/// the page down with it.
///
/// The handle belongs to the other process, so it is drawn with and never
/// destroyed, and it is only good for as long as that window is - which is why
/// the caller asks again whenever it re-reads the list rather than remembering
/// one. A window that has neither, or a favourite whose window is shut, answers
/// `None`; that is the honest answer, and a list leaves the slot empty instead
/// of inventing a decoration to fill it.
pub fn icon_of(hwnd: HWND) -> Option<HICON> {
    if hwnd.is_invalid() {
        return None;
    }
    let asked = |which: u32| -> Option<HICON> {
        let mut result = 0usize;
        let replied = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(which as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                ICON_TIMEOUT_MS,
                Some(&mut result),
            )
        };
        // A timeout or a hung target answers zero, and a window with no icon of
        // that kind answers zero too. Both mean "ask the class record".
        (replied.0 != 0 && result != 0).then_some(HICON(result as *mut core::ffi::c_void))
    };
    asked(ICON_SMALL2)
        .or_else(|| asked(ICON_SMALL))
        .or_else(|| {
            let raw = unsafe { GetClassLongPtrW(hwnd, GCLP_HICONSM) };
            (raw != 0).then_some(HICON(raw as *mut core::ffi::c_void))
        })
}

/// The same icon at the size the dial wants: the large one Windows asks a
/// window for, falling back to the small one and then to the class record.
fn icon_large_of(hwnd: HWND) -> Option<HICON> {
    if hwnd.is_invalid() {
        return None;
    }
    let asked = |which: u32| -> Option<HICON> {
        let mut result = 0usize;
        let replied = unsafe {
            SendMessageTimeoutW(
                hwnd,
                WM_GETICON,
                WPARAM(which as usize),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                ICON_TIMEOUT_MS,
                Some(&mut result),
            )
        };
        (replied.0 != 0 && result != 0).then_some(HICON(result as *mut core::ffi::c_void))
    };
    asked(ICON_BIG)
        .or_else(|| asked(ICON_SMALL2))
        .or_else(|| asked(ICON_SMALL))
        .or_else(|| {
            let raw = unsafe { GetClassLongPtrW(hwnd, GCLP_HICON) };
            (raw != 0).then_some(HICON(raw as *mut core::ffi::c_void))
        })
}

/// The window's icon as premultiplied RGBA, ready for the dial's raster.
///
/// `DrawIconEx` would be simpler, but GDI drops the alpha channel on the way
/// out, so an icon drawn that way arrives on a black square. Reading the two
/// bitmaps behind the handle keeps the shape: the colour bitmap supplies the
/// pixels and the AND mask supplies the transparency the oldest icon formats
/// still use. Both bitmaps belong to us - `GetIconInfo` made them - and both
/// are freed here. The icon handle itself is shared and is never destroyed.
/// Icons belong to the application that owns the window and change almost
/// never, so the pixels read once are kept for the life of the process. The
/// pid is stored beside them because Windows recycles window handles: a
/// different process at the same handle must not inherit the old icon.
struct IconEntry {
    pid: u32,
    data: Option<(Vec<u8>, u32, u32)>,
}

fn icon_cache() -> &'static Mutex<HashMap<isize, IconEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<isize, IconEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn icon_rgba(hwnd: HWND) -> Option<(Vec<u8>, u32, u32)> {
    if hwnd.is_invalid() {
        return None;
    }
    let pid = pid_of(hwnd);
    let key = hwnd.0 as isize;
    {
        let cache = icon_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = cache.get(&key) {
            if entry.pid == pid {
                return entry.data.clone();
            }
        }
    }
    let data = read_icon_rgba(hwnd);
    let mut cache = icon_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(
        key,
        IconEntry {
            pid,
            data: data.clone(),
        },
    );
    data
}

fn read_icon_rgba(hwnd: HWND) -> Option<(Vec<u8>, u32, u32)> {
    let icon = icon_large_of(hwnd)?;
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let out = if info.hbmColor.is_invalid() {
            None
        } else {
            bitmap_rgba(info.hbmColor, info.hbmMask)
        };
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor.into());
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
        out
    }
}

unsafe fn bitmap_rgba(color: HBITMAP, mask: HBITMAP) -> Option<(Vec<u8>, u32, u32)> {
    unsafe {
        let mut bm = BITMAP::default();
        let asked = GetObjectW(
            color.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut core::ffi::c_void),
        );
        if asked == 0 {
            return None;
        }
        let w = bm.bmWidth.max(0) as u32;
        let h = bm.bmHeight.max(0) as u32;
        if w == 0 || h == 0 || w > 512 || h > 512 {
            return None;
        }
        let hdc = CreateCompatibleDC(None);
        if hdc.is_invalid() {
            return None;
        }
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                // Top-down, so row zero is the top of the icon.
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let read = GetDIBits(
            hdc,
            color,
            0,
            h,
            Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
            &mut bi,
            DIB_RGB_COLORS,
        );
        if read == 0 {
            let _ = DeleteDC(hdc);
            return None;
        }
        // GDI hands back blue first, and tiny-skia wants red first.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        if !mask.is_invalid() {
            let stride = w.div_ceil(32) as usize * 4;
            let mut mask_bi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w as i32,
                    biHeight: -(h as i32),
                    biPlanes: 1,
                    biBitCount: 1,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = vec![0u8; stride * h as usize];
            let read = GetDIBits(
                hdc,
                mask,
                0,
                h,
                Some(bits.as_mut_ptr() as *mut core::ffi::c_void),
                &mut mask_bi,
                DIB_RGB_COLORS,
            );
            if read != 0 {
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let transparent = bits[y * stride + x / 8] & (0x80 >> (x % 8)) != 0;
                        let pixel = &mut pixels[(y * w as usize + x) * 4..][..4];
                        if transparent {
                            pixel[3] = 0;
                        } else if pixel[3] == 0 {
                            pixel[3] = 255;
                        }
                    }
                }
            }
        }
        let _ = DeleteDC(hdc);
        // tiny-skia's raster is premultiplied; the bitmap is not.
        for pixel in pixels.chunks_exact_mut(4) {
            let a = pixel[3] as u32;
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u32 * a + 127) / 255) as u8;
            }
        }
        Some((pixels, w, h))
    }
}

/// What we know about one process. Opening a process and reading its token is
/// three syscalls; the dial asks for the same process once per window and every
/// time it opens, so the answers are kept for a minute.
#[derive(Clone)]
struct ProcInfo {
    path: Option<String>,
    start: u64,
    elevated: Option<bool>,
    at: Instant,
}

const PROC_CACHE_TTL: Duration = Duration::from_secs(60);

fn proc_cache() -> &'static Mutex<HashMap<u32, ProcInfo>> {
    static CACHE: OnceLock<Mutex<HashMap<u32, ProcInfo>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn empty_proc() -> ProcInfo {
    ProcInfo {
        path: None,
        start: 0,
        elevated: None,
        at: Instant::now(),
    }
}

fn read_proc_info(pid: u32) -> ProcInfo {
    ProcInfo {
        path: raw_process_path(pid),
        start: raw_process_start(pid),
        elevated: raw_process_elevated(pid),
        at: Instant::now(),
    }
}

fn proc_info(pid: u32) -> ProcInfo {
    if pid == 0 {
        return empty_proc();
    }
    let cached = {
        let cache = proc_cache()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        cache.get(&pid).cloned()
    };
    if let Some(info) = cached {
        if info.at.elapsed() < PROC_CACHE_TTL {
            return info;
        }
    }
    let info = read_proc_info(pid);
    let mut cache = proc_cache()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    cache.insert(pid, info.clone());
    info
}

fn raw_process_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle);
        if !ok || len == 0 {
            return None;
        }
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

fn raw_process_start(pid: u32) -> u64 {
    if pid == 0 {
        return 0;
    }
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return 0;
        };
        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return 0;
        }
        ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64
    }
}

fn raw_process_elevated(pid: u32) -> Option<bool> {
    if pid == 0 {
        return None;
    }
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut token = windows::Win32::Foundation::HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token).is_ok();
        let _ = CloseHandle(process);
        if !opened {
            return None;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut core::ffi::c_void),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if ok {
            Some(elevation.TokenIsElevated != 0)
        } else {
            None
        }
    }
}

/// The window's process, or `None` when we are not allowed to look.
pub fn process_path(pid: u32) -> Option<String> {
    proc_info(pid).path
}

/// Creation time of a process. Zero when the process cannot be inspected.
pub fn process_start(pid: u32) -> u64 {
    proc_info(pid).start
}

/// Whether a process runs elevated. `None` when the token cannot be read.
pub fn process_elevated(pid: u32) -> Option<bool> {
    proc_info(pid).elevated
}

/// Whether *we* run elevated. Cached: this is a syscall, and `check` runs once
/// for every window on the desktop each time the dial is built, so it must not
/// pay for it four times a window.
pub fn self_elevated() -> bool {
    static ELEVATED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ELEVATED.get_or_init(|| process_elevated(std::process::id()).unwrap_or(false))
}

/// UWP windows and windows on another virtual desktop are "cloaked": they
/// exist, but the user cannot see them. The taskbar distinguishes the two cases
/// by asking the virtual desktop manager, and so do we: a window on another
/// desktop is worth offering, an application's suspended window is not.
pub fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut core::ffi::c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };
    ok.is_ok() && cloaked != 0
}

// The COM object Windows publishes for asking about virtual desktops. It is
// undocumented as a class name only: the interface itself is public, and one
// instance per thread is enough. If COM or the object is unavailable - the
// shell has changed, a session has no desktop manager - every window is treated
// as being on the current desktop, which is what the program assumed before.
thread_local! {
    static DESKTOP: std::cell::RefCell<Option<IVirtualDesktopManager>> = const { std::cell::RefCell::new(None) };
}

fn with_desktop<T>(f: impl FnOnce(&IVirtualDesktopManager) -> T) -> Option<T> {
    DESKTOP.with(|cell| {
        if cell.borrow().is_none() {
            let made = unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                let clsid = windows::core::GUID::from_u128(0xaa509086_5ca9_4c25_8f95_589d3c07b48a);
                CoCreateInstance(&clsid, None, CLSCTX_ALL).ok()
            };
            *cell.borrow_mut() = made;
        }
        let slot = cell.borrow();
        slot.as_ref().map(f)
    })
}

/// Whether a window is on the desktop the user is looking at. A window we cannot
/// ask about is assumed to be here, which keeps the old behaviour.
pub fn on_current_desktop(hwnd: HWND) -> bool {
    with_desktop(|desktops| unsafe {
        desktops
            .IsWindowOnCurrentVirtualDesktop(hwnd)
            .map(|on| on.as_bool())
            .unwrap_or(true)
    })
    .unwrap_or(true)
}

/// Brings a window from another virtual desktop to the one the pointer is on,
/// then reports whether it is here. The only documented way to name the current
/// desktop is to ask a window that is on it, so a foreground window is the anchor
/// when there is one; otherwise a throwaway window is made on the active desktop
/// and asked instead. Nothing is activated here - the caller raises the window.
pub fn move_to_current_desktop(hwnd: HWND) -> bool {
    if on_current_desktop(hwnd) {
        return true;
    }
    let Some(guid) = with_desktop(|desktops| unsafe {
        let mut anchor = GetForegroundWindow();
        if anchor.is_invalid()
            || !desktops
                .IsWindowOnCurrentVirtualDesktop(anchor)
                .map(|on| on.as_bool())
                .unwrap_or(false)
        {
            anchor = CreateWindowExW(
                WS_EX_TOOLWINDOW,
                windows::core::w!("STATIC"),
                windows::core::w!(""),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                None,
                None,
            )
            .unwrap_or_default();
        }
        let id = desktops.GetWindowDesktopId(anchor).ok();
        if anchor != GetForegroundWindow() && !anchor.is_invalid() {
            let _ = DestroyWindow(anchor);
        }
        id
    })
    .flatten() else {
        return false;
    };
    let moved = with_desktop(|desktops| unsafe { desktops.MoveWindowToDesktop(hwnd, &guid) });
    let accepted = moved.map(|result| result.is_ok()).unwrap_or(false);
    // The call is a request; the desktop manager is the only one that can say
    // the window landed. A move that reports success but leaves the window
    // cloaked on the old desktop is exactly the case the caller has to know
    // about before it tries to raise something nobody can see.
    accepted && on_current_desktop(hwnd)
}

/// The rectangle DWM actually draws, without the invisible resize border.
/// Falls back to `GetWindowRect` for windows DWM does not manage.
pub fn visible_rect(hwnd: HWND) -> Option<Rect> {
    unsafe {
        let mut frame = RECT::default();
        let got = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut frame as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
        .is_ok();
        if got {
            let r = Rect {
                left: frame.left,
                top: frame.top,
                right: frame.right,
                bottom: frame.bottom,
            };
            if !r.is_empty() {
                return Some(r);
            }
        }
        rect_of(hwnd)
    }
}

pub fn rect_of(hwnd: HWND) -> Option<Rect> {
    unsafe {
        let mut rect = RECT::default();
        GetWindowRect(hwnd, &mut rect).ok()?;
        Some(Rect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        })
    }
}

/// Why the wheel cannot reach a window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    Gone,
    Ours,
    Hidden,
    NoTitle,
    ToolWindow,
    Cloaked,
    Minimized,
    /// UIPI: an unelevated process cannot raise an elevated window.
    Elevated,
    TooSmall,
}

pub fn check(hwnd: HWND, self_pid: u32) -> Result<WindowInfo, Rejection> {
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(Rejection::Gone);
    }
    let pid = pid_of(hwnd);
    if self_pid != 0 && pid == self_pid {
        return Err(Rejection::Ours);
    }
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return Err(Rejection::Hidden);
    }
    if is_cloaked(hwnd) && on_current_desktop(hwnd) {
        return Err(Rejection::Cloaked);
    }
    let here = on_current_desktop(hwnd);
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let is_tool = ex & WS_EX_TOOLWINDOW.0 != 0;
    let is_app = ex & WS_EX_APPWINDOW.0 != 0;
    if is_tool && !is_app {
        return Err(Rejection::ToolWindow);
    }
    let class = class_name(hwnd);
    if IGNORED_CLASSES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&class))
    {
        return Err(Rejection::ToolWindow);
    }
    let title = title(hwnd);
    if title.trim().is_empty() {
        return Err(Rejection::NoTitle);
    }
    if unsafe { IsIconic(hwnd).as_bool() } {
        return Err(Rejection::Minimized);
    }
    let Some(rect) = visible_rect(hwnd) else {
        return Err(Rejection::Gone);
    };
    if rect.width() < 120 || rect.height() < 80 {
        return Err(Rejection::TooSmall);
    }
    // UIPI keeps us from raising a higher-integrity window, but that is a
    // limitation of *this* process, not a reason to hide the window. Task
    // Manager and anything launched with "Run as administrator" belong on the
    // wheel; the dial says so and refuses the switch instead.
    let elevated = process_elevated(pid) == Some(true);
    let accessible = self_elevated() || !elevated;
    let exe_path = process_path(pid).unwrap_or_default();
    let exe = std::path::Path::new(&exe_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| class.to_lowercase());
    Ok(WindowInfo {
        hwnd: raw_of(hwnd),
        pid,
        title,
        class,
        exe,
        exe_path,
        rect,
        maximized: unsafe { IsZoomed(hwnd).as_bool() },
        process_start: process_start(pid),
        on_current_desktop: here,
        elevated,
        accessible,
    })
}

/// Every window the wheel can switch to, front to back.
///
/// One rule is relaxed compared with [`check`]: a minimized window counts. Half
/// the windows worth switching to are sitting in the taskbar. Their rectangle is
/// the icon position Windows keeps for them, so the "big enough to be a window"
/// test is skipped along with the restore.
pub fn list_switchable(self_pid: u32) -> Vec<WindowInfo> {
    let mut found: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_switchable),
            LPARAM(&mut found as *mut Vec<WindowInfo> as isize),
        );
    }
    found.retain(|w| w.pid != self_pid);
    found
}

unsafe extern "system" fn collect_switchable(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };
    if let Ok(info) = check_switchable(hwnd, 0) {
        list.push(info);
    }
    true.into()
}

static WARM_RUNNING: AtomicBool = AtomicBool::new(false);
static WARM_LAST: Mutex<Option<Instant>> = Mutex::new(None);

/// Fills the process and icon caches ahead of the dial, on a worker thread.
///
/// Reading an icon means asking another application for it, with a timeout
/// when that application is busy; doing that on the UI thread the first time
/// the dial opens is the stutter the user feels. The work is throttled to once
/// every two seconds and never runs twice at the same time, so calling it from
/// the poll costs nothing.
pub fn warm_windows(self_pid: u32) {
    if WARM_RUNNING.load(Ordering::Relaxed) {
        return;
    }
    {
        let mut last = WARM_LAST
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(at) = *last {
            if at.elapsed() < Duration::from_secs(2) {
                return;
            }
        }
        *last = Some(Instant::now());
    }
    if WARM_RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    std::thread::spawn(move || {
        let mut targets: Vec<(isize, u32)> = Vec::new();
        unsafe {
            let _ = EnumWindows(
                Some(collect_warm),
                LPARAM(&mut targets as *mut Vec<(isize, u32)> as isize),
            );
        }
        for (raw, pid) in targets {
            if pid == self_pid {
                continue;
            }
            let _ = proc_info(pid);
            let known = {
                let cache = icon_cache()
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                cache.get(&raw).is_some_and(|entry| entry.pid == pid)
            };
            if !known {
                let _ = icon_rgba(hwnd_of(raw));
            }
        }
        WARM_RUNNING.store(false, Ordering::Release);
    });
}

unsafe extern "system" fn collect_warm(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<(isize, u32)>) };
    let pid = pid_of(hwnd);
    if pid != 0 && unsafe { IsWindowVisible(hwnd).as_bool() } {
        list.push((hwnd.0 as isize, pid));
    }
    true.into()
}

/// [`check`] without its two minimize-related rules. Everything else - no title,
/// not on this desktop, a tool window, too small to be a window - is a reason
/// the wheel has no business reaching for the window either. A higher-integrity
/// window is listed, with `accessible` telling the dial what it may not do.
fn check_switchable(hwnd: HWND, self_pid: u32) -> Result<WindowInfo, Rejection> {
    let iconic = unsafe { IsIconic(hwnd).as_bool() };
    if !iconic {
        return check(hwnd, self_pid);
    }
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        return Err(Rejection::Gone);
    }
    let pid = pid_of(hwnd);
    if self_pid != 0 && pid == self_pid {
        return Err(Rejection::Ours);
    }
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return Err(Rejection::Hidden);
    }
    if is_cloaked(hwnd) && on_current_desktop(hwnd) {
        return Err(Rejection::Cloaked);
    }
    let here = on_current_desktop(hwnd);
    let ex = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let is_tool = ex & WS_EX_TOOLWINDOW.0 != 0;
    let is_app = ex & WS_EX_APPWINDOW.0 != 0;
    if is_tool && !is_app {
        return Err(Rejection::ToolWindow);
    }
    let class = class_name(hwnd);
    if IGNORED_CLASSES
        .iter()
        .any(|c| c.eq_ignore_ascii_case(&class))
    {
        return Err(Rejection::ToolWindow);
    }
    let title = title(hwnd);
    if title.trim().is_empty() {
        return Err(Rejection::NoTitle);
    }
    let elevated = process_elevated(pid) == Some(true);
    let accessible = self_elevated() || !elevated;
    let exe_path = process_path(pid).unwrap_or_default();
    let exe = std::path::Path::new(&exe_path)
        .file_name()
        .map(|f| f.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| class.to_lowercase());
    Ok(WindowInfo {
        hwnd: raw_of(hwnd),
        pid,
        title,
        class,
        exe,
        exe_path,
        // A minimized window reports the corner Windows keeps its icon in, not
        // the place it will have when it comes back. Nothing here needs that.
        rect: rect_of(hwnd).unwrap_or(Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        }),
        maximized: unsafe { IsZoomed(hwnd).as_bool() },
        process_start: process_start(pid),
        on_current_desktop: here,
        elevated,
        accessible,
    })
}

pub fn foreground() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

/// Gives a window the foreground, whatever is holding it now. Windows only lets
/// the foreground process steal focus, so the plain call is retried with the two
/// input queues joined to ours.
pub fn force_foreground(hwnd: HWND) {
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = SetForegroundWindow(hwnd);
        if GetForegroundWindow() == hwnd {
            return;
        }
        // Attaching threads serialises the input queues for as long as it lasts.
        // Both ends are needed: joining the foreground thread is not enough for
        // Windows to accept the switch any more, but with the target thread in
        // the same queue the activation goes through. Worth it to get the focus
        // back, not worth doing when the plain call was going to work.
        let fg = GetForegroundWindow();
        let our_thread = GetCurrentThreadId();
        let fg_thread = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let target_thread = GetWindowThreadProcessId(hwnd, None);
        let attach_fg = fg_thread != 0
            && fg_thread != our_thread
            && AttachThreadInput(our_thread, fg_thread, true).as_bool();
        let attach_target = target_thread != 0
            && target_thread != our_thread
            && target_thread != fg_thread
            && AttachThreadInput(our_thread, target_thread, true).as_bool();
        let _ = SetActiveWindow(hwnd);
        let _ = SetForegroundWindow(hwnd);
        if attach_target {
            let _ = AttachThreadInput(our_thread, target_thread, false);
        }
        if attach_fg {
            let _ = AttachThreadInput(our_thread, fg_thread, false);
        }
    }
}

/// Puts a window that is already on the desktop at the top of the stack and
/// gives it the focus, leaving it exactly where the user left it.
///
/// Used when the wheel picks a window that is already on the desktop: the only
/// thing wrong with it is that something is on top of it. Only the z-order of
/// the normal layer moves, so the dial stays above it while the window comes up.
pub fn raise(hwnd: HWND) {
    lift(hwnd);
    force_foreground(hwnd);
}

/// What one activation attempt did, in the terms the wheel writes to its log.
///
/// A pick that "succeeded" while the window stayed minimized, on another virtual
/// desktop, behind the foreground lock, or over our own layers is the failure the
/// user cannot see and cannot explain. Every step of the attempt is reflected here,
/// so the one line the caller logs says which one was refused.
#[derive(Clone, Copy, Debug)]
pub struct Activation {
    /// The foreground window when the attempt began.
    pub from: isize,
    /// The foreground window when the attempt ended.
    pub to: isize,
    /// The step that produced the switch: `front` (it already had it), `restore`,
    /// `plain`, `attach`, `switch`, `cloaked` or `refused`.
    pub method: &'static str,
    /// Whether the target ended up visible, restored and in front.
    pub foreground: bool,
    /// Whether the target is still hidden by the compositor after the move to this
    /// desktop was tried: a window on another virtual desktop, or a suspended one.
    pub cloaked: bool,
    /// Whether the target process runs at a higher integrity level than us, which
    /// is the one refusal no amount of retrying can get past (UIPI).
    pub elevated: bool,
}

/// Gives a window the foreground and reports exactly how that went.
///
/// `SetForegroundWindow` alone is refused whenever the caller is not the foreground
/// process, which is the normal case here: the wheel is a topmost layer that never
/// takes the foreground, so the click that released it went to the window under the
/// pointer, not to us. The steps are the standard ones, cheapest first:
///
/// 1. a plain `SetForegroundWindow`;
/// 2. the window moved to the top of the normal layer, our input queue joined to
///    both the foreground thread and the target thread, then `BringWindowToTop`
///    and `SetForegroundWindow` again;
/// 3. `SwitchToThisWindow`, which is the path Alt+Tab uses and the last thing that
///    works when the foreground lock keeps refusing the ordinary calls.
///
/// A minimized window is restored before any of that. A cloaked window is carried
/// to the current virtual desktop first; if it stays cloaked there is nothing to
/// raise, and saying so is more useful than pretending the switch failed.
pub fn activate(hwnd: HWND) -> Activation {
    unsafe {
        let from = raw_of(GetForegroundWindow());
        let elevated = process_elevated(pid_of(hwnd)) == Some(true) && !self_elevated();
        let mut cloaked = is_cloaked(hwnd);
        let arrived = || {
            GetForegroundWindow() == hwnd
                && IsWindowVisible(hwnd).as_bool()
                && !IsIconic(hwnd).as_bool()
        };
        let mut method = "refused";
        if arrived() {
            method = "front";
        } else {
            if cloaked && move_to_current_desktop(hwnd) {
                cloaked = is_cloaked(hwnd);
            }
            if cloaked {
                method = "cloaked";
            } else {
                if IsIconic(hwnd).as_bool() {
                    // A minimized window cannot take the foreground at all; the
                    // restore is also an activation, so it may be the whole answer.
                    let _ = ShowWindow(hwnd, SW_RESTORE);
                    if arrived() {
                        method = "restore";
                    }
                }
                if !arrived() {
                    let _ = SetForegroundWindow(hwnd);
                    if arrived() {
                        method = "plain";
                    }
                }
                if !arrived() {
                    // Ask for the top of the normal layer first; the call that
                    // activates is the one below it, once the queues are joined.
                    let _ = SetWindowPos(
                        hwnd,
                        Some(HWND_TOP),
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                    );
                    let fg = GetForegroundWindow();
                    let our_thread = GetCurrentThreadId();
                    let fg_thread = if fg.0.is_null() {
                        0
                    } else {
                        GetWindowThreadProcessId(fg, None)
                    };
                    let target_thread = GetWindowThreadProcessId(hwnd, None);
                    let attach_fg = fg_thread != 0
                        && fg_thread != our_thread
                        && AttachThreadInput(our_thread, fg_thread, true).as_bool();
                    let attach_target = target_thread != 0
                        && target_thread != our_thread
                        && target_thread != fg_thread
                        && AttachThreadInput(our_thread, target_thread, true).as_bool();
                    let _ = BringWindowToTop(hwnd);
                    let _ = SetActiveWindow(hwnd);
                    let _ = SetForegroundWindow(hwnd);
                    if attach_target {
                        let _ = AttachThreadInput(our_thread, target_thread, false);
                    }
                    if attach_fg {
                        let _ = AttachThreadInput(our_thread, fg_thread, false);
                    }
                    if arrived() {
                        method = "attach";
                    }
                }
                if !arrived() {
                    SwitchToThisWindow(hwnd, true);
                    if arrived() {
                        method = "switch";
                    }
                }
            }
        }
        Activation {
            from,
            to: raw_of(GetForegroundWindow()),
            method,
            foreground: arrived(),
            cloaked,
            elevated,
        }
    }
}

/// Moves a window to the front of the normal layer and asks for nothing else.
///
/// This is [`raise`] without the focus, which is what a preview needs: the wheel
/// lifts the window the pointer is resting on so the user can read its contents
/// through the dim, and a window that took the foreground on the way would be a
/// switch the user never released a button for. The caret, the audio and the
/// clipboard all stay with the window the gesture started from until the pick.
///
/// `HWND_TOP` is the position asked for, but Windows reserves the right to move a
/// window in the stack for the process that holds the foreground, and refuses
/// everyone else without saying so: measured in this project's headless test
/// session, the call answers "ok" and the window stays behind the one covering it.
/// The same slot is granted to anyone who asks for it relative to a window, so the
/// fallback asks for the slot under the neighbour above the window that is in the way -
/// which is the slot directly over it. One window moves: the preview. Pushing the other
/// window down instead is a call its own process gets to answer, and measured here it
/// does - a folder window stayed over a preview whose log said it had passed it.
///
/// "The front one" is the front one that paints. Measured here, `HWND_TOP` left a
/// window under a hidden popup site and a minimized explorer - both hold a place in
/// the stack and cover nothing - so the anchor is the first window that can actually
/// be in the way. See [`covering_front`].
pub fn lift(hwnd: HWND) -> Lifted {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        if topmost(hwnd) {
            return Lifted::default();
        }
        let Some(front) = covering_front(hwnd) else {
            return Lifted::default();
        };
        if front == hwnd {
            return Lifted::default();
        }
        let above = GetWindow(front, GW_HWNDPREV).unwrap_or_default();
        if above.0.is_null() {
            // Nothing is over the window in the way, so `HWND_TOP` above was the whole
            // ask and there is no neighbour to be put under.
            return Lifted {
                passed: Some(raw_of(front)),
                moved: true,
                reached: is_above(hwnd, front),
            };
        }
        let moved = SetWindowPos(
            hwnd,
            Some(above),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        // That ask can land the window in the pinned layer, because the neighbour
        // above the front painting window is sometimes a pinned one. One demote puts
        // it at the front of the normal layer - the slot the lift was asking for -
        // and keeps it there without the preview being pinned for the rest of the
        // session.
        if topmost(hwnd) {
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Lifted {
            passed: Some(raw_of(front)),
            moved: moved.is_ok(),
            reached: is_above(hwnd, front),
        }
    }
}

/// What [`lift`] asked for and what Windows gave: the wheel says all of it in its log,
/// because a preview that stayed one slot behind the window it was meant to pass is
/// indistinguishable from a lift that never happened by looking at the screen.
#[derive(Clone, Copy, Default)]
pub struct Lifted {
    /// The front window of the painting layer this lift aimed past. `None` when the
    /// window went to the top on the first call, or when there was nothing to pass.
    pub passed: Option<isize>,
    /// Whether the call that asks for the slot over it was granted. A refusal leaves
    /// the preview under the window it was supposed to cover, which is the failure a
    /// user reports as "the wheel did not bring it up".
    pub moved: bool,
    /// Whether the preview is in fact over that window as the call returns. `moved`
    /// without `reached` is the interesting one: Windows took the order and the
    /// window's own process changed its mind afterwards.
    pub reached: bool,
}

/// Puts a window directly below `above` in the stack, leaving the rest of it alone.
///
/// Only the two named windows move, which is what makes it safe for undoing a
/// preview: the wheel knows which window was over the one it lifted, and putting
/// it back under that one puts it back where the user had left it.
pub fn place_below(hwnd: HWND, above: HWND) {
    unsafe {
        if topmost(above) {
            // A pinned neighbour would pull the window into the pinned layer and it
            // would stay there after the wheel closed. The front of the normal layer
            // is where a window that sat under that neighbour came from.
            let _ = SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            Some(above),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// Whether the window is in the taskbar rather than on the desktop.
pub fn iconic(hwnd: HWND) -> bool {
    unsafe { IsIconic(hwnd).as_bool() }
}

/// Whether this handle is still a window. The desktop changes while the user holds
/// the wheel open - an application can close - and a z-order restored against a
/// handle that has gone is a call Windows refuses without saying so.
pub fn exists(hwnd: HWND) -> bool {
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

/// Every top-level window, front first, minus this process's own.
///
/// The whole stack and not the filtered list the dial is seated from: what a
/// preview promises to give back is the window's place *among its neighbours*, and
/// a neighbour the wheel never saw is a neighbour it cannot put anything back
/// against. Our own layers are left out because they are above every window here
/// and would be an anchor for nothing.
pub fn z_order(self_pid: u32) -> Vec<isize> {
    let mut found: Vec<(isize, u32)> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_rank),
            LPARAM(&mut found as *mut Vec<(isize, u32)> as isize),
        );
    }
    found.retain(|(_, pid)| *pid != self_pid);
    found.into_iter().map(|(hwnd, _)| hwnd).collect()
}

unsafe extern "system" fn collect_rank(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<(isize, u32)>) };
    list.push((raw_of(hwnd), pid_of(hwnd)));
    true.into()
}

/// Whether the window is pinned above everything that is not pinned.
fn topmost(hwnd: HWND) -> bool {
    unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 != 0 }
}

/// Whether anything is drawn where this window stands.
///
/// The stack Windows keeps is not the stack the user sees. A minimized window holds
/// its slot, a hidden one holds it too, and a window on another virtual desktop is
/// cloaked - none of them paints, so none of them can be in the way of a preview.
fn paints(hwnd: HWND) -> bool {
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return false;
    }
    if iconic(hwnd) || is_cloaked(hwnd) {
        return false;
    }
    visible_rect(hwnd).is_some_and(|r| !r.is_empty())
}

/// The window a lift has to pass: the first in the normal layer that paints.
///
/// The front of the whole stack is usually a window that paints nothing - measured
/// on this machine, the ten slots above the front of the visible layer hold a hidden
/// popup site and a minimized explorer - and putting a previewed window in front of
/// one of those buys the covering the preview is supposed to avoid.
///
/// `for_window`'s own popups are skipped too. Windows keeps an owned window above
/// whoever owns it, so a browser's own notice is not something the preview can get
/// over; aiming past it would leave the preview under the notice and above nothing.
fn covering_front(for_window: HWND) -> Option<HWND> {
    let mut cur = unsafe { GetTopWindow(None).unwrap_or_default() };
    for _ in 0..4096 {
        if cur.0.is_null() {
            return None;
        }
        if !topmost(cur) && paints(cur) && !owned_by(cur, for_window) {
            return Some(cur);
        }
        cur = next_window(cur);
    }
    None
}

/// Whether `hwnd` is a popup belonging to `owner`. Windows keeps such a window above
/// the one it belongs to wherever the rest of the stack goes, so it is not a cover
/// that a lift can promise to pass.
fn owned_by(hwnd: HWND, owner: HWND) -> bool {
    !owner.0.is_null() && unsafe { GetWindow(hwnd, GW_OWNER) }.unwrap_or_default() == owner
}

/// The window just below this one in the z-order. Windows reports "no next
/// window" as a failure, which for a walk like this just means the end.
fn next_window(hwnd: HWND) -> HWND {
    unsafe { GetWindow(hwnd, GW_HWNDNEXT).unwrap_or_default() }
}

/// Whether `a` sits somewhere above `b` in the stack. The walk is the answer Windows
/// gives, so it counts topmost windows and everything in between: this is "would `b`
/// be the one the user sees", not "is `a` first".
pub fn is_above(a: HWND, b: HWND) -> bool {
    let mut cur = unsafe { GetWindow(b, GW_HWNDPREV) }.unwrap_or_default();
    for _ in 0..4096 {
        if cur.0.is_null() {
            return false;
        }
        if cur == a {
            return true;
        }
        cur = unsafe { GetWindow(cur, GW_HWNDPREV) }.unwrap_or_default();
    }
    false
}
