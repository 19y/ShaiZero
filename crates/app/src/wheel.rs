//! The wheel at the cursor: the gesture, the two layers it draws on, and what a
//! release does.
//!
//! Two windows, both created at startup and kept off screen until a gesture needs them. The
//! **veil** is the size of the monitor and dims everything except the window under
//! the pointer; the **disc** is a square the size of the dial and carries the
//! sectors. They are separate because they change at different rates: the disc is
//! repainted every frame of the gesture, the veil only when a different window
//! becomes the one being previewed, and a full-screen `UpdateLayeredWindow` is the
//! most expensive call this program makes.
//!
//! The hook callback and both window procedures run on the same thread - a low
//! level hook only ever calls back while the thread that installed it pumps
//! messages - so the split between them is by cost, not by safety. The callback
//! runs the state machine, remembers the pointer and posts; the window procedure
//! reads the window list, paints, and raises. Nothing here pumps messages and
//! nothing in the callback touches another process, because the desktop is waiting
//! for the callback's return value.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use shaiwin_core::gesture::{self, Button, Machine, Mods, PressContext, Trigger, TriggerInput};
use shaiwin_core::theme::{DEFAULT_SECTOR_COLOUR, Rgb};
use shaiwin_core::wheel::{self as dial, Aim, Dial, Geometry};
use shaiwin_platform_win::hook::{self, Modifiers, MouseButton, Observation, Verdict};
use shaiwin_platform_win::monitor::{self, Monitor};
use shaiwin_platform_win::thumb;
use shaiwin_platform_win::window::{self, WindowInfo};
use tiny_skia::Pixmap;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{present_layer, Dib};
use crate::render::{compose_veil_into, Painter, WheelPreview, WheelSector, WheelView};
use crate::settings::Settings;

const DISC_CLASS: PCWSTR = w!("PanWen.Wheel");
const VEIL_CLASS: PCWSTR = w!("PanWen.Veil");
/// The dim that lands on the *other* screen when a hovered window lives there.
/// It is a separate layer because it is a different size, a different origin
/// and a different repaint rate from the wheel's own veil.
const LOCATOR_CLASS: PCWSTR = w!("PanWen.Locator");
/// The window the compositor draws the previewed window *into*. Not layered:
/// DWM thumbnails are composed into an ordinary window's surface.
const PREVIEW_CLASS: PCWSTR = w!("PanWen.Preview");
/// How far the thumbnail is inset from the hole, so the rim the veil strokes
/// around the hole stays visible over it.
const PREVIEW_INSET: i32 = 4;

/// The dial's own frame tick and the three intents the hook can hand back. Only
/// the disc window ever receives them.
const WHEEL_TIMER: usize = 0x57_C0;
const WM_WHEEL_OPEN: u32 = WM_APP + 20;
const WM_WHEEL_PICK: u32 = WM_APP + 21;
/// The turn the user dragged the dial to, sent to the app window so the settings
/// file can keep it. The angle is in hundredths of a degree in `wParam`.
pub const WM_WHEEL_ROTATE: u32 = WM_APP + 22;

/// Opens the dial at the point packed into `lParam`, for checks that cannot move
/// the pointer. A real gesture records the press and then sends `WM_WHEEL_OPEN`;
/// this records the press and sends the same message, so everything after the
/// point is the path a user takes.
const WM_WHEEL_OPEN_AT: u32 = WM_APP + 30;

// How long the dial takes to arrive. A radial menu that opens slower than this
// feels like it is deciding whether to open at all.

/// How long one sector takes to lean out towards the pointer, and to lean back.
const POP_MS: f32 = 90.0;
/// How far past the outermost ring the pointer has to be before a drag turns the
/// whole dial instead of aiming. In logical pixels, so it is the same grab on a
/// 125% screen as on a 100% one.
const ROTATE_GRAB_MARGIN: f32 = 8.0;
/// A gesture nobody finishes: no input event at all for this long means the wheel
/// is being held on screen by something we did not see - a suspended session, a
/// remote tool that ate the release.
const WATCHDOG_SECS: f64 = 10.0;
/// The rim the veil draws when there is nothing to frame: the hub, a gap, a
/// window on another desktop, or one whose frame could not be read at all.
const NO_OUTLINE: Rgb = Rgb::new(120, 190, 255);

/// One window seated on the dial.
struct Seat {
    /// The id the dial was built with: this seat's index in the window list the
    /// wheel read when it opened, which is what an [`Aim::Slot`] comes back as.
    id: usize,
    info: WindowInfo,
    accent: Rgb,
    ring: usize,
    pos: usize,
    /// Degrees clockwise from 12 o'clock, as the dial seated it.
    bearing: f32,
    span: f32,
    /// The window already in front: releasing here raises nothing, so the artwork
    /// has to say so before the user's hand does.
    current: bool,
    pinned: bool,
    /// 0..1, how far this sector is leaned out towards the pointer.
    pop: f32,
    /// The window's own title, on the sector and at the hub.
    head: String,
    /// The one character that names the application, drawn large enough to be the
    /// sector's mark when the ring is too crowded for a title at all.
    glyph: String,
}

/// What the wheel is being asked to show, and what it is doing about it.
struct Wheel {
    machine: Machine,
    /// The dial's ruler, in logical pixels and never scaled: [`Geometry::new`]
    /// clamps what it is given, so feeding it a DPI-multiplied radius would
    /// quietly hand back a wheel that is not the one the user asked for. The
    /// pointer's offset is divided down to logical instead - see [`Wheel::offset`].
    geometry: Geometry,
    dial_plan: Option<Dial>,
    enabled: bool,
    /// From the frame the threshold is crossed to the frame it is closed.
    open: bool,
    /// Where the finger went down, in device pixels: the centre the dial wants.
    press: (i32, i32),
    /// Where the centre ended up, after the whole dial was pulled inside the work
    /// area. Everything the user aims at is measured from here, not from `press`.
    centre: (f32, f32),
    monitor: Monitor,
    seats: Vec<Seat>,
    aim: Aim,
    opened_at: Instant,
    painted_at: Instant,
    /// The seat under the pointer, or `usize::MAX` for the hub and the gaps.
    hot: usize,
    /// The seat the veil is currently lighting, so the dim is recomposed only when
    /// the preview actually changes.
    veiled: usize,
    /// The monitor the cross-screen locator is covering, if it is up. Bookkeeping
    /// only: `veiled` is the one that decides when to recompose. It lets two
    /// hovers on the same second screen skip a redundant `SetWindowPos`.
    located: Option<isize>,
    /// The trigger exactly as the settings file has it. Kept beside the machine's
    /// copy because the machine's is re-scaled for the monitor of each press -
    /// see [`Policy`]'s `press` - and the number the user wrote down is the one
    /// that has to survive the next save.
    trigger: Trigger,
    /// The dial arrives at full size instead of unfurling. It is the same promise
    /// the settings switch makes about its own knob, and it is here because this is
    /// the only animation the program has left.
    reduced: bool,
    /// The dial's look, from the settings file: the named theme, the accent row,
    /// the air between sectors, and how long the disc takes to arrive.
    theme: u32,
    accent: u32,
    seam: f32,
    /// Degrees clockwise the whole dial is turned, so the user can put the first
    /// sector where their hand expects it. Moved by dragging outside the rim.
    rotate: f32,
    /// The pointer's bearing when the drag began, less the turn at that moment.
    /// `None` while the pointer is not out past the rim. Keeping the difference
    /// rather than the raw bearing is what stops the dial jumping when the drag
    /// starts.
    rotate_grab: Option<f32>,
    /// The turn the last painted frame used, so a drag asks for a repaint even
    /// when no sector is leaning.
    painted_rotate: f32,
    /// The turn already written to the settings file, so letting go only writes
    /// when the drag actually moved it.
    rotate_committed: f32,
    open_ms: f32,
    /// How many windows are drawn, and whether favourites come first. The dial is
    /// a quick target: the rest of the desktop stays in the panel's list.
    limit: u32,
    order: u32,
    /// `0` leaves a picked window where it was; `1` puts it in the middle of the
    /// work area of the screen the pointer is on. Read at the release, so the
    /// panel's choice applies to the next pick without re-registering anything.
    activate_pos: u32,
    /// The monitor's scale shrunk to fit the dial on screen when eight rings are
    /// more than the work area can hold at full size.
    view_scale: f32,
    /// The hovered seat whose `iconic` answer is held, and the answer. `iconic`
    /// asks the window's process a question, and the hub asks it once per frame
    /// while a sector is lit; the answer cannot change while the pointer stays
    /// on the same window, so it is asked once per hover instead.
    iconic_hot: Option<(usize, bool)>,
}

impl Wheel {
    fn new(trigger: Trigger, geometry: Geometry) -> Wheel {
        Wheel {
            machine: Machine::new(trigger),
            trigger,
            geometry,
            dial_plan: None,
            enabled: true,
            open: false,
            press: (0, 0),
            centre: (0.0, 0.0),
            monitor: monitor::primary(),
            seats: Vec::new(),
            aim: Aim::Hub,
            opened_at: Instant::now(),
            painted_at: Instant::now(),
            hot: usize::MAX,
            veiled: usize::MAX,
            located: None,
            reduced: false,
            theme: crate::settings::WHEEL_THEME_DEFAULT,
            accent: crate::settings::WHEEL_ACCENT_DEFAULT,
            seam: crate::settings::WHEEL_SEAM_DEFAULT,
            rotate: crate::settings::WHEEL_ROTATE_DEFAULT,
            rotate_grab: None,
            painted_rotate: crate::settings::WHEEL_ROTATE_DEFAULT,
            rotate_committed: crate::settings::WHEEL_ROTATE_DEFAULT,
            open_ms: crate::settings::WHEEL_OPEN_MS_DEFAULT as f32,
            limit: crate::settings::WHEEL_LIMIT_DEFAULT,
            order: crate::settings::DIAL_ORDER_DEFAULT,
            activate_pos: 0,
            view_scale: monitor::primary().scale,
            iconic_hot: None,
        }
    }

    /// How far the pointer is from the wheel centre, in logical pixels: `dx`, `dy`.
    fn offset(&self) -> (f32, f32) {
        let (x, y) = monitor::cursor();
        let s = self.view_scale.max(0.01);
        (
            (x as f32 - self.centre.0) / s,
            (y as f32 - self.centre.1) / s,
        )
    }

    /// What the pointer is on. Recomputed every frame, and only here: the hook
    /// stores a coordinate and nothing else.
    fn read_aim(&mut self) {
        let Some(plan) = self.dial_plan.as_ref() else {
            self.aim = Aim::Hub;
            self.hot = usize::MAX;
            return;
        };
        let (dx, dy) = self.offset();
        // The sectors are drawn at their plan bearing plus `rotate`, so the
        // pointer's offset is turned back by the same amount before the ruler
        // reads it. Without this a turned dial would answer as if it were
        // upright.
        let (sin, cos) = (-self.rotate).to_radians().sin_cos();
        let aim = self
            .geometry
            .aim(plan, dx * cos - dy * sin, dx * sin + dy * cos);
        self.aim = aim;
        self.hot = match aim {
            Aim::Slot { id, .. } => id,
            _ => usize::MAX,
        };
    }

    /// Turns the whole dial while the pointer is out past the rim. The rim is
    /// the dial's own reach, so the handle is the empty ring the disc window
    /// already occupies - the pointer does not have to leave the screen.
    fn turn_if_outside(&mut self) {
        let Some(plan) = self.dial_plan.as_ref() else {
            self.rotate_grab = None;
            return;
        };
        let reach = self.geometry.extent(plan.rings()) + ROTATE_GRAB_MARGIN;
        let (dx, dy) = self.offset();
        if (dx * dx + dy * dy).sqrt() <= reach {
            self.rotate_grab = None;
            return;
        }
        let bearing = (dy.atan2(dx).to_degrees() + 90.0).rem_euclid(360.0);
        let grab = *self
            .rotate_grab
            .get_or_insert_with(|| (bearing - self.rotate).rem_euclid(360.0));
        self.rotate = (bearing - grab).rem_euclid(360.0);
    }

    fn seat(&self, id: usize) -> Option<&Seat> {
        self.seats.iter().find(|s| s.id == id)
    }
}

/// The windows the user keeps on the dial, in the order they were added.
///
/// A list and not a set, because the order is the seating: the dial takes the
/// inner ring in this sequence, and the settings file writes it back the same way.
/// It is a copy of what `settings.txt` holds, and one-directional - the app owns
/// the file, the panel edits it, and the dial reads what it was told.
fn favorites() -> &'static Mutex<Vec<String>> {
    static FAVORITES: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    FAVORITES.get_or_init(|| Mutex::new(Vec::new()))
}

/// The dial's remembered seating order. A copy of the settings file, like the
/// favourites: the app owns the file and the dial reads what it was told. It is
/// never invented from the desktop's z-order while the wheel is running.
fn seat_order() -> &'static Mutex<Vec<String>> {
    static ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    ORDER.get_or_init(|| Mutex::new(Vec::new()))
}

/// The seats in that order the user took off the dial.
fn parked() -> &'static Mutex<Vec<String>> {
    static PARKED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    PARKED.get_or_init(|| Mutex::new(Vec::new()))
}

/// The fingerprints the user bound to the dial by hand. They are seated even
/// when `wheel_limit` has already handed out every automatic place.
fn bound() -> &'static Mutex<Vec<String>> {
    static BOUND: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    BOUND.get_or_init(|| Mutex::new(Vec::new()))
}

/// Re-seeded whenever a window is seated or taken off, so the very next opening
/// of the dial already knows the arrangement the management page just drew.
pub fn seed_seats(order: &[String], bound_marks: &[String], off: &[String]) {
    {
        let mut held = seat_order()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        held.clear();
        held.extend(order.iter().cloned());
    }
    let mut held = parked()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    held.extend(off.iter().cloned());
    drop(held);
    let mut held = bound()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    held.extend(bound_marks.iter().cloned());
}

/// The applications the user took off the dial, by lower-cased executable name.
///
/// A copy of the settings file's excluded list, one-directional like the
/// favourites are: the file belongs to the app, the panel edits it, and the dial
/// seats only what it was told to seat.
fn excluded() -> &'static Mutex<Vec<String>> {
    static EXCLUDED: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    EXCLUDED.get_or_init(|| Mutex::new(Vec::new()))
}

/// The windows the user took off the dial by hand, by window handle. A copy of
/// the settings file's `off_dial` list, one-directional like the others: the
/// file belongs to the app, the panel edits it, and the dial seats only what
/// it was told to seat. A handle and not a fingerprint, so striking one window
/// off leaves its look-alikes on the dial.
fn off_dial() -> &'static Mutex<Vec<isize>> {
    static OFF_DIAL: OnceLock<Mutex<Vec<isize>>> = OnceLock::new();
    OFF_DIAL.get_or_init(|| Mutex::new(Vec::new()))
}

/// Re-seeded whenever a window is taken off the dial or put back, so the next
/// rebuild already knows what the panel just decided.
pub fn seed_off_dial(list: &[isize]) {
    let mut held = off_dial()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    held.extend(list.iter().cloned());
    if !held.is_empty() {
        crate::log!("wheel: {} window(s) kept off the dial", held.len());
    }
}

/// The windows the user seated by hand, by handle: each one holds a sector of
/// its own even when its look-alikes share a fingerprint. A copy of the settings
/// file's `seat_hwnds` list, one-directional like the others.
fn seat_windows() -> &'static Mutex<Vec<isize>> {
    static SEAT_WINDOWS: OnceLock<Mutex<Vec<isize>>> = OnceLock::new();
    SEAT_WINDOWS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Re-seeded whenever a window is seated or taken off, so the next opening of
/// the dial already knows the windows the panel just named.
pub fn seed_seat_windows(list: &[isize]) {
    let mut held = seat_windows()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    held.extend(list.iter().cloned());
}

/// What the user typed for each window. A map keyed by the same fingerprint a pin
/// uses, so a tag and a favourite and a pin all speak about one window the same
/// way. [`fingerprint`] hands a live window the mark it was first seen with, so
/// a page that renames its tab does not take the label with it; across a restart
/// the title as it stands then is all there is to key on.
fn tags() -> &'static Mutex<BTreeMap<String, String>> {
    static TAGS: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();
    TAGS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Start-up only, and again whenever the panel changes either list: the file's
/// favourites and labels become what the dial seats and draws by.
pub fn seed_favorites(list: &[String]) {
    let mut held = favorites()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    held.extend(list.iter().cloned());
    crate::log!(
        "wheel: {} favourite window(s) read from the settings",
        held.len()
    );
}

/// The applications that do not come on the dial at all. Re-seeded whenever the
/// panel strikes one off or puts it back.
pub fn seed_excluded(list: &[String]) {
    let mut held = excluded()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    held.extend(list.iter().cloned());
    if !held.is_empty() {
        crate::log!("wheel: {} application(s) kept off the dial", held.len());
    }
}

/// The other half of [`seed_favorites`].
pub fn seed_tags(tags_list: &[crate::settings::Label]) {
    let mut held = tags()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    for label in tags_list {
        held.insert(label.window.clone(), label.tag.clone());
    }
    crate::log!("wheel: {} window tag(s) read from the settings", held.len());
}

