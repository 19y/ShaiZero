//! Two measurements the wheel design refuses to assume, in one 25-second run.
//!
//! 1. Does touch input reach `WH_MOUSE_LL` at all, and does it arrive flagged
//!    as injected? If it does not, the touch trigger needs a different route and
//!    the wheel cannot promise a finger works.
//! 2. Is `Alt` + a drag on a window's *border* the system's own resize loop? If
//!    it is, the hook has to let those presses through near the frame edges
//!    instead of turning them into wheel gestures.
//!
//! Run it, then do the gestures it prints. It only observes - every event is
//! passed straight through, so nothing on the desktop changes.
//!
//! ```text
//! cargo run -p shaiwin-platform-win --example hook-probe -- 25
//! ```

use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    mouse_event, GetAsyncKeyState, MOUSEEVENTF_MOVE, VK_LBUTTON, VK_LMENU, VK_RBUTTON, VK_RMENU,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetAncestor, GetClientRect, GetForegroundWindow,
    GetGUIThreadInfo, GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId, PeekMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WindowFromPoint, GA_ROOT,
    GUITHREADINFO, GUI_INMOVESIZE, HHOOK, MSG, MSLLHOOKSTRUCT, PM_REMOVE, SM_DIGITIZER,
    WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
    WM_RBUTTONUP,
};

/// What the pointer was doing when the button went down, and where the window
/// stood at that instant. `hwnd` is kept as a number because a raw handle is not
/// `Send`, and this rides inside a static mutex.
#[derive(Clone, Copy)]
struct Watch {
    hwnd: isize,
    start: RECT,
    border: bool,
    down_ms: u128,
    /// The first move-loop state Windows reported for this gesture.
    loop_flag: &'static str,
}

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static WATCH: Mutex<Option<Watch>> = Mutex::new(None);
static T0: Mutex<Option<Instant>> = Mutex::new(None);
/// Injected events seen, split by whether they came from a move or a button -
/// touch produces both, and only the buttons matter for the gesture.
static MOVES_INJECTED: Mutex<u32> = Mutex::new(0);
static BUTTONS_INJECTED: Mutex<u32> = Mutex::new(0);
/// And the same totals without the injected filter. Without these two the run
/// cannot tell "the hook never fired" from "the hook fired and you did nothing",
/// which are opposite answers to question (a).
static MOVES_TOTAL: Mutex<u32> = Mutex::new(0);
static BUTTONS_TOTAL: Mutex<u32> = Mutex::new(0);
static MOVES_LOGGED: Mutex<u32> = Mutex::new(0);
/// Moves carrying exactly `LLMHF_INJECTED` - which is what `mouse_event` from
/// this very process produces. A touch pointer arrives as injected *and*
/// lower-IL (`0x11`), so this counter cannot be polluted by the finger, and it
/// is not polluted by the user's own mouse either. It is the proof that the
/// callback runs at all.
static MOVES_SELF_INJECTED: Mutex<u32> = Mutex::new(0);
/// Injected moves that are *not* ours. These are the only ones that can answer
/// question (a); without the split the self-test would print a false "touch
/// works" every single run.
static MOVES_FOREIGN_INJECTED: Mutex<u32> = Mutex::new(0);

fn log(line: String) {
    if let Ok(mut v) = LINES.lock() {
        v.push(line);
    }
}

fn ms_now() -> u128 {
    let started = T0.lock().ok().and_then(|t| *t).unwrap_or_else(Instant::now);
    started.elapsed().as_millis()
}

unsafe fn alt_held() -> bool {
    GetAsyncKeyState(VK_LMENU.0 as i32) < 0 || GetAsyncKeyState(VK_RMENU.0 as i32) < 0
}

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

/// Which button a message is for, if it is a button release at all.
fn button_of(msg: u32) -> Option<char> {
    match msg {
        m if m == WM_LBUTTONUP => Some('L'),
        m if m == WM_RBUTTONUP => Some('R'),
        m if m == WM_MBUTTONUP => Some('M'),
        _ => None,
    }
}

fn is_move(msg: u32) -> bool {
    msg == WM_MOUSEMOVE
}

/// Which button a message is for, if it is a button press at all. Recorded in
/// the callback rather than from the message queue: a low-level hook does not
/// put these in our thread's queue, so a `PeekMessage` loop would never see them.
fn down_of(msg: u32) -> Option<char> {
    match msg {
        m if m == WM_LBUTTONDOWN => Some('L'),
        m if m == WM_RBUTTONDOWN => Some('R'),
        _ => None,
    }
}

