//! Does the hook do what design §6.3 claims, on this machine, right now?
//!
//! Nothing in here needs a person at the keyboard. The test creates one blank
//! window of its own, injects mouse events that do not carry the mark `hook`
//! ignores, and reads back three separate things:
//!
//! 1. **Observation** - did the policy get told about the press, the move and
//!    the release at all?
//! 2. **Swallowing** - with the policy answering `Swallow`, does the window
//!    underneath stop receiving the click? This is the whole reason a hook
//!    replaces the 50 ms poll, and it is the claim that would be embarrassing to
//!    have guessed wrong about.
//! 3. **Replay** - does [`shaiwin_platform_win::hook::replay_click`] put a click
//!    back into a real window, and does the hook honour its own injection mark
//!    well enough not to try swallowing it a second time?
//!
//! Run it with the desktop otherwise idle: while `Swallow` is armed, injected
//! events are eaten for a few hundred milliseconds.
//!
//! ```text
//! cargo run -p shaiwin-platform-win --example hook-selftest
//! ```

use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    mouse_event, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MOVE,
    MOUSEEVENTF_VIRTUALDESK,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetSystemMetrics, GetWindowRect, PeekMessageW, RegisterClassExW, SetCursorPos,
    SetProcessDPIAware, TranslateMessage, MSG, PM_REMOVE, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, WINDOW_EX_STYLE, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use shaiwin_platform_win::hook::{self, GesturePolicy, MouseButton, Observation, Pt, Verdict};

/// The event not-a-driver-would-never-mark: what this file injects so that the
/// hook *should* see it. `hook`'s own mark is the one it ignores.
const FOREIGN: usize = 0x1234_5678;

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// 0 = Forward, 1 = Swallow. Flipped by the test between its two phases.
static VERDICT: AtomicUsize = AtomicUsize::new(0);
/// What the throwaway window actually received.
static GOT_DOWN: AtomicI32 = AtomicI32::new(0);
static GOT_UP: AtomicI32 = AtomicI32::new(0);
static GOT_MOVE: AtomicI32 = AtomicI32::new(0);

struct Recorder;

impl GesturePolicy for Recorder {
    fn observe(&self, ev: Observation) -> Verdict {
        let line = match ev {
            Observation::Press {
                pt,
                button,
                mods,
                on_frame,
                exclusive_foreground,
            } => format!(
                "press {:?} at ({},{}) alt={} frame={} exclusive={}",
                button, pt.x, pt.y, mods.alt, on_frame, exclusive_foreground
            ),
            Observation::Release { pt, button, .. } => {
                format!("release {:?} at ({},{})", button, pt.x, pt.y)
            }
            Observation::Move { pt } => format!("move at ({},{})", pt.x, pt.y),
            Observation::KeyDown { vk, .. } => format!("keydown {vk}"),
            Observation::KeyUp { vk, .. } => format!("keyup {vk}"),
            Observation::ReplayFailed => "replay failed".to_string(),
        };
        if let Ok(mut v) = EVENTS.lock() {
            v.push(line);
        }
        match VERDICT.load(Ordering::Acquire) {
            1 => Verdict::Swallow,
            _ => Verdict::Forward,
        }
    }

    fn watches_moves(&self) -> bool {
        true
    }
}

static RECORDER: Recorder = Recorder;

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_LBUTTONDOWN {
        GOT_DOWN.fetch_add(1, Ordering::SeqCst);
    } else if msg == WM_LBUTTONUP {
        GOT_UP.fetch_add(1, Ordering::SeqCst);
    } else if msg == WM_MOUSEMOVE {
        GOT_MOVE.fetch_add(1, Ordering::SeqCst);
    } else if msg == WM_LBUTTONDBLCLK {
        GOT_DOWN.fetch_add(2, Ordering::SeqCst);
    }
    DefWindowProcW(hwnd, msg, w, l)
}

