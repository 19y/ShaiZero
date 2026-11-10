//! ShaiZero: the windows on the desktop, on a wheel that opens where the cursor is.
//!
//! The Windows entry point. It does four things: makes sure only one copy runs,
//! asks Windows for per-monitor DPI awareness before any window exists, creates
//! the overlay, and runs the message loop.

#![windows_subsystem = "windows"]

mod app;
mod i18n;
mod ico;
mod log;
mod render;
mod settings;
mod settings_ui;
mod tag_ui;
mod text;
mod tray;
mod wheel;

use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, TerminateProcess};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostMessageW, TranslateMessage, MSG,
};

fn main() {
    // The journal first: the single-instance decision is one of the things worth
    // reading afterwards, and it happens before any window exists.
    log::init();
    unsafe {
        // Only one line per user session. The copy that is already running has
        // no window to click - it lives in the notification area - so a second
        // launch is the user asking for its settings page. Hand the request to
        // that copy and leave, rather than exiting as if nothing happened.
        let _mutex = CreateMutexW(None, true, w!("Local\\PanWen.SingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // A cold start registers the window class and creates the window
            // within a few tens of milliseconds, so finding nothing at once is
            // not the same as finding nothing at all. Waiting for it is what
            // separates "another copy is alive" from "the last run left a mutex
            // nobody answers". One second is many times the cold start, and it
            // is short enough that a run which went down badly does not hold the
            // next launch up for long.
            for _ in 0..20 {
                if let Some(hwnd) = app::instance_window() {
                    let posted = PostMessageW(
                        Some(hwnd),
                        app::WM_OPEN_SETTINGS,
                        WPARAM(settings_ui::SECTION_DIAL),
                        LPARAM(0),
                    )
                    .is_ok();
                    crate::log!(
                        "single instance: asked {:?} to open the settings (posted {posted})",
                        hwnd
                    );
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            // The mutex is held, but no copy of this program answers. That is the
            // stuck state a killed or hung run leaves behind: refusing to start
            // would keep the desktop without a working line until the user finds
            // the process by hand. Start instead - this copy becomes the one that
            // answers from here on - and say so in the journal.
            crate::log!(
                "single instance: the mutex is held but no window answered in 1 s; starting anyway"
            );
        }
        // Before the first window: crisp text on every monitor, at its own DPI.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // The shell's tray and menu code wants a single-threaded apartment.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }

    // A GUI build has no console, so a panic would otherwise leave nothing
    // behind: the process disappears and the log stops mid-sentence. Record the
    // message and where it happened so the next report can name the line.
    std::panic::set_hook(Box::new(|info| {
        crate::log!("panic: {info}");
    }));
    settings::migrate_legacy();

    let Some(raw) = app::App::create() else {
        crate::log!("could not create the overlay window");
        return;
    };

    let mut msg = MSG::default();
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            // The settings window uses standard controls; the dialog manager
            // gives them Tab order, Enter and Escape.
            if settings_ui::is_dialog_message(&msg) {
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let mut app = Box::from_raw(raw);
        app.shutdown();
        // Everything this process held has been given back by `shutdown`: the
        // hook is out, the hotkeys are free, the icon is gone, the settings are
        // written. What is left is Windows' own teardown of the loaded DLLs, and
        // that is the one stage this program cannot watch: a shell DLL that
        // blocks in its `DllMain` there leaves a windowless process behind that
        // still owns the single-instance mutex - the state that met the next
        // launch with "a copy is already running" and no window to show for it.
        // End the process outright instead, so the way out of the tray always
        // ends with nothing left on the machine.
        let _ = TerminateProcess(GetCurrentProcess(), 0);
    }
}
