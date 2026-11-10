//! Does the wheel's preview do what design §4 of `wheel-at-cursor` claims, on
//! this machine, right now?
//!
//! The preview is three Win32 promises and nothing else: that a window can be
//! brought to the front of the stack *without* taking the focus, that the place it
//! came from can be given back by putting it under the window that was over it, and
//! that a window in the taskbar can be shown for the length of a hover and put back
//! without handing the foreground to whatever happens to be next.
//!
//! Each one is a claim about what Windows allows a process that does not hold the
//! foreground to do to another window, which is exactly the kind of claim this
//! project measures instead of assuming. Nothing here needs a person at the
//! keyboard: the windows are this process's own, no hook is installed, and the
//! desktop's other windows are only read, never moved.
//!
//! ```text
//! cargo run -p shaiwin-platform-win --example zorder-selftest
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, EnumWindows, GetClassNameW,
    GetForegroundWindow, GetWindow, GetWindowTextW, IsIconic, IsZoomed, PeekMessageW,
    PostQuitMessage, RegisterClassExW, SetForegroundWindow, SetProcessDPIAware, ShowWindow,
    TranslateMessage, GW_HWNDPREV, MSG, PM_REMOVE, SW_MINIMIZE, SW_SHOWMAXIMIZED,
    SW_SHOWNOACTIVATE, WINDOW_EX_STYLE, WM_CREATE, WM_DESTROY, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
    WS_VISIBLE,
};

use shaiwin_platform_win::park;
use shaiwin_platform_win::window::{self, hwnd_of, raw_of};

const CLASS: &str = "ShaiWinZOrderProbe";
/// The probe's own windows, held as numbers because `HWND` is a raw pointer and a
/// `Mutex<Vec<HWND>>` therefore could not cross the thread that made them.
static HANDLES: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static READY: AtomicBool = AtomicBool::new(false);
static QUIT: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            if let Ok(mut list) = HANDLES.lock() {
                list.push(raw_of(hwnd));
            }
        }
        WM_DESTROY => PostQuitMessage(0),
        _ => {}
    }
    DefWindowProcW(hwnd, msg, w, l)
}

/// Three plain titled windows, all on one thread that pumps for them. They are the
/// only windows this file creates, and the only ones it moves.
fn spawn_targets() -> thread::JoinHandle<()> {
    thread::spawn(|| unsafe {
        let hmod: HINSTANCE = GetModuleHandleW(None).unwrap_or_default().into();
        let class: Vec<u16> = format!("{CLASS}\0").encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hmod,
            lpszClassName: PCWSTR(class.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassExW(&wc);
        let mut created = Vec::new();
        for (i, title) in ["probe front", "probe middle", "probe back"]
            .iter()
            .enumerate()
        {
            let wide: Vec<u16> = format!("{title}\0").encode_utf16().collect();
            if let Ok(hwnd) = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class.as_ptr()),
                PCWSTR(wide.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                140 + (i as i32) * 60,
                140 + (i as i32) * 60,
                420,
                300,
                None,
                None,
                Some(hmod),
                None,
            ) {
                created.push(hwnd);
            }
        }
        READY.store(created.len() == 3, Ordering::Release);
        let mut msg = MSG::default();
        while !QUIT.load(Ordering::Acquire) {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(2));
        }
        for h in created {
            let _ = DestroyWindow(h);
        }
    })
}

/// Is `a` painted over `b`? Walk up from `b` through the windows in front of it and
/// see whether `a` is on the way. This is the second, independent reading of the
/// stack: [`window::z_order`] is the one the wheel uses, and a test that measures
/// its own ruler proves nothing.
fn is_above(a: HWND, b: HWND) -> bool {
    unsafe {
        let mut cur = GetWindow(b, GW_HWNDPREV).unwrap_or_default();
        for _ in 0..4096 {
            if cur.0.is_null() {
                return false;
            }
            if cur == a {
                return true;
            }
            cur = GetWindow(cur, GW_HWNDPREV).unwrap_or_default();
        }
        false
    }
}

fn title_of(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf);
        if n <= 0 {
            return "<no title>".to_string();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn class_of(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 256];
        let n = GetClassNameW(hwnd, &mut buf);
        if n <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..n as usize])
    }
}

fn foreground() -> Option<HWND> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

fn describe(hwnd: Option<HWND>) -> String {
    hwnd.map(title_of).unwrap_or_else(|| "none".into())
}

fn is_probe(hwnd: HWND) -> bool {
    class_of(hwnd) == CLASS
}

/// This process's own probe windows, in the order [`window::z_order`] reports them -
/// which is the walk the wheel takes, filtered to the three it may move.
fn probe_stack() -> Vec<isize> {
    window::z_order(0)
        .into_iter()
        .filter(|raw| {
            let hwnd = hwnd_of(*raw);
            window::pid_of(hwnd) == std::process::id() && is_probe(hwnd)
        })
        .collect()
}

