//! The global low-level input hook, and nothing else.
//!
//! ShaiWin used to have no global input at all: a 50 ms `TIMER_POLL` reading
//! `GetAsyncKeyState`. That is enough to notice that a key is held, and useless
//! for a gesture that has to *take* a click, because by the time the poll runs
//! the text editor underneath already started a selection. A `WH_MOUSE_LL`
//! callback runs before the message is queued for anyone, so it can answer one
//! of three things: let it through, swallow it, or swallow it and put an
//! equivalent click back later.
//!
//! This file deliberately knows no policy. Deciding whether a press belongs to
//! the wheel is the application's job - it lives next to the geometry it needs,
//! in `shaiwin_core::gesture`, where it can be unit-tested without a desktop.
//! The hook hands over what it saw and does whatever comes back.
//!
//! # What the guards in here are for
//!
//! A low-level hook is the most dangerous thing a desktop app can install: while
//! the callback runs, the whole system waits. Windows pulls a hook that runs too
//! long (`LowLevelHooksTimeout`) by itself, and a callback that blocks or
//! crashes can strand a mouse button halfway down forever. So:
//!
//! * Every callback body runs inside `catch_unwind`. A panic forwards the event -
//!   an unexpected wheel is bad, a lost click is worse.
//! * A re-entrancy guard forwards anything raised while already inside.
//! * Input this module injects carries [`OUR_INPUT`] in `dwExtraInfo`, and the
//!   callback ignores that mark on sight. Without it a replayed press arrives as
//!   fresh input and the policy tries to swallow it again.
//! * While [`gesture_disabled`] is set the hook does not even ask about a press,
//!   so a refused replay cannot leave the policy holding a button the desktop
//!   already delivered.
//! * While [`is_suspended`] is set - a window is asking for a key combination -
//!   keys are forwarded untouched and a press is not taken, so the window hears
//!   the whole keyboard and no dial opens over it.
//! * [`seconds_since_last_event`] backs the caller's watchdog: a gesture that
//!   has heard nothing for ten seconds is abandoned rather than left holding a
//!   swallowed button.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    keybd_event, GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_MOUSE, KEYEVENTF_KEYUP,
    MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT, MOUSE_EVENT_FLAGS,
    VIRTUAL_KEY, VK_LCONTROL, VK_LMENU,
    VK_LSHIFT, VK_LWIN, VK_NONAME, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetAncestor, GetClassNameW, GetClientRect, GetCursorPos, GetForegroundWindow,
    GetSystemMetrics, GetWindowRect, SetWindowsHookExW, UnhookWindowsHookEx, WindowFromPoint,
    GA_ROOT, HHOOK, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
    WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};

use crate::monitor;

/// Marks the input this module injects. Any value a real driver would not put in
/// `dwExtraInfo` works; this one spells SHAI.
const OUR_INPUT: usize = 0x5348_4149;

/// How close to a window's edge a press has to be before it counts as the resize
/// border. The visible frame is thinner than the window rectangle by the
/// invisible margin Windows reserves for grabbing, so both are consulted.
/// Only the fallback path needs this: when a window will not report its client
/// rectangle we refuse the strip we can prove rather than the whole frame,
/// because refusing too much costs the gesture entirely.
const BORDER_PX: i32 = 8;

/// A location in physical screen pixels, virtual-desktop coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pt {
    pub x: i32,
    pub y: i32,
}

impl Pt {
    pub fn new(x: i32, y: i32) -> Pt {
        Pt { x, y }
    }
}

/// Which modifier keys were down at the instant of the event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub alt: bool,
    pub ctrl: bool,
    pub shift: bool,
    pub win: bool,
}

/// Which mouse button an event is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// `XBUTTON1`, usually the browser's Back key on the side of the mouse.
    X1,
    /// `XBUTTON2`, usually Forward.
    X2,
}

