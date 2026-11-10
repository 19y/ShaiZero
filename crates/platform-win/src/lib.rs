//! Everything that touches Win32 for ShaiWin.
//!
//! The app crate owns the windows it draws; this crate owns the parts that need
//! to talk to other people's windows:
//!
//! * [`window`] — enumerating, describing and raising other applications' windows.
//! * [`park`] — the placement of a window, and the quiet minimize it takes to
//!   move one off the dial and back on again.
//! * [`hook`] — the low-level mouse hook that decides a gesture before the click
//!   reaches the application under the pointer.
//! * [`monitor`] — monitor geometry and DPI.
//!
//! Everything here is synchronous and may block for a few milliseconds (or, for
//! an unresponsive application, much longer).

#![cfg(windows)]

pub mod hook;
pub mod monitor;
pub mod park;
pub mod thumb;
pub mod window;

/// The six numbers that describe where a window sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    pub fn center(&self) -> (i32, i32) {
        ((self.left + self.right) / 2, (self.top + self.bottom) / 2)
    }

    pub fn from_xywh(x: i32, y: i32, w: i32, h: i32) -> Self {
        Rect {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        }
    }

    pub fn overlaps(&self, other: &Rect) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }
}
