//! The placement of a window, and the quiet way it moves between the desktop and
//! the taskbar.
//!
//! A window on the dial that is minimized has no rectangle to cut a hole in: the
//! preview would be the dim claiming something the user cannot check. So the dial
//! puts it on the desktop for as long as the pointer rests on its sector, and puts
//! it back on the taskbar when the pointer leaves.
//!
//! `WINDOWPLACEMENT` is the whole trick, because it is the record Windows itself
//! uses for a taskbar button. [`Placement::capture`] reads it and normalizes the
//! show state - "minimized" becomes what the window looked like before it was
//! minimized, and a window that was maximized keeps `WPF_RESTORETOMAXIMIZED` - so
//! applying the record brings the window back at its own size and position in one
//! call. Applying that same record with "minimized" written back in is the one way
//! to minimize a window that does not hand the focus to whatever is next in the
//! stack.
//!
//! Neither call is quiet about the focus by itself: measured 2026-11-06 with
//! `examples/zorder-selftest`, applying the record that brings a taskbar window
//! onto the desktop activates it, and the focus stays there after the window goes
//! back. Both halves therefore hand the foreground to whoever held it when the
//! call started - see [`keep_foreground`].

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetWindowPlacement, IsIconic, IsWindow, SetWindowPlacement, ShowWindow,
    SW_SHOWMINNOACTIVE, WINDOWPLACEMENT, WINDOWPLACEMENT_FLAGS,
};

use crate::Rect;

/// The numbers a window's record needs to come back exactly as it was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    pub flags: u32,
    pub show_cmd: u32,
    pub min_x: i32,
    pub min_y: i32,
    pub max_x: i32,
    pub max_y: i32,
    pub normal: Rect,
}

/// `WINDOWPLACEMENT.showCmd` values this module cares about.
const SW_SHOWMINIMIZED_CMD: u32 = 2;
const SW_SHOWMAXIMIZED_CMD: u32 = 3;
/// `WPF_RESTORETOMAXIMIZED`: a minimized window that was maximized before.
const WPF_RESTORETOMAXIMIZED: u32 = 0x0002;

/// The show state a window should come back in. A minimized window's `showCmd` is
/// "minimized", and applying that on the way out leaves the window minimized -
/// seen by the user as "that sector did nothing". What the record must keep is how
/// the window looked before it was minimized.
fn visible_show_cmd(flags: u32, show_cmd: u32) -> u32 {
    if show_cmd == SW_SHOWMINIMIZED_CMD {
        if flags & WPF_RESTORETOMAXIMIZED != 0 {
            SW_SHOWMAXIMIZED_CMD
        } else {
            1 // SW_SHOWNORMAL
        }
    } else {
        show_cmd
    }
}

impl Placement {
    pub fn capture(hwnd: HWND) -> Option<Placement> {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return None;
            }
            let mut wp = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                ..Default::default()
            };
            if GetWindowPlacement(hwnd, &mut wp).is_err() {
                return None;
            }
            Some(Placement {
                flags: wp.flags.0,
                show_cmd: visible_show_cmd(wp.flags.0, wp.showCmd),
                min_x: wp.ptMinPosition.x,
                min_y: wp.ptMinPosition.y,
                max_x: wp.ptMaxPosition.x,
                max_y: wp.ptMaxPosition.y,
                normal: Rect {
                    left: wp.rcNormalPosition.left,
                    top: wp.rcNormalPosition.top,
                    right: wp.rcNormalPosition.right,
                    bottom: wp.rcNormalPosition.bottom,
                },
            })
        }
    }

    /// Restores size, position and the maximized/minimized state in one call.
    pub fn apply(&self, hwnd: HWND) -> bool {
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return false;
            }
            let wp = WINDOWPLACEMENT {
                length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
                flags: WINDOWPLACEMENT_FLAGS(self.flags),
                showCmd: self.show_cmd,
                ptMinPosition: POINT {
                    x: self.min_x,
                    y: self.min_y,
                },
                ptMaxPosition: POINT {
                    x: self.max_x,
                    y: self.max_y,
                },
                rcNormalPosition: RECT {
                    left: self.normal.left,
                    top: self.normal.top,
                    right: self.normal.right,
                    bottom: self.normal.bottom,
                },
            };
            SetWindowPlacement(hwnd, &wp).is_ok()
        }
    }
}

