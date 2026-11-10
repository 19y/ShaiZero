//! Does the compositor draw a DWM thumbnail of a *minimized* window?
//!
//! The hover no longer restores minimized windows - that restore was the lag. The
//! preview now has to come from the thumbnail alone, and a hole with a blank
//! picture in it would look like a broken sector, so the claim in
//! [`shaiwin_platform_win::thumb`] ("still there when the window is covered or
//! minimised") has to be checked by pixels rather than trusted.
//!
//! A window of `--source` is thumbnailed into a black host window, the screen is
//! sampled through the host before and after the source is minimized, and the
//! count of the source colour tells the story.
//!
//! ```text
//! cargo run -p shaiwin-platform-win --example thumb-probe -- <hwnd>
//! ```

use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, GetDC, GetPixel, ReleaseDC, HBRUSH,
    PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use shaiwin_platform_win::thumb;

const HOST_X: i32 = 700;
const HOST_Y: i32 = 120;
const HOST_W: i32 = 400;
const HOST_H: i32 = 300;

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => {
            // Black on purpose: anything the thumbnail draws has to differ from it.
            let dc = windows::Win32::Graphics::Gdi::HDC(wp.0 as *mut core::ffi::c_void);
            let brush = CreateSolidBrush(COLORREF(0));
            let mut r = windows::Win32::Foundation::RECT::default();
            let _ = GetClientRect(hwnd, &mut r);
            windows::Win32::Graphics::Gdi::FillRect(dc, &r, brush);
            let _ = DeleteObject(brush.into());
            LRESULT(1)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let _ = BeginPaint(hwnd, &mut ps);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    while Instant::now() < until {
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            unsafe {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// How much of the host's client area is no longer the black it started as.
/// Anything lit means the compositor drew the source there.
fn sample() -> (usize, usize) {
    unsafe {
        let dc = GetDC(None);
        let mut hit = 0;
        let mut total = 0;
        let name = w!("ThumbProbeHost");
        let host = FindWindowW(name, None);
        if let Ok(host) = host {
            let mut r = windows::Win32::Foundation::RECT::default();
            let _ = GetWindowRect(host, &mut r);
            let (w, h) = (r.right - r.left, r.bottom - r.top);
            let mut y = (h as f32 * 0.13) as i32;
            while y < (h as f32 * 0.87) as i32 {
                let mut x = (w as f32 * 0.10) as i32;
                while x < (w as f32 * 0.90) as i32 {
                    let c = GetPixel(dc, r.left + x, r.top + y);
                    total += 1;
                    let cr = (c.0 & 0xFF) as u32;
                    let cg = ((c.0 >> 8) & 0xFF) as u32;
                    let cb = ((c.0 >> 16) & 0xFF) as u32;
                    if cr + cg + cb > 60 {
                        hit += 1;
                    }
                    x += (w as f32 * 0.055) as i32;
                }
                y += (h as f32 * 0.066) as i32;
            }
        }
        ReleaseDC(None, dc);
        (hit, total)
    }
}

fn main() {
    let arg = std::env::args().nth(1).expect("usage: thumb-probe <hwnd>");
    let raw = arg
        .strip_prefix("0x")
        .map(|hex| isize::from_str_radix(hex, 16).expect("hex hwnd"))
        .unwrap_or_else(|| arg.parse().expect("numeric hwnd"));
    let source = HWND(raw as *mut core::ffi::c_void);

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let instance = GetModuleHandleW(None).expect("module");
        let class = WNDCLASSW {
            lpfnWndProc: Some(host_proc),
            hInstance: instance.into(),
            lpszClassName: w!("ThumbProbeHost"),
            hbrBackground: HBRUSH::default(),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
        let host = CreateWindowExW(
            WS_EX_TOPMOST,
            w!("ThumbProbeHost"),
            w!("ThumbProbeHost"),
            WS_POPUP | WS_VISIBLE,
            HOST_X,
            HOST_Y,
            HOST_W,
            HOST_H,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .expect("host window");
        let _ = ShowWindow(host, SW_SHOW);
        pump(400);
    }

    let Some(id) = thumb::register(host(), source) else {
        println!("FAIL: the compositor refused the thumbnail outright");
        std::process::exit(1);
    };
    unsafe {
        let _ = SetWindowPos(
            host(),
            None,
            HOST_X,
            HOST_Y,
            HOST_W,
            HOST_H,
            SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOZORDER,
        );
    }
    let ok = thumb::update(id, HOST_W, HOST_H, 255);
    println!("register ok, update={ok}");
    pump(700);
    let (hit, total) = sample();
    println!("source visible:    {hit}/{total} cells lit");

    unsafe {
        let _ = ShowWindow(source, SW_MINIMIZE);
    }
    pump(900);
    let (min_hit, min_total) = sample();
    println!("source minimized:  {min_hit}/{min_total} cells lit");

    thumb::unregister(id);
    if hit * 2 < total {
        println!("INCONCLUSIVE: the control picture never arrived");
        std::process::exit(2);
    }
    if min_hit * 2 >= min_total {
        println!("PASS: a minimized window still has a thumbnail");
    } else {
        println!("FAIL: a minimized window's thumbnail is blank");
        std::process::exit(1);
    }
}

/// The host handle, looked up by class because the pump owns no state.
fn host() -> HWND {
    unsafe { FindWindowW(w!("ThumbProbeHost"), None).expect("host window") }
}
