//! Design §6.4(b), answered with numbers instead of a gesture: is `Alt` + a drag
//! on a window's border the system's own resize loop?
//!
//! The wheel swallows the trigger press before the application ever sees it. If
//! the system owns a press on the resize border, taking it would steal a resize -
//! so the border either has to be exempt, or it doesn't, and that is the question
//! this file settles.
//!
//! It is answered with synthesised input rather than a person at the keyboard
//! because both halves of that were measured first (see `hook-selftest`): the
//! pointer can be addressed from a process the agent shell starts, and injected
//! clicks really are delivered to a window. What synthesis cannot prove is what a
//! finger does on hardware this machine has no digitizer for - that stays open.
//!
//! The target window lives on its own thread on purpose: a system move or resize
//! runs a *modal loop* inside the target's message dispatch, and if that target
//! were this thread, the drag would never end and nothing else could be read.
//!
//! ```text
//! cargo run -p shaiwin-platform-win --example alt-drag-probe
//! ```

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, mouse_event, KEYEVENTF_KEYUP, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_VIRTUALDESK, VK_LMENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetAncestor, GetClientRect,
    GetCursorPos, GetGUIThreadInfo, GetSystemMetrics, GetWindowRect, PeekMessageW, PostQuitMessage,
    RegisterClassExW, SetProcessDPIAware, SetWindowPos, TranslateMessage, GA_ROOT, MSG, PM_REMOVE,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE,
    SWP_NOZORDER, WINDOW_EX_STYLE, WINDOW_STYLE, WM_CREATE, WM_DESTROY, WNDCLASSEXW,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

/// The target window's handle, published by the thread that created it.
static TARGET: AtomicUsize = AtomicUsize::new(0);
static TARGET_THREAD: AtomicU32 = AtomicU32::new(0);
static READY: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);

/// The target's frame rectangle in physical pixels, or `None` when the window is
/// gone. Every case re-reads it: the whole point of the measurement is that the
/// previous case may have moved or resized the window.
fn rect() -> Option<RECT> {
    let root = hwnd()?;
    let mut r = RECT::default();
    if unsafe { GetWindowRect(root, &mut r) }.is_ok() {
        Some(r)
    } else {
        None
    }
}

/// The root window the system actually hit-tests against, once.
fn hwnd() -> Option<HWND> {
    let raw = TARGET.load(Ordering::Acquire);
    if raw == 0 {
        return None;
    }
    let hwnd = HWND(raw as *mut _);
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    (!root.0.is_null()).then_some(root)
}

/// Put the frame back where it was. Without this, case 2 would be aimed at a
/// border that case 1 already dragged 90 px further right, and every number
/// after the first control would be measuring the client area.
fn reset_to(r: &RECT) {
    let Some(root) = hwnd() else { return };
    let w = (r.right - r.left).max(1);
    let h = (r.bottom - r.top).max(1);
    unsafe {
        let _ = SetWindowPos(
            root,
            None,
            r.left,
            r.top,
            w,
            h,
            SWP_NOACTIVATE | SWP_NOZORDER,
        );
    }
    // The frame geometry has to settle before the next point is derived from it.
    std::thread::sleep(Duration::from_millis(60));
}

/// How far the client area starts below the top of the frame — the band that
/// holds both the resize border and the caption. Pressing near its bottom is a
/// caption press; near its top is a border press, which is the whole dispute.
fn caption_band(r: &RECT) -> i32 {
    let Some(root) = hwnd() else {
        return 24;
    };
    let mut client = RECT::default();
    if unsafe { GetClientRect(root, &mut client) }.is_err() {
        return 24;
    }
    let mut pt = POINT { x: 0, y: 0 };
    if !unsafe { ClientToScreen(root, &mut pt) }.as_bool() {
        return 24;
    }
    (pt.y - r.top).max(8)
}

#[derive(Clone, Copy)]
enum Aim {
    RightBorder,
    Client,
    Caption,
}