/// Anything the hook noticed. One type for both hooks so the policy has a single
/// entry point to reason about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observation {
    /// A button went down. `on_frame` means the point sits on a window's
    /// non-client frame - resize border, caption, its buttons - and
    /// `exclusive_foreground` that the point sits on a window that has its
    /// whole screen to itself: the two cases the wheel must not reach into.
    Press {
        pt: Pt,
        button: MouseButton,
        mods: Modifiers,
        on_frame: bool,
        exclusive_foreground: bool,
    },
    /// A button came up. `button` is the one the message names, not necessarily
    /// the one the policy is watching - by the time a release arrives the policy
    /// has to know which it cares about.
    Release {
        pt: Pt,
        button: MouseButton,
        mods: Modifiers,
    },
    /// The cursor moved. Only sent while it can matter - see
    /// [`GesturePolicy::watches_moves`] - because this is the one event that
    /// arrives a hundred times a second.
    Move { pt: Pt },
    /// A key went down, by virtual-key code.
    KeyDown { vk: u32, mods: Modifiers },
    /// A key came up, by virtual-key code.
    KeyUp { vk: u32, mods: Modifiers },
    /// A click we owed an application could not be put back. The gesture is
    /// already switched off in here; this is the chance to write it to the log.
    ReplayFailed,
}

/// What the hook is told to do with what it saw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Deliver it to the desktop untouched. The answer whenever the policy is not
    /// interested, and whenever anything went wrong.
    Forward,
    /// Eat it. Nothing underneath ever learns the event happened.
    Swallow,
    /// Do not eat it, but put the mask key in first: an `Alt` that is about to
    /// come up would otherwise light up the application's menu bar while the
    /// wheel still holds the gesture.
    ForwardWithMaskKey,
}

/// The application's share of the gesture.
pub trait GesturePolicy: Send + Sync {
    /// Called for every event the hook reports. Must be quick - the desktop is
    /// waiting - and must not pump messages.
    fn observe(&self, ev: Observation) -> Verdict;

    /// Whether cursor moves should be delivered at all.
    fn watches_moves(&self) -> bool;
}

static POLICY: OnceLock<&'static dyn GesturePolicy> = OnceLock::new();
/// Both raw hook handles, kept as numbers: an `HHOOK` is not `Send`, and these
/// are only ever handed back to `UnhookWindowsHookEx` on the thread that
/// installed them.
static MOUSE_HOOK: AtomicUsize = AtomicUsize::new(0);
static KEYBOARD_HOOK: AtomicUsize = AtomicUsize::new(0);
/// Set once when a replay is refused, or when the user turns the wheel off.
static DISABLED: AtomicBool = AtomicBool::new(false);
/// Set while a window is asking for a key combination. A press belongs to that
/// window for as long as the flag stands, and a key is a candidate rather than
/// a trigger.
static SUSPENDED: AtomicBool = AtomicBool::new(false);
/// True while inside a callback, so anything the callback itself provokes is
/// forwarded instead of recursing.
static IN_CALLBACK: AtomicBool = AtomicBool::new(false);
/// `GetTickCount64()`-shaped uptime stamp: milliseconds since the process first
/// needed the time, monotonic across a sleep or a clock change.
static LAST_EVENT_MS: AtomicU64 = AtomicU64::new(0);
/// Reported [`Observation::ReplayFailed`] once, so a log cannot fill up.
static REPLAY_REPORTED: AtomicBool = AtomicBool::new(false);
/// Uptime origin for [`now_ms`], created the first time an event is stamped.
static CLOCK: OnceLock<Instant> = OnceLock::new();