/// Where the point sits on the window under it: inside the client area, on the
/// frame border, or on the title band. The border is the case that worries us,
/// because there the system may already own the drag.
fn region_of(hwnd: HWND, pt: POINT) -> &'static str {
    let mut wr = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut wr) }.is_err() {
        return "unknown";
    }
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return "unknown";
    }
    let mut top = POINT { x: 0, y: 0 };
    if !unsafe { ClientToScreen(hwnd, &mut top) }.as_bool() {
        return "unknown";
    }
    let in_client = pt.x >= top.x
        && pt.x < top.x + client.right
        && pt.y >= top.y
        && pt.y < top.y + client.bottom;
    if in_client {
        return "client";
    }
    // The DWM frame is what the user actually sees; the window rect is bigger
    // on Windows 10/11 by the invisible resize margin.
    match frame_bounds(hwnd) {
        Some(frame) => {
            let edge = 8.max(((frame.right - frame.left) as f64 * 0.004) as i32);
            let on_edge = (pt.x - frame.left).abs() <= edge
                || (frame.right - pt.x).abs() <= edge
                || (frame.bottom - pt.y).abs() <= edge;
            if on_edge {
                "border"
            } else if pt.y >= frame.top && pt.y < frame.top + 34 {
                "title"
            } else {
                "outside-visible"
            }
        }
        None => "non-client",
    }
}

fn move_loop_state(hwnd: isize) -> &'static str {
    let h = HWND(hwnd as *mut _);
    let thread = unsafe { GetWindowThreadProcessId(h, None) };
    let mut info = GUITHREADINFO {
        cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetGUIThreadInfo(thread, &mut info) }.is_err() {
        return "no-gui-info";
    }
    // Windows reports the drag loop but not which kind; the rectangle deltas in
    // the gesture line say that - a border drag that only changes dw/dh is the
    // system's own resize.
    if info.flags.contains(GUI_INMOVESIZE) {
        if info.hwndMoveSize.0.is_null() {
            "in-move-size(no hwnd)"
        } else {
            "in-move-size"
        }
    } else {
        "idle"
    }
}

/// Whether this machine has a touch digitizer at all. If it has none, question
/// (a) cannot be measured here - a finger gesture cannot be performed on a
/// screen that has no finger - and the answer belongs to the settings text, not
/// to this file.
fn digitizer() -> i32 {
    unsafe { GetSystemMetrics(SM_DIGITIZER) }
}