/// Where a case presses, derived from the frame as it stands right now. The
/// right edge of the window rectangle is the grab strip; the DWM draws the
/// visible border just inside it, so two pixels in is unambiguously the edge.
fn aim(kind: Aim, r: &RECT, band: i32) -> ((i32, i32), (i32, i32)) {
    let mid_x = (r.left + r.right) / 2;
    let mid_y = (r.top + r.bottom) / 2;
    match kind {
        Aim::RightBorder => ((r.right - 2, mid_y), (r.right + 88, mid_y)),
        Aim::Client => ((mid_x + 60, mid_y + 40), (mid_x + 150, mid_y + 80)),
        Aim::Caption => {
            let y = r.top + band - 5;
            ((mid_x, y), (mid_x + 90, y + 60))
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            TARGET_THREAD.store(GetCurrentThreadId(), Ordering::Release);
            TARGET.store(hwnd.0 as usize, Ordering::Release);
            READY.store(true, Ordering::Release);
        }
        WM_DESTROY => PostQuitMessage(0),
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}

/// A normal resizable window - the only kind that has a resize border to fight
/// over. `WS_OVERLAPPEDWINDOW` carries `WS_THICKFRAME`, which is what makes the
/// edge draggable at all.
fn spawn_target() -> thread::JoinHandle<()> {
    thread::spawn(|| unsafe {
        let hmod: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let class: Vec<u16> = "ShaiWinAltDragProbe\0".encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hmod,
            lpszClassName: windows::core::PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
        let title: Vec<u16> = "alt drag probe target\0".encode_utf16().collect();
        let style = WINDOW_STYLE(WS_OVERLAPPEDWINDOW.0 | WS_VISIBLE.0);
        let created = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            windows::core::PCWSTR(class.as_ptr()),
            windows::core::PCWSTR(title.as_ptr()),
            style,
            120,
            120,
            520,
            360,
            None,
            None,
            Some(hmod),
            None,
        );
        let Ok(hwnd) = created else {
            READY.store(false, Ordering::Release);
            return;
        };
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        while !QUIT.load(Ordering::Acquire) {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(2));
        }
        let _ = DestroyWindow(hwnd);
    })
}

fn warp(x: i32, y: i32) {
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1);
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1);
    let nx = (((x - left) as f64 * 65535.0) / width as f64).round() as i32;
    let ny = (((y - top) as f64 * 65535.0) / height as f64).round() as i32;
    unsafe {
        mouse_event(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            nx,
            ny,
            0,
            0,
        )
    };
}

/// The state Windows reports for the target's thread while the drag is running -
/// `GUI_INMOVESIZE` is the system saying "I own this gesture right now".
fn loop_state(thread_id: u32) -> (&'static str, bool) {
    let mut info = windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO {
        cbSize: std::mem::size_of::<windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO>()
            as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(thread_id, &mut info) }.is_err() {
        return ("no-gui-info", false);
    }
    let active = info
        .flags
        .contains(windows::Win32::UI::WindowsAndMessaging::GUI_INMOVESIZE);
    (if active { "in-move-size" } else { "idle" }, active)
}

/// Drag from one point to another, optionally holding `Alt`, and report what the
/// window did while the button was down.
fn drag(from: (i32, i32), to: (i32, i32), alt: bool) -> DragOutcome {
    let Some(before) = rect() else {
        return DragOutcome::default();
    };
    let thread_id = TARGET_THREAD.load(Ordering::Acquire);
    let mut outcome = DragOutcome {
        loop_name: "idle",
        ..Default::default()
    };
    if alt {
        unsafe { keybd_event(VK_LMENU.0 as u8, 0, Default::default(), 0) };
        std::thread::sleep(Duration::from_millis(30));
    }
    warp(from.0, from.1);
    std::thread::sleep(Duration::from_millis(30));
    unsafe { mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0) };
    // The modal loop runs on the target thread; this thread watches it from
    // outside and pushes the pointer along in steps.
    let until = Instant::now() + Duration::from_millis(600);
    let steps = 12;
    for i in 1..=steps {
        let t = i as f64 / steps as f64;
        warp(
            (from.0 as f64 + (to.0 - from.0) as f64 * t) as i32,
            (from.1 as f64 + (to.1 - from.1) as f64 * t) as i32,
        );
        let (name, active) = loop_state(thread_id);
        if active && !outcome.saw_loop {
            outcome.saw_loop = true;
            outcome.loop_name = name;
        }
        std::thread::sleep(Duration::from_millis(40));
        if Instant::now() > until {
            break;
        }
    }
    unsafe { mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0) };
    if alt {
        unsafe { keybd_event(VK_LMENU.0 as u8, 0, KEYEVENTF_KEYUP, 0) };
    }
    std::thread::sleep(Duration::from_millis(80));
    let after = rect().unwrap_or(before);
    outcome.dx = after.left - before.left;
    outcome.dy = after.top - before.top;
    outcome.dw = (after.right - after.left) - (before.right - before.left);
    outcome.dh = (after.bottom - after.top) - (before.bottom - before.top);
    outcome
}

#[derive(Default, Debug)]
struct DragOutcome {
    dx: i32,
    dy: i32,
    dw: i32,
    dh: i32,
    saw_loop: bool,
    loop_name: &'static str,
}

impl DragOutcome {
    /// A resize changed width or height; a move changed position. Neither is
    /// what the wheel wants to be sitting on top of.
    fn describe(&self) -> String {
        format!(
            "loop={} dx={} dy={} dw={} dh={}",
            self.loop_name, self.dx, self.dy, self.dw, self.dh
        )
    }
}