fn now_ms() -> u64 {
    // A monotonic clock, not `GetTickCount64`: the watchdog compares against
    // itself, and a wall clock that jumps forward a day when the laptop wakes
    // would strand whatever gesture was in flight.
    CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn held(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

fn mods_now() -> Modifiers {
    Modifiers {
        alt: held(VK_LMENU) || held(VK_RMENU),
        ctrl: held(VK_LCONTROL) || held(VK_RCONTROL),
        shift: held(VK_LSHIFT) || held(VK_RSHIFT),
        win: held(VK_LWIN) || held(VK_RWIN),
    }
}

/// Which button a mouse message names, and whether it is the press. `mouse_data`
/// is the low-level hook's `mouseData`, whose high word says which of the two
/// side buttons moved.
fn button_of(msg: u32, mouse_data: u32) -> Option<(MouseButton, bool)> {
    if msg == WM_LBUTTONDOWN {
        Some((MouseButton::Left, true))
    } else if msg == WM_LBUTTONUP {
        Some((MouseButton::Left, false))
    } else if msg == WM_RBUTTONDOWN {
        Some((MouseButton::Right, true))
    } else if msg == WM_RBUTTONUP {
        Some((MouseButton::Right, false))
    } else if msg == WM_MBUTTONDOWN {
        Some((MouseButton::Middle, true))
    } else if msg == WM_MBUTTONUP {
        Some((MouseButton::Middle, false))
    } else if msg == WM_XBUTTONDOWN || msg == WM_XBUTTONUP {
        // XBUTTON1 is 1, XBUTTON2 is 2, both in the high word of mouseData.
        let button = if (mouse_data >> 16) & 0xFFFF == 2 {
            MouseButton::X2
        } else {
            MouseButton::X1
        };
        Some((button, msg == WM_XBUTTONDOWN))
    } else {
        None
    }
}

/// The window under a point, as a root.
fn root_under(pt: POINT) -> Option<HWND> {
    let h = unsafe { WindowFromPoint(pt) };
    if h.0.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(h, GA_ROOT) };
    Some(if root.0.is_null() { h } else { root })
}

/// The frame the DWM actually draws, which is inside the window rectangle.
fn frame_bounds(hwnd: HWND) -> Option<RECT> {
    let mut r = RECT::default();
    let got = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut r as *mut RECT as *mut core::ffi::c_void,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    if got.is_ok() {
        Some(r)
    } else {
        None
    }
}

/// Whether a press landed on the part of a window that is the system's own: the
/// resize border, the caption, and the buttons drawn on it.
///
/// Measured 2026-11-05 with `examples/alt-drag-probe`: a drag on a window's
/// right edge starts the system resize loop (`dw +90`) and a drag on the caption
/// starts the system move loop (`dx +90 dy +61`) - and in both cases the numbers
/// are *identical with Alt held*. So the frame belongs to the system even under
/// the wheel's trigger keys, and swallowing a press there would cost a move or a
/// resize the user really asked for. Everything outside the window's client
/// rectangle is therefore passed through, not only the outer strip: the caption
/// is 45 px of non-client here, and a border-only rule would have eaten it.
///
/// Computed from geometry rather than from `WM_NCHITTEST` on purpose: sending
/// that message to a window in another process can block, and a low-level hook
/// that blocks costs the whole desktop its input.
fn on_frame(root: HWND, pt: POINT) -> bool {
    let mut wr = RECT::default();
    if unsafe { GetWindowRect(root, &mut wr) }.is_err() {
        return false;
    }
    if !(pt.x >= wr.left && pt.x < wr.right && pt.y >= wr.top && pt.y < wr.bottom) {
        return false;
    }
    let mut client = RECT::default();
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { GetClientRect(root, &mut client) }.is_ok()
        && unsafe { ClientToScreen(root, &mut origin) }.as_bool()
        && client.right > 0
        && client.bottom > 0
    {
        let in_client = pt.x >= origin.x
            && pt.x < origin.x + client.right
            && pt.y >= origin.y
            && pt.y < origin.y + client.bottom;
        return !in_client;
    }
    // A window that will not say where its client area is (or has none we can
    // measure) is only refused its outer strip: a wrong guess there would cost
    // the whole gesture, and the gesture is the reason this process exists.
    let near_edge = (pt.x - wr.left).abs() <= BORDER_PX
        || (wr.right - pt.x).abs() <= BORDER_PX
        || (pt.y - wr.top).abs() <= BORDER_PX
        || (wr.bottom - pt.y).abs() <= BORDER_PX;
    near_edge || !drawn_frame_contains(root, pt)
}

/// The frame the DWM actually paints, which is inside the window rectangle by
/// the invisible grab margin. `None` when the query fails, which is treated as
/// "inside" so that no window gets exempted on a measurement error.
fn drawn_frame_contains(root: HWND, pt: POINT) -> bool {
    match frame_bounds(root) {
        Some(f) => pt.x >= f.left && pt.x < f.right && pt.y >= f.top && pt.y < f.bottom,
        None => true,
    }
}

/// The frame classification a press would be given, for a point instead of an
/// event. Exposed so a probe can check the geometry on a real window without
/// injecting a click into its caption: a synthetic press there starts the
/// system's modal move loop on the very thread that asked for it, which is the
/// opposite of a test.
///
/// The point is in the same physical pixels the hook reports, so the asking
/// process has to be DPI-aware (`app.manifest`) for the two to agree.
pub fn frame_at(pt: Pt) -> bool {
    let p = POINT { x: pt.x, y: pt.y };
    root_under(p).map(|root| on_frame(root, p)).unwrap_or(false)
}

/// True when the foreground window covers its monitor's whole bounds, which is
/// how a full-screen exclusive game looks from the outside - **and** the press
/// is on it. Used only to *refuse* the gesture, so a false positive costs one
/// missed wheel and never a broken game.
///
/// Both halves of that matter on a desk with more than one screen. A full-screen
/// window used to refuse every press on every monitor, so once the user clicked
/// one - the desktop shell is full-screen too - the wheel went dead everywhere
/// until something else took the foreground. The gesture only has to stand down
/// for the window it is actually reaching into.
///
/// The shell's own desktop is never "exclusive": it is full-screen by
/// definition, and a click on it is the most ordinary thing a desktop does.
fn exclusive_foreground(pt: POINT) -> bool {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() || is_shell_desktop(fg) {
        return false;
    }
    let mut r = RECT::default();
    if unsafe { GetWindowRect(fg, &mut r) }.is_err() {
        return false;
    }
    let mon = monitor::for_window(fg.0 as isize);
    exclusive_takes(r, mon.bounds, pt)
}

/// Whether a full-screen window's rectangle claims a press: it has to cover the
/// display it is on, and the pointer has to be inside it. Split out from the
/// Win32 reads so the geometry can be checked without a second monitor.
fn exclusive_takes(r: RECT, bounds: crate::Rect, pt: POINT) -> bool {
    let covers = r.left <= bounds.left
        && r.top <= bounds.top
        && r.right >= bounds.right
        && r.bottom >= bounds.bottom;
    covers
        && pt.x >= r.left
        && pt.x < r.right
        && pt.y >= r.top
        && pt.y < r.bottom
}

/// `Progman` and its `WorkerW` children are the desktop the shell paints. They
/// cover the screen and are never a game.
fn is_shell_desktop(hwnd: HWND) -> bool {
    let mut buf = [0u16; 32];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return false;
    }
    matches!(
        String::from_utf16_lossy(&buf[..len as usize]).as_str(),
        "Progman" | "WorkerW"
    )
}

