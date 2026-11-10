//! Monitor geometry and DPI.
//!
//! Everything the overlay does is per-monitor: the dial opens on the display
//! under the pointer, at that display's DPI, and never straddles two screens.

use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, POINT, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromPoint, MonitorFromRect, HDC, HMONITOR,
    MONITORINFO, MONITOR_DEFAULTTONEAREST, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

use crate::Rect;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    pub handle: isize,
    /// Full bounds, in virtual-screen device pixels.
    pub bounds: Rect,
    /// Bounds minus the taskbar and other appbars.
    pub work: Rect,
    /// DPI factor: 1.0, 1.25, 1.5, 1.75, 2.0...
    pub scale: f32,
    pub primary: bool,
}

impl Monitor {
    pub fn width(&self) -> i32 {
        self.bounds.width()
    }

    pub fn height(&self) -> i32 {
        self.bounds.height()
    }
}

pub fn monitors() -> Vec<Monitor> {
    let mut found: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM(&mut found as *mut Vec<Monitor> as isize),
        );
    }
    if found.is_empty() {
        // A headless session (some CI machines) still deserves a sane fallback.
        found.push(Monitor {
            handle: 0,
            bounds: Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
            work: Rect {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1040,
            },
            scale: 1.0,
            primary: true,
        });
    }
    found.sort_by_key(|m| (!m.primary, m.bounds.left));
    found
}

unsafe extern "system" fn collect(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<Monitor>) };
    if let Some(monitor) = unsafe { read(hmon) } {
        list.push(monitor);
    }
    true.into()
}

/// One monitor from its handle, without enumerating every display.
unsafe fn read(hmon: HMONITOR) -> Option<Monitor> {
    unsafe {
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !GetMonitorInfoW(hmon, &mut info).as_bool() {
            return None;
        }
        let mut sx = 96u32;
        let mut sy = 96u32;
        let scale = match GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut sx, &mut sy) {
            Ok(()) if sx > 0 => sx as f32 / 96.0,
            _ => 1.0,
        };
        Some(Monitor {
            handle: hmon.0 as isize,
            bounds: Rect {
                left: info.rcMonitor.left,
                top: info.rcMonitor.top,
                right: info.rcMonitor.right,
                bottom: info.rcMonitor.bottom,
            },
            work: Rect {
                left: info.rcWork.left,
                top: info.rcWork.top,
                right: info.rcWork.right,
                bottom: info.rcWork.bottom,
            },
            scale,
            primary: info.dwFlags & 1 != 0,
        })
    }
}

/// The monitor containing a screen point.
pub fn at_point(x: i32, y: i32) -> Monitor {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        // Asking the handle it named is enough: enumerating every display and
        // its DPI to find the same one again is a cost paid every time the
        // dial opens.
        read(hmon).unwrap_or_else(|| monitors()[0])
    }
}

/// The monitor a window is mostly on.
pub fn for_window(hwnd: isize) -> Monitor {
    unsafe {
        let hwnd = crate::window::hwnd_of(hwnd);
        let mut rect = RECT::default();
        if windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut rect).is_err() {
            return primary();
        }
        let hmon = MonitorFromRect(&rect, MONITOR_DEFAULTTONEAREST);
        read(hmon).unwrap_or_else(primary)
    }
}

/// The monitor a rectangle is mostly on. The wheel's highlight uses the DWM
/// extended frame bounds rather than `GetWindowRect`, so asking about the exact
/// rectangle it is about to draw keeps the dim and the outline on one screen
/// even for a window that straddles two.
pub fn for_rect(rect: Rect) -> Monitor {
    unsafe {
        let raw = RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        let hmon = MonitorFromRect(&raw, MONITOR_DEFAULTTONEAREST);
        read(hmon).unwrap_or_else(primary)
    }
}

pub fn primary() -> Monitor {
    unsafe {
        let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        read(hmon).unwrap_or_else(|| monitors()[0])
    }
}

pub fn cursor() -> (i32, i32) {
    let mut p = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut p);
    }
    (p.x, p.y)
}

/// Where a frame of `width`×`height` goes so that it sits in the middle of
/// `work`. Horizontally it is centred even when the frame is wider than the
/// work area, because an application that insists on being wider than a screen
/// still belongs in the middle of it. Vertically a frame taller than the work
/// area is pinned to `work.top`: a title bar above the top of the screen
/// cannot be grabbed, and a window nobody can move is worse than one that
/// overflows at the bottom.
pub fn centre_frame(width: i32, height: i32, work: Rect) -> (i32, i32) {
    let x = work.left + (work.width() - width) / 2;
    let y = if height >= work.height() {
        work.top
    } else {
        work.top + (work.height() - height) / 2
    };
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::centre_frame;
    use crate::Rect;

    fn work(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn a_frame_smaller_than_the_screen_sits_in_the_middle() {
        let area = work(0, 0, 1920, 1040);
        // 800 wide leaves 1120 px, 600 high leaves 440 px: half of each is
        // 560 and 220, and that is what "centred" has to mean on this screen.
        assert_eq!(centre_frame(800, 600, area), (560, 220));
    }

    #[test]
    fn a_second_screen_is_centred_in_its_own_coordinates() {
        let area = work(1920, 0, 3840, 1040);
        // The same arithmetic, 1920 px to the right: the frame has to land on
        // the second screen, not back on the first one.
        assert_eq!(centre_frame(800, 600, area), (2480, 220));
    }

    #[test]
    fn a_frame_wider_than_the_work_area_overflows_but_stays_centred() {
        let area = work(0, 0, 1920, 1040);
        // 2400 wide cannot fit; halves still overlap evenly, so x is negative.
        assert_eq!(centre_frame(2400, 600, area), (-240, 220));
    }

    #[test]
    fn a_frame_taller_than_the_work_area_keeps_its_title_bar_reachable() {
        let area = work(0, 0, 1920, 1040);
        // Pinned to the top of the work area: the header stays on screen and
        // the bottom is what hangs off.
        assert_eq!(centre_frame(800, 1200, area), (560, 0));
        let shifted = work(0, 40, 1920, 1080);
        assert_eq!(centre_frame(800, 1200, shifted), (560, 40));
    }

    #[test]
    fn a_frame_exactly_the_work_area_lands_on_its_origin() {
        let area = work(1920, 24, 3840, 1064);
        assert_eq!(centre_frame(1920, 1040, area), (1920, 24));
    }
}