/// A blank window in a corner nobody works in. The test only ever points at this.
/// It carries a caption on purpose: the frame geometry is part of what is being
/// measured, and a `WS_POPUP` has no frame to classify.
unsafe fn make_target() -> HWND {
    // A handle this process cannot get is not a thing that happens; a default
    // one keeps the example from carrying an error path for it.
    let hmod = GetModuleHandleW(None).unwrap_or_default();
    let class: Vec<u16> = "ShaiWinHookSelftest\0".encode_utf16().collect();
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(wndproc),
        hInstance: hmod.into(),
        lpszClassName: windows::core::PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    let _ = RegisterClassExW(&wc);
    let name: Vec<u16> = "hook selftest target\0".encode_utf16().collect();
    CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        windows::core::PCWSTR(class.as_ptr()),
        windows::core::PCWSTR(name.as_ptr()),
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        60,
        60,
        320,
        200,
        None,
        None,
        Some(hmod.into()),
        None,
    )
    .unwrap_or(HWND(std::ptr::null_mut()))
}

/// Moves the real cursor to a screen point, virtual-desktop normalised.
unsafe fn warp(x: i32, y: i32) {
    let left = GetSystemMetrics(SM_XVIRTUALSCREEN);
    let top = GetSystemMetrics(SM_YVIRTUALSCREEN);
    let width = GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1);
    let height = GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1);
    let nx = (((x - left) as f64 * 65535.0) / width as f64).round() as i32;
    let ny = (((y - top) as f64 * 65535.0) / height as f64).round() as i32;
    mouse_event(
        MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        nx,
        ny,
        0,
        FOREIGN,
    );
}

unsafe fn click(x: i32, y: i32) {
    warp(x, y);
    mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, FOREIGN);
    mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, FOREIGN);
}