/// Whether the window currently in front belongs to an elevated process. If it
/// does and the hook is silent, that is UIPI drawing the line, not the wheel
/// failing to install a hook.
fn foreground_elevated() -> String {
    let fg = unsafe { GetForegroundWindow() };
    if fg.0.is_null() {
        return "no foreground window".to_string();
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
    match shaiwin_platform_win::window::process_elevated(pid) {
        Some(e) => format!("{e} (pid {pid})"),
        None => format!("unknown (pid {pid})"),
    }
}

unsafe extern "system" fn proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if code >= 0 {
        let m = &*(l.0 as *const MSLLHOOKSTRUCT);
        let msg = w.0 as u32;
        let injected = (m.flags & 0x1) != 0;
        let lower_il = (m.flags & 0x2) != 0;
        if is_move(msg) {
            if let Ok(mut n) = MOVES_TOTAL.lock() {
                *n += 1;
            }
            if injected {
                if let Ok(mut n) = MOVES_INJECTED.lock() {
                    *n += 1;
                }
            }
            if m.flags == 0x1 {
                if let Ok(mut n) = MOVES_SELF_INJECTED.lock() {
                    *n += 1;
                }
            } else if injected {
                if let Ok(mut n) = MOVES_FOREIGN_INJECTED.lock() {
                    *n += 1;
                }
            }
            // A handful of moves get logged with their flags, so an injected
            // one is visible in the listing and not only in a counter.
            let worth_logging = injected || {
                let mut guard = MOVES_LOGGED.lock().unwrap();
                if *guard >= 3 {
                    false
                } else {
                    *guard += 1;
                    true
                }
            };
            if worth_logging {
                log(format!(
                    "{:>6}ms move {} flags={} pt=({},{}) alt={}",
                    ms_now(),
                    if injected { "INJ" } else { "   " },
                    m.flags,
                    m.pt.x,
                    m.pt.y,
                    if alt_held() { "yes" } else { "no" }
                ));
            }
            // Only look while a gesture is being watched: this runs for every
            // move on the desktop and must stay cheap.
            let active = WATCH.lock().ok().and_then(|g| *g).is_some();
            if active {
                if let Ok(mut g) = WATCH.lock() {
                    if let Some(watch) = g.as_mut() {
                        if watch.loop_flag == "idle" {
                            watch.loop_flag = move_loop_state(watch.hwnd);
                            if watch.loop_flag != "idle" {
                                log(format!(
                                    "{:>6}ms   move-loop -> {} (alt was {})",
                                    ms_now(),
                                    watch.loop_flag,
                                    if alt_held() { "held" } else { "not held" }
                                ));
                            }
                        }
                    }
                }
            }
        } else if let Some(btn) = down_of(msg) {
            let alt = alt_held();
            note_press(m.pt);
            let watched = WATCH.lock().ok().and_then(|g| *g).is_some();
            log(format!(
                "{:>6}ms down {} btn={} alt={} flags={} pt=({},{}) watching={}",
                ms_now(),
                if injected { "INJ" } else { "   " },
                btn,
                alt,
                m.flags,
                m.pt.x,
                m.pt.y,
                watched
            ));
        } else if let Some(btn) = button_of(msg) {
            if let Ok(mut n) = BUTTONS_TOTAL.lock() {
                *n += 1;
            }
            if injected {
                if let Ok(mut n) = BUTTONS_INJECTED.lock() {
                    *n += 1;
                }
            }
            let alt = alt_held();
            let lbtn = GetAsyncKeyState(VK_LBUTTON.0 as i32) < 0;
            let rbtn = GetAsyncKeyState(VK_RBUTTON.0 as i32) < 0;
            let root = {
                let h = WindowFromPoint(m.pt);
                if h.0.is_null() {
                    h
                } else {
                    GetAncestor(h, GA_ROOT)
                }
            };
            let region = if root.0.is_null() {
                "none"
            } else {
                region_of(root, m.pt)
            };
            log(format!(
                "{:>6}ms up   {} btn={} alt={} lbtn={} rbtn={} flags={}{} pt=({},{}) win={:p} on={}",
                ms_now(),
                if injected { "INJ" } else { "   " },
                btn,
                alt,
                lbtn,
                rbtn,
                m.flags,
                if lower_il { " lowerIL" } else { "" },
                m.pt.x,
                m.pt.y,
                root.0,
                region
            ));
            // The release closes the observation: did that window move or
            // change size while the button was down?
            let closed = WATCH.lock().ok().and_then(|mut g| g.take());
            if let Some(watch) = closed {
                let h = HWND(watch.hwnd as *mut _);
                let mut now = RECT::default();
                if unsafe { GetWindowRect(h, &mut now) }.is_ok() {
                    log(format!(
                        "         gesture on {:p} region={} loop={} -> dx={} dy={} dw={} dh={} ({} ms)",
                        h.0,
                        if watch.border { "border" } else { "other" },
                        watch.loop_flag,
                        now.left - watch.start.left,
                        now.top - watch.start.top,
                        now.right - now.left - (watch.start.right - watch.start.left),
                        now.bottom - now.top - (watch.start.bottom - watch.start.top),
                        ms_now() - watch.down_ms
                    ));
                }
            }
        }
    }
    CallNextHookEx(None, code, w, l)
}

/// Called when a button goes *down*: remembers the window and its rectangle so
/// the matching release can say what the drag did to it.
unsafe fn note_press(pt: POINT) {
    let root = {
        let h = WindowFromPoint(pt);
        if h.0.is_null() {
            h
        } else {
            GetAncestor(h, GA_ROOT)
        }
    };
    if root.0.is_null() {
        return;
    }
    let mut start = RECT::default();
    if unsafe { GetWindowRect(root, &mut start) }.is_err() {
        return;
    }
    let border = region_of(root, pt) == "border";
    if let Ok(mut g) = WATCH.lock() {
        *g = Some(Watch {
            hwnd: root.0 as isize,
            start,
            border,
            down_ms: ms_now(),
            loop_flag: "idle",
        });
    }
}

