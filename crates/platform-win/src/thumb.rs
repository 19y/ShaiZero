//! DWM thumbnails: a live picture of another window, drawn by the compositor.
//!
//! Raising a window out of the way of whatever covers it works for a window the
//! foreground process is allowed to move. It does not work for an elevated
//! window, and the target application can put itself back a moment later. A
//! thumbnail asks the compositor for the window's own surface instead, which is
//! still there when the window is covered or minimised - so the hole in the dim
//! shows the window the user is pointing at, not whatever happens to be over it.

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmRegisterThumbnail, DwmUnregisterThumbnail, DwmUpdateThumbnailProperties,
    DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY, DWM_TNP_RECTDESTINATION,
    DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
};

/// Registers `source` as a thumbnail drawn into `dest`. The returned id is what
/// `update` and `unregister` take; `None` means the compositor refused, which is
/// the case for a window on another virtual desktop or one this process may not
/// read.
pub fn register(dest: HWND, source: HWND) -> Option<isize> {
    if dest.is_invalid() || source.is_invalid() {
        return None;
    }
    unsafe { DwmRegisterThumbnail(dest, source).ok() }
}

/// Places the picture in the destination window's own coordinates and fades it
/// in. The destination rectangle is the whole client area the caller sized the
/// destination window to.
pub fn update(id: isize, width: i32, height: i32, opacity: u8) -> bool {
    let properties = DWM_THUMBNAIL_PROPERTIES {
        dwFlags: DWM_TNP_RECTDESTINATION
            | DWM_TNP_OPACITY
            | DWM_TNP_VISIBLE
            | DWM_TNP_SOURCECLIENTAREAONLY,
        rcDestination: RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        },
        rcSource: RECT::default(),
        opacity,
        fVisible: true.into(),
        // The frame is part of where the window is; showing the client area only
        // would put the picture inside a hole cut for the whole window.
        fSourceClientAreaOnly: false.into(),
    };
    unsafe { DwmUpdateThumbnailProperties(id, &properties).is_ok() }
}

pub fn unregister(id: isize) {
    unsafe {
        let _ = DwmUnregisterThumbnail(id);
    }
}