/// Where a window sits on the favourites list, if it is on it.
fn favorite_rank(fingerprint: &str) -> Option<usize> {
    favorites()
        .lock()
        .ok()
        .and_then(|list| list.iter().position(|f| f == fingerprint))
}

/// The colours the user picked by hand, keyed like the tags.
fn colors() -> &'static Mutex<BTreeMap<String, Rgb>> {
    static COLORS: OnceLock<Mutex<BTreeMap<String, Rgb>>> = OnceLock::new();
    COLORS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// The other half of [`seed_tags`].
pub fn seed_colors(list: &[crate::settings::Colour]) {
    let mut held = colors()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    for colour in list {
        held.insert(colour.window.clone(), colour.rgb);
    }
    crate::log!(
        "wheel: {} hand-picked sector colour(s) read from the settings",
        held.len()
    );
}

/// The colour the user picked for a window, if any.
fn color_for(fingerprint: &str) -> Option<Rgb> {
    colors()
        .lock()
        .ok()
        .and_then(|map| map.get(fingerprint).copied())
}

/// The sector sizes the user picked by hand, keyed like the colours.
fn weights() -> &'static Mutex<BTreeMap<String, f32>> {
    static WEIGHTS: OnceLock<Mutex<BTreeMap<String, f32>>> = OnceLock::new();
    WEIGHTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// The other half of [`seed_colors`]: loads the hand-picked sizes at start and
/// re-seeds them whenever the panel saves a change, so the next dial the user
/// opens already reflects the resize without a restart.
pub fn seed_weights(list: &[crate::settings::WindowWeight]) {
    let mut held = weights()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    held.clear();
    for w in list {
        held.insert(w.window.clone(), w.weight);
    }
}

/// The size a window's sector claims, or `None` when the user left it at the
/// even default - `None` is what the dial builder understands as "1.0", so the
/// default path does no extra work.
fn weight_for(fingerprint: &str) -> Option<f32> {
    weights()
        .lock()
        .ok()
        .and_then(|map| map.get(fingerprint).copied())
}

/// The rarity-colour scheme the user picked at the panel. Cached once at start
/// and every time a draft applies, so the live dial reads the current preset
/// without dragging the whole [`Settings`] table into its render hot path.
fn colour_scheme() -> &'static AtomicU32 {
    static SCHEME: AtomicU32 = AtomicU32::new(0);
    &SCHEME
}
pub fn seed_colour_scheme(scheme: u32) {
    colour_scheme().store(scheme, Ordering::Relaxed);
}

/// The label a window carries, if the user wrote one.
fn tag_for(fingerprint: &str) -> Option<String> {
    tags()
        .lock()
        .ok()
        .and_then(|map| map.get(fingerprint).cloned())
}

/// The large mark of a sector: the label the user typed when the window has one,
/// because a tag exists to be found at a glance, and the application's first
/// letter when it does not. The title stays on the small line either way - a tag
/// that hides which window is which is a tag the user takes back off.
fn sector_mark(tag: Option<&str>, letter: &str) -> String {
    tag.filter(|t| !t.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| letter.to_string())
}

/// The identity a pin is remembered by.
///
/// It is the executable, the window class and the first word of the title. The
/// first word is what tells two windows of one program apart - and also what
/// used to hand a browser a new identity every time the page renamed the tab.
/// [`fingerprint`] pins the first answer a live window gave, so the rest of the
/// program can keep using the title word without the window moving underneath
/// it.
fn mark_of(info: &WindowInfo) -> String {
    let first_word = info
        .title
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    format!("{}|{}|{}", info.exe_path, info.class, first_word)
}

// The mark a live window was first seen with.
//
// A window's title is not part of the window the user made a decision about: a
// browser rewrites it on every page, and a seat or a label keyed on it was lost
// the moment the page changed. The handle and the process instance behind it do
// not change while the window lives, so the first fingerprint is the one the
// window keeps, and its seat, label and favourite stay where the user put them.
// The title on the sector still follows the page - only the identity does not.
//
// A handle can be reused once its window is gone, so a remembered mark is only
// trusted while the handle still names the same process instance, executable
// and window class.
thread_local! {
    static MARKS: RefCell<BTreeMap<(isize, u32, u64), String>> =
        const { RefCell::new(BTreeMap::new()) };
}

pub(crate) fn fingerprint(info: &WindowInfo) -> String {
    let key = (info.hwnd, info.pid, info.process_start);
    let prefix = format!("{}|{}|", info.exe_path, info.class);
    MARKS.with(|cell| {
        let mut held = cell.borrow_mut();
        if let Some(mark) = held.get(&key) {
            if mark.starts_with(&prefix) {
                return mark.clone();
            }
        }
        let mark = mark_of(info);
        held.insert(key, mark.clone());
        mark
    })
}

/// The whole mutable state, behind one lock. The hook callback and both window
/// procedures run on the UI thread, so it is never really contended; the lock
/// exists to make the `static` legal and to keep that true if a second thread ever
/// appears. A poisoned lock still holds good data, and the only way it got poisoned
/// is a panic in a closure that cannot recover anyway - so it is taken apart and
/// used regardless rather than turning one bad frame into no wheel at all.
fn with<T>(f: impl FnOnce(&mut Wheel) -> T) -> T {
    static STATE: OnceLock<Mutex<Wheel>> = OnceLock::new();
    let state = STATE.get_or_init(|| {
        Mutex::new(Wheel::new(
            Trigger::default(),
            Geometry::new(dial::RADIUS_DEFAULT, dial::DEAD_ZONE_DEFAULT),
        ))
    });
    let mut guard = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    f(&mut guard)
}

// The GDI bitmap behind each layer, kept out of [`Wheel`] on purpose: a `Dib`
// holds raw pointers, so a state that contains one could not sit in the `static`
// the hook reads. Both are only ever touched on the UI thread, which is also why
// the painter lives here rather than in the state.
thread_local! {
    static DISC_DIB: RefCell<Option<Dib>> = const { RefCell::new(None) };
    static VEIL_DIB: RefCell<Option<Dib>> = const { RefCell::new(None) };
    /// The locator has its own DIB and raster: the second monitor is usually a
    /// different size, and sharing would reallocate on every alternating frame.
    static LOCATOR_DIB: RefCell<Option<Dib>> = const { RefCell::new(None) };
    static LOCATOR_CANVAS: RefCell<Option<(u32, u32, Pixmap)>> = const { RefCell::new(None) };
    static PAINTER: RefCell<Option<Painter>> = const { RefCell::new(None) };
    /// The dial's raster, reused while the size stays the same.
    static DISC_CANVAS: RefCell<Option<(u32, Pixmap)>> = const { RefCell::new(None) };
    /// The dial's still half, kept between frames. Shadow, glass and rim do not
    /// move while a sector leans out under the pointer, and drawing them again
    /// was the larger part of every hover frame.
    static DISC_STILL: RefCell<Option<Still>> = const { RefCell::new(None) };
    /// The veil's raster, keyed by the monitor size. The veil is the size of the
    /// whole screen, so allocating one per hover was a multi-megabyte allocation
    /// on every sector the pointer crossed - the stutter while choosing.
    static VEIL_CANVAS: RefCell<Option<(u32, u32, Pixmap)>> = const { RefCell::new(None) };
    /// The thumbnail currently registered with the compositor. Only touched on
    /// the UI thread, like the layers themselves.
    static PREVIEW: RefCell<Option<Preview>> = const { RefCell::new(None) };
    /// Frame bookkeeping: frames seen, frames actually painted, summed cost and
    /// the worst one. Zeroed every time it is written to the log.
    static FRAME_METER: RefCell<(u32, u32, f32, f32)> = const { RefCell::new((0, 0, 0.0, 0.0)) };
}

struct Preview {
    id: isize,
    source: isize,
}

static DISC_HWND: AtomicIsize = AtomicIsize::new(0);
static VEIL_HWND: AtomicIsize = AtomicIsize::new(0);
static LOCATOR_HWND: AtomicIsize = AtomicIsize::new(0);
static PREVIEW_HWND: AtomicIsize = AtomicIsize::new(0);
/// The line's window, which is the app's window: the one that owns the settings
/// file and the journal. Learned at start-up so a pin can ask for a save without
/// the wheel holding a borrow of `App` across a hook callback.
static APP_HWND: AtomicIsize = AtomicIsize::new(0);

fn disc() -> HWND {
    HWND(DISC_HWND.load(Ordering::Relaxed) as *mut core::ffi::c_void)
}

fn veil() -> HWND {
    HWND(VEIL_HWND.load(Ordering::Relaxed) as *mut core::ffi::c_void)
}

fn locator() -> HWND {
    HWND(LOCATOR_HWND.load(Ordering::Relaxed) as *mut core::ffi::c_void)
}

fn preview() -> HWND {
    HWND(PREVIEW_HWND.load(Ordering::Relaxed) as *mut core::ffi::c_void)
}

/// The layer styles: above everything, never activated, never a target for a click
/// that was not meant for it. The disc is click-through because a wheel is read
/// and released, never clicked.
fn layer_ex() -> WINDOW_EX_STYLE {
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE
}

/// Builds the two layers, takes the settings and learns where to send a save.
/// Called once at startup; both sit 1x1 in the
/// corner until a gesture needs them, because creating a layered window at the
/// size it will eventually be costs a full-screen DIB for a wheel that may never
/// open.
pub fn startup(settings: &Settings, app: HWND) {
    APP_HWND.store(app.0 as isize, Ordering::Relaxed);
    seed_favorites(&settings.favorites);
    seed_seats(&settings.seat_order, &settings.bound, &settings.parked);
    seed_tags(&settings.tags);
    seed_colors(&settings.colors);
    seed_weights(&settings.weights);
    seed_colour_scheme(settings.colour_scheme);
    seed_excluded(&settings.excluded);
    seed_off_dial(&settings.off_dial);
    seed_seat_windows(&settings.seat_hwnds);
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else {
            return;
        };
        let hinst: HINSTANCE = hinst.into();
        let disc_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(disc_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: DISC_CLASS,
            ..Default::default()
        };
        let veil_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(veil_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: VEIL_CLASS,
            ..Default::default()
        };
        let locator_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(locator_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            lpszClassName: LOCATOR_CLASS,
            ..Default::default()
        };
        let preview_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(preview_proc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            // Anything the thumbnail does not cover is black on purpose: a
            // preview that is still loading must not flash the desktop white.
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: PREVIEW_CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&disc_class);
        let _ = RegisterClassExW(&veil_class);
        let _ = RegisterClassExW(&locator_class);
        let _ = RegisterClassExW(&preview_class);
        let made = |class: PCWSTR, name: PCWSTR| {
            CreateWindowExW(
                layer_ex(),
                class,
                name,
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(hinst),
                None,
            )
            .ok()
        };
        // The veil first: two top-most windows of the same band stack in creation
        // order, and the disc has to be the one on top of the dim. The locator is
        // made between them, so if the two ever overlap the disc still wins.
        let Some(v) = made(VEIL_CLASS, w!("PanWenVeil")) else {
            crate::log!("wheel: the veil window would not be made");
            return;
        };
        let Some(l) = made(LOCATOR_CLASS, w!("PanWenLocator")) else {
            crate::log!("wheel: the locator window would not be made");
            return;
        };
        let Some(d) = made(DISC_CLASS, w!("PanWenWheel")) else {
            crate::log!("wheel: the disc window would not be made");
            return;
        };
        // The thumbnail is drawn into an ordinary window: a layered surface has
        // no composition of its own for the compositor to draw into. It is
        // created last so it sits above the veil and below the disc.
        let preview = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            PREVIEW_CLASS,
            w!("PanWenPreview"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            Some(hinst),
            None,
        )
        .ok();
        if let Some(p) = preview {
            PREVIEW_HWND.store(p.0 as isize, Ordering::Relaxed);
        } else {
            crate::log!("wheel: the preview window would not be made");
        }
        VEIL_HWND.store(v.0 as isize, Ordering::Relaxed);
        LOCATOR_HWND.store(l.0 as isize, Ordering::Relaxed);
        DISC_HWND.store(d.0 as isize, Ordering::Relaxed);
        reconfigure(settings);
        crate::log!(
            "wheel ready: radius {:.0}, hub {:.2}, {}",
            settings.wheel_radius,
            settings.wheel_dead,
            if settings.wheel { "on" } else { "off" }
        );
    }
}

/// Puts the settings into the wheel: the ruler, the trigger, and whether any of it
/// is switched on. Called again when the settings panel saves.
pub fn reconfigure(settings: &Settings) {
    let trigger = settings.trigger();
    let geometry = Geometry::new(settings.wheel_radius, settings.wheel_dead);
    with(|w| {
        w.machine.set_trigger(trigger);
        w.trigger = trigger;
        w.geometry = geometry;
        w.enabled = settings.wheel;
        w.reduced = settings.reduced_motion;
        w.theme = settings.wheel_theme;
        w.accent = settings.wheel_accent;
        w.seam = settings.wheel_seam;
        w.rotate = settings.wheel_rotate;
        w.painted_rotate = w.rotate;
        w.rotate_committed = w.rotate;
        w.open_ms = settings.wheel_open_ms as f32;
        w.limit = settings.wheel_limit;
        w.order = settings.dial_order;
        w.activate_pos = settings.activate_pos;
    });
    // The order is not one of the rows the panel edits, but a save rewrites the
    // whole file, so this keeps the dial and the copy the app just wrote in step.
    seed_seats(&settings.seat_order, &settings.bound, &settings.parked);
    seed_seat_windows(&settings.seat_hwnds);
    if settings.wheel {
        // Turning the wheel on in the panel has to mean it works now, which for a
        // hook means going in - and `install_hook` answers `true` when it is
        // already there, so this is safe on every save.
        install_hook();
    } else if hook::is_installed() {
        // Switching it off has to give the clicks back, which means the hook comes
        // out rather than staying in to promise `Forward`.
        hook::uninstall();
    }
}

/// Takes over the desktop's mouse. False is the same answer as "the wheel is off"
/// as far as the user is concerned, and is logged either way.
pub fn install_hook() -> bool {
    static POLICY: Policy = Policy;
    if hook::is_installed() {
        return true;
    }
    match hook::install(&POLICY) {
        Ok(()) => {
            crate::log!("wheel: the hook is in");
            true
        }
        Err(error) => {
            crate::log!("wheel: the hook would not install ({error})");
            false
        }
    }
}

/// The language changed, and every word the dial caches was drawn in the old one -
/// in a different face, sometimes, which a cache keyed on the text and its size has
/// no way to notice.
pub fn relanguage() {
    PAINTER.with(|cell| {
        if let Some(painter) = cell.borrow_mut().as_mut() {
            painter.forget_sprites();
        }
    });
}

/// The way out: the hook comes off the desktop and both layers go away. Called on
/// quit and on session end, when a swallowed press would otherwise be one the user
/// never got back.
pub fn shutdown() {
    if hook::is_installed() {
        hook::uninstall();
        crate::log!("wheel: the hook is out");
    }
    with(|w| {
        if let Some(pt) = w.machine.abandon() {
            if let Some(button) = trigger_button() {
                let _ = hook::replay_click(hook::Pt::new(pt.x, pt.y), button);
            }
        }
        w.open = false;
    });
    close_layers();
}

/// The watchdog, from the app's own poll: a gesture with no input for ten seconds
/// is not a gesture, it is a wheel left on the screen.
pub fn tick() {
    if !with(|w| w.open) {
        return;
    }
    if hook::seconds_since_last_event() > WATCHDOG_SECS {
        crate::log!("wheel: nothing has moved for {WATCHDOG_SECS}s, closing");
        cancel();
    }
}

/// The clock behind the long press. Called from the app's poll: a hold has no
/// mouse message of its own, so without this the wheel would wait for a move
/// that may never come. Opens once, at the press point.
pub fn poll_hold() {
    let opened = with(|w| {
        if !w.enabled || w.open {
            return false;
        }
        let Some(point) = w
            .machine
            .after_hold(Duration::from_millis(gesture::HOLD_MS))
        else {
            return false;
        };
        w.press = (point.x, point.y);
        w.open = true;
        w.opened_at = Instant::now();
        w.painted_at = Instant::now();
        true
    });
    if opened {
        // Built in the disc window's own message, like every other opening: the
        // hook's thread must not read the desktop.
        send(WM_WHEEL_OPEN, 0);
        let (x, y) = with(|w| w.press);
        let monitor = monitor::at_point(x, y);
        crate::log!(
            "wheel: opened on a held trigger at ({x},{y}), monitor 0x{:x} bounds=({},{},{},{}) work=({},{},{},{}) scale {:.2}",
            monitor.handle,
            monitor.bounds.left,
            monitor.bounds.top,
            monitor.bounds.right,
            monitor.bounds.bottom,
            monitor.work.left,
            monitor.work.top,
            monitor.work.right,
            monitor.work.bottom,
            monitor.scale
        );
    }
}

/// The button the gesture is aimed with, or `None` when the trigger is a key.
fn trigger_button() -> Option<MouseButton> {
    match with(|w| w.machine.trigger().input) {
        TriggerInput::Mouse(Button::Left) => Some(MouseButton::Left),
        TriggerInput::Mouse(Button::Right) => Some(MouseButton::Right),
        TriggerInput::Mouse(Button::Middle) => Some(MouseButton::Middle),
        TriggerInput::Mouse(Button::X1) => Some(MouseButton::X1),
        TriggerInput::Mouse(Button::X2) => Some(MouseButton::X2),
        TriggerInput::Key(_) => None,
    }
}