fn main() {
    let secs: f64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(25.0);
    *T0.lock().unwrap() = Some(Instant::now());

    let hook: HHOOK = match unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(proc), None, 0) } {
        Ok(h) => h,
        Err(e) => {
            println!("hook failed: {e} - the wheel cannot use a global gesture on this machine");
            return;
        }
    };

    println!("watching for {secs:.0}s. Please do, one after another:");
    println!("  A. tap the screen with a finger, then drag a finger across it");
    println!("  B. Alt + left-drag a window's CLIENT area (text, blank space)");
    println!("  C. Alt + left-drag a window's BORDER edge (the thin strip you resize with)");
    println!("  D. Alt + right-drag anywhere, then a normal left-drag on a title bar");
    println!();
    println!(
        "hook handle {:p}; this process elevated={}, foreground process elevated={}",
        hook.0,
        shaiwin_platform_win::window::self_elevated(),
        foreground_elevated()
    );
    println!("(a hook that cannot see input while an elevated window has focus is a UIPI answer, not a bug in the wheel)");
    let dig = digitizer();
    println!(
        "SM_DIGITIZER={dig} - {}",
        if dig == 0 {
            "this machine has NO touch digitizer, so question (a) cannot be answered by gestures here"
        } else {
            "a digitizer is present, so the tap in step A is a real test"
        }
    );

    let deadline = Instant::now() + Duration::from_secs_f64(secs);
    let started = Instant::now();
    let mut msg = MSG::default();
    let mut self_test_seen = 0u32;
    let mut self_test_done = false;
    let mut last_beat = 0u64;
    unsafe {
        while Instant::now() < deadline {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // Prove the callback is reachable before blaming the desktop. The
            // hook runs in *this* thread while it pumps, so the events only
            // arrive during the drain below - reading the counter straight
            // after `mouse_event` returns would always say zero.
            if !self_test_done && started.elapsed() > Duration::from_millis(1500) {
                mouse_event(MOUSEEVENTF_MOVE, 1, 0, 0, 0);
                mouse_event(MOUSEEVENTF_MOVE, -1, 0, 0, 0);
                let drain = Instant::now() + Duration::from_millis(250);
                while Instant::now() < drain {
                    while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                self_test_seen = *MOVES_SELF_INJECTED.lock().unwrap();
                self_test_done = true;
                println!(
                    "self-test: injected 2 moves at t=1.5s; the callback ran {} of 2 times {}",
                    self_test_seen,
                    if self_test_seen == 0 {
                        "- THE HOOK IS NOT RUNNING in this process context"
                    } else {
                        "- the hook is alive"
                    }
                );
            }
            let secs_in = started.elapsed().as_secs();
            if secs_in / 15 != last_beat {
                last_beat = secs_in / 15;
                println!(
                    "t={secs_in:>3}s  moves={} releases={} presses_logged={}",
                    *MOVES_TOTAL.lock().unwrap(),
                    *BUTTONS_TOTAL.lock().unwrap(),
                    LINES.lock().unwrap().len()
                );
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let _ = UnhookWindowsHookEx(hook);
    }

    let foreign_injected = *MOVES_FOREIGN_INJECTED.lock().unwrap();
    let buttons_injected = *BUTTONS_INJECTED.lock().unwrap();
    let moves_total = *MOVES_TOTAL.lock().unwrap();
    let buttons_total = *BUTTONS_TOTAL.lock().unwrap();
    let self_injected = *MOVES_SELF_INJECTED.lock().unwrap();
    let real_moves = moves_total - self_injected;
    println!("--- events ---");
    for l in LINES.lock().unwrap().iter() {
        println!("{l}");
    }
    println!("--- verdict ---");
    if self_test_done {
        let seen = self_test_seen;
        println!(
            "self-test: the hook callback ran for {seen} of the 2 moves this process injected itself - {}",
            if seen == 0 {
                "THE HOOK IS NOT RUNNING (installation refused, wrong desktop, or a policy that blocks global hooks). Nothing else in this file can be trusted."
            } else {
                "the hook is alive; anything missing below really did not reach it"
            }
        );
    }
    println!(
        "hook saw {moves_total} moves ({self_injected} of them injected by this self-test, {real_moves} from anywhere else) and {buttons_total} button releases; {foreign_injected} foreign moves and {buttons_injected} releases carried the injected flag"
    );
    if self_test_seen == 0 {
        println!(
            "(a) INCONCLUSIVE - the hook did not even see its own injection, so this run says nothing about touch."
        );
    } else if real_moves + buttons_total == 0 {
        println!(
            "(a) INCONCLUSIVE - the hook is alive but the desktop was idle: no real move and no press reached it. Run it again and do the gestures."
        );
    } else if foreign_injected + buttons_injected > 0 {
        println!(
            "(a) touch reaches WH_MOUSE_LL: YES - {foreign_injected} injected moves and {buttons_injected} injected releases came from outside this process, so the hook can tell a finger from a mouse."
        );
    } else {
        println!(
            "(a) with the hook demonstrably alive and {real_moves} real moves seen, no foreign injected event arrived. If you did tap the screen, the touch route is elsewhere (WM_POINTER / raw input) and the finger trigger is not built on this hook."
        );
    }
    println!("(b) read the 'gesture on ... region=... loop=...' lines: loop=in-move-size with");
    println!("    dw/dh changing means the system owned that border drag, so the hook must");
    println!("    let those presses through. loop=idle with dx/dy/dw/dh all zero means the");
    println!("    Alt+border drag did nothing to that window, so taking it is safe.");
}