/// Asks the policy. Rail 1: whatever happens in there, the desktop gets its
/// input back.
fn ask(ev: Observation) -> Verdict {
    let Some(policy) = POLICY.get() else {
        return Verdict::Forward;
    };
    match catch_unwind(AssertUnwindSafe(|| policy.observe(ev))) {
        Ok(v) => v,
        Err(_) => Verdict::Forward,
    }
}

fn watching_moves() -> bool {
    POLICY.get().map(|p| p.watches_moves()).unwrap_or(false)
}

unsafe extern "system" fn mouse_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, w, l);
    }
    let ms = &*(l.0 as *const MSLLHOOKSTRUCT);
    if ms.dwExtraInfo == OUR_INPUT {
        // Something this file injected. Never looked at twice.
        return CallNextHookEx(None, code, w, l);
    }
    if IN_CALLBACK.swap(true, Ordering::AcqRel) {
        return CallNextHookEx(None, code, w, l);
    }
    let verdict = decide_mouse(ms, w.0 as u32);
    IN_CALLBACK.store(false, Ordering::Release);
    match verdict {
        Verdict::Swallow => LRESULT(1),
        _ => CallNextHookEx(None, code, w, l),
    }
}

/// The interesting half of the mouse callback, split out so it can be reached
/// without an `extern` ABI in the way.
fn decide_mouse(ms: &MSLLHOOKSTRUCT, msg: u32) -> Verdict {
    let pt = POINT {
        x: ms.pt.x,
        y: ms.pt.y,
    };
    if let Some((button, is_down)) = button_of(msg, ms.mouseData) {
        LAST_EVENT_MS.store(now_ms(), Ordering::Relaxed);
        if is_down && (DISABLED.load(Ordering::Acquire) || SUSPENDED.load(Ordering::Acquire)) {
            // The switch-off is honoured here rather than in the policy so a
            // press can never be taken while a replay is already broken. A
            // suspended hook adds the other reason: while a window is asking for
            // a key, the pointer belongs to that window. A release is still
            // handed over, so a gesture already in flight finishes and gets its
            // button back.
            return Verdict::Forward;
        }
        let mods = mods_now();
        if is_down {
            let root = root_under(pt);
            ask(Observation::Press {
                pt: Pt::new(pt.x, pt.y),
                button,
                mods,
                on_frame: root.map(|h| on_frame(h, pt)).unwrap_or(false),
                exclusive_foreground: exclusive_foreground(pt),
            })
        } else {
            ask(Observation::Release {
                pt: Pt::new(pt.x, pt.y),
                button,
                mods,
            })
        }
    } else if msg == WM_MOUSEMOVE {
        if watching_moves() {
            LAST_EVENT_MS.store(now_ms(), Ordering::Relaxed);
            ask(Observation::Move {
                pt: Pt::new(pt.x, pt.y),
            })
        } else {
            Verdict::Forward
        }
    } else {
        Verdict::Forward
    }
}