/// The hook's half of the wheel. Every verdict is decided here; every action is
/// posted, because the desktop is waiting on this function's return value.
struct Policy;

impl hook::GesturePolicy for Policy {
    fn watches_moves(&self) -> bool {
        // A move is the only thing that says the threshold has been crossed.
        with(|w| w.enabled)
    }

    fn observe(&self, ev: Observation) -> Verdict {
        if !with(|w| w.enabled) {
            return Verdict::Forward;
        }
        match ev {
            Observation::Press {
                pt,
                button,
                mods,
                on_frame,
                exclusive_foreground,
            } => press(pt.x, pt.y, button, mods, on_frame, exclusive_foreground),
            Observation::Release { pt, button, .. } => release(pt.x, pt.y, button),
            Observation::Move { pt } => move_to(pt.x, pt.y),
            Observation::KeyDown { vk, mods } => key_down(vk, mods),
            Observation::KeyUp { vk, .. } => key_up(vk),
            Observation::ReplayFailed => {
                // Switched off inside the hook already. Saying so is the whole
                // answer: the user's click is gone and all that is left is to write
                // down where it went.
                crate::log!("wheel: a click we owed could not be paid back");
                Verdict::Forward
            }
        }
    }
}

fn press(
    x: i32,
    y: i32,
    button: MouseButton,
    mods: Modifiers,
    on_frame: bool,
    exclusive_foreground: bool,
) -> Verdict {
    let want = match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
        MouseButton::X1 => Button::X1,
        MouseButton::X2 => Button::X2,
    };
    if with(|w| w.open) {
        // The dial is on screen, so anything else the user presses is aimed at it:
        // the other button cancels, the same one again does nothing until it is let
        // go, and none of it reaches the window underneath.
        if button == MouseButton::Right {
            cancel();
        }
        return Verdict::Swallow;
    }
    let took = with(|w| {
        let answer = w.machine.on_press(
            gesture::Pt::new(x, y),
            Mods {
                alt: mods.alt,
                ctrl: mods.ctrl,
                shift: mods.shift,
                win: mods.win,
            },
            want,
            PressContext {
                on_frame,
                exclusive_foreground,
            },
        );
        if answer == gesture::OnPress::Take {
            w.press = (x, y);
        }
        answer
    });
    if took == gesture::OnPress::Take {
        Verdict::Swallow
    } else {
        if on_frame || exclusive_foreground {
            let want_mods = with(|w| w.trigger.mods);
            let held = Mods {
                alt: mods.alt,
                ctrl: mods.ctrl,
                shift: mods.shift,
                win: mods.win,
            };
            if held.covers(&want_mods) {
                // The trigger was pulled and the wheel let it go. Saying so is how
                // the next reader tells "the hook never saw it" from "the wheel
                // decided this press belonged to somebody else".
                crate::log!(
                    "wheel: the press at ({},{}) was {} - the gesture was left with it",
                    x,
                    y,
                    if on_frame {
                        "on a window's frame"
                    } else {
                        "over a full-screen window"
                    }
                );
            }
        }
        Verdict::Forward
    }
}

fn move_to(x: i32, y: i32) -> Verdict {
    let event = with(|w| w.machine.on_move(gesture::Pt::new(x, y)));
    match event {
        gesture::OnMove::Nothing | gesture::OnMove::Aim(_) => {
            // Past the rim the move is the dial's own handle rather than an aim:
            // the whole wheel turns under the hand, which is how a user puts the
            // first sector where it is comfortable.
            with(|w| {
                if w.open {
                    w.turn_if_outside();
                }
            });
            // The pointer keeps moving: the wheel is aimed at where it is, and
            // freezing it would be the one thing a radial menu must not do.
            Verdict::Forward
        }
    }
}

fn release(x: i32, y: i32, button: MouseButton) -> Verdict {
    if !with(|w| w.machine.is_in_flight()) {
        return Verdict::Forward;
    }
    if trigger_button() != Some(button) {
        // Some other button came up while ours is still held: the gesture is not
        // finished, and that press was the desktop's own.
        return Verdict::Forward;
    }
    let answer = with(|w| w.machine.on_release(gesture::Pt::new(x, y)));
    match answer {
        gesture::OnRelease::Pick(_) | gesture::OnRelease::Cancel => {
            send(WM_WHEEL_PICK, 0);
            Verdict::Swallow
        }
        gesture::OnRelease::ReplayClick(pt) => {
            // Travelled less than the threshold with the key held: a click the user
            // meant for an application, whose down we took. Paying it back here,
            // inside the hook, is the one place the ordering still makes sense.
            let paid = trigger_button()
                .map(|button| hook::replay_click(hook::Pt::new(pt.x, pt.y), button))
                .unwrap_or(false);
            if !paid {
                crate::log!(
                    "wheel: the click owed at ({},{}) could not be replayed",
                    pt.x,
                    pt.y
                );
            }
            Verdict::Swallow
        }
        gesture::OnRelease::NotOurs => Verdict::Forward,
    }
}

fn key_down(vk: u32, mods: Modifiers) -> Verdict {
    if vk == VK_ESCAPE.0 as u32 && with(|w| w.open) {
        cancel();
        return Verdict::Swallow;
    }
    if with(|w| w.open) {
        return Verdict::Forward;
    }
    let (x, y) = monitor::cursor();
    let took = with(|w| {
        w.machine.on_key_down(
            vk as u16,
            Mods {
                alt: mods.alt,
                ctrl: mods.ctrl,
                shift: mods.shift,
                win: mods.win,
            },
            gesture::Pt::new(x, y),
            PressContext {
                on_frame: false,
                exclusive_foreground: false,
            },
        )
    });
    if took == gesture::OnPress::Take {
        // Held at the pointer's current place; the dial unfurls there when the
        // key stays down for the threshold.
        with(|w| w.press = (x, y));
        let held = [
            (mods.alt, "alt"),
            (mods.ctrl, "ctrl"),
            (mods.shift, "shift"),
            (mods.win, "win"),
        ]
        .iter()
        .filter(|(on, _)| *on)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join("+");
        let held = if held.is_empty() { "none".to_string() } else { held };
        // The first layer of the multi-screen report: whether the key arrived at
        // all, and where the pointer was when it did. A hook that is not in the
        // chain, or a press point on a monitor nobody can see, is visible here
        // before any window is built.
        crate::log!(
            "wheel: trigger key vk={vk} mods={} hook={} cursor=({x},{y})",
            held,
            if hook::is_installed() { "in" } else { "out" }
        );
        Verdict::Swallow
    } else {
        Verdict::Forward
    }
}

fn key_up(vk: u32) -> Verdict {
    if vk == VK_ESCAPE.0 as u32 && with(|w| w.open) {
        return Verdict::Swallow;
    }
    // The trigger's own modifier leaving while the gesture still holds: the wheel
    // stays, but an `Alt` that goes silently would light up the menu bar of
    // whatever is behind it.
    if is_trigger_key(vk) && with(|w| w.machine.needs_mask_key()) {
        return Verdict::ForwardWithMaskKey;
    }
    let TriggerInput::Key(want) = with(|w| w.machine.trigger().input) else {
        return Verdict::Forward;
    };
    if vk as u16 != want || !with(|w| w.machine.is_in_flight()) {
        return Verdict::Forward;
    }
    let (x, y) = monitor::cursor();
    let answer = with(|w| w.machine.on_key_up(vk as u16, gesture::Pt::new(x, y)));
    match answer {
        gesture::OnRelease::Pick(_) | gesture::OnRelease::Cancel => {
            send(WM_WHEEL_PICK, 0);
            Verdict::Swallow
        }
        // A key can never owe a click, and another key's release is not ours.
        gesture::OnRelease::NotOurs | gesture::OnRelease::ReplayClick(_) => Verdict::Forward,
    }
}

/// Whether this key is one the wheel's trigger is holding down. Windows reports
/// the generic code for the left copy and its own for the right, and either one
/// releasing is the same loss of the modifier, so both count. `Ctrl` is not here
/// because the wheel never closes on it: it is the shipped shortcut modifier and
/// pressing it mid-drag is ordinary desktop business.
fn is_trigger_key(vk: u32) -> bool {
    const SHIFT: [u32; 3] = [0x10, 0xA0, 0xA1];
    const MENU: [u32; 3] = [0x12, 0xA4, 0xA5];
    const WIN: [u32; 2] = [0x5B, 0x5C];
    let mods = with(|w| w.machine.trigger().mods);
    (mods.alt && MENU.contains(&vk))
        || (mods.shift && SHIFT.contains(&vk))
        || (mods.win && WIN.contains(&vk))
}

fn send(msg: u32, wp: usize) {
    let hwnd = disc();
    if !hwnd.is_invalid() {
        unsafe {
            let _ = PostMessageW(Some(hwnd), msg, WPARAM(wp), LPARAM(0));
        }
    }
}

/// Writes the turn the drag left behind, once the gesture is over. The wheel
/// does not own the file, so it hands the number to the app window; saving on
/// every frame of the drag would put a file write between the hand and the
/// picture.
fn commit_rotation() {
    let angle = with(|w| {
        if (w.rotate - w.rotate_committed).abs() < 0.01 {
            return None;
        }
        w.rotate_committed = w.rotate;
        Some(w.rotate)
    });
    let Some(angle) = angle else {
        return;
    };
    let hwnd = HWND(APP_HWND.load(Ordering::Relaxed) as *mut core::ffi::c_void);
    if hwnd.is_invalid() {
        return;
    }
    let hundredths = (angle.rem_euclid(360.0) * 100.0).round() as usize;
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_WHEEL_ROTATE, WPARAM(hundredths), LPARAM(0));
    }
    crate::log!("wheel: turned to {angle:.1} degrees");
}

/// Abandons the gesture and takes the dial down. Escape, the right button, the
/// watchdog and a release inside the hub all end here.
fn cancel() {
    // The dial may have been turned while the hand was out past the rim, and a
    // turn is a choice the next opening has to keep.
    commit_rotation();
    // The machine may still hold a swallowed press, and `abandon()` is what
    // decides whether an application is owed a click for it. Here the dial had
    // opened, so the press was spent and the answer is normally none.
    let owed = with(|w| {
        w.open = false;
        w.machine.abandon()
    });
    if let Some(pt) = owed {
        if let Some(button) = trigger_button() {
            let _ = hook::replay_click(hook::Pt::new(pt.x, pt.y), button);
        }
    }
    close_layers();
    crate::log!("wheel: cancelled");
}

fn close_layers() {
    // Registered thumbnails outlive a hidden window, so they are given back
    // before the surfaces they were drawn into go away.
    hide_preview();
    unsafe {
        let _ = KillTimer(Some(disc()), WHEEL_TIMER);
        for hwnd in [veil(), disc(), locator()] {
            if !hwnd.is_invalid() {
                let _ = ShowWindow(hwnd, SW_HIDE);
            }
        }
    }
    with(|w| w.located = None);
}

/// A layer window belongs to the virtual desktop it was created on. The three
/// wheel windows are made once at startup, so after a virtual desktop switch
/// they are cloaked on the old one - moving them with `SetWindowPos` and
/// showing them does not bring them across. Asking the desktop manager to move
/// each one is the only way to have it on the desktop the user is looking at.
/// A window already here answers `true` without a call.
fn claim_current_desktop() {
    let mut missed: Vec<&str> = Vec::new();
    for (name, hwnd) in [
        ("veil", veil()),
        ("disc", disc()),
        ("preview", preview()),
        ("locator", locator()),
    ] {
        if hwnd.is_invalid() {
            continue;
        }
        if !window::move_to_current_desktop(hwnd) {
            missed.push(name);
        }
    }
    crate::log!(
        "wheel: layers claimed on the current desktop (missed: {})",
        if missed.is_empty() {
            "none".to_string()
        } else {
            missed.join(", ")
        }
    );
}

// MARK: - The windows