/// Brings a minimized window onto the desktop without asking for the focus.
///
/// `ShowWindow(SW_RESTORE)` would do it, but it also activates, and a preview that
/// switches the window the user never released a button for is a different feature.
/// Applying the window's own placement - whose show state [`Placement::capture`]
/// has already normalized from "minimized" to what it was before - restores size,
/// position and the maximized state in one call. That call does activate the
/// window, so the focus is handed back afterwards: the window the user was in when
/// the preview started is the window they are still in when it ends.
///
/// The placement as it was while minimized comes back with the window, because the
/// only quiet way to put it back is to apply that same record with "minimized"
/// written into it again: see [`minimize_quietly`]. `None` when the window was on
/// the desktop already, or when it would not come.
pub fn show_quietly(hwnd: HWND) -> Option<Placement> {
    unsafe {
        if !IsIconic(hwnd).as_bool() {
            return None;
        }
        let placement = Placement::capture(hwnd)?;
        let held = foreground();
        if placement.apply(hwnd) && !IsIconic(hwnd).as_bool() {
            keep_foreground(held);
            Some(placement)
        } else {
            None
        }
    }
}

/// The window that holds the focus right now, or `None` in the sessions where
/// nothing does - a desktop with no foreground window has no focus to give back,
/// and asking for one there would be the dial inventing a focus the user never had.
fn foreground() -> Option<HWND> {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        None
    } else {
        Some(fg)
    }
}

/// Puts the focus back on the window that held it, for the cases where the call in
/// between moved it. Does nothing when the focus is already where it was, when the
/// window that held it has gone away, or when it is back on the taskbar - the last
/// one because a minimized window cannot hold the focus, and forcing it there
/// would bring it onto the desktop behind the dial's back.
fn keep_foreground(held: Option<HWND>) {
    unsafe {
        let Some(held) = held else { return };
        if GetForegroundWindow() == held {
            return;
        }
        if !IsWindow(Some(held)).as_bool() || IsIconic(held).as_bool() {
            return;
        }
        crate::window::force_foreground(held);
    }
}

/// The other half: the window a preview restored goes back to the taskbar.
///
/// The plain `SW_MINIMIZE` would hand the foreground to whatever is next in the
/// stack, which is the same switch in the other direction. Only when the placement
/// is refused - a window that ignores the show state - does the fallback run, and
/// even then the window ends up where the user left it. The focus goes back to the
/// window that held it when the preview started, for the same reason.
pub fn minimize_quietly(hwnd: HWND, placement: &Placement) -> bool {
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return false;
        }
        let held = foreground();
        if minimized_placement(placement).apply(hwnd) && IsIconic(hwnd).as_bool() {
            keep_foreground(held);
            return true;
        }
        let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
        let done = IsIconic(hwnd).as_bool();
        keep_foreground(held);
        done
    }
}

/// The record with "minimized" as its show state. A window that was maximized
/// keeps `WPF_RESTORETOMAXIMIZED`, so un-minimizing brings it back full screen
/// rather than as a small window in the corner.
fn minimized_placement(placement: &Placement) -> Placement {
    let mut next = *placement;
    next.flags &= !WPF_RESTORETOMAXIMIZED;
    if next.show_cmd == SW_SHOWMAXIMIZED_CMD {
        next.flags |= WPF_RESTORETOMAXIMIZED;
    }
    next.show_cmd = SW_SHOWMINIMIZED_CMD;
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minimized_window_comes_back_visible() {
        // A minimized window's record says "minimized". Applying that on the way
        // out leaves the window minimized, which reads as "that sector did
        // nothing".
        assert_eq!(visible_show_cmd(0, SW_SHOWMINIMIZED_CMD), 1);
        // It was maximized before it was minimized: it comes back maximized.
        assert_eq!(
            visible_show_cmd(WPF_RESTORETOMAXIMIZED, SW_SHOWMINIMIZED_CMD),
            SW_SHOWMAXIMIZED_CMD
        );
        // Other show states are left alone.
        assert_eq!(visible_show_cmd(0, 1), 1);
        assert_eq!(
            visible_show_cmd(0, SW_SHOWMAXIMIZED_CMD),
            SW_SHOWMAXIMIZED_CMD
        );
    }

    #[test]
    fn a_window_goes_back_to_the_taskbar_without_losing_its_place() {
        let normal = Rect {
            left: 10,
            top: 20,
            right: 810,
            bottom: 620,
        };
        let record = Placement {
            flags: 0,
            show_cmd: 1,
            min_x: -1,
            min_y: -1,
            max_x: -2,
            max_y: -2,
            normal,
        };
        let stowed = minimized_placement(&record);
        assert_eq!(stowed.show_cmd, SW_SHOWMINIMIZED_CMD);
        assert_eq!(stowed.normal, normal);
        // It was maximized before: un-minimizing brings it back maximized.
        let maximized = Placement {
            show_cmd: SW_SHOWMAXIMIZED_CMD,
            ..record
        };
        let stowed = minimized_placement(&maximized);
        assert_eq!(stowed.show_cmd, SW_SHOWMINIMIZED_CMD);
        assert_ne!(stowed.flags & WPF_RESTORETOMAXIMIZED, 0);
        // An ordinary window must not come back maximized.
        assert_eq!(
            minimized_placement(&record).flags & WPF_RESTORETOMAXIMIZED,
            0
        );
    }
}