unsafe extern "system" fn key_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(None, code, w, l);
    }
    let kb = &*(l.0 as *const KBDLLHOOKSTRUCT);
    if kb.dwExtraInfo == OUR_INPUT {
        return CallNextHookEx(None, code, w, l);
    }
    if SUSPENDED.load(Ordering::Acquire) {
        // A window is asking for a combination, so every key is a candidate and
        // none of them is a trigger: nothing is swallowed, and the mask key that
        // a releasing `Alt` otherwise brings with it is not injected either - it
        // would land in the box as the key the person meant to name.
        return CallNextHookEx(None, code, w, l);
    }
    if IN_CALLBACK.swap(true, Ordering::AcqRel) {
        return CallNextHookEx(None, code, w, l);
    }
    let msg = w.0 as u32;
    let is_down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
    let is_up = msg == WM_KEYUP || msg == WM_SYSKEYUP;
    if !(is_down || is_up) {
        IN_CALLBACK.store(false, Ordering::Release);
        return CallNextHookEx(None, code, w, l);
    }
    LAST_EVENT_MS.store(now_ms(), Ordering::Relaxed);
    let mods = mods_now();
    let vk = kb.vkCode;
    let verdict = if is_down {
        ask(Observation::KeyDown { vk, mods })
    } else {
        ask(Observation::KeyUp { vk, mods })
    };
    IN_CALLBACK.store(false, Ordering::Release);
    match verdict {
        Verdict::Swallow => LRESULT(1),
        Verdict::ForwardWithMaskKey => {
            // Between the `Alt` going up and the desktop noticing, one nameless
            // key is enough to keep a menu bar from lighting. Both events carry
            // our mark, so the hook ignores them the way it ignores the replay.
            let vk = VK_NONAME.0 as u8;
            keybd_event(vk, 0, Default::default(), OUR_INPUT);
            keybd_event(vk, 0, KEYEVENTF_KEYUP, OUR_INPUT);
            CallNextHookEx(None, code, w, l)
        }
        Verdict::Forward => CallNextHookEx(None, code, w, l),
    }
}

/// Absolute coordinate for the whole virtual desktop, so a replay can land on a
/// second monitor and not be clamped onto the primary one.
fn normalise(value: i32, start: i32, extent: i32) -> i32 {
    let extent = extent.max(1);
    (((value - start) as f64 * 65535.0) / extent as f64)
        .round()
        .clamp(0.0, 65535.0) as i32
}

fn mouse_input(dx: i32, dy: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: OUR_INPUT,
            },
        },
    }
}

fn move_input(x: i32, y: i32) -> INPUT {
    mouse_input(
        x,
        y,
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
    )
}

fn button_input(flags: MOUSE_EVENT_FLAGS) -> INPUT {
    mouse_input(0, 0, flags)
}

/// A side button carries which one it is in `mouseData`, not in its flags.
fn xbutton_input(which: u16, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    let mut input = button_input(flags);
    input.Anonymous.mi.mouseData = which as u32;
    input
}

fn button_event(button: MouseButton, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    match button {
        MouseButton::X1 => xbutton_input(1, flags),
        MouseButton::X2 => xbutton_input(2, flags),
        _ => button_input(flags),
    }
}