unsafe extern "system" fn disc_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_WHEEL_OPEN => {
                build();
                if with(|w| w.open && !w.seats.is_empty()) {
                    claim_current_desktop();
                    show_layers();
                    // The page may be holding the mouse for a sector drag; the
                    // dial has just taken the desktop, so the page must let go.
                    crate::settings_ui::wheel_opened();
                    // A layered window keeps the pixels of its last showing: a
                    // theme (or size, or accent) changed since the dial was last
                    // open would come back dressed in the old look until the
                    // first hover happened to repaint it. `frame` only paints
                    // when something moves, so the first frame is composed here,
                    // before anything is on screen.
                    paint_disc();
                    frame();
                    let _ = SetTimer(Some(hwnd), WHEEL_TIMER, 16, None);
                } else {
                    with(|w| w.open = false);
                }
                LRESULT(0)
            }
            WM_WHEEL_PICK => {
                pick();
                LRESULT(0)
            }
            WM_WHEEL_OPEN_AT => {
                let x = (lp.0 as u32 & 0xffff) as i16 as i32;
                let y = ((lp.0 as u32 >> 16) & 0xffff) as i16 as i32;
                with(|w| {
                    w.press = (x, y);
                    w.open = true;
                    w.opened_at = Instant::now();
                    w.painted_at = Instant::now();
                });
                crate::log!("wheel: open requested at ({x},{y})");
                disc_proc(hwnd, WM_WHEEL_OPEN, WPARAM(0), LPARAM(0))
            }
            // A DPI change is a different scale for the same press point. The
            // ruler is in logical pixels, so the window has to be measured and
            // placed again rather than left at the old size on the new screen.
            WM_DPICHANGED => {
                if with(|w| w.open) {
                    let before = with(|w| w.view_scale);
                    place();
                    claim_current_desktop();
                    show_layers();
                    frame();
                    crate::log!(
                        "wheel: WM_DPICHANGED dpi={} scale {:.2} -> {:.2}",
                        wp.0 as u32 & 0xffff,
                        before,
                        with(|w| w.view_scale)
                    );
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == WHEEL_TIMER => {
                frame();
                LRESULT(0)
            }
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            // The disc is drawn over the desktop and is not a thing to click. Every
            // click that lands on it belongs to the window underneath, which is
            // also what the hook says while the gesture is running.
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe extern "system" fn veil_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// The cross-screen dim is click-through like the veil, and just as uninterested
/// in focus: it points at a window, it is not one to use.
unsafe extern "system" fn locator_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// The thumbnail's window: never clicked, never focused, only a surface for the
/// compositor to paint another window into.
unsafe extern "system" fn preview_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
            WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
            WM_NCDESTROY => {
                hide_preview();
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// One window as the settings preview sees it: enough to paint the sector and to
/// turn a click back into the window it belongs to.
#[derive(Clone, Debug)]
pub struct PreviewSeat {
    pub ring: usize,
    pub bearing: f32,
    pub span: f32,
    /// The application's own colour, before the theme has touched it. The
    /// settings page needs it to draw a thumbnail of every other theme without
    /// re-reading the desk.
    pub base_accent: Rgb,
    pub accent: Rgb,
    pub glyph: String,
    pub head: String,
    pub fingerprint: String,
    pub hwnd: isize,
    pub pinned: bool,
    pub current: bool,
    /// Set while the settings page is dragging this sector: the painter shows it
    /// lit, so the eye can tell the one being moved from the ones yielding.
    pub hot: bool,
}

/// Applies the two rules that keep a window off the dial: the applications the
/// user struck out, and the windows taken off by hand.
///
/// The answer is exactly the windows that survived. It used to fall back to the
/// whole desk when every window had been struck off, on the theory that an empty
/// dial was worse than a dial that ignored the user. That fallback was the bug:
/// taking the last window off put it straight back, and because the list and the
/// picture reached the fallback through two slightly different lists (one had the
/// excluded applications removed already) they could disagree about what was on
/// the dial. The wheel now opens empty instead, which is the honest answer to a
/// desk whose every window was taken off.
fn sift<X, O>(list: Vec<WindowInfo>, excluded: X, taken_off: O) -> Vec<WindowInfo>
where
    X: Fn(&WindowInfo) -> bool,
    O: Fn(isize) -> bool,
{
    list.into_iter()
        .filter(|info| !excluded(info) && !taken_off(info.hwnd))
        .collect()
}

/// The windows that actually get a sector: the first `limit` places of the
/// remembered order, plus every window the user bound by hand, minus the ones
/// the user parked or closed.
///
/// Both the live wheel and the settings preview go through this, so the picture
/// the panel draws and the dial that opens cannot disagree about which windows
/// exist.
///
/// The order is the whole point. The desktop's z-order changes every time a
/// window is used, so seating by recency reshuffled the entire dial after one
/// pick; `seat_order` is the remembered arrangement the user learned. A window
/// that is parked (taken off) or simply not running leaves a gap - the sector
/// count shrinks, but **no other window is promoted into it**. A window that
/// was never given a place waits in the management list until `Ctrl+2` or a
/// double-click seats it, which is what stops a down-dial from instantly
/// refilling itself.
///
/// The limit is the size of the selection the dial makes *by itself*, not a cap
/// on the dial: a window the user bound explicitly is seated no matter how many
/// automatic places the limit has handed out.
fn seat_list(
    list: Vec<WindowInfo>,
    limit: u32,
    dial_order: u32,
    favorites: &[String],
    seat_order: &[String],
    bound_marks: &[String],
    parked: &[String],
    explicit: &[isize],
) -> Vec<WindowInfo> {
    let limit = (limit as usize).clamp(
        crate::settings::WHEEL_LIMIT_MIN as usize,
        crate::settings::WHEEL_LIMIT_MAX as usize,
    );
    // One fingerprint per window, read once. The old recency cut rebuilt these
    // strings inside every comparison and every "is this app already seated"
    // check, which was the largest avoidable cost of opening the dial.
    let marks: Vec<String> = list.iter().map(fingerprint).collect();
    let mut by_mark: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, mark) in marks.iter().enumerate() {
        by_mark.entry(mark.as_str()).or_default().push(i);
    }
    let mut taken = vec![false; list.len()];
    // `dial_order == 1` asks for the favourites list in front. It is a stable
    // sort of the remembered order, never a new arrangement invented from the
    // desktop's current stack.
    let mut plan: Vec<&str> = seat_order.iter().map(String::as_str).collect();
    if dial_order > 0 {
        plan.sort_by_key(|mark| favorites.iter().position(|f| f == mark).unwrap_or(usize::MAX));
    }
    // The limit counts *places*, not windows: a place whose window was taken
    // off or has closed stays empty, and the next window in the order is not
    // pulled forward into it. Counting seated windows instead was exactly the
    // backfill bug: parking one seat let the next candidate slide in. A window
    // the user bound by hand is exempt from the cut - an explicit seat is not
    // part of the automatic selection the limit sizes.
    let bound_marks: BTreeSet<&str> = bound_marks.iter().map(String::as_str).collect();
    let parked: BTreeSet<&str> = parked.iter().map(String::as_str).collect();
    let named: BTreeSet<isize> = explicit.iter().copied().filter(|hwnd| *hwnd != 0).collect();
    let mut seated: Vec<WindowInfo> = Vec::with_capacity(plan.len().min(list.len()));
    for (place, mark) in plan.into_iter().enumerate() {
        if place >= limit && !bound_marks.contains(mark) {
            continue;
        }
        if parked.contains(mark) {
            continue;
        }
        // Once the user has named windows of an application by hand, its place
        // belongs to those windows and not to whichever look-alike the desktop
        // happens to stack first. This is also what stops a taken-off window
        // from being replaced by its sibling: no other window fills the place.
        let named_here = || {
            list.iter()
                .enumerate()
                .find(|(i, info)| !taken[*i] && named.contains(&info.hwnd) && marks[*i] == mark)
                .map(|(i, _)| i)
        };
        let at = named_here().or_else(|| {
            by_mark
                .get(mark)
                .and_then(|at| at.iter().find(|&&i| !taken[i]))
                .copied()
        });
        let Some(at) = at else {
            continue;
        };
        taken[at] = true;
        seated.push(list[at].clone());
    }
    // Windows the user named by hand seat after the remembered plan, each in
    // its own sector: the handle points at exactly the window that was clicked,
    // not whichever look-alike happens to sit on top of the desktop stack.
    // They are exempt from the limit and from a parked fingerprint, and go last
    // so they never shift the sectors the plan already laid out.
    for &hwnd in explicit {
        if hwnd == 0 {
            continue;
        }
        if let Some(at) = list.iter().position(|info| info.hwnd == hwnd) {
            if taken[at] {
                continue;
            }
            taken[at] = true;
            seated.push(list[at].clone());
        }
    }
    seated
}

/// A still frame of the dial, built from the settings without opening it.
///
/// The preview must show what the dial would show, so it goes through the same
/// list, fingerprints, favourites and seating plan the live wheel uses.
/// The only difference is that nothing here touches the desktop.
#[derive(Clone, Debug)]
pub struct DialSnapshot {
    pub seats: Vec<PreviewSeat>,
    pub geometry: Geometry,
    pub bounds: Vec<shaiwin_core::wheel::Ring>,
    pub seam: f32,
    pub theme: u32,
    /// How loud the accent colours are, 0 soft to 2 vivid: structure is the
    /// theme's, this is the only thing the row moves.
    pub accent: u32,
}

/// The desk as the dial sees it: every window that can be switched to, less the
/// excluded ones, and the window in front. `list_switchable` walks every top
/// level window and reads a title from each, which is cheap once and not cheap
/// sixty times a second - so the settings page reads this when it opens, when
/// the rows move, and when the desk is likely to have changed, and rebuilds the
/// picture from the copy it already has.
pub fn desk(settings: &crate::settings::Settings) -> (Vec<WindowInfo>, Option<isize>) {
    let pid = std::process::id();
    let list = sift(
        window::list_switchable(pid),
        |info| settings.is_excluded(&info.exe),
        |hwnd| settings.is_off_dial(hwnd),
    );
    (list, window::foreground().map(window::raw_of))
}

/// The handles the dial would seat from a desk that has already been read.
/// Same pipeline as [`snapshot_of`] - excluded, taken-off, limit, bound seats,
/// order and favourites - without building the pixels. The panel uses it to
/// tell an on-dial row from a candidate without opening the dial, and it answers
/// in handles rather than fingerprints: several windows of one app share a
/// fingerprint, and each can hold a sector of its own.
pub fn seated_hwnds(settings: &crate::settings::Settings, list: &[WindowInfo]) -> Vec<isize> {
    let mut list: Vec<WindowInfo> = sift(
        list.to_vec(),
        |info| settings.is_excluded(&info.exe),
        |hwnd| settings.is_off_dial(hwnd),
    );
    list = seat_list(
        list,
        settings.wheel_limit,
        settings.dial_order,
        &settings.favorites,
        &settings.seat_order,
        &settings.bound,
        &settings.parked,
        &settings.seat_hwnds,
    );
    list.iter().map(|info| info.hwnd).collect()
}

/// The snapshot of a desk that has already been read. Everything here is
/// arithmetic over the list - no window is asked anything - which is what makes
/// a stepper click cost a redraw rather than a walk of the desktop.
pub fn snapshot_of(
    settings: &crate::settings::Settings,
    mut list: Vec<WindowInfo>,
    front: Option<isize>,
) -> DialSnapshot {
    list = sift(list, |_| false, |hwnd| settings.is_off_dial(hwnd));
    list = seat_list(
        list,
        settings.wheel_limit,
        settings.dial_order,
        &settings.favorites,
        &settings.seat_order,
        &settings.bound,
        &settings.parked,
        &settings.seat_hwnds,
    );
    // The foreground window used to be pulled out and pushed to the back here,
    // so that 12 o'clock was always "the previous window". That rotated every
    // sector on every opening and undid the remembered order; `current` below
    // still marks the window in front without moving anyone.
    let marks: Vec<String> = list.iter().map(fingerprint).collect();
    // One colour per seat, with duplicates resolved across the whole dial. A
    // hand-picked colour overrides the auto one. The user also picks a scheme:
    // 0 = auto palette + dedup, 1 = Hawkeye red/gold/violet/blue/green, 2 =
    // Classic cobalt/teal/violet/orange/rose. A scheme is a preset, not auto:
    // it hands colours in fixed order to the windows at the front of the dial
    // (favourite rank 0 first, then rank 1, then the rest), so the user decides
    // which ladder their most-used window lives on.
    let (scheme_cols, scheme_weights) = match settings.colour_scheme {
        1 => (
            shaiwin_core::theme::SCHEME_HAWKEYE_COLOURS.as_slice(),
            shaiwin_core::theme::SCHEME_HAWKEYE_WEIGHTS.as_slice(),
        ),
        2 => (
            shaiwin_core::theme::SCHEME_CLASSIC_COLOURS.as_slice(),
            shaiwin_core::theme::SCHEME_CLASSIC_WEIGHTS.as_slice(),
        ),
        _ => (&[][..] as &[Rgb], &[][..] as &[f32]),
    };
    let auto_accents: Vec<Rgb> = if settings.colour_scheme == 0 {
        // Scheme 0 is the quiet default: one grey for every window, so the
        // only colour on the dial is a colour the user picked by hand.
        vec![DEFAULT_SECTOR_COLOUR; marks.len()]
    } else {
        marks
            .iter()
            .enumerate()
            .map(|(i, _)| scheme_cols[i % scheme_cols.len()])
            .collect()
    };
    // Weights mirror the colours only when a scheme is chosen: with scheme 0
    // every slice starts even and only hand-picked sizes move them apart. With
    // a scheme, the colours and sizes descend together so rank 0 is both the
    // loudest colour and the widest slice.
    let scheme_weights_opt: Vec<Option<f32>> = if settings.colour_scheme == 0 {
        vec![None; marks.len()]
    } else {
        marks
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let w = scheme_weights[i % scheme_weights.len()];
                if (w - 1.0).abs() > 0.01 {
                    Some(w)
                } else {
                    None
                }
            })
            .collect()
    };
    let entries: Vec<dial::Entry> = marks
        .iter()
        .enumerate()
        .map(|(id, mark)| {
            // A hand-picked weight trumps the scheme suggestion; everything else
            // falls back to whatever the scheme handed the slot, which is `None`
            // when scheme is 0 so all sectors start even.
            let pin_w = if let Some(w) = settings.custom_weight_for(mark) {
                if (w - 1.0).abs() > 0.01 {
                    Some(w)
                } else {
                    None
                }
            } else {
                scheme_weights_opt[id]
            };
            dial::Entry {
                id,
                pinned_at: None,
                pinned_ring: None,
                pinned_weight: pin_w,
                favourite_at: settings.favorite_rank(mark),
            }
        })
        .collect();
    let plan = Dial::build(&entries);
    let mut seats = Vec::with_capacity(plan.slots().len());
    for slot in plan.slots() {
        let Some(id) = slot.occupant else { continue };
        let info = &list[id];
        let tag = settings.tag_for(&marks[id]);
        let (written, app) = crate::i18n::wheel_parts(&info.title, &info.app_name());
        // What the person named the window is what the slice has to say. The
        // window's own title is only the fallback for a window nobody has named
        // yet; reading it first is why a label could be saved and still never
        // appear on the dial.
        let head = tag
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or(written);
        // A hand-picked colour must arrive as it was chosen: dial_accent tints
        // everything to match the theme's paper/neon/glass, which is fine for
        // the palette defaults but a lie when the user picked something precise.
        let custom_base = settings.colour_for(&marks[id]);
        let base = custom_base.unwrap_or(auto_accents[id]);
        // The quiet default arrives as it is too: tinting the grey towards a
        // theme's family would put colour back on a dial the user asked to keep
        // quiet, and it would drift apart across themes for no reason.
        let tinted = if custom_base.is_some() || settings.colour_scheme == 0 {
            base
        } else {
            dial_accent(base, settings.wheel_theme, settings.wheel_accent)
        };
        seats.push(PreviewSeat {
            ring: slot.ring,
            bearing: (slot.bearing + settings.wheel_rotate).rem_euclid(360.0),
            span: slot.span,
            base_accent: base,
            accent: tinted,
            glyph: sector_mark(tag, &crate::i18n::wheel_glyph(&app)),
            head,
            fingerprint: marks[id].clone(),
            hwnd: info.hwnd,
            pinned: false,
            current: info.hwnd == front.unwrap_or(0),
            hot: false,
        });
    }
    let geometry = Geometry::new(settings.wheel_radius, settings.wheel_dead);
    DialSnapshot {
        bounds: geometry.ring_bounds(plan.rings()),
        seats,
        geometry,
        seam: settings.wheel_seam,
        theme: settings.wheel_theme,
        accent: settings.wheel_accent,
    }
}

/// Why the dial would refuse a window a sector. The settings page asks this
/// after a seat attempt has already been written and read back, so the row can
/// go back to "not on the dial" and say which of the five walls it hit rather
/// than showing a button that promises a sector the dial does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeatProblem {
    /// No window of this application is running; a seat for a shut window is a
    /// promise held by the favourites list, not something to place now.
    NotRunning,
    /// The application is struck off the dial.
    Excluded,
    /// No place in the remembered order: the management list is where this
    /// window waits until somebody seats it by hand.
    Placement,
}

/// Asks the desk why `mark` did not get a seat, going through exactly the
/// pipeline that hand out seats - [`window::list_switchable`], the excluded and
/// off-dial filters, and [`seat_list`] - so the answer and the picture cannot
/// disagree. `None` means the window *is* seated; the caller only asks after
/// reading the seat list and finding it missing.
pub fn seat_problem(
    settings: &crate::settings::Settings,
    mark: &str,
    hwnd: isize,
) -> Option<SeatProblem> {
    seat_problem_of(
        settings,
        window::list_switchable(std::process::id()),
        mark,
        hwnd,
    )
}

/// The arithmetic of [`seat_problem`] over a desk that has already been read.
fn seat_problem_of(
    settings: &crate::settings::Settings,
    live: Vec<WindowInfo>,
    mark: &str,
    hwnd: isize,
) -> Option<SeatProblem> {
    let excluded = live
        .iter()
        .any(|info| fingerprint(info) == mark && settings.is_excluded(&info.exe));
    let visible: Vec<WindowInfo> = live
        .iter()
        .filter(|info| {
            !settings.is_excluded(&info.exe) && !settings.is_off_dial(info.hwnd)
        })
        .cloned()
        .collect();
    if !visible.iter().any(|info| fingerprint(info) == mark) {
        return Some(if excluded {
            SeatProblem::Excluded
        } else {
            SeatProblem::NotRunning
        });
    }
    if !visible.iter().any(|info| info.hwnd == hwnd) {
        return Some(SeatProblem::NotRunning);
    }
    let chosen = seat_list(
        visible,
        settings.wheel_limit,
        settings.dial_order,
        &settings.favorites,
        &settings.seat_order,
        &settings.bound,
        &settings.parked,
        &settings.seat_hwnds,
    );
    if chosen.iter().any(|info| info.hwnd == hwnd) {
        return None;
    }
    // The window is on the desk but holds no sector: either its fingerprint is
    // parked or it was never given a place. Both are answered the same way -
    // the management list is where it waits.
    Some(SeatProblem::Placement)
}

impl DialSnapshot {
    /// The window under a click inside the preview, if any. `x`/`y` are relative
    /// to the preview's top-left corner, in device pixels, and `scale` is the
    /// number that turned the dial's own pixels into the preview's.
    pub fn hit(&self, x: f32, y: f32, scale: f32, side: u32) -> Option<&PreviewSeat> {
        let c = side as f32 / 2.0;
        let dx = (x - c) / scale;
        let dy = (y - c) / scale;
        let d = (dx * dx + dy * dy).sqrt();
        if d < self.geometry.hub() {
            return None;
        }
        let bearing = (dy.atan2(dx).to_degrees() + 90.0).rem_euclid(360.0);
        let ring = self.bounds.iter().position(|b| d >= b.lo && d <= b.hi)?;
        self.seats.iter().find(|seat| {
            // The distance from the sector's centre has to be taken as an
            // angle, not as a bearing offset: `(seat - point)` alone wraps a
            // point just on the clockwise side to ~356 degrees and drops it.
            // Half of every sector then answered "no window here" - which is
            // why a click on the left flank of a slice cleared the selection
            // instead of opening its label, and why some presses meant for a
            // drag never grabbed a seat at all.
            let delta = (bearing - seat.bearing + 180.0).rem_euclid(360.0) - 180.0;
            seat.ring == ring && delta.abs() <= seat.span / 2.0
        })
    }
}

/// The still half of the picture: shadow and glass. Made once per size and theme
/// and copied back under every frame of a drag.
pub fn render_snapshot_base(
    snapshot: &DialSnapshot,
    side: u32,
    scale: f32,
) -> Option<Pixmap> {
    let mut canvas = Pixmap::new(side, side)?;
    let empty: Vec<WheelSector> = Vec::new();
    let view = preview_view(snapshot, scale, &empty, None);
    PAINTER.with(|cell| {
        let mut cell = cell.borrow_mut();
        let painter = cell.get_or_insert_with(Painter::new);
        painter.compose_base_into(&mut canvas, &view);
    });
    Some(canvas)
}

/// One frame of the moving half: the sectors at the bearings in `seats`, drawn
/// over `base`. `out` is the same square the base is, reused by the caller so a
/// drag does not allocate a megabyte sixty times a second.
pub fn render_snapshot_seats(
    base: &Pixmap,
    snapshot: &DialSnapshot,
    seats: &[PreviewSeat],
    side: u32,
    scale: f32,
    out: &mut Pixmap,
) {
    if out.width() != side || out.height() != side || out.width() == 0 {
        return;
    }
    let pixels = base.data();
    out.data_mut().copy_from_slice(pixels);
    let sectors = sectors_of(seats);
    let note = if sectors.is_empty() {
        Some(crate::i18n::t("dial_preview_empty"))
    } else {
        None
    };
    let view = preview_view(snapshot, scale, &sectors, note);
    PAINTER.with(|cell| {
        let mut cell = cell.borrow_mut();
        let painter = cell.get_or_insert_with(Painter::new);
        painter.compose_seats_into(out, &view);
    });
}