fn main() {
    // Physical pixels everywhere, or the border this file aims at is not the
    // border the system hit-tests. The real app gets this from its manifest.
    let _ = unsafe { SetProcessDPIAware() };
    let worker = spawn_target();
    for _ in 0..100 {
        if READY.load(Ordering::Acquire) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    if !READY.load(Ordering::Acquire) {
        println!("no target window could be created here, so the measurement is impossible");
        QUIT.store(true, Ordering::Release);
        let _ = worker.join();
        return;
    }
    let home = rect().unwrap_or_default();
    let mut cursor = POINT::default();
    let had_cursor = unsafe { GetCursorPos(&mut cursor) }.is_ok();
    println!(
        "target rect {} {} {} {}",
        home.left, home.top, home.right, home.bottom
    );
    println!("(a blank resizable window will appear for a few seconds and be destroyed)");
    std::thread::sleep(Duration::from_millis(200));
    let band = caption_band(&home);
    println!("caption band: the first {band} px below the top of the frame");

    let cases: [(&str, Aim, bool); 5] = [
        ("border, no Alt (control)", Aim::RightBorder, false),
        ("border, WITH Alt", Aim::RightBorder, true),
        ("client, WITH Alt", Aim::Client, true),
        ("caption, no Alt (control)", Aim::Caption, false),
        ("caption, WITH Alt", Aim::Caption, true),
    ];

    let mut results = Vec::new();
    for (label, kind, alt) in cases {
        // Every case starts from the same frame, and every point is derived from
        // the rect as it exists *after* that reset. Case 1 resizes the window by
        // 90 px; aiming case 2 at the old right edge would have hit the client.
        reset_to(&home);
        let r = rect().unwrap_or(home);
        let (from, to) = aim(kind, &r, band);
        let o = drag(from, to, alt);
        println!("{label:>30}: pressed at {from:?} {}", o.describe());
        results.push((label, o));
        std::thread::sleep(Duration::from_millis(150));
    }
    reset_to(&home);

    println!("--- verdict ---");
    let control = &results[0].1;
    let with_alt = &results[1].1;
    let resized = |o: &DragOutcome| o.dw.abs() > 2 || o.dh.abs() > 2;
    if !resized(control) {
        println!(
            "RESULT: a plain border drag did NOT resize the window ({}) - injected input cannot start the system resize loop here, so §6.4(b) stays unmeasured and needs a real hand.",
            control.describe()
        );
    } else {
        println!(
            "a plain border drag resized the window ({}), so the resize loop is reachable from injected input - this measurement is valid.",
            control.describe()
        );
        println!(
            "Alt + the same border drag: {} -> {}",
            with_alt.describe(),
            if resized(with_alt) && with_alt.saw_loop {
                "STILL the system's resize. The wheel must keep letting border presses through."
            } else if with_alt.saw_loop {
                "the loop started but the size did not change - Alt moved the window instead of resizing it. Border is still the system's."
            } else {
                "no loop at all - Alt disabled that drag. Taking a border press would only cost the user a resize they did not ask for."
            }
        );
        let client_alt = &results[2].1;
        println!(
            "Alt + client drag: {} -> {}",
            client_alt.describe(),
            if client_alt.dx.abs() <= 2 && client_alt.dy.abs() <= 2 && !client_alt.saw_loop {
                "the window did not move, so Alt+client is not a system move and the wheel can take it."
            } else {
                "something moved - the client area is not free to take either."
            }
        );
        let caption_alt = &results[4].1;
        let moved = |o: &DragOutcome| o.dx.abs() > 2 || o.dy.abs() > 2;
        println!(
            "Alt + caption drag: {} (control without Alt: {}) -> {}",
            caption_alt.describe(),
            results[3].1.describe(),
            if caption_alt.saw_loop {
                "the system owns the caption drag with Alt held too, exactly as it does without it. So Alt is not reserved by the frame; the only thing it costs is a caption move, and the wheel's exemption has to be geometric (the border), not 'anywhere the frame is'."
            } else if moved(caption_alt) {
                "it moved without the system loop - the app did that, not Windows."
            } else {
                "Alt cancelled the caption drag that works without it."
            }
        );
        println!(
            "RESULT: {}",
            if with_alt.saw_loop {
                "measured - Alt + a border drag is still the system's move/size loop, so the wheel's border exemption is load-bearing, not defensive."
            } else {
                "measured - Alt does not suppress the border drag, and the border is the system's either way."
            }
        );
    }

    QUIT.store(true, Ordering::Release);
    let _ = worker.join();
    if had_cursor {
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::SetCursorPos(cursor.x, cursor.y);
        }
    }
}