/// One event, inserted. `SendInput` answers with how many it actually took, so a
/// refusal is a number rather than a hope.
fn send(input: INPUT) -> bool {
    unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) == 1 }
}

fn press_flags(button: MouseButton) -> (MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS) {
    match button {
        MouseButton::Left => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
        MouseButton::Right => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
        MouseButton::Middle => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
        MouseButton::X1 | MouseButton::X2 => (MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP),
    }
}

/// Puts back a click the wheel took and then did not want: move to the original
/// press point, click, move back. The application underneath sees one ordinary
/// click and cannot tell it was ever away.
///
/// Returns false when the system refused any part of it. That switches the
/// gesture off for the rest of the session and says so once, through
/// [`Observation::ReplayFailed`] - automatic, not a dialog.
pub fn replay_click(pt: Pt, button: MouseButton) -> bool {
    let mut here = POINT::default();
    let got_here = unsafe { GetCursorPos(&mut here) }.is_ok();
    let left = unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) };
    let top = unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) };
    let width = unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) };
    let height = unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) };
    let (down_flag, up_flag) = press_flags(button);

    let placed = send(move_input(
        normalise(pt.x, left, width),
        normalise(pt.y, top, height),
    ));
    let pressed = placed && send(button_event(button, down_flag));
    let released = pressed && send(button_event(button, up_flag));
    if got_here && (placed || pressed || released) {
        // Whatever else happened, the cursor goes back where the user left it.
        let _ = send(move_input(
            normalise(here.x, left, width),
            normalise(here.y, top, height),
        ));
    }
    if !(placed && pressed && released) {
        DISABLED.store(true, Ordering::Release);
        if !REPLAY_REPORTED.swap(true, Ordering::AcqRel) {
            ask(Observation::ReplayFailed);
        }
        return false;
    }
    true
}

/// Why the last install failed, when it did: Windows' error code, for the log.
pub fn last_error() -> u32 {
    unsafe { windows::Win32::Foundation::GetLastError().0 as u32 }
}

/// Installs both hooks and starts reporting to `policy`. The calling thread must
/// pump messages, because that is where the callbacks run.
///
/// `Err(code)` means Windows refused - a `LowLevelHooksTimeout` of zero, a
/// policy that blocks global hooks, or a non-interactive window station. That is
/// the feature turning off while the program keeps running, not a crash, and not
/// a popup asking the user to fix anything.
pub fn install(policy: &'static dyn GesturePolicy) -> Result<(), u32> {
    if POLICY.get().is_none() {
        let _ = POLICY.set(policy);
    }
    if MOUSE_HOOK.load(Ordering::Acquire) != 0 {
        return Ok(());
    }
    let mouse = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) } {
        Ok(h) => h,
        Err(_) => return Err(last_error()),
    };
    let keyboard = match unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), None, 0) } {
        Ok(h) => h,
        Err(_) => {
            unsafe {
                let _ = UnhookWindowsHookEx(mouse);
            }
            return Err(last_error());
        }
    };
    LAST_EVENT_MS.store(now_ms(), Ordering::Relaxed);
    MOUSE_HOOK.store(mouse.0 as usize, Ordering::Release);
    KEYBOARD_HOOK.store(keyboard.0 as usize, Ordering::Release);
    Ok(())
}

/// Removes both hooks. Call this on the thread that installed them: at process
/// exit, on `WM_ENDSESSION`, and from the watchdog.
pub fn uninstall() {
    let mouse = MOUSE_HOOK.swap(0, Ordering::AcqRel);
    let keyboard = KEYBOARD_HOOK.swap(0, Ordering::AcqRel);
    unsafe {
        if mouse != 0 {
            let _ = UnhookWindowsHookEx(HHOOK(mouse as *mut _));
        }
        if keyboard != 0 {
            let _ = UnhookWindowsHookEx(HHOOK(keyboard as *mut _));
        }
    }
}

pub fn is_installed() -> bool {
    MOUSE_HOOK.load(Ordering::Acquire) != 0
}