fn sectors_of(seats: &[PreviewSeat]) -> Vec<WheelSector> {
    seats
        .iter()
        .map(|seat| WheelSector {
            ring: seat.ring,
            bearing: seat.bearing,
            span: seat.span,
            accent: seat.accent,
            glyph: seat.glyph.clone(),
            hwnd: seat.hwnd,
            name: seat.head.clone(),
            hot: seat.hot,
            pinned: seat.pinned,
            current: seat.current,
            pop: 0.0,
            appear: 1.0,
        })
        .collect()
}

fn preview_view<'a>(
    snapshot: &'a DialSnapshot,
    scale: f32,
    sectors: &'a [WheelSector],
    note: Option<&'a str>,
) -> WheelView<'a> {
    WheelView {
        scale,
        radius: snapshot.geometry.radius,
        hub: snapshot.geometry.hub(),
        bounds: &snapshot.bounds,
        sectors,
        preview: None,
        note,
        cancelling: false,
        open: 1.0,
        seam: snapshot.seam,
        theme: snapshot.theme,
    }
}

/// Draws one theme at the size of a filmstrip cell. It goes through the same
/// `render_snapshot_base`/`render_snapshot_seats` the preview and the live dial
/// use, so a thumbnail cannot promise a look the dial refuses: only the theme
/// and the accent row are overridden, and the sector colours are re-derived
/// from the application's own accent.
pub fn render_thumb(
    snapshot: &DialSnapshot,
    theme: u32,
    accent: u32,
    side: u32,
) -> Option<Pixmap> {
    if side < 8 {
        return None;
    }
    let mut still = snapshot.clone();
    still.theme = theme;
    still.accent = accent;
    for seat in &mut still.seats {
        seat.accent = dial_accent(seat.base_accent, theme, accent);
        seat.hot = false;
    }
    let rings = still.bounds.len().max(1);
    let extent = still.geometry.extent(rings).max(1.0);
    let scale = side as f32 / (2.0 * extent);
    let base = render_snapshot_base(&still, side, scale)?;
    let mut out = base.clone();
    render_snapshot_seats(&base, &still, &still.seats, side, scale, &mut out);
    Some(out)
}

/// How far each theme pulls an application's accent, by accent row: soft,
/// medium, vivid. Rows are the theme ids; the value is the amount mixed towards
/// the theme's own family (grey, white, paper, cyan). Only the sector colour
/// reads this - plate, seams, strokes and shadows are structure and never move.
const ACCENT_MIX: [[f32; 3]; 7] = [
    [0.62, 0.55, 0.66],
    [0.00, 0.00, 0.10],
    [0.08, 0.18, 0.28],
    [0.10, 0.16, 0.26],
    [0.00, 0.00, 0.14],
    [0.30, 0.16, 0.00],
    [0.45, 0.35, 0.25],
];

/// A sector's colour under the chosen theme and accent row. The application's
/// own accent is the default; outline pulls every sector towards one family so
/// the dial reads as one object instead of a bag of brands, glow and neon pull
/// each accent towards white, paper warms it towards the sheet and the site
/// pulls towards its cyan. Blending is a plain linear mix.
fn dial_accent(base: Rgb, theme: u32, accent: u32) -> Rgb {
    let level = (accent as usize).min(2);
    let t = ACCENT_MIX[(theme as usize).min(6)][level];
    let towards = match theme {
        0 => Rgb::new(150, 160, 176),
        3 => Rgb::new(232, 220, 200),
        5 => Rgb::new(248, 245, 238),
        6 => Rgb::new(0, 229, 255),
        _ => Rgb::new(255, 255, 255),
    };
    let mixed = blend(base, towards, t);
    // Neon at its loudest is a step brighter than the window's own accent, not
    // just whiter: the tubes have to read as lit.
    if theme == 4 && level == 2 {
        blend(mixed, Rgb::new(255, 255, 255), 0.14)
    } else {
        mixed
    }
}

/// Linear blend of two colours, `t` running 0 (a) to 1 (b).
fn blend(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let Rgb(ar, ag, ab) = a;
    let Rgb(br, bg, bb) = b;
    let t = t.clamp(0.0, 1.0);
    Rgb(
        (ar as f32 * (1.0 - t) + br as f32 * t).round() as u8,
        (ag as f32 * (1.0 - t) + bg as f32 * t).round() as u8,
        (ab as f32 * (1.0 - t) + bb as f32 * t).round() as u8,
    )
}

/// Reads the desktop and seats the windows on the dial.
///
/// The list comes out of `EnumWindows` in z-order, but the dial no longer seats
/// from it: the desk's order changes every time a window is used, and a wheel
/// whose sectors move after one pick is a wheel nobody can learn. The remembered
/// `seat_order` is the arrangement; z-order only decides which window of one
/// application holds a place.
fn build() {
    let started = Instant::now();
    let pid = std::process::id();
    let theme = with(|w| w.theme);
    let accent = with(|w| w.accent);
    let front = window::foreground().map(window::raw_of);
    // Struck-off applications go before the seating and before the count, so a
    // window the user took off the dial neither takes a sector nor leaves a ring
    // wider than it needs to be. The windows the panel took off by name go
    // before the seating too: the decision has to outrank recency or the next
    // build would quietly undo it.
    let struck = excluded()
        .lock()
        .map(|held| held.clone())
        .unwrap_or_default();
    let taken_off = off_dial()
        .lock()
        .map(|held| held.clone())
        .unwrap_or_default();
    let mut list = sift(
        window::list_switchable(pid),
        |info| struck.iter().any(|exe| exe == &info.exe),
        |hwnd| taken_off.contains(&hwnd),
    );
    let all = list.len();
    let (limit, order) = with(|w| (w.limit, w.order));
    let favourites = favorites()
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default();
    let remembered_order = seat_order()
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default();
    let parked_marks = parked()
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default();
    let bound_marks = bound()
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default();
    let named_windows = seat_windows()
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default();
    list = seat_list(
        list,
        limit,
        order,
        &favourites,
        &remembered_order,
        &bound_marks,
        &parked_marks,
        &named_windows,
    );
    // No rotation of the foreground window: the remembered order is the point.
    // `current` is still marked on the seat from `front` below.
    crate::log!(
        "wheel: showing {} of {} window(s), limit={limit} order={order}, front={} on_current={}",
        list.len(),
        all,
        front
            .map(|f| format!("0x{f:x}"))
            .unwrap_or_else(|| "none".to_string()),
        front
            .map(|f| window::on_current_desktop(HWND(f as *mut core::ffi::c_void)).to_string())
            .unwrap_or_else(|| "-".to_string())
    );
    if list.is_empty() {
        crate::log!(
            "wheel: {} switchable window(s), so nothing to open",
            list.len()
        );
        with(|w| {
            w.open = false;
            w.dial_plan = None;
            w.seats.clear();
        });
        return;
    }
    if list.len() == 1 {
        // One window is not a switch in the old sense, but it is still the only
        // thing the user can pick on this desktop: a desktop that was just
        // switched to has one window on it, and refusing here is refusing to
        // draw a dial the user can see. It opens with a single sector.
        crate::log!(
            "wheel: only {} switchable window; the dial has one sector",
            list.len()
        );
    }
    let marks: Vec<String> = list.iter().map(fingerprint).collect();
    // The live dial mirrors the settings page exactly: a user-chosen scheme hands
    // colours and sizes in fixed order, or scheme 0 falls back to the auto palette
    // with no scheme-driven size changes.
    let scheme = colour_scheme().load(Ordering::Relaxed);
    let (scheme_cols, scheme_weights) = match scheme {
        1 => (
            shaiwin_core::theme::SCHEME_HAWKEYE_COLOURS.as_slice(),
            shaiwin_core::theme::SCHEME_HAWKEYE_WEIGHTS.as_slice(),
        ),
        2 => (
            shaiwin_core::theme::SCHEME_CLASSIC_COLOURS.as_slice(),
            shaiwin_core::theme::SCHEME_CLASSIC_WEIGHTS.as_slice(),
        ),
        _ => (&[][..] as &[Rgb], &[][..] as &[f32]),
    };
    let auto_accents: Vec<Rgb> = if scheme == 0 {
        // Scheme 0 is the quiet default: one grey for every window, so the
        // only colour on the dial is a colour the user picked by hand.
        vec![DEFAULT_SECTOR_COLOUR; marks.len()]
    } else {
        marks
            .iter()
            .enumerate()
            .map(|(i, _)| scheme_cols[i % scheme_cols.len()])
            .collect()
    };
    let scheme_weights_opt: Vec<Option<f32>> = if scheme == 0 {
        vec![None; marks.len()]
    } else {
        marks
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let w = scheme_weights[i % scheme_weights.len()];
                if (w - 1.0).abs() > 0.01 {
                    Some(w)
                } else {
                    None
                }
            })
            .collect()
    };
    let entries: Vec<dial::Entry> = marks
        .iter()
        .enumerate()
        .map(|(id, mark)| {
            // A hand-picked weight trumps the scheme suggestion. Only hand-picked
            // weights and scheme-driven weights differ from `None`, which the dial
            // builder understands as "1.0 even".
            let pin_w = if let Some(w) = weight_for(mark) {
                if (w - 1.0).abs() > 0.01 {
                    Some(w)
                } else {
                    None
                }
            } else {
                scheme_weights_opt[id]
            };
            dial::Entry {
                id,
                pinned_at: None,
                pinned_ring: None,
                pinned_weight: pin_w,
                favourite_at: favorite_rank(mark),
            }
        })
        .collect();
    let plan = Dial::build(&entries);
    let mut seats = Vec::with_capacity(plan.slots().len());
    for slot in plan.slots() {
        let Some(id) = slot.occupant else { continue };
        let info = &list[id];
        let tag = tag_for(&marks[id]);
        let (written, app) = crate::i18n::wheel_parts(&info.title, &info.app_name());
        let head = tag
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or(written);
        let custom_base = color_for(&marks[id]);
        let base = custom_base.unwrap_or(auto_accents[id]);
        // The quiet default arrives as it is too: tinting the grey towards a
        // theme's family would put colour back on a dial the user asked to keep
        // quiet, and it would drift apart across themes for no reason.
        let tinted = if custom_base.is_some() || scheme == 0 {
            base
        } else {
            dial_accent(base, theme, accent)
        };
        seats.push(Seat {
            id,
            accent: tinted,
            ring: slot.ring,
            pos: slot.pos,
            bearing: slot.bearing,
            span: slot.span,
            current: info.hwnd == front.unwrap_or(0),
            // The pin feature is cut, so no sector ever draws the nail. The
            // flag and the drawing behind it stay for a later use.
            pinned: false,
            pop: 0.0,
            head,
            glyph: sector_mark(tag.as_deref(), &crate::i18n::wheel_glyph(&app)),
            info: info.clone(),
        });
    }
    with(|w| {
        w.dial_plan = Some(plan);
        w.seats = seats;
        w.hot = usize::MAX;
        w.veiled = usize::MAX;
        w.located = None;
        w.painted_at = Instant::now();
        // A fresh gesture starts with no handle held: the grab from the last
        // gesture would turn the dial the moment the pointer crossed the rim.
        w.rotate_grab = None;
        w.painted_rotate = w.rotate;
        w.rotate_committed = w.rotate;
    });
    place();
    // A window above this app's integrity is listed because it is a window the
    // user can see; it just cannot be raised. Saying which ones in the log is
    // what lets a user check why a pick ends with a balloon instead of a switch.
    let needing_admin: Vec<&str> = list
        .iter()
        .filter(|info| !info.accessible)
        .map(|info| info.exe.as_str())
        .collect();
    if !needing_admin.is_empty() {
        crate::log!(
            "wheel: listed for viewing only, needs administrator: {}",
            needing_admin.join(", ")
        );
    }
    crate::log!(
        "wheel: build {} window(s) in {:.1}ms",
        list.len(),
        started.elapsed().as_secs_f64() * 1000.0
    );
}

/// Works out where the dial goes: on the press, and if that would crop it, moved -
/// never clipped. The reach is scaled to the monitor the press was on, because
/// `wheel_centre` answers in the same unit it is given and the window has to be
/// positioned in device pixels.
fn place() {
    with(|w| {
        let press = w.press;
        w.monitor = monitor::at_point(press.0, press.1);
        if w.monitor.work.width() <= 0 || w.monitor.work.height() <= 0 {
            // A press that does not map onto a work area is not a place the dial
            // can be drawn: rather than build a zero-sized ring, fall back to the
            // primary screen and say so.
            crate::log!(
                "wheel: press ({},{}) has no work area, falling back to the primary screen",
                press.0,
                press.1
            );
            w.monitor = monitor::primary();
        }
        let rings = w.dial_plan.as_ref().map(|d| d.rings()).unwrap_or(1);
        let monitor_scale = w.monitor.scale;
        let work = w.monitor.work;
        let raw = w.geometry.extent(rings) * 2.0 * monitor_scale;
        let fit = (work.width() as f32 / raw)
            .min(work.height() as f32 / raw)
            .clamp(0.30, 1.0);
        w.view_scale = monitor_scale * fit;
        let extent = w.geometry.extent(rings) * w.view_scale;
        let area = shaiwin_core::hit::Rect::new(
            work.left as f32,
            work.top as f32,
            work.width() as f32,
            work.height() as f32,
        );
        let (cx, cy, moved) = dial::wheel_centre(press.0 as f32, press.1 as f32, extent, area);
        if moved {
            crate::log!("wheel: near an edge, so the whole dial moved to stay whole");
        }
        if fit < 1.0 {
            crate::log!(
                "wheel: {rings} ring(s) need {raw:.0}px, so the dial is shown at {:.0}% to fit",
                fit * 100.0
            );
        }
        w.centre = (cx, cy);
    });
}

/// Puts both layers where they belong and shows them.
fn show_layers() {
    let (side, cx, cy, bounds, scale, rings, handle) = with(|w| {
        let rings = w.dial_plan.as_ref().map(|d| d.rings()).unwrap_or(1);
        let side = ((w.geometry.extent(rings) + crate::render::DISC_SHADOW_PAD)
            * 2.0
            * w.view_scale)
            .round() as i32;
        (
            side,
            w.centre.0,
            w.centre.1,
            w.monitor.bounds,
            w.view_scale,
            rings,
            w.monitor.handle,
        )
    });
    if side < 2 {
        return;
    }
    // The dial's own measure, in the unit the window is positioned in. A wheel
    // that is the wrong size on someone's 175% screen is otherwise a report about
    // a picture nobody can capture.
    crate::log!(
        "wheel: {side}px square at scale {:.2}, {rings} ring(s), centred at ({:.0},{:.0}), monitor 0x{handle:x} veil=({},{},{},{})",
        scale,
        cx,
        cy,
        bounds.left,
        bounds.top,
        bounds.right,
        bounds.bottom
    );
    unsafe {
        let v = veil();
        if !v.is_invalid() {
            let _ = ShowWindow(v, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                v,
                Some(HWND_TOPMOST),
                bounds.left,
                bounds.top,
                bounds.width().max(1),
                bounds.height().max(1),
                SWP_NOACTIVATE,
            );
        }
        let d = disc();
        if !d.is_invalid() {
            let _ = ShowWindow(d, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                d,
                Some(HWND_TOPMOST),
                (cx - side as f32 / 2.0).round() as i32,
                (cy - side as f32 / 2.0).round() as i32,
                side,
                side,
                SWP_NOACTIVATE,
            );
            crate::log!(
                "wheel: disc shown at ({},{}) {}x{}",
                (cx - side as f32 / 2.0).round() as i32,
                (cy - side as f32 / 2.0).round() as i32,
                side,
                side
            );
        }
        paint_veil();
        paint_locator();
    }
    // `show_layers` also runs on a DPI or display change, with the pointer still
    // on a sector. Forget what the veil was composed for so the next frame
    // repaints both layers at the new scale instead of leaving a stale locator.
    with(|w| w.veiled = usize::MAX);
}

/// The displays changed arrangement under an open dial: the press point may now
/// belong to a different work area, so it is measured and put down again rather
/// than left hanging half off a screen that moved.
pub fn replaced() {
    if with(|w| w.open) {
        claim_current_desktop();
        place();
        show_layers();
        frame();
    }
}