/// Drains the queue so the hook callbacks - which run on this thread - actually
/// execute, and any posted click reaches the window.
fn pump(ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    let mut msg = MSG::default();
    unsafe {
        while Instant::now() < until {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

fn snapshot() -> Vec<String> {
    EVENTS.lock().map(|v| v.clone()).unwrap_or_default()
}

fn event_count() -> usize {
    EVENTS.lock().map(|v| v.len()).unwrap_or(0)
}

fn clear_events() {
    if let Ok(mut v) = EVENTS.lock() {
        v.clear();
    }
}

fn counts() -> (i32, i32, i32) {
    (
        GOT_DOWN.load(Ordering::SeqCst),
        GOT_UP.load(Ordering::SeqCst),
        GOT_MOVE.load(Ordering::SeqCst),
    )
}

fn report(label: &str, ok: bool, detail: String) {
    println!("{} {}: {}", if ok { "PASS" } else { "FAIL" }, label, detail);
}

fn centre_of(hwnd: HWND) -> Pt {
    let mut r = RECT::default();
    let _ = unsafe { GetWindowRect(hwnd, &mut r) };
    Pt::new((r.left + r.right) / 2, (r.top + r.bottom) / 2)
}

fn main() {
    // Physical pixels, so the frame this file measures is the frame the hook
    // reports. The real application gets the same thing from app.manifest.
    let _ = unsafe { SetProcessDPIAware() };
    let mut origin = POINT::default();
    let had_cursor = unsafe { GetCursorPos(&mut origin) }.is_ok();
    let target = unsafe { make_target() };
    if target.0.is_null() {
        println!(
            "no window to aim at - this session cannot create one, so the self-test is impossible"
        );
        return;
    }
    match hook::install(&RECORDER) {
        Ok(()) => println!("hook installed (mouse + keyboard)"),
        Err(code) => {
            println!("hook refused by Windows, error {code} - the wheel has no global gesture");
            return;
        }
    }
    pump(50);
    clear_events();

    let mid = centre_of(target);
    let mut failures = 0usize;

    // ---- 1. observation -------------------------------------------------
    unsafe { click(mid.x, mid.y) };
    pump(150);
    let seen = snapshot();
    let pressed = seen.iter().any(|l| l.starts_with("press"));
    let released = seen.iter().any(|l| l.starts_with("release"));
    let (d0, u0, _m0) = counts();
    report(
        "observation",
        pressed && released,
        format!(
            "hook saw {} event(s): press={press} release={rel}; window got down={d0} up={u0}",
            seen.len(),
            press = pressed,
            rel = released
        ),
    );
    failures += usize::from(!(pressed && released));

    // The click must have reached the window while the policy forwards.
    let delivered = d0 >= 1 && u0 >= 1;
    report(
        "passthrough",
        delivered,
        format!("with Verdict::Forward the target received down={d0} up={u0}"),
    );
    failures += usize::from(!delivered);

    // ---- 2. swallowing --------------------------------------------------
    VERDICT.store(1, Ordering::Release);
    clear_events();
    let (d_before, u_before, _) = counts();
    unsafe { click(mid.x + 3, mid.y + 3) };
    pump(150);
    let (d_after, u_after, _) = counts();
    let watched = event_count() >= 2;
    let eaten = d_after == d_before && u_after == u_before;
    report(
        "swallow",
        watched && eaten,
        format!(
            "policy was asked {} time(s) and the window's totals stayed at down={} up={}",
            event_count(),
            d_after,
            u_after
        ),
    );
    failures += usize::from(!(watched && eaten));
    VERDICT.store(0, Ordering::Release);

    // ---- 3. replay ------------------------------------------------------
    let before_events = event_count();
    let (d_pre, u_pre, _) = counts();
    let ok = hook::replay_click(mid, MouseButton::Left);
    pump(150);
    let (d_post, u_post, _) = counts();
    let grew = d_post > d_pre && u_post > u_pre;
    let ignored_by_hook = event_count() == before_events;
    report(
        "replay",
        ok && grew && ignored_by_hook,
        format!(
            "replay_click returned {ok}; the window gained down={} up={}; the hook saw {} extra events (0 means our own mark was honoured)",
            d_post - d_pre,
            u_post - u_pre,
            event_count() - before_events
        ),
    );
    failures += usize::from(!(ok && grew && ignored_by_hook));

    // ---- 4. the rails that are pure plumbing ----------------------------
    let disabled_before = hook::gesture_disabled();
    hook::disable_gesture();
    clear_events();
    unsafe { click(mid.x - 4, mid.y - 4) };
    pump(150);
    let (d_now, u_now, _) = counts();
    // The rail is about presses only: a release has to keep reaching the policy
    // so a gesture already in flight can finish, and moves are not gated at all.
    let took_a_press = snapshot().iter().any(|l| l.starts_with("press"));
    let still_delivered = d_now > d_before && u_now > u_before;
    report(
        "disabled",
        !took_a_press && still_delivered,
        format!(
            "with the gesture off the hook took {} press(es) and the window still got down={d_now} up={u_now} ({} event(s) reached the policy: press={})",
            usize::from(took_a_press),
            event_count(),
            took_a_press
        ),
    );
    failures += usize::from(took_a_press || !still_delivered);
    hook::enable_gesture();
    report(
        "re-enabled",
        !disabled_before && !hook::gesture_disabled(),
        format!(
            "gesture_disabled() went {} -> {}",
            disabled_before,
            hook::gesture_disabled()
        ),
    );

    // ---- 4b. the other rail: a window is asking for a combination ----------
    // [`hook::suspend`] is what a shortcut box promises the desktop: while it
    // listens, no press is ours and no key is taken; when it stops, both are.
    // The promise is armed with `Swallow` and asked twice - a flag the mouse
    // path honours but the key path forgot, or a resume nobody calls, shows up
    // here as the same click behaving differently the second time.
    VERDICT.store(1, Ordering::Release);
    hook::suspend();
    let (d_susp_before, u_susp_before, _) = counts();
    clear_events();
    unsafe { click(mid.x + 6, mid.y + 6) };
    pump(150);
    let (d_susp_after, u_susp_after, _) = counts();
    let asked = snapshot().iter().any(|l| l.starts_with("press"));
    let delivered = d_susp_after > d_susp_before && u_susp_after > u_susp_before;
    report(
        "suspended",
        hook::is_suspended() && !asked && delivered,
        format!(
            "while suspended the hook asked about {} press(es) and the window got down={} up={} (it had {} and {} before)",
            usize::from(asked),
            d_susp_after,
            u_susp_after,
            d_susp_before,
            u_susp_before
        ),
    );
    failures += usize::from(asked || !delivered);
    hook::resume();
    let (d_res_before, u_res_before, _) = counts();
    clear_events();
    unsafe { click(mid.x - 6, mid.y - 6) };
    pump(150);
    let (d_res_after, u_res_after, _) = counts();
    let asked_again = snapshot().iter().any(|l| l.starts_with("press"));
    let eaten = d_res_after == d_res_before && u_res_after == u_res_before;
    report(
        "resumed",
        !hook::is_suspended() && asked_again && eaten,
        format!(
            "after resume the hook asked={} and the window stayed at down={} up={} - the armed Swallow is taking clicks again",
            asked_again, d_res_after, u_res_after
        ),
    );
    failures += usize::from(!asked_again || !eaten);
    VERDICT.store(0, Ordering::Release);

    let quiet = hook::seconds_since_last_event();
    unsafe { click(mid.x, mid.y) };
    pump(50);
    let busy = hook::seconds_since_last_event();
    report(
        "watchdog clock",
        busy < quiet,
        format!("seconds_since_last_event was {quiet:.3} before an event and {busy:.3} after one"),
    );
    failures += usize::from(busy >= quiet);

    // ---- 5. the frame geometry ------------------------------------------
    // Design §6.5: the caption is the system's as much as the border is, so the
    // exemption covers the whole non-client band instead of the outer 8 px.
    // Checked by asking the very function the hook asks, on a real framed window,
    // and by asking it *without* injecting anything: a synthetic press on that
    // caption starts the system's modal move loop on this thread, and the rest of
    // the test would never get its turn.
    let mut fr = RECT::default();
    let _ = unsafe { GetWindowRect(target, &mut fr) };
    let edges = [
        ("top-left corner", Pt::new(fr.left + 2, fr.top + 2)),
        ("left edge", Pt::new(fr.left + 2, mid.y)),
        ("bottom edge", Pt::new(fr.right - 40, fr.bottom - 2)),
        ("caption, 14 px below the top", Pt::new(mid.x, fr.top + 14)),
    ];
    let lines: Vec<String> = edges
        .iter()
        .map(|(label, p)| format!("{label}={}", hook::frame_at(*p)))
        .collect();
    let misfiled: Vec<&str> = edges
        .iter()
        .filter(|(_, p)| !hook::frame_at(*p))
        .map(|(label, _)| *label)
        .collect();
    let client_is_free = !hook::frame_at(mid);
    report(
        "frame geometry",
        misfiled.is_empty() && client_is_free,
        format!(
            "{}; centre of the client classified as frame={}",
            lines.join(" "),
            hook::frame_at(mid)
        ),
    );
    failures += usize::from(!misfiled.is_empty() || !client_is_free);
    if !misfiled.is_empty() {
        println!("      the frame said these were client area: {misfiled:?}");
    }

    // Not a rail, a fact about this session: whether a process can address the
    // pointer at all. `SetCursorPos` is what ShaiWin's own tests have been
    // avoiding because it reportedly fails here, while `mouse_event` with
    // `MOUSEEVENTF_ABSOLUTE` demonstrably works - measure which claim is true
    // instead of inheriting either.
    let mut before = POINT::default();
    let read_ok = unsafe { GetCursorPos(&mut before) }.is_ok();
    let set = unsafe { SetCursorPos(mid.x + 40, mid.y + 40) };
    let mut after = POINT::default();
    let _ = unsafe { GetCursorPos(&mut after) };
    pump(30);
    let _ = unsafe { GetCursorPos(&mut after) };
    println!(
        "note SetCursorPos({},{}) -> {set:?}; the cursor read back at ({},{}) from ({}) - {}",
        mid.x + 40,
        mid.y + 40,
        after.x,
        after.y,
        if read_ok {
            format!("{},{}", before.x, before.y)
        } else {
            "unreadable".to_string()
        },
        if set.is_ok() && after.x == mid.x + 40 && after.y == mid.y + 40 {
            "it works in this session"
        } else {
            "it did not move the pointer"
        }
    );
    let _ = unsafe { SetCursorPos(mid.x, mid.y) };

    hook::uninstall();
    unsafe {
        let _ = DestroyWindow(target);
        if had_cursor {
            let _ = SetCursorPos(origin.x, origin.y);
        }
    }
    println!("--- events this run ---");
    for line in snapshot() {
        println!("  {line}");
    }
    if failures == 0 {
        println!("RESULT: every rail the wheel's gesture depends on held.");
    } else {
        println!("RESULT: {failures} rail(s) did not hold - read the FAIL lines above.");
    }
}