/// The whole stack as `EnumWindows` hands it back, which is what `z_order` is built
/// on. Recorded here so the front-first claim is read off the same walk with the
/// wheel's own pid filter switched off.
fn enum_order() -> Vec<isize> {
    let mut found: Vec<isize> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(collect_one),
            LPARAM(&mut found as *mut Vec<isize> as isize),
        );
    }
    found
}

unsafe extern "system" fn collect_one(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let list = unsafe { &mut *(lparam.0 as *mut Vec<isize>) };
    list.push(raw_of(hwnd));
    true.into()
}

fn rank_of(stack: &[isize], hwnd: HWND) -> Option<usize> {
    stack.iter().position(|r| *r == raw_of(hwnd))
}

fn main() {
    unsafe {
        let _ = SetProcessDPIAware();
    }
    let worker = spawn_targets();
    for _ in 0..200 {
        if READY.load(Ordering::Acquire) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if !READY.load(Ordering::Acquire) {
        println!("the probe windows were never created - nothing to measure");
        QUIT.store(true, Ordering::Release);
        let _ = worker.join();
        std::process::exit(1);
    }
    thread::sleep(Duration::from_millis(300));

    let handles: Vec<HWND> = HANDLES
        .lock()
        .map(|list| list.iter().map(|raw| hwnd_of(*raw)).collect())
        .unwrap_or_default();
    if handles.len() != 3 {
        println!(
            "only {} probe windows exist; nothing to measure",
            handles.len()
        );
        QUIT.store(true, Ordering::Release);
        let _ = worker.join();
        std::process::exit(1);
    }
    let (front, mid, back) = (handles[0], handles[1], handles[2]);
    let mut failures = 0usize;
    let mut skips = 0usize;
    let mut check = |label: &str, ok: bool, detail: String| {
        println!(
            "{}  {label}{}",
            if ok { "PASS" } else { "FAIL" },
            if detail.is_empty() {
                String::new()
            } else {
                format!(" - {detail}")
            }
        );
        if !ok {
            failures += 1;
        }
    };

    // A session with no foreground window at all - which is what this machine
    // reports when nobody is signed in at the console - cannot read the "before"
    // or the "after", and "none equals none" would print as a pass while proving
    // nothing. Those claims are named as unmeasured here rather than claimed.
    macro_rules! check_focus {
        ($label:expr, $held:expr) => {{
            let now = foreground();
            if $held.is_none() && now.is_none() {
                println!(
                    "SKIP  {} - this session has no foreground window, so a claim about not taking it cannot be checked here",
                    $label
                );
                skips += 1;
            } else {
                check(
                    $label,
                    now == $held,
                    format!("was {}, is now {}", describe($held), describe(now)),
                );
            }
        }};
    }

    // Put the three in a known order: `back` deepest, `front` frontmost. The
    // foreground is handed to each one first, because this is the arrangement step
    // and not the claim under test - the claims below move windows from a process
    // that does *not* hold the foreground.
    for (i, h) in [back, mid, front].iter().enumerate() {
        unsafe {
            let _ = SetForegroundWindow(*h);
            window::raise(*h);
        }
        thread::sleep(Duration::from_millis(80 + (i as u64) * 40));
    }
    let order = probe_stack();
    check(
        "the three probe windows sit in the order they were raised",
        order == vec![raw_of(front), raw_of(mid), raw_of(back)],
        order
            .iter()
            .map(|r| title_of(hwnd_of(*r)))
            .collect::<Vec<_>>()
            .join(", "),
    );

    // 1. `z_order` walks the stack front first, and it is the order Windows gives.
    let stack = window::z_order(0);
    let (r_front, r_mid, r_back) = (
        rank_of(&stack, front),
        rank_of(&stack, mid),
        rank_of(&stack, back),
    );
    check(
        "z_order() walks the stack front first",
        is_above(front, mid)
            && is_above(mid, back)
            && matches!((r_front, r_mid, r_back), (Some(a), Some(b), Some(c)) if a < b && b < c),
        format!(
            "the walk says {} over {} over {}; z_order ranks {r_front:?} {r_mid:?} {r_back:?}",
            title_of(front),
            title_of(mid),
            title_of(back)
        ),
    );
    check(
        "and the same handles in the same places come out of EnumWindows",
        enum_order()
            .into_iter()
            .filter(|r| is_probe(hwnd_of(*r)))
            .collect::<Vec<_>>()
            == probe_stack(),
        String::new(),
    );

    // 2. The lift: `back` comes forward without taking the focus.
    let held = foreground();
    let above_back = r_back
        .filter(|r| *r > 0)
        .and_then(|r| stack.get(r - 1))
        .copied();
    let below_back = r_back.and_then(|r| stack.get(r + 1)).copied();
    window::lift(back);
    thread::sleep(Duration::from_millis(150));
    check(
        "lift() puts the window above the one that was over it",
        above_back.map(|a| is_above(back, hwnd_of(a))) == Some(true),
        format!(
            "{} came above {}",
            title_of(back),
            above_back.map(|a| title_of(hwnd_of(a))).unwrap_or_default()
        ),
    );
    check_focus!("lift() does not take the foreground", held);

    // 3. The give-back: put it under the window that was over it, and it is exactly
    //    where the snapshot said it had been.
    if let Some(above) = above_back {
        window::place_below(back, hwnd_of(above));
        thread::sleep(Duration::from_millis(150));
        let is_where_it_was = is_above(hwnd_of(above), back)
            && below_back
                .map(|b| is_above(back, hwnd_of(b)))
                .unwrap_or(true);
        check(
            "place_below() returns it to exactly that place",
            is_where_it_was,
            format!(
                "{} under {}, and above {}",
                title_of(back),
                title_of(hwnd_of(above)),
                below_back
                    .map(|b| title_of(hwnd_of(b)))
                    .unwrap_or_else(|| "nothing".into())
            ),
        );
    }

    // 4. A window in the taskbar: shown for the preview, and put back, with the
    //    foreground never leaving the window that held it.
    unsafe {
        let _ = ShowWindow(mid, SW_SHOWNOACTIVATE);
    }
    thread::sleep(Duration::from_millis(150));
    unsafe {
        let _ = SetForegroundWindow(front);
    }
    unsafe {
        let _ = ShowWindow(mid, SW_MINIMIZE);
    }
    thread::sleep(Duration::from_millis(250));
    check(
        "the middle probe window is in the taskbar for this case",
        unsafe { IsIconic(mid).as_bool() },
        String::new(),
    );
    let held = foreground();
    let shown = park::show_quietly(mid);
    thread::sleep(Duration::from_millis(250));
    check(
        "show_quietly() brings a taskbar window onto the desktop",
        shown.is_some() && unsafe { !IsIconic(mid).as_bool() },
        format!("placement handed back: {}", shown.is_some()),
    );
    check_focus!("and it does not take the foreground either", held);
    check(
        "a window already on the desktop needs no show, so it answers None",
        park::show_quietly(mid).is_none(),
        String::new(),
    );
    if let Some(placement) = shown {
        let put_away = park::minimize_quietly(mid, &placement);
        thread::sleep(Duration::from_millis(250));
        check(
            "minimize_quietly() sends it back to the taskbar",
            put_away && unsafe { IsIconic(mid).as_bool() },
            String::new(),
        );
        check_focus!("and the foreground never moved", held);
    }

    // 5. The maximized case: the placement has to bring it back full screen, because
    //    a preview that restores a maximized window to a corner is a window the
    //    wheel moved.
    unsafe {
        let _ = ShowWindow(back, SW_SHOWMAXIMIZED);
    }
    thread::sleep(Duration::from_millis(250));
    let was_zoomed = unsafe { IsZoomed(back).as_bool() };
    unsafe {
        let _ = ShowWindow(back, SW_MINIMIZE);
    }
    thread::sleep(Duration::from_millis(250));
    let placement = park::show_quietly(back);
    thread::sleep(Duration::from_millis(250));
    check(
        "a maximized window comes back maximized",
        was_zoomed
            && placement.is_some()
            && unsafe { IsZoomed(back).as_bool() && !IsIconic(back).as_bool() },
        format!("was maximized: {was_zoomed}, now maximized: {}", unsafe {
            IsZoomed(back).as_bool()
        }),
    );
    if let Some(placement) = placement {
        let _ = park::minimize_quietly(back, &placement);
        thread::sleep(Duration::from_millis(250));
        let again = park::show_quietly(back);
        thread::sleep(Duration::from_millis(250));
        check(
            "and the round trip survives being borrowed twice",
            again.is_some() && unsafe { IsZoomed(back).as_bool() },
            String::new(),
        );
        if let Some(again) = again {
            let _ = park::minimize_quietly(back, &again);
        }
    }

    QUIT.store(true, Ordering::Release);
    let _ = worker.join();
    println!();
    if skips > 0 {
        println!(
            "UNVERIFIED HERE: {skips} focus claim(s). The lift and the give-back are measured; \
             whether the focus stays put needs a session with a foreground window - check it at the console."
        );
    }
    if failures == 0 {
        if skips == 0 {
            println!("PASS: the preview lifts, gives back, and never takes the focus");
        } else {
            println!("PASS: the preview lifts and gives back the way §4 says it should");
        }
    } else {
        println!("FAIL: {failures} measurement(s) say the preview does not do what §4 claims");
        std::process::exit(1);
    }
}