/// One frame: what the pointer is on, whether the dim has to change, and the dial.
fn frame() {
    let started = Instant::now();
    let opened = with(|w| {
        if !w.open {
            return None;
        }
        w.read_aim();
        let now = Instant::now();
        let dt = (now - w.painted_at).as_secs_f32() * 1000.0 / POP_MS;
        w.painted_at = now;
        let hot = w.hot;
        let mut stepped = false;
        for seat in &mut w.seats {
            let lit = seat.id == hot;
            let before = seat.pop;
            seat.pop = if w.reduced {
                // The same row that stops the dial unfurling stops the sector
                // leaning out towards the pointer: it is either lit or not.
                f32::from(lit)
            } else {
                let step = dt.clamp(0.0, 1.0);
                if lit {
                    (seat.pop + step).min(1.0)
                } else {
                    (seat.pop - step).max(0.0)
                }
            };
            stepped |= (seat.pop - before).abs() > 0.0001;
        }
        // Whether this frame has anything left to draw. A sector still leaning or
        // still settling, or the dial itself still unfurling, means yes; a dial at
        // rest with the pointer on the same sector means the last frame is still
        // right, and composing another one is work nobody asked for. This is what
        // keeps a resting wheel off the CPU - it used to redraw the whole disc
        // sixty times a second whether or not anything had moved.
        let settling = w.seats.iter().any(|seat| {
            let target = if seat.id == hot { 1.0 } else { 0.0 };
            (seat.pop - target).abs() > 0.001
        });
        let unfolding = !w.reduced
            && w.open_ms > 0.0
            && (now - w.opened_at).as_secs_f32() * 1000.0 < w.open_ms;
        // A turn is its own reason to repaint: no sector has to be leaning for a
        // drag to have moved every bearing on the dial.
        let turned = (w.rotate - w.painted_rotate).abs() > 0.01;
        if turned {
            w.painted_rotate = w.rotate;
        }
        Some(stepped || settling || unfolding || turned)
    });
    let Some(moving) = opened else {
        return;
    };
    // The dim is the expensive one: a screen-sized layered update, asked for only
    // when the previewed window is a different window than the last frame's.
    let changed = with(|w| w.hot != w.veiled);
    if changed {
        // The dim and the hole follow the pointer, and the hole is cut over the
        // preview's own rectangle - the window itself is never touched.
        paint_veil();
        // A hovered window on another screen gets its own dim, outline and name
        // over there; the wheel itself stays exactly where it was opened.
        paint_locator();
    }
    // Nothing moved and nothing is settling: the disc on screen is already this
    // frame. Skipping the compose and the layered update is the single largest
    // saving on the hover path, because the pointer spends most of its time
    // inside one sector, not crossing seams.
    if changed || moving {
        paint_disc();
    }
    note_frame_cost(started.elapsed().as_secs_f32() * 1000.0, changed || moving);
}

/// The wheel's own stopwatch, kept out of the state because it is bookkeeping,
/// not behaviour. Every so often the average and the worst frame go to the log,
/// which is the only honest way to say whether the pointer is smooth on a
/// machine that is not this one.
fn note_frame_cost(ms: f32, painted: bool) {
    FRAME_METER.with(|cell| {
        let mut meter = cell.borrow_mut();
        meter.0 += 1;
        if painted {
            meter.1 += 1;
            meter.2 += ms;
            meter.3 = meter.3.max(ms);
        }
        if meter.0 >= 240 {
            let avg = if meter.1 > 0 { meter.2 / meter.1 as f32 } else { 0.0 };
            crate::log!(
                "wheel frame: frames={} painted={} avg={:.2}ms max={:.2}ms fps~{}",
                meter.0,
                meter.1,
                avg,
                meter.3,
                if avg > 0.0 { (1000.0 / avg).round() } else { 0.0 }
            );
            *meter = (0, 0, 0.0, 0.0);
        }
    });
}

// MARK: - Painting

/// Draws the dial. Everything the painter needs is taken out of the state in one
//  lock, because the strings it draws belong to the seats and the frame must not
/// hold the lock while it rasterises.
fn paint_disc() {
    let Some(frame) = with(|w| {
        if !w.open || w.seats.is_empty() {
            return None;
        }
        let plan = w.dial_plan.as_ref()?;
        let scale = w.view_scale;
        // The dial's turn belongs to the painting, not to the seats: the seats
        // keep the plan's bearings so a drag can turn the wheel without rebuilding
        // it, and the hit test reads the same number back.
        let rotate = w.rotate;
        let now = Instant::now();
        // Reduced motion means the dial is simply there: `open` at 1.0 leaves no
        // unfurl to interpolate, so the rings and the seats arrive together.
        let open = if w.reduced || w.open_ms <= 0.0 {
            1.0
        } else {
            ((now - w.opened_at).as_secs_f32() * 1000.0 / w.open_ms).clamp(0.0, 1.0)
        };
        let bounds = w.geometry.ring_bounds(plan.rings());
        let sectors: Vec<WheelSector> = w
            .seats
            .iter()
            .map(|seat| {
                // Sectors arrive from the inside out and clockwise from 12, so a
                // three-ring dial unfurls instead of all of it appearing at once.
                let delay = 0.05 * seat.ring as f32 + 0.02 * seat.pos as f32;
                let appear = ((open - delay) / (1.0 - delay).max(0.3)).clamp(0.0, 1.0);
                WheelSector {
                    ring: seat.ring,
                    bearing: (seat.bearing + rotate).rem_euclid(360.0),
                    span: seat.span,
                    accent: seat.accent,
                    glyph: seat.glyph.clone(),
                    hwnd: seat.info.hwnd,
                    name: seat.head.clone(),
                    hot: seat.id == w.hot,
                    pinned: seat.pinned,
                    current: seat.current,
                    pop: seat.pop,
                    appear,
                }
            })
            .collect();
        let hot_seat = w.seat(w.hot).map(|seat| {
            (
                seat.id,
                seat.head.clone(),
                seat.current,
                seat.pinned,
                seat.accent,
                seat.info.hwnd,
            )
        });
        let preview = hot_seat.map(|(id, head, current, pinned, accent, hwnd)| {
            let minimized = match w.iconic_hot {
                Some((cached, answer)) if cached == id => answer,
                _ => {
                    let answer = window::iconic(window::hwnd_of(hwnd));
                    w.iconic_hot = Some((id, answer));
                    answer
                }
            };
            (head, current, pinned, minimized, accent)
        });
        // The hub only speaks this line when it has no window to show, which is
        // exactly when the pointer is where letting go means nothing: on the hub
        // itself, or in the seam between two sectors.
        let note = crate::i18n::wheel_cancel_hint();
        let side = ((w.geometry.extent(plan.rings()) + crate::render::DISC_SHADOW_PAD)
            * 2.0
            * scale)
            .round() as u32;
        Some(Frame {
            scale,
            radius: w.geometry.radius,
            hub: w.geometry.hub(),
            bounds,
            sectors,
            preview,
            note,
            cancelling: matches!(w.aim, Aim::Hub),
            open,
            seam: w.seam,
            theme: w.theme,
            side,
            origin: (
                (w.centre.0 - side as f32 / 2.0).round() as i32,
                (w.centre.1 - side as f32 / 2.0).round() as i32,
            ),
        })
    }) else {
        return;
    };
    // The canvas is reused between frames: allocating a full-monitor RGBA
    // bitmap on every 16ms tick is most of what made the unfurl stutter.
    // The hub's preview says the window's own title in its own face. The line under
    // it is not the application's name - the sector's initial and accent colour
    // already say that - but the one thing the wheel knows that the user cannot
    // see: whether this window is the one already in front.
    let preview = frame
        .preview
        .as_ref()
        .map(|(head, current, pinned, minimized, accent)| {
            WheelPreview {
                head: head.as_str(),
                sub: crate::i18n::wheel_sub(*current, *pinned, *minimized),
                // A picture of the window itself would have to come from
                // `PrintWindow`, which talks to the target process and can wait for as
                // long as that process is busy - which is exactly the moment a switcher
                // is being used. The dim does the showing instead: the lit rectangle is
                // the window, at its real size, on the real desktop.
                shot: None,
                accent: *accent,
            }
        });
    let view = WheelView {
        scale: frame.scale,
        radius: frame.radius,
        hub: frame.hub,
        bounds: &frame.bounds,
        sectors: &frame.sectors,
        preview,
        note: Some(frame.note),
        cancelling: frame.cancelling,
        open: frame.open,
        seam: frame.seam,
        theme: frame.theme,
    };
    DISC_CANVAS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let same = slot.as_ref().is_some_and(|(side, _)| *side == frame.side);
        if !same {
            match Pixmap::new(frame.side, frame.side) {
                Some(pixmap) => *slot = Some((frame.side, pixmap)),
                None => return,
            }
        }
        let Some((_, canvas)) = slot.as_mut() else {
            return;
        };
        // The still half of the dial is the same picture for every frame of a
        // hover, and it is the larger half: keep it and start each frame from a
        // copy instead of drawing the shadow and the glass again. It only has to
        // be made again when the size, the theme or the opening changes.
        let still_key = StillKey {
            side: frame.side,
            theme: frame.theme,
            radius: frame.radius.to_bits(),
            hub: frame.hub.to_bits(),
            outer: frame
                .bounds
                .last()
                .map(|b| b.hi)
                .unwrap_or(frame.radius)
                .to_bits(),
            open: frame.open.to_bits(),
        };
        DISC_STILL.with(|cell| {
            let mut cached = cell.borrow_mut();
            let fresh = !cached
                .as_ref()
                .is_some_and(|still| still.key == still_key && still.pixmap.width() == frame.side);
            if fresh {
                match Pixmap::new(frame.side, frame.side) {
                    Some(pixmap) => {
                        *cached = Some(Still {
                            key: still_key,
                            pixmap,
                        })
                    }
                    None => *cached = None,
                }
            }
            PAINTER.with(|cell| {
                let mut cell = cell.borrow_mut();
                let painter = cell.get_or_insert_with(Painter::new);
                let Some(still) = cached.as_mut() else {
                    // No room for the copy: compose the whole frame the old way
                    // rather than show a stale one.
                    painter.compose_wheel_into(canvas, &view);
                    return;
                };
                if fresh {
                    painter.compose_still_into(&mut still.pixmap, &view);
                }
                canvas.data_mut().copy_from_slice(still.pixmap.data());
                // Themes older than paper and neon paint the moving half once
                // more in their base pass; `compose_base_into` keeps that order,
                // and so does this frame.
                if frame.theme < 4 {
                    painter.compose_seats_into(canvas, &view);
                }
                painter.compose_seats_into(canvas, &view);
            });
        });
        DISC_DIB.with(|cell| {
            let mut dib = cell.borrow_mut();
            present_layer(disc(), canvas, &mut dib, frame.origin, &mut false);
        });
    });
}

/// What the still half was made from. The opening animation moves `open` on
/// every frame of the unfurl, so the cache misses until the dial has arrived and
/// then holds for as long as the hovering does.
#[derive(PartialEq)]
struct StillKey {
    side: u32,
    theme: u32,
    radius: u32,
    hub: u32,
    outer: u32,
    open: u32,
}

struct Still {
    key: StillKey,
    pixmap: Pixmap,
}

/// Everything one frame of the dial needs, held outside the state lock.
struct Frame {
    scale: f32,
    radius: f32,
    hub: f32,
    bounds: Vec<shaiwin_core::wheel::Ring>,
    sectors: Vec<WheelSector>,
    /// The window under the pointer: its title, whether it is already in front,
    /// whether a direction remembers it, whether it is off on the taskbar, and the
    /// colour of the application it belongs to. The hub's own line is not stored
    /// here — it is one of four words read from those three booleans, at the moment
    /// the frame is drawn.
    preview: Option<(String, bool, bool, bool, Rgb)>,
    note: &'static str,
    cancelling: bool,
    open: f32,
    seam: f32,
    theme: u32,
    side: u32,
    origin: (i32, i32),
}

/// Registers the window under the pointer with the compositor and puts the
/// thumbnail into the hole the veil cut for it. This is what makes the preview
/// show the window itself rather than whatever happens to cover it.
fn show_preview(source_raw: isize, rect: shaiwin_platform_win::Rect) -> bool {
    let dest = preview();
    if dest.is_invalid() || rect.width() <= 0 || rect.height() <= 0 {
        return false;
    }
    // The rim is stroked on the hole itself, so the picture stops just inside it.
    let left = rect.left + PREVIEW_INSET;
    let top = rect.top + PREVIEW_INSET;
    let width = rect.width() - PREVIEW_INSET * 2;
    let height = rect.height() - PREVIEW_INSET * 2;
    if width <= 0 || height <= 0 {
        hide_preview();
        return false;
    }
    let existing = PREVIEW.with(|cell| cell.borrow().as_ref().map(|p| (p.id, p.source)));
    let id = match existing {
        Some((id, source)) if source == source_raw => id,
        _ => {
            hide_preview();
            let source = window::hwnd_of(source_raw);
            let Some(id) = thumb::register(dest, source) else {
                crate::log!("wheel: the compositor refused a thumbnail of {source_raw:#x}");
                return false;
            };
            crate::log!("wheel: preview thumbnail registered for {source_raw:#x}");
            PREVIEW.with(|cell| {
                *cell.borrow_mut() = Some(Preview {
                    id,
                    source: source_raw,
                });
            });
            id
        }
    };
    if !thumb::update(id, width, height, 255) {
        return false;
    }
    unsafe {
        let _ = SetWindowPos(
            dest,
            Some(disc()),
            left,
            top,
            width,
            height,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
    true
}

fn hide_preview() {
    if let Some(held) = PREVIEW.with(|cell| cell.borrow_mut().take()) {
        thumb::unregister(held.id);
    }
    let dest = preview();
    if !dest.is_invalid() {
        unsafe {
            let _ = ShowWindow(dest, SW_HIDE);
        }
    }
}

/// The dim, with the previewed window drawn a rim and opened where its own
/// pixels really are.
fn paint_veil() {
    let (monitor, target) = with(|w| {
        w.veiled = w.hot;
        let press_handle = w.monitor.handle;
        // 「鼠标屏居中」 moves the window to this screen on release, so the
        // highlight belongs here too, at the frame the release will leave it in.
        let relocate = w.activate_pos == 1;
        let work = w.monitor.work;
        let seat = w.seat(w.hot);
        // Every hovered window is answered, the one that was already in front
        // included: it is on screen anyway, so its own pixels are the preview.
        // Leaving that seat bare made the wheel look dead on the window the user
        // had just been working in.
        let target = seat
            .filter(|s| s.info.on_current_desktop)
            .and_then(|s| {
                if relocate {
                    let rect = final_frame(s.info.hwnd, work)?;
                    // Nothing of the window is under that frame, so the cut
                    // always needs the compositor's picture - never `current`.
                    return Some((s.info.hwnd, rect, s.accent, false));
                }
                let handle = window::hwnd_of(s.info.hwnd);
                // A minimized window is previewed where it would come back to,
                // read from the window manager. Restoring it for the length of a
                // hover is what made a sweep across the dial feel heavy.
                let rect = if window::iconic(handle) {
                    home_frame(s.info.hwnd)?
                } else {
                    window::visible_rect(handle)?
                };
                // A window on another monitor cannot have a hole on this one: its
                // rectangle is outside the veil's canvas, so the old code drew an
                // off-canvas rim and dropped a stray black thumbnail on that
                // screen. `paint_locator` covers the screen it really is on.
                if monitor::for_rect(rect).handle != press_handle {
                    return None;
                }
                Some((s.info.hwnd, rect, s.accent, s.current))
            });
        (w.monitor, target)
    });
    // A window already in front needs no thumbnail - the cut shows it. Anything
    // else does, and a hole with nothing in it is a bright empty rectangle, so
    // when the compositor refuses the picture the outline is drawn without the
    // cut. A frame on the dim is still an answer; nothing at all is not.
    let (outline, cut, accent) = match target {
        Some((raw, rect, accent, current)) => {
            let cut = if current {
                hide_preview();
                true
            } else {
                show_preview(raw, rect)
            };
            if !cut {
                hide_preview();
            }
            // Which window a direction holds is not visible from outside, and a
            // preview that keeps its promise cannot be told from one that shows
            // the window behind it without this line.
            let (pinned, minimized) = with(|w| {
                let seat = w.seat(w.hot)?;
                Some((seat.pinned, window::iconic(window::hwnd_of(raw))))
            })
            .unwrap_or((false, false));
            crate::log!(
                "wheel: preview of {raw:#x} at ({},{}) {}x{}{}{}{}{}",
                rect.left,
                rect.top,
                rect.width(),
                rect.height(),
                if minimized { ", minimized" } else { "" },
                if current { ", in front" } else { "" },
                if pinned { ", on a pinned direction" } else { "" },
                if cut { "" } else { ", no picture" }
            );
            (
                Some((
                    (rect.left - monitor.bounds.left) as f32,
                    (rect.top - monitor.bounds.top) as f32,
                    rect.width().max(1) as f32,
                    rect.height().max(1) as f32,
                )),
                cut,
                accent,
            )
        }
        None => {
            hide_preview();
            (None, false, NO_OUTLINE)
        }
    };
    let w_px = monitor.bounds.width().max(1) as u32;
    let h_px = monitor.bounds.height().max(1) as u32;
    // Reuse the veil's raster while the monitor stays the same size. `fill` is
    // cheap; allocating a screen-sized RGBA bitmap per hovered sector is not.
    VEIL_CANVAS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let same = slot
            .as_ref()
            .is_some_and(|(w, h, _)| *w == w_px && *h == h_px);
        if !same {
            match Pixmap::new(w_px, h_px) {
                Some(pixmap) => *slot = Some((w_px, h_px, pixmap)),
                None => return,
            }
        }
        let Some((_, _, canvas)) = slot.as_mut() else {
            return;
        };
        canvas.fill(tiny_skia::Color::TRANSPARENT);
        compose_veil_into(canvas, outline, cut, accent);
        VEIL_DIB.with(|dib| {
            let mut dib = dib.borrow_mut();
            present_layer(
                veil(),
                canvas,
                &mut dib,
                (monitor.bounds.left, monitor.bounds.top),
                &mut false,
            );
        });
    });
}

