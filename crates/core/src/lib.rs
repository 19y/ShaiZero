//! The reusable heart of ShaiWin.
//!
//! Nothing in this crate touches an operating system. It owns the things that
//! must feel identical on every platform:
//!
//! * [`wheel`] — the ruler and the seating plan of the dial that opens where the
//!   finger pressed: ring bands, what a pointer offset means, and which sector
//!   each window and each remembered direction gets.
//! * [`gesture`] — whether a physical press belongs to the dial, decided before
//!   the click reaches the application: the trigger's modifiers and button, the
//!   drag threshold, and the two cases the wheel must never touch.
//! * [`theme`] — colours, and the stable accent colour each app gets (this is
//!   what makes "who belongs to whom" readable at a glance).
//! * [`hit`] — plain rectangles and hit testing.
//!
//! The Windows shell lives in `shaiwin-platform-win` and `shaiwin`. A shell for
//! another system would consume this crate the same way the Windows one does:
//! geometry and rules here, operating-system calls there.

#![forbid(unsafe_code)]

pub mod gesture;
pub mod hit;
pub mod theme;
pub mod wheel;

pub use hit::Rect;
pub use theme::Rgb;
