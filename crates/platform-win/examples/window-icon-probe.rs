//! Does another process's icon actually draw in *this* process?
//!
//! [`shaiwin_platform_win::window::icon_of`] hands back a handle it did not
//! create and may not destroy, which is the usual recipe - and the usual recipe
//! is also the usual place a cross-process GDI handle turns out to be a number
//! that means nothing over here. A settings row whose icon slot is empty looks
//! like a bug in the row, not like a bad handle, and this machine cannot print a
//! window to look at, so the claim has to be checked by numbers instead:
//!
//! 1. the handle answers `GetIconInfo` - it resolves in this process;
//! 2. `DrawIconEx` succeeds against an in-memory surface;
//! 3. pixels *change* - the surface is prefilled with a key colour and the count
//!    of pixels still showing it is reported, so "drew transparent nothing"
//!    cannot pass as "drew an icon".
//!
//! ```text
//! cargo run -p shaiwin-platform-win --example window-icon-probe
//! ```

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetIconInfo, GetSystemMetrics, GetWindowRect, DI_NORMAL, HICON, ICONINFO,
    SM_CXSMICON,
};

use shaiwin_platform_win::window;

const KEY: u32 = 0x00FF_00FF;

fn main() {
    let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16) as usize;
    let mut checked = 0usize;
    let mut resolving = 0usize;
    let mut drawing = 0usize;
    let mut changed = 0usize;
    let mut largest = 0usize;

    for info in window::list_switchable(std::process::id()) {
        let hwnd = window::hwnd_of(info.hwnd);
        let Some(icon) = window::icon_of(hwnd) else {
            println!("{:<28} no icon of its own", trim(&info.exe));
            continue;
        };
        checked += 1;

        // 1. Does the handle mean something in this process?
        let mut ii = ICONINFO::default();
        let resolves = unsafe { GetIconInfo(icon, &mut ii).is_ok() };
        if resolves {
            resolving += 1;
            for bitmap in [ii.hbmColor, ii.hbmMask] {
                if !bitmap.is_invalid() {
                    let _ = unsafe { DeleteObject(bitmap.into()) };
                }
            }
        }

        // 2 + 3. Draw it into a surface that starts out a colour no icon is.
        let (drew, painted) = draw(hwnd, icon, size);
        if drew {
            drawing += 1;
        }
        if painted > largest {
            largest = painted;
        }
        if painted > 0 {
            changed += 1;
        }
        let mut r = RECT::default();
        let _ = unsafe { GetWindowRect(hwnd, &mut r) };
        println!(
            "{:<28} {:>4}x{:<4} resolves={:<5} drew={:<5} pixels set={:5} of {}",
            trim(&info.exe),
            r.right - r.left,
            r.bottom - r.top,
            resolves,
            drew,
            painted,
            size * size,
        );
    }

    println!();
    println!("windows asked for:      {checked}");
    println!("handles that resolve:   {resolving}");
    println!("DrawIconEx succeeded:   {drawing}");
    println!("with pixels to show:    {changed} (busiest icon set {largest} px)");
    if checked == 0 {
        println!("nothing to look at: no switchable windows on this desktop");
    }
}

/// The same three-step dance the settings page does, against a bitmap nobody
/// sees. Returns whether `DrawIconEx` answered success and how many pixels it
/// left behind that are not the key colour.
fn draw(hwnd: HWND, icon: HICON, size: usize) -> (bool, usize) {
    unsafe {
        let screen = GetDC(Some(hwnd));
        let hdc = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size as i32,
                biHeight: -(size as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let Ok(bitmap) = CreateDIBSection(Some(hdc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            return (false, 0);
        };
        let old = SelectObject(hdc, bitmap.into());
        // A key colour no icon is drawn in, so a transparent "success" cannot
        // pass for pixels.
        let plane = std::slice::from_raw_parts_mut(bits as *mut u32, size * size);
        for px in plane.iter_mut() {
            *px = KEY;
        }
        let drew = DrawIconEx(
            hdc,
            0,
            0,
            icon,
            size as i32,
            size as i32,
            0,
            None,
            DI_NORMAL,
        )
        .is_ok();
        let painted = plane.iter().filter(|px| **px != KEY).count();
        SelectObject(hdc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(hdc);
        ReleaseDC(Some(hwnd), screen);
        (drew, painted)
    }
}

fn trim(text: &str) -> String {
    let cut = text.rfind('.').unwrap_or(text.len());
    text[..cut].chars().take(28).collect()
}