/// The hovered window lives on a screen the wheel did not open on. Cover THAT
/// screen with a dim, stroke the window's own rectangle and write its name, so
/// the sector points somewhere the eye can find. The dial itself never moves:
/// moving it would break the remembered bearings the user is aiming from.
fn paint_locator() {
    let (press, target) = with(|w| {
        let press = w.monitor.handle;
        // Under 「鼠标屏居中」 the hovered window is drawn on the wheel's own
        // screen at the frame the release will give it, so there is nothing to
        // point at on the screen it still occupies.
        if w.activate_pos == 1 {
            return (press, None);
        }
        let seat = w.seat(w.hot);
        let target = seat
            .filter(|s| s.info.on_current_desktop)
            .and_then(|s| {
                let hwnd = window::hwnd_of(s.info.hwnd);
                // A vanished window has no honest rectangle. A minimized one is
                // pointed at where it would come back to, which is where the eye
                // last saw it.
                if !window::exists(hwnd) {
                    return None;
                }
                let rect = if window::iconic(hwnd) {
                    home_frame(s.info.hwnd)?
                } else {
                    window::visible_rect(hwnd)?
                };
                Some((rect, s.head.clone(), s.accent))
            });
        (press, target)
    });
    let Some((rect, name, accent)) = target else {
        hide_locator();
        return;
    };
    let mon = monitor::for_rect(rect);
    // The wheel's own veil already highlights a window on the press monitor.
    // A second full-screen layer there would only double the dim, and a window
    // cloaked off screen has nothing to point at.
    if mon.handle == press
        || rect.width() <= 0
        || rect.height() <= 0
        || !rect.overlaps(&mon.bounds)
    {
        hide_locator();
        return;
    }
    let w_px = mon.bounds.width().max(1) as u32;
    let h_px = mon.bounds.height().max(1) as u32;
    let outline = (
        (rect.left - mon.bounds.left) as f32,
        (rect.top - mon.bounds.top) as f32,
        rect.width().max(1) as f32,
        rect.height().max(1) as f32,
    );
    // `bounds` and the DWM frame are both device pixels; the only thing the DPI
    // factor changes is how large the name has to be to stay readable.
    let px = (14.0 * mon.scale).round().max(11.0);
    let layer = locator();
    if layer.is_invalid() {
        return;
    }
    unsafe {
        let _ = ShowWindow(layer, SW_SHOWNOACTIVATE);
        let moving = with(|w| w.located != Some(mon.handle));
        if moving {
            // Below the disc, so even if the two ever overlap the wheel reads
            // first. Both are top-most, and this keeps them in the same band.
            let _ = SetWindowPos(
                layer,
                Some(disc()),
                mon.bounds.left,
                mon.bounds.top,
                w_px as i32,
                h_px as i32,
                SWP_NOACTIVATE,
            );
        }
    }
    LOCATOR_CANVAS.with(|cell| {
        let mut slot = cell.borrow_mut();
        let same = slot
            .as_ref()
            .is_some_and(|(w, h, _)| *w == w_px && *h == h_px);
        if !same {
            match Pixmap::new(w_px, h_px) {
                Some(pixmap) => *slot = Some((w_px, h_px, pixmap)),
                None => return,
            }
        }
        let Some((_, _, canvas)) = slot.as_mut() else {
            return;
        };
        canvas.fill(tiny_skia::Color::TRANSPARENT);
        PAINTER.with(|painter| {
            let mut painter = painter.borrow_mut();
            painter
                .get_or_insert_with(Painter::new)
                .compose_locator_into(canvas, Some(outline), accent, &name, px);
        });
        LOCATOR_DIB.with(|dib| {
            let mut dib = dib.borrow_mut();
            present_layer(
                layer,
                canvas,
                &mut dib,
                (mon.bounds.left, mon.bounds.top),
                &mut false,
            );
        });
    });
    with(|w| w.located = Some(mon.handle));
}

fn hide_locator() {
    let layer = locator();
    if !layer.is_invalid() {
        unsafe {
            let _ = ShowWindow(layer, SW_HIDE);
        }
    }
    with(|w| w.located = None);
}

// MARK: - Picking

/// The release reached the window procedure: say what it meant, and do it.
fn pick() {
    let (aim, activate_pos) = with(|w| {
        // The pointer may have stopped somewhere the last frame never looked at,
        // and this is the one answer that has to be about where it is now.
        w.read_aim();
        (w.aim, w.activate_pos)
    });
    let id = match aim {
        Aim::Slot { id, .. } => id,
        // The hub, a seam, or past the rim: the user went looking for nothing and
        // let go. Letting go inside the wheel is how a wheel is closed.
        _ => {
            cancel();
            return;
        }
    };
    let Some(target) = with(|w| {
        w.seat(id)
            .map(|seat| (seat.info.hwnd, seat.info.title.clone(), seat.info.accessible))
    }) else {
        cancel();
        return;
    };
    let (hwnd, title, accessible) = target;
    commit_rotation();
    with(|w| {
        w.open = false;
    });
    close_layers();
    if !accessible {
        // Listing the window is honest; pretending we can switch to it is not.
        // Windows refuses the raise, so the user gets told what would work.
        crate::tray::balloon_remembered(
            crate::i18n::t("app_name"),
            crate::i18n::t("wheel_needs_admin"),
            true,
        );
        crate::log!("wheel: {title} needs administrator rights; not raised");
    } else {
        let handle = window::hwnd_of(hwnd);
        // The list was read when the dial opened; the desktop is asked again
        // because the user may have switched while the wheel was on screen, and
        // a window that is not here cannot be raised here.
        let here = window::on_current_desktop(handle);
        let moved = here || window::move_to_current_desktop(handle);
        if !moved {
            crate::log!("wheel: {title} could not be carried to this virtual desktop");
            crate::tray::balloon_remembered(
                crate::i18n::t("app_name"),
                crate::i18n::t("wheel_raise_cloaked"),
                true,
            );
            return;
        }
        if !here {
            crate::log!("wheel: {hwnd:#x} carried to the current virtual desktop");
        }
        if activate_pos == 1 {
            place_on_pointer_screen(hwnd);
        }
        // Every pick says what it did, whatever the outcome. The window already in
        // front is included: the dial's idea of "current" was taken when it opened,
        // and by the time the button comes up something else may be in front (our
        // own settings page, for one). Skipping the call there was the "picked it
        // and nothing happened" report - a window that never got the check.
        let activation = window::activate(handle);
        crate::log!(
            "activate: hwnd=0x{:x} from=0x{:x} to=0x{:x} method={} foreground={} cloaked={} elevated={}",
            hwnd,
            activation.from,
            activation.to,
            activation.method,
            activation.foreground,
            activation.cloaked,
            activation.elevated
        );
        if !activation.foreground {
            // Silence here was the old bug: the user pressed, the dial closed,
            // and the window stayed where it was. Say which refusal it was, since
            // the fix is different for each one.
            let (key, line) = if activation.elevated {
                (
                    "wheel_needs_admin",
                    format!("wheel: {title} is above this app; administrator rights are needed"),
                )
            } else if activation.cloaked {
                (
                    "wheel_raise_cloaked",
                    format!("wheel: {title} is still cloaked after the move to this desktop"),
                )
            } else {
                (
                    "wheel_raise_failed",
                    format!("wheel: {title} did not come to the front (method={})", activation.method),
                )
            };
            crate::log!("{line}");
            crate::tray::balloon_remembered(crate::i18n::t("app_name"), crate::i18n::t(key), true);
        }
    }
}

/// The frame 「鼠标屏居中」 will leave `hwnd` in on `work`, read from the window
/// manager without touching the window: a maximized one will cover that work
/// area, anything else keeps its normal size in the middle of it. A hover must
/// not restore or move the real window just to find out.
fn final_frame(hwnd: isize, work: shaiwin_platform_win::Rect) -> Option<shaiwin_platform_win::Rect> {
    let handle = window::hwnd_of(hwnd);
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    let read = unsafe { GetWindowPlacement(handle, &mut placement) }.is_ok();
    let maximized = unsafe { IsZoomed(handle).as_bool() }
        || (read && placement.showCmd == SW_SHOWMAXIMIZED.0 as u32);
    let normal = if read {
        let r = placement.rcNormalPosition;
        Some((r.right - r.left, r.bottom - r.top))
    } else {
        None
    }
    .filter(|(w, h)| *w > 0 && *h > 0)
    .or_else(|| {
        let mut r = RECT::default();
        unsafe {
            let _ = GetWindowRect(handle, &mut r);
        }
        Some((r.right - r.left, r.bottom - r.top))
    });
    placed_frame(normal, maximized, work)
}

/// Where a minimized window would come back to, read without restoring it: its
/// normal frame, or the work area of the monitor that frame sits on when the
/// window remembers it was maximized. A hover reads this and draws; it never
/// shows, raises or restores the real window.
fn home_frame(hwnd: isize) -> Option<shaiwin_platform_win::Rect> {
    let handle = window::hwnd_of(hwnd);
    let mut placement = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(handle, &mut placement) }.is_err() {
        return None;
    }
    let r = placement.rcNormalPosition;
    let rect = shaiwin_platform_win::Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    };
    if rect.width() <= 0 || rect.height() <= 0 {
        return None;
    }
    if placement.flags.contains(WPF_RESTORETOMAXIMIZED) {
        return Some(monitor::for_rect(rect).work);
    }
    Some(rect)
}

/// The geometry half of [`final_frame`], split out so it can be checked without
/// windows: a maximized window covers `work`, a normal one is centred in it.
fn placed_frame(
    size: Option<(i32, i32)>,
    maximized: bool,
    work: shaiwin_platform_win::Rect,
) -> Option<shaiwin_platform_win::Rect> {
    if maximized {
        return Some(work);
    }
    let (width, height) = size?;
    if width <= 0 || height <= 0 {
        return None;
    }
    let (x, y) = monitor::centre_frame(width, height, work);
    Some(shaiwin_platform_win::Rect::from_xywh(x, y, width, height))
}