/// Milliseconds since the hook last looked at real input. A caller with a
/// gesture in flight uses this to walk away from one that stalled - a wheel left
/// open while the user went to lunch is a swallowed button nobody can see.
pub fn seconds_since_last_event() -> f64 {
    let last = LAST_EVENT_MS.load(Ordering::Relaxed);
    if last == 0 {
        return 0.0;
    }
    now_ms().saturating_sub(last) as f64 / 1000.0
}

/// Whether a virtual key is down right now. The shortcut that summons the dial
/// gets a message when it is pressed and none when it is let go, so the caller
/// polls the key itself; this is the only reading it needs.
pub fn key_held(vk: u16) -> bool {
    unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
}

/// Switches the gesture off - the user's own setting, or the aftermath of a
/// refused replay. Presses stop being taken; releases still reach the policy so
/// a gesture already in flight can finish cleanly.
pub fn disable_gesture() {
    DISABLED.store(true, Ordering::Release);
}

pub fn enable_gesture() {
    DISABLED.store(false, Ordering::Release);
    REPLAY_REPORTED.store(false, Ordering::Release);
}

/// True while nothing may be taken.
pub fn gesture_disabled() -> bool {
    DISABLED.load(Ordering::Acquire)
}

/// Stop taking input, because a window is asking the user for a combination.
/// Presses are forwarded untouched - the one that starts a gesture belongs to
/// that window now - and keys are forwarded as they came, without the mask key
/// that a releasing trigger otherwise brings with it. Releases still reach the
/// policy, so a gesture already in flight finishes and its button is paid back.
///
/// The flag is process-wide and a caller is expected to hold it only while one
/// of its own controls listens. Nothing here counts entries: two overlapping
/// suspensions are one suspension, and the second `resume` ends it.
pub fn suspend() {
    SUSPENDED.store(true, Ordering::Release);
}

/// Start taking input again. Call this from the place that suspended, and from
/// the death of the window that asked - a desktop left mute is worse than the
/// click that was never going to arrive.
pub fn resume() {
    SUSPENDED.store(false, Ordering::Release);
}

/// True while a window has the keyboard.
pub fn is_suspended() -> bool {
    SUSPENDED.load(Ordering::Acquire)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN_A: crate::Rect = crate::Rect {
        left: 0,
        top: 0,
        right: 1920,
        bottom: 1080,
    };

    fn full_screen_on_a() -> RECT {
        RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        }
    }

    /// A press on a full-screen window on its own screen is still refused - the
    /// game keeps its click.
    #[test]
    fn a_press_inside_the_full_screen_window_is_refused() {
        assert!(exclusive_takes(
            full_screen_on_a(),
            SCREEN_A,
            POINT { x: 960, y: 540 }
        ));
    }

    /// A press on another monitor is not: this is the multi-screen case where one
    /// full-screen window used to kill the wheel everywhere.
    #[test]
    fn a_press_on_another_monitor_is_not_refused() {
        assert!(!exclusive_takes(
            full_screen_on_a(),
            SCREEN_A,
            POINT { x: 2500, y: 540 }
        ));
    }

    /// A window that does not cover its display is not exclusive, even with the
    /// pointer over it.
    #[test]
    fn a_window_that_does_not_cover_its_monitor_is_not_exclusive() {
        let r = RECT {
            left: 100,
            top: 100,
            right: 700,
            bottom: 500,
        };
        assert!(!exclusive_takes(r, SCREEN_A, POINT { x: 300, y: 300 }));
    }

    /// The rectangle is half-open, the way Win32 describes hit tests: the far
    /// edge belongs to whatever is next.
    #[test]
    fn the_far_edges_do_not_count() {
        assert!(!exclusive_takes(
            full_screen_on_a(),
            SCREEN_A,
            POINT { x: 1920, y: 540 }
        ));
        assert!(!exclusive_takes(
            full_screen_on_a(),
            SCREEN_A,
            POINT { x: 960, y: 1080 }
        ));
        assert!(exclusive_takes(
            full_screen_on_a(),
            SCREEN_A,
            POINT { x: 1919, y: 1079 }
        ));
    }

    /// A window smaller than its display is never exclusive, even when it
    /// happens to be flush with one corner.
    #[test]
    fn a_window_that_leaves_a_corner_showing_is_not_exclusive() {
        let r = RECT {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1000,
        };
        assert!(!exclusive_takes(r, SCREEN_A, POINT { x: 960, y: 500 }));
    }
}