/// The 「鼠标所在屏居中」 policy: the window goes in the middle of the work area
/// of the monitor under the pointer, whichever monitor it was on before.
///
/// `WINDOWPLACEMENT.showCmd` is not a trustworthy answer to "is this window
/// maximized". Chromium/Electron windows (VS Code among them) draw their own
/// frame and can report `SW_SHOWNORMAL` while `IsZoomed()` is true. The old
/// check then took the `SetWindowPos` path, which Windows silently ignores for
/// a maximized window - the window never left its screen. A maximized window
/// is now restored, its *normal* frame is moved, and it is maximized again: the
/// compositor maximizes onto whichever monitor that normal frame is on.
///
/// A minimized window is restored first, because while iconic its frame is a
/// 160×28 placeholder parked off screen and centring that would put it nowhere.
fn place_on_pointer_screen(hwnd: isize) {
    let (px, py) = monitor::cursor();
    let monitor = monitor::at_point(px, py);
    let work = monitor.work;
    unsafe {
        let handle = window::hwnd_of(hwnd);
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        let _ = GetWindowPlacement(handle, &mut placement);
        let mut before = RECT::default();
        let _ = GetWindowRect(handle, &mut before);
        let minimized = placement.showCmd == SW_SHOWMINIMIZED.0 as u32;
        if minimized {
            let _ = ShowWindow(handle, SW_RESTORE);
        }
        let maximized = IsZoomed(handle).as_bool()
            || (!minimized && placement.showCmd == SW_SHOWMAXIMIZED.0 as u32);
        if maximized {
            let _ = ShowWindow(handle, SW_RESTORE);
        }
        let _ = GetWindowRect(handle, &mut before);
        let width = before.right - before.left;
        let height = before.bottom - before.top;
        let mut fallback = false;
        if width > 0 && height > 0 {
            let (x, y) = monitor::centre_frame(width, height, work);
            let _ = SetWindowPos(
                handle,
                None,
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let mut landed = RECT::default();
            let _ = GetWindowRect(handle, &mut landed);
            if frame_is_on(landed, work) {
                if maximized {
                    let _ = ShowWindow(handle, SW_MAXIMIZE);
                }
            } else {
                // An app that keeps its own idea of its frame can swallow the
                // first move. A placement change goes through the window
                // manager rather than that app's message loop.
                fallback = true;
                placement.rcNormalPosition = RECT {
                    left: x,
                    top: y,
                    right: x + width,
                    bottom: y + height,
                };
                placement.showCmd = if maximized {
                    SW_SHOWMAXIMIZED.0 as u32
                } else {
                    SW_SHOWNORMAL.0 as u32
                };
                let _ = SetWindowPlacement(handle, &placement);
            }
        } else if maximized {
            let _ = ShowWindow(handle, SW_MAXIMIZE);
        }
        let mut after = RECT::default();
        let _ = GetWindowRect(handle, &mut after);
        // The evidence for "centred" is these two rectangles: the frame before
        // and after, the work area it was aimed at, and whether a fallback was
        // needed to make the move stick.
        crate::log!(
            "wheel: activation before=({},{},{},{}) after=({},{},{},{}) monitor 0x{:x} work=({},{},{},{}) maximized={maximized} minimized={minimized} fallback={fallback}",
            before.left,
            before.top,
            before.right,
            before.bottom,
            after.left,
            after.top,
            after.right,
            after.bottom,
            monitor.handle,
            work.left,
            work.top,
            work.right,
            work.bottom
        );
    }
}

/// Whether a frame's centre lies inside a work area. Decides whether a move
/// actually landed: a window whose own loop swallowed `SetWindowPos` is still
/// centred on the screen it started on.
fn frame_is_on(frame: RECT, work: shaiwin_platform_win::Rect) -> bool {
    let cx = (frame.left as i64 + frame.right as i64) / 2;
    let cy = (frame.top as i64 + frame.bottom as i64) / 2;
    cx >= work.left as i64
        && cx < work.right as i64
        && cy >= work.top as i64
        && cy < work.bottom as i64
}

#[cfg(test)]
mod tests {
    use super::{placed_frame, seat_list, sector_mark, SeatProblem, WindowInfo};

    /// A dial that has already learned the order of this desk.
    fn remember(live: &[WindowInfo]) -> crate::settings::Settings {
        let mut settings = crate::settings::Settings::default();
        for info in live {
            let mark = super::fingerprint(info);
            if !settings.seat_order.contains(&mark) {
                settings.seat_order.push(mark);
            }
        }
        settings
    }

    fn win(id: usize) -> WindowInfo {
        WindowInfo {
            hwnd: id as isize,
            pid: id as u32,
            title: format!("window {id}"),
            class: "Test".to_string(),
            exe: "test.exe".to_string(),
            exe_path: format!(r"C:\test\app{id}.exe"),
            rect: shaiwin_platform_win::Rect {
                left: 0,
                top: 0,
                right: 100,
                bottom: 100,
            },
            maximized: false,
            process_start: 0,
            on_current_desktop: true,
            elevated: false,
            accessible: true,
        }
    }

    /// Two handles the dial cannot tell apart: same executable, class and first
    /// word of the title, which is exactly what a fingerprint is made of.
    fn twin(id: usize) -> WindowInfo {
        let mut info = win(id);
        info.exe_path = r"C:\test\same.exe".to_string();
        info.title = "Same window".to_string();
        info
    }

    #[test]
    fn taking_one_of_eight_look_alikes_off_leaves_the_other_seven() {
        // Eight windows one fingerprint cannot tell apart. Striking one off is a
        // decision about a window, so the other seven have to stay seated - and
        // the seat count, the list and the picture all come from this list.
        let twins: Vec<WindowInfo> = (1..=8).map(twin).collect();
        let mut settings = crate::settings::Settings::default();
        assert!(settings.mark_off_dial(twins[3].hwnd));
        let kept = super::sift(twins.clone(), |_| false, |hwnd| settings.is_off_dial(hwnd));
        assert_eq!(kept.len(), 7);
        assert!(!kept.iter().any(|info| info.hwnd == twins[3].hwnd));
        assert!(kept
            .iter()
            .all(|info| super::fingerprint(info) == super::fingerprint(&twins[0])));
        // Putting that one window back is the same decision in reverse.
        assert!(settings.clear_off_dial(twins[3].hwnd));
        let kept = super::sift(twins.clone(), |_| false, |hwnd| settings.is_off_dial(hwnd));
        assert_eq!(kept.len(), 8);
    }

    #[test]
    fn taking_the_last_window_off_leaves_the_dial_empty_in_both_views() {
        // The bug this guards: the sift used to fall back to the whole desk when
        // every window had been taken off. The list reaches it through the full
        // desk (excluded applications included) while the picture is handed a
        // desk with those applications already removed, so the two fell back to
        // different lists - the list called excluded windows "on the dial" and
        // the picture drew none of them. Worse, the last window a user removed
        // came straight back. Both views now have to answer with nothing.
        let mut live = (1..=3).map(win).collect::<Vec<_>>();
        live.push(WindowInfo {
            exe: "other.exe".to_string(),
            ..win(4)
        });
        let mut settings = remember(&live);
        settings.excluded.push("other.exe".to_string());
        for info in live.iter().filter(|info| info.exe == "test.exe") {
            assert!(settings.mark_off_dial(info.hwnd));
        }

        let picture_desk = live
            .iter()
            .filter(|info| !settings.is_excluded(&info.exe))
            .cloned()
            .collect();
        let seated = super::seated_hwnds(&settings, &live);
        let picture = super::snapshot_of(&settings, picture_desk, None);
        assert!(seated.is_empty(), "the list still holds {seated:?}");
        assert!(picture.seats.is_empty(), "the picture still holds a sector");

        // And one window left on the dial agrees in both views too.
        assert!(settings.clear_off_dial(live[1].hwnd));
        let picture_desk = live
            .iter()
            .filter(|info| !settings.is_excluded(&info.exe))
            .cloned()
            .collect();
        let seated = super::seated_hwnds(&settings, &live);
        let picture: Vec<isize> = super::snapshot_of(&settings, picture_desk, None)
            .seats
            .iter()
            .map(|seat| seat.hwnd)
            .collect();
        assert_eq!(seated, vec![live[1].hwnd]);
        assert_eq!(picture, seated);
    }

    #[test]
    fn a_struck_off_application_is_told_it_was_struck_off() {
        let mut settings = crate::settings::Settings::default();
        settings.excluded.push("test.exe".to_string());
        let live = vec![win(1)];
        let mark = super::fingerprint(&live[0]);
        assert_eq!(
            super::seat_problem_of(&settings, live, &mark, 1),
            Some(SeatProblem::Excluded)
        );
    }

    #[test]
    fn a_window_that_is_not_open_is_told_it_is_not_running() {
        let settings = crate::settings::Settings::default();
        let live = vec![win(1)];
        let absent = super::fingerprint(&win(0));
        assert_eq!(
            super::seat_problem_of(&settings, live, &absent, 0),
            Some(SeatProblem::NotRunning)
        );
    }

    #[test]
    fn a_second_window_of_one_app_gets_a_seat_of_its_own() {
        let live = vec![win(0), win(1), twin(2), twin(3)];
        let mut settings = remember(&live);
        settings.wheel_limit = crate::settings::WHEEL_LIMIT_MIN;
        let mark = super::fingerprint(&live[3]);
        // The remembered order seats the first twin only: one entry, one sector.
        assert_eq!(super::seated_hwnds(&settings, &live), vec![0, 1, 2]);
        // Taking the app off parks its place, so neither twin comes back on its
        // own.
        settings.seat_park(&mark);
        assert_eq!(super::seated_hwnds(&settings, &live), vec![0, 1]);
        // Double-clicking the second twin names that exact handle: the window
        // the user pointed at gets the sector, not the look-alike.
        assert!(settings.seat_window(live[3].hwnd));
        assert_eq!(super::seated_hwnds(&settings, &live), vec![0, 1, 3]);
        assert_eq!(
            super::seat_problem_of(&settings, live.clone(), &mark, 3),
            None
        );
        // A second named window of the same app adds a sector of its own.
        assert!(settings.seat_window(live[2].hwnd));
        assert_eq!(super::seated_hwnds(&settings, &live), vec![0, 1, 3, 2]);
    }

    #[test]
    fn a_window_named_by_hand_seats_even_when_its_fingerprint_is_parked() {
        // The real failure: several Explorer "文档" windows share a fingerprint,
        // that fingerprint is parked, and the clicked window was off the dial.
        let live: Vec<WindowInfo> = (1..=3).map(twin).collect();
        let mut settings = remember(&live);
        let mark = super::fingerprint(&live[1]);
        settings.seat_park(&mark);
        for info in &live {
            settings.mark_off_dial(info.hwnd);
        }
        // Nothing of that app sits on the dial.
        assert!(super::seated_hwnds(&settings, &live).is_empty());
        // Double-clicking the middle window clears that handle and names it:
        // the exact window the user clicked gets a sector, not a look-alike.
        settings.clear_off_dial(live[1].hwnd);
        assert!(settings.seat_window(live[1].hwnd));
        assert_eq!(super::seated_hwnds(&settings, &live), vec![2]);
        assert_eq!(super::seat_problem_of(&settings, live, &mark, 2), None);
    }

    #[test]
    fn the_real_documents_case_seats_the_clicked_window() {
        // Mirrors the settings file this was reported against: twelve other
        // seats fill wheel_limit, the Explorer "文档" place sits past it, and
        // that place is parked by an earlier down-dial. Four 文档 windows are
        // open and all of them are in the "not on the dial" list.
        let mut list: Vec<WindowInfo> = (0..12).map(win).collect();
        let twins: Vec<WindowInfo> = (100..104).map(twin).collect();
        list.extend(twins.iter().cloned());
        let mut settings = remember(&list);
        settings.wheel_limit = 11;
        let mark = super::fingerprint(&twins[2]);
        assert_eq!(settings.seat_order.last(), Some(&mark));
        settings.seat_park(&mark);
        for info in &twins {
            settings.mark_off_dial(info.hwnd);
        }
        assert_eq!(
            super::seated_hwnds(&settings, &list),
            (0..11isize).collect::<Vec<_>>()
        );
        // The double-click: lift that handle's off mark, name it, and give its
        // application a place. This is what a failed seating never did.
        let clicked = twins[2].hwnd;
        settings.clear_off_dial(clicked);
        assert!(settings.seat_window(clicked));
        assert!(settings.seat_add(&mark));
        // The exact window the user clicked holds a sector - past the limit,
        // because a named seat is exempt - and no look-alike came back with it.
        assert_eq!(
            super::seated_hwnds(&settings, &list),
            (0..11isize).chain(std::iter::once(clicked)).collect::<Vec<_>>()
        );
        assert_eq!(
            super::seat_problem_of(&settings, list, &mark, clicked),
            None
        );
    }

    #[test]
    fn taking_one_of_several_same_app_seats_off_keeps_the_rest() {
        let live: Vec<WindowInfo> = (1..=3).map(twin).collect();
        let mut settings = crate::settings::Settings::default();
        for info in &live {
            assert!(settings.seat_window(info.hwnd));
        }
        // Three windows of one app each hold a sector (naming a window is
        // exempt from the limit).
        assert_eq!(super::seated_hwnds(&settings, &live), vec![1, 2, 3]);
        // Taking the middle one off removes a single seat and marks that handle
        // off: the other two stay, and nothing slides into the gap.
        assert!(settings.unseat_window(live[1].hwnd));
        settings.mark_off_dial(live[1].hwnd);
        assert_eq!(super::seated_hwnds(&settings, &live), vec![1, 3]);
    }

    #[test]
    fn the_limit_counts_default_seats_only_and_binding_outranks_it() {
        let live: Vec<WindowInfo> = (0..6).map(win).collect();
        let mut settings = remember(&live);
        settings.wheel_limit = crate::settings::WHEEL_LIMIT_MIN;
        // The dial seats its default selection and stops there.
        assert_eq!(super::seated_hwnds(&settings, &live), vec![0, 1, 2]);
        // Ctrl+2 on the last window: an explicit seat is not counted against
        // the limit, so it gets a sector although the default places are gone.
        let mark = super::fingerprint(&live[5]);
        assert!(settings.seat_add(&mark));
        assert_eq!(
            super::seat_problem_of(&settings, live.clone(), &mark, 5),
            None
        );
        assert_eq!(super::seated_hwnds(&settings, &live), vec![0, 1, 2, 5]);
        // Raising the limit fills the next default places; the bound window
        // keeps its sector instead of being pushed past the cut.
        settings.wheel_limit = 5;
        assert_eq!(
            super::seated_hwnds(&settings, &live),
            vec![0, 1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn a_window_without_a_place_is_told_it_is_placement() {
        // No seat in the remembered order: the management list, not the dial,
        // is where this window waits.
        let mut settings = crate::settings::Settings::default();
        settings.wheel_limit = crate::settings::WHEEL_LIMIT_MIN;
        let live: Vec<WindowInfo> = (0..4).map(win).collect();
        let mark = super::fingerprint(&live[3]);
        assert_eq!(
            super::seat_problem_of(&settings, live, &mark, 3),
            Some(SeatProblem::Placement)
        );
    }

    #[test]
    fn a_frame_only_counts_as_moved_when_its_centre_reaches_the_target() {
        use windows::Win32::Foundation::RECT;
        let work = shaiwin_platform_win::Rect {
            left: 1920,
            top: 0,
            right: 3840,
            bottom: 1080,
        };
        let rect = |l, t, r, b| RECT {
            left: l,
            top: t,
            right: r,
            bottom: b,
        };
        assert!(super::frame_is_on(rect(2400, 300, 3200, 900), work));
        // Still on the old screen: the move never landed.
        assert!(!super::frame_is_on(rect(200, 300, 1000, 900), work));
        // A frame straddling the seam counts by its centre, not its left edge.
        assert!(super::frame_is_on(rect(1700, 300, 2700, 900), work));
        assert!(!super::frame_is_on(rect(1700, 300, 1900, 900), work));
    }

    #[test]
    fn the_order_is_remembered_and_a_parked_window_leaves_a_gap() {
        let list: Vec<WindowInfo> = (0..6).map(win).collect();
        let order: Vec<String> = [0, 3, 5, 1, 2, 4]
            .iter()
            .map(|&i| super::fingerprint(&list[i]))
            .collect();
        let favourites: Vec<String> = vec![];
        // z-order says 0,1,2,...; the remembered order says 0,3,5.
        let kept = seat_list(list.clone(), 3, 0, &favourites, &order, &[], &[], &[]);
        assert_eq!(
            kept.iter().map(|w| w.hwnd).collect::<Vec<_>>(),
            vec![0, 3, 5]
        );
        // Taking one off shrinks the count and leaves a gap: no other window
        // from the desk is promoted into its place.
        let parked = vec![order[1].clone()];
        let kept = seat_list(list, 3, 0, &favourites, &order, &[], &parked, &[]);
        assert_eq!(
            kept.iter().map(|w| w.hwnd).collect::<Vec<_>>(),
            vec![0, 5]
        );
    }

    #[test]
    fn an_explicit_add_claims_the_freed_place_without_moving_the_others() {
        let list: Vec<WindowInfo> = (0..6).map(win).collect();
        let mut settings = crate::settings::Settings::default();
        settings.wheel_limit = 3;
        settings.seat_order = (0..6).map(|i| super::fingerprint(&list[i])).collect();
        settings.seat_park(&super::fingerprint(&list[1]));
        let kept = seat_list(
            list.clone(),
            3,
            0,
            &[],
            &settings.seat_order,
            &settings.bound,
            &settings.parked,
            &settings.seat_hwnds,
        );
        assert_eq!(kept.iter().map(|w| w.hwnd).collect::<Vec<_>>(), vec![0, 2]);
        // A double-click in the "not on the dial" list is an explicit request:
        // it takes the place that was emptied, and the two windows that stayed
        // keep their order and their neighbours.
        settings.seat_add(&super::fingerprint(&list[4]));
        let kept = seat_list(
            list,
            3,
            0,
            &[],
            &settings.seat_order,
            &settings.bound,
            &settings.parked,
            &settings.seat_hwnds,
        );
        assert_eq!(
            kept.iter().map(|w| w.hwnd).collect::<Vec<_>>(),
            vec![0, 4, 2]
        );
    }

    #[test]
    fn a_window_without_a_place_is_never_automatically_seated() {
        let list: Vec<WindowInfo> = (0..6).map(win).collect();
        let order: Vec<String> = vec![super::fingerprint(&list[0])];
        let kept = seat_list(list, 9, 0, &[], &order, &[], &[], &[]);
        assert_eq!(kept.len(), 1, "recency may not fill the empty sectors");
        assert_eq!(kept[0].hwnd, 0);
    }

    #[test]
    fn favourites_first_is_a_stable_sort_of_the_remembered_order() {
        let list: Vec<WindowInfo> = (0..4).map(win).collect();
        let order: Vec<String> = (0..4).map(|i| super::fingerprint(&list[i])).collect();
        let favourites = vec![super::fingerprint(&list[3])];
        let kept = seat_list(list, 9, 1, &favourites, &order, &[], &[], &[]);
        assert_eq!(
            kept.iter().map(|w| w.hwnd).collect::<Vec<_>>(),
            vec![3, 0, 1, 2]
        );
    }

    #[test]
    fn a_snapshot_of_a_known_desk_uses_the_remembered_order() {
        // `snapshot_of` is what the settings page calls after a stepper click:
        // the desk is already in hand, so the answer must be the same as reading
        // it again would give - only without the walk of every top-level window.
        let desk: Vec<WindowInfo> = (1..6).map(win).collect();
        let mut settings = crate::settings::Settings::default();
        settings.wheel_limit = 3;
        settings.seat_order = desk.iter().map(super::fingerprint).collect();
        let snapshot = super::snapshot_of(&settings, desk, None);
        assert_eq!(snapshot.seats.len(), 3);
        // Every seat knows its window by handle, which is the thing the drag and
        // the accent colour are keyed by.
        assert!(snapshot.seats.iter().all(|seat| seat.hwnd != 0));
    }

    #[test]
    fn measure_veil_canvas_reuse() {
        use std::time::Instant;
        let (w, h) = (1920u32, 1080u32);
        let outline = Some((100.0, 100.0, 400.0, 300.0));
        let accent = shaiwin_core::theme::Rgb::new(90, 150, 210);
        let Some(mut canvas) = tiny_skia::Pixmap::new(w, h) else {
            return;
        };
        let samples = 20u32;
        let mut compose = 0u64;
        for i in 0..samples {
            let started = Instant::now();
            canvas.fill(tiny_skia::Color::TRANSPARENT);
            crate::render::compose_veil_into(&mut canvas, outline, true, accent);
            if i >= 4 {
                compose += started.elapsed().as_micros() as u64;
            }
        }
        let mut alloc = 0u64;
        for i in 0..samples {
            let started = Instant::now();
            let _pixmap = tiny_skia::Pixmap::new(w, h);
            if i >= 4 {
                alloc += started.elapsed().as_micros() as u64;
            }
        }
        println!(
            "veil: compose avg={:.2}ms alloc avg={:.2}ms",
            compose as f32 / (samples - 4) as f32 / 1000.0,
            alloc as f32 / (samples - 4) as f32 / 1000.0,
        );
    }

    #[test]
    fn a_tag_becomes_the_mark_of_the_sector_and_a_blank_one_does_not() {
        assert_eq!(sector_mark(Some("tax"), "C"), "tax");
        assert_eq!(sector_mark(None, "C"), "C");
        // A label that cleaned down to nothing is not a label, and the letter is
        // still there to name the application with.
        assert_eq!(sector_mark(Some(""), "C"), "C");
    }

    /// 「鼠标屏居中」: a window still on a second screen previews where the release
    /// will put it - the middle of the *wheel's* work area, which starts at the
    /// second screen's own coordinates and is nowhere near the window's frame.
    #[test]
    fn a_window_on_another_screen_previews_in_the_middle_of_this_one() {
        let work = shaiwin_platform_win::Rect::from_xywh(1920, 0, 1920, 1040);
        let frame = placed_frame(Some((800, 600)), false, work).expect("a preview frame");
        assert_eq!(
            frame,
            shaiwin_platform_win::Rect::from_xywh(2480, 220, 800, 600)
        );
    }

    /// A maximized window comes back maximized on the wheel's screen, so its
    /// preview is that screen's whole work area rather than a centred frame.
    #[test]
    fn a_maximized_window_previews_the_work_area_it_will_cover() {
        let work = shaiwin_platform_win::Rect::from_xywh(1920, 0, 1920, 1040);
        assert_eq!(placed_frame(Some((800, 600)), true, work), Some(work));
    }

    /// Nothing to preview is `None`, not a hole at the origin.
    #[test]
    fn a_window_with_no_frame_has_no_preview() {
        let work = shaiwin_platform_win::Rect::from_xywh(0, 0, 1920, 1040);
        assert_eq!(placed_frame(Some((0, 0)), false, work), None);
        assert_eq!(placed_frame(None, false, work), None);
    }

    /// A browser rewrites its title on every page. The window the user seated
    /// and labelled did not change, so its mark must not either.
    #[test]
    fn retitling_a_window_does_not_change_its_mark() {
        let mut info = win(9101);
        let before = super::fingerprint(&info);
        info.title = "a different page - browser".to_string();
        assert_eq!(super::fingerprint(&info), before);
        // The title the sector draws is free to follow the page; only the
        // identity stays behind.
        assert_ne!(info.title, "window 9101");
    }

    /// The seat comes from the remembered order, and that order is keyed by the
    /// mark. A retitled window used to miss its own entry and fall off the dial.
    #[test]
    fn a_retitled_window_keeps_the_sector_it_was_seated_in() {
        let mut info = win(9102);
        let mut settings = crate::settings::Settings::default();
        settings.seat_order = vec![super::fingerprint(&info)];
        info.title = "another page - browser".to_string();
        let seated = super::seat_list(
            vec![info.clone()],
            8,
            0,
            &[],
            &settings.seat_order,
            &[],
            &[],
            &[],
        );
        assert_eq!(seated.len(), 1);
        assert_eq!(seated[0].hwnd, info.hwnd);
    }

    /// Handles outlive their windows. A mark is only trusted while the handle
    /// still names the same process instance, executable and class.
    #[test]
    fn a_reused_handle_does_not_inherit_the_old_window_mark() {
        let mut info = win(9103);
        let old = super::fingerprint(&info);
        info.exe_path = r"C:\test\other.exe".to_string();
        info.pid = 9104;
        assert_ne!(super::fingerprint(&info), old);
    }
}
