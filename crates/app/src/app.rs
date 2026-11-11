//! The Windows shell behind the dial: the tray icon, the two hot keys, and the
//! window that owns the label editor and the settings page.
//!
//! The dial drives and paints itself on its own layers. What lives here is the
//! part that has to exist before anybody can ask for it: one invisible window
//! that holds the shortcuts, the timers and the tray, and the settings file,
//! which has exactly one writer.

use std::cell::RefCell;
use std::ffi::c_void;

use shaiwin_platform_win::monitor::{self, Monitor};
use shaiwin_platform_win::window;
use shaiwin_platform_win::Rect as WRect;
use tiny_skia::Pixmap;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject,
    SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION,
    DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_NOREPEAT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

use shaiwin_core::theme::Rgb;

use crate::i18n::t;
use crate::settings::{self, Settings};
use crate::{ico, settings_ui, tray};

const CLASS: PCWSTR = w!("PanWen.App");

/// The window of the copy that is already running, if there is one.
///
/// A second launch needs it to hand over the request rather than refuse to
/// start. It is searched by walking the desktop and comparing class names,
/// not by asking Windows for a window by name: a class registered by another
/// process is found that way even when the class-atom lookup comes back empty,
/// and an empty answer here is the difference between "already running" and
/// "the last run left a mutex nobody answers".
pub fn instance_window() -> Option<HWND> {
    unsafe extern "system" fn visit(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
        unsafe {
            let mut name = [0u16; 64];
            let written = GetClassNameW(hwnd, &mut name);
            let wanted = CLASS.as_wide();
            if written as usize == wanted.len() && &name[..wanted.len()] == wanted {
                (*(lp.0 as *mut isize)) = hwnd.0 as isize;
                return FALSE;
            }
            TRUE
        }
    }
    unsafe {
        let mut found: isize = 0;
        let _ = EnumWindows(Some(visit), LPARAM(&mut found as *mut isize as isize));
        (found != 0).then_some(HWND(found as *mut c_void))
    }
}

/// The clock the dial's watchdog rides. Nothing else in this window is on a timer.
const TIMER_POLL: usize = 2;
const HOTKEY_FAV: i32 = 1;
const HOTKEY_PANEL: i32 = 2;

const CMD_SETTINGS: u32 = 3;
const CMD_LOGIN: u32 = 4;
const CMD_QUIT: u32 = 5;

/// Automation hook: open the settings window, so a script can look at it.
/// A second launch posts this to the copy already running: it is the user
/// asking for the page, not asking for another process.
pub const WM_OPEN_SETTINGS: u32 = WM_APP + 15;

pub enum Deferred {
    None,
    Menu,
}

/// Drops the handles of windows that have closed since they were taken off the
/// dial. Called when the file is read and again before it is written, so the
/// list holds one entry per window that still exists rather than one per window
/// that was ever struck off. Answers whether anything was dropped, which is the
/// only reason to write the file back.
fn forget_closed_off_dial(list: &mut Vec<isize>) -> bool {
    let before = list.len();
    list.retain(|hwnd| window::exists(window::hwnd_of(*hwnd)));
    list.len() != before
}

pub(crate) struct Dib {
    hdc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    w: i32,
    h: i32,
}

impl Dib {
    fn new(w: i32, h: i32) -> Option<Dib> {
        unsafe {
            let hdc = CreateCompatibleDC(None);
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut c_void = std::ptr::null_mut();
            let bitmap =
                CreateDIBSection(Some(hdc), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()?;
            let old = SelectObject(hdc, bitmap.into());
            Some(Dib {
                hdc,
                bitmap,
                old,
                bits: bits as *mut u8,
                w,
                h,
            })
        }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.hdc);
        }
    }
}

pub struct App {
    hwnd: HWND,
    self_elevated: bool,

    settings: Settings,
    monitor: Monitor,
    /// What the label editor is naming, or `None` while no editor is open.
    tag_open: Option<crate::tag_ui::Target>,

    /// Large icon for Alt-Tab and small icon for the title bar, taskbar and
    /// tray. Both are loaded at the size Windows asks for at this DPI, so no
    /// surface has to rescale one of them into a different-looking icon.
    window_icon: HICON,
    tray_icon: HICON,
    /// Text last given to the shell. Sending an unchanged tooltip is a shell
    /// round trip that redraws the notification area for nothing.
    tray_tip: String,
    /// Whether the shell accepted the icon. A login launch can beat Explorer's
    /// own start-up, and `NIM_ADD` then fails without saying so: the icon is
    /// missing until it is offered again, so the add is retried from the clock.
    tray_added: bool,
    /// How many clock ticks the tray add has been retried for. Counted rather
    /// than clocked so the retry keeps working when the timer is reset.
    tray_wait: u32,
    settings_wnd: HWND,
}

pub fn register_class(hinst: HINSTANCE, big: HICON, small: HICON) {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: big,
            hIconSm: small,
            lpszClassName: CLASS,
            ..Default::default()
        };
        RegisterClassExW(&class);
    }
}

impl App {
    pub fn create() -> Option<*mut App> {
        unsafe {
            let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
            let mut settings = Settings::load();
            // A window that was struck off and has since closed should not leave
            // a line behind forever: the list is per window, so it forgets the
            // handles whose windows are gone the first time it is read.
            if forget_closed_off_dial(&mut settings.off_dial) {
                settings.save();
            }
            // A hand-seated window is named by its handle, and handles do not
            // survive a restart: the windows those handles pointed at are gone
            // (even if the program opened new ones). The remembered fingerprint
            // order is what carries a reservation across launches.
            let named = settings.seat_hwnds.len();
            settings
                .seat_hwnds
                .retain(|hwnd| window::exists(window::hwnd_of(*hwnd)));
            if settings.seat_hwnds.len() != named {
                settings.save();
            }
            // First run only: the desk as it stands becomes the dial's fixed
            // order. From here on a window keeps its place until the user moves
            // it - opening the dial or switching to a window never reshuffles
            // the sectors, and a down-dial leaves a gap instead of promoting
            // the next window in the z-order.
            if settings.seat_order.is_empty() {
                let live: Vec<String> = window::list_switchable(std::process::id())
                    .iter()
                    .map(crate::wheel::fingerprint)
                    .collect();
                settings.seed_seat_order(&live);
                settings.save();
                crate::log!("dial: remembered {} seat(s) from the open desk", live.len());
            }
            // Before anything is drawn or said: the language. Then the shipped
            // languages are written out into the folder, so a person who wants a
            // third has a file with every line in it already filled in.
            crate::i18n::activate(&settings.lang);
            crate::i18n::export_templates();
            let window_icon = ico::load(ico::large_size()).unwrap_or_default();
            let tray_icon = ico::load(ico::small_size()).unwrap_or_default();
            register_class(hinst, window_icon, tray_icon);
            settings_ui::register_class(hinst, window_icon, tray_icon);
            crate::tag_ui::register_class(hinst, window_icon, tray_icon);
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                w!("ShaiZero"),
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
            .ok()?;

            // The wheel's two layers are made here and stay hidden until a gesture
            // asks for them. The hook does not go in until `start()`, so a refusal
            // to install cannot leave the app holding a half-switcher.
            crate::wheel::startup(&settings, hwnd);
            let (x, y) = monitor::cursor();
            let monitor = monitor::at_point(x, y);
            let app = Box::new(App {
                hwnd,
                self_elevated: window::self_elevated(),
                settings,
                monitor,
                tag_open: None,
                window_icon,
                tray_icon,
                tray_tip: t("app_name").to_string(),
                tray_added: false,
                tray_wait: 0,
                settings_wnd: HWND::default(),
            });
            let ptr = Box::into_raw(app);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
            (*ptr).start();
            Some(ptr)
        }
    }

    fn start(&mut self) {
        // The wheel takes the desktop's mouse as soon as there is an app ready to
        // answer it. Switched off in the settings it never goes in at all, and
        // every click stays where the user put it.
        if self.settings.wheel {
            crate::wheel::install_hook();
        }
        let tray_added = if !self.tray_icon.is_invalid() {
            self.tray_tip = t("app_name").to_string();
            let added = tray::add(self.hwnd, self.tray_icon, &self.tray_tip);
            // Where the shell put it is worth knowing: the area itself means the
            // user can see it, the overflow means Windows decided to hide it
            // until it is dragged out, and a failed lookup means it is not there
            // at all. That distinction is what a "the icon is missing" report
            // needs, and only the shell can answer it.
            crate::log!(
                "tray: icon {} on hwnd {:?}: {}",
                if added { "added" } else { "refused" },
                self.hwnd,
                tray::place(self.hwnd)
            );
            added
        } else {
            crate::log!("tray: no icon was loaded");
            false
        };
        self.tray_added = tray_added;
        unsafe {
            if !self.window_icon.is_invalid() {
                let _ = SendMessageW(
                    self.hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_BIG as usize)),
                    Some(LPARAM(self.window_icon.0 as isize)),
                );
            }
            if !self.tray_icon.is_invalid() {
                let _ = SendMessageW(
                    self.hwnd,
                    WM_SETICON,
                    Some(WPARAM(ICON_SMALL as usize)),
                    Some(LPARAM(self.tray_icon.0 as isize)),
                );
            }
        }
        let mut failed: Vec<String> = Vec::new();
        unsafe {
            if self.settings.fav_vk != 0 {
                let mods = HOT_KEY_MODIFIERS(self.settings.fav_mods) | MOD_NOREPEAT;
                if let Err(error) =
                    RegisterHotKey(Some(self.hwnd), HOTKEY_FAV, mods, self.settings.fav_vk)
                {
                    crate::log!("hotkey {} was refused: {error}", self.settings.fav_label());
                    failed.push(self.settings.fav_label());
                }
            }
            if self.settings.panel_vk != 0 {
                let mods = HOT_KEY_MODIFIERS(self.settings.panel_mods) | MOD_NOREPEAT;
                if let Err(error) =
                    RegisterHotKey(Some(self.hwnd), HOTKEY_PANEL, mods, self.settings.panel_vk)
                {
                    crate::log!(
                        "hotkey {} was refused: {error}",
                        self.settings.panel_label()
                    );
                    failed.push(self.settings.panel_label());
                }
            }
            SetTimer(Some(self.hwnd), TIMER_POLL, 16, None);
        }
        if !failed.is_empty() && !self.tray_icon.is_invalid() {
            let list = failed.join(", ");
            let body = crate::i18n::t_fmt("hotkey_failed_body", &[("{list}", list.as_str())]);
            tray::balloon(self.hwnd, self.tray_icon, t("hotkey_failed"), &body, true);
        }
        // Elevation is a real safety trade-off, not a success: say so once.
        if self.self_elevated {
            tray::balloon(
                self.hwnd,
                self.tray_icon,
                t("app_name"),
                t("elevated_tip"),
                true,
            );
        }
        crate::log!(
            "app: started pid {} hwnd {:?}, tray {}",
            std::process::id(),
            self.hwnd,
            if tray_added { "added" } else { "missing" }
        );
        // A launch by the user - a double click - must end somewhere visible.
        // The tray icon can be in the overflow and a missing icon is invisible
        // by definition, so the page is opened outright. A launch by the login
        // entry passes `--tray` and stays in the notification area.
        let background = std::env::args().any(|arg| arg == "--tray");
        if !background || !tray_added {
            self.open_settings_at(settings_ui::SECTION_DIAL);
        }
    }

    pub fn shutdown(&mut self) {
        // Out of the desktop's mouse first: a hook left installed while this
        // process is on its way out is a click swallowed and never answered.
        crate::wheel::shutdown();
        self.settings.save();
        unsafe {
            if !self.settings_wnd.is_invalid() {
                let _ = DestroyWindow(self.settings_wnd);
                self.settings_wnd = HWND::default();
            }
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_FAV);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PANEL);
            let _ = KillTimer(Some(self.hwnd), TIMER_POLL);
        }
        tray::remove(self.hwnd);
        if !self.window_icon.is_invalid() {
            ico::release(self.window_icon);
        }
        if !self.tray_icon.is_invalid() {
            ico::release(self.tray_icon);
        }
        crate::log!("app: shut down (hotkeys released, tray icon gone, settings written)");
    }

    // MARK: Window messages

    fn on_message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> (LRESULT, Deferred) {
        let mut deferred = Deferred::None;
        let result = match msg {
            WM_TIMER => {
                if wp.0 == TIMER_POLL {
                    self.poll();
                }
                LRESULT(0)
            }
            WM_HOTKEY => {
                match wp.0 as i32 {
                    HOTKEY_FAV => self.favourite_foreground(),
                    HOTKEY_PANEL => self.open_dial(),
                    _ => {}
                }
                LRESULT(0)
            }
            WM_OPEN_SETTINGS => {
                self.open_settings_at(wp.0);
                LRESULT(0)
            }
            crate::wheel::WM_WHEEL_ROTATE => {
                // The dial was turned by dragging outside its rim. The wheel
                // hands the angle over when the gesture ends so the setting is
                // written once, not once per frame of the drag.
                let angle = (wp.0 as f32 / 100.0).rem_euclid(360.0);
                if (self.settings.wheel_rotate - angle).abs() > 0.01 {
                    self.settings.wheel_rotate = angle;
                    self.settings.save();
                    crate::wheel::reconfigure(&self.settings);
                }
                LRESULT(0)
            }
            tray::WM_TRAY => {
                // Version 4 delivers the menu as `WM_CONTEXTMENU` and a click as
                // `NIN_SELECT`, which is the pair the icon is registered for. The
                // version-0 `WM_*BUTTONUP` spellings are answered too, because the
                // shell falls back to them when it cannot honour the version.
                match (lp.0 & 0xFFFF) as u32 {
                    WM_LBUTTONUP | tray::NIN_SELECT | tray::NIN_KEYSELECT => self.open_dial(),
                    WM_RBUTTONUP | tray::WM_CONTEXTMENU => deferred = Deferred::Menu,
                    _ => {}
                }
                LRESULT(0)
            }
            WM_COMMAND => {
                self.command((wp.0 & 0xFFFF) as u32);
                LRESULT(0)
            }
            WM_DISPLAYCHANGE => {
                // The editor and the panel both come up on the screen the pointer
                // is on, so a monitor change is worth re-reading it. An open dial
                // was placed against the old arrangement, so it is placed again.
                let (x, y) = monitor::cursor();
                self.monitor = monitor::at_point(x, y);
                crate::wheel::replaced();
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                let (x, y) = monitor::cursor();
                self.monitor = monitor::at_point(x, y);
                LRESULT(0)
            }
            WM_QUERYENDSESSION => LRESULT(1),
            WM_ENDSESSION => {
                // The session is going: the hook has to come out of it now, in
                // case this process outlives the shutdown it was announced in.
                // The icon goes with it - a logoff leaves no notification area
                // behind to hold it - and the settings are written while there
                // is still a disk to write them to.
                if wp.0 != 0 {
                    crate::wheel::shutdown();
                    tray::remove(self.hwnd);
                    self.tray_added = false;
                    self.settings.save();
                    crate::log!("app: session ending; hook, tray icon and settings put away");
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                // The tray's 退出 is the way out, and this is the same door: the
                // hidden window is the process. Destroying it runs WM_DESTROY,
                // which posts the quit the loop is waiting for, and the shutdown
                // after the loop takes the hook, the hotkeys and the icon with it.
                crate::log!("app: quitting");
                unsafe {
                    let _ = DestroyWindow(self.hwnd);
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                unsafe {
                    PostQuitMessage(0);
                }
                LRESULT(0)
            }
            m if m == taskbar_created() => {
                // Explorer restarted: the notification area is a new one and the
                // icon that was added to the old one went with it. Offer it again.
                if !self.tray_icon.is_invalid() {
                    self.tray_tip = t("app_name").to_string();
                    self.tray_added = tray::add(self.hwnd, self.tray_icon, &self.tray_tip);
                    crate::log!(
                        "tray: icon {} after TaskbarCreated: {}",
                        if self.tray_added { "added" } else { "refused" },
                        tray::place(self.hwnd)
                    );
                }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(self.hwnd, msg, wp, lp) },
        };
        (result, deferred)
    }

    fn finish_menu(&mut self) -> Option<u32> {
        let menu = self.build_menu();
        let hwnd = self.hwnd;
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
        let at = tray::cursor();
        let cmd = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD,
                at.x,
                at.y,
                None,
                hwnd,
                None,
            )
            .0 as u32
        };
        unsafe {
            let _ = DestroyMenu(menu);
            let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
        }
        (cmd != 0).then_some(cmd)
    }

    fn build_menu(&self) -> HMENU {
        unsafe {
            let menu = CreatePopupMenu().unwrap_or_default();
            let add = |id: u32, text: &str, flags: MENU_ITEM_FLAGS| {
                let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = AppendMenuW(menu, flags | MF_STRING, id as usize, PCWSTR(wide.as_ptr()));
            };
            let sep = || {
                let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
            };
            add(CMD_SETTINGS, t("settings"), MF_ENABLED);
            sep();
            let check = if tray::start_at_login() {
                MF_CHECKED
            } else {
                MF_UNCHECKED
            };
            add(CMD_LOGIN, t("open_at_login"), check | MF_ENABLED);
            sep();
            add(CMD_QUIT, t("quit"), MF_ENABLED);
            menu
        }
    }

    fn command(&mut self, id: u32) {
        match id {
            CMD_SETTINGS => self.open_settings_at(settings_ui::SECTION_DIAL),
            CMD_LOGIN => {
                tray::set_start_at_login(!tray::start_at_login());
            }
            CMD_QUIT => unsafe {
                // The one way out: the tray menu's 退出. Everything a kept process
                // holds is given back in `shutdown`, which runs once this window
                // is gone.
                crate::log!("app: quitting from the tray menu");
                let _ = DestroyWindow(self.hwnd);
            },
            _ => {}
        }
    }

    // MARK: Polling

    fn poll(&mut self) {
        // The tray icon first: a login launch can arrive before Explorer has a
        // notification area to add it to, and a missing icon is the one state the
        // process cannot show by being on screen.
        self.poll_tray();
        // The wheel's watchdog rides this clock: a dial left on screen by a
        // release nobody saw is the one failure that would otherwise sit there
        // until the user opened a second one.
        crate::wheel::tick();
        // A hold is a moment without a mouse message, so the clock is what opens
        // the dial; movement after that only aims it.
        crate::wheel::poll_hold();
        // Icons and process details are read on a worker thread ahead of time,
        // so the first open of the dial does not pay for every window at once.
        window::warm_windows(std::process::id());
    }

    /// Offer the icon again until the shell takes it. Explorer owns the
    /// notification area and at login it can still be starting; the first
    /// `NIM_ADD` is then refused, and nothing on screen says so. Every half
    /// second for ten seconds is enough for any Explorer start-up; after that
    /// the `TaskbarCreated` broadcast is the right way back in, and that arrives
    /// by itself when Explorer returns.
    fn poll_tray(&mut self) {
        if self.tray_added || self.tray_icon.is_invalid() {
            return;
        }
        self.tray_wait += 1;
        // The clock is 16 ms, so every 32nd tick is about half a second.
        if self.tray_wait % 32 != 0 || self.tray_wait > 32 * 20 {
            return;
        }
        self.tray_tip = t("app_name").to_string();
        self.tray_added = tray::add(self.hwnd, self.tray_icon, &self.tray_tip);
        crate::log!(
            "tray: icon {} on attempt {}: {}",
            if self.tray_added { "added" } else { "refused" },
            self.tray_wait / 32,
            tray::place(self.hwnd)
        );
    }

    // MARK: Labels, favourites and the panel

    pub(crate) fn set_window_tag(&mut self, fingerprint: &str, tag: &str) {
        self.settings.set_tag(fingerprint, tag);
        self.settings.save();
        crate::wheel::seed_tags(&self.settings.tags);
        crate::settings_ui::refresh_rows();
        // Which word goes on the line is decided by what the file holds now, not by
        // what it held beforehand: striking a label out takes it away, and a log
        // that calls that "labelled" is a log nobody can read afterwards.
        crate::log!(
            "tag: window {} {}",
            if self.settings.tag_for(fingerprint).is_some() {
                "labelled"
            } else {
                "unlabelled"
            },
            fingerprint
        );
    }

    /// The hand-picked sector colour for this window, if the user chose one. The
    /// panel uses it to decide whether to offer "reset to auto" and to seed the
    /// picker with the colour already in place rather than the palette default.
    pub(crate) fn custom_colour(&self, fingerprint: &str) -> Option<Rgb> {
        self.settings.colour_for(fingerprint)
    }

    /// Writes, replaces or clears a hand-picked sector colour and makes the dial
    /// show it at once. The panel's own picture is rebuilt by `refresh_rows`;
    /// the live dial reads the seeded map the next time it opens.
    pub(crate) fn set_window_color(&mut self, fingerprint: &str, color: Option<Rgb>) {
        if !self.settings.set_colour(fingerprint, color) {
            return;
        }
        self.settings.save();
        crate::wheel::seed_colors(&self.settings.colors);
        crate::settings_ui::refresh_rows();
        crate::log!(
            "colour: window {} {}",
            if color.is_some() { "coloured" } else { "back to auto" },
            fingerprint
        );
    }

    /// The same write as [`set_window_color`] without the file save: the colour
    /// card calls this on every step of a drag so the dial shows the colour under
    /// the finger, and only the accepted colour is written down. A cancelled card
    /// therefore leaves the file exactly as it found it.
    pub(crate) fn preview_window_color(&mut self, fingerprint: &str, color: Option<Rgb>) {
        if !self.settings.set_colour(fingerprint, color) {
            return;
        }
        crate::wheel::seed_colors(&self.settings.colors);
    }

    /// How much of its ring this window's sector claims, `1.0` being the even
    /// share. The panel uses it to step the size up or down from where it already
    /// is, so repeated clicks keep moving rather than snapping back to a guess.
    pub(crate) fn window_weight(&self, fingerprint: &str) -> f32 {
        self.settings.weight_for(fingerprint)
    }

    /// Writes, replaces or clears a hand-picked sector size and makes the dial
    /// show it at once. Mirrors [`App::set_window_color`]: the panel repaints
    /// through `refresh_rows`, the live dial re-seeds the size map for its next
    /// opening.
    pub(crate) fn set_window_weight(&mut self, fingerprint: &str, weight: Option<f32>) {
        if !self.settings.set_weight(fingerprint, weight) {
            return;
        }
        self.settings.save();
        crate::wheel::seed_weights(&self.settings.weights);
        crate::wheel::seed_colour_scheme(self.settings.colour_scheme);
        crate::settings_ui::refresh_rows();
        crate::log!(
            "size: window {} {}",
            if weight.is_some() { "resized" } else { "back to even" },
            fingerprint
        );
    }

    /// The anchor drag's answer: it moves a whole rim of sectors at once, so
    /// every changed size is written through one save instead of one save per
    /// sector - a drag crossing six seams must not rewrite the settings file
    /// six hundred times a second.
    pub(crate) fn set_window_weights(&mut self, updates: &[(String, f32)]) {
        let mut changed = false;
        for (fingerprint, weight) in updates {
            if self.settings.set_weight(fingerprint, Some(*weight)) {
                changed = true;
            }
        }
        if !changed {
            return;
        }
        self.settings.save();
        crate::wheel::seed_weights(&self.settings.weights);
        crate::wheel::seed_colour_scheme(self.settings.colour_scheme);
        crate::settings_ui::refresh_weights();
    }

    /// Exchanges the places of two seats, which is what a drag from one sector to
    /// another in the preview means. The order list is tried first; two windows
    /// that share a fingerprint are seated by handle, so their handle order is
    /// tried second. Either way the two windows must still hold sectors after the
    /// change - a swap that would push one off the dial is rolled back, because a
    /// drag is a rearrangement, never a removal.
    pub(crate) fn swap_seats(
        &mut self,
        a: &str,
        a_hwnd: isize,
        b: &str,
        b_hwnd: isize,
        shown: &[(String, isize)],
    ) {
        if a_hwnd == b_hwnd {
            return;
        }
        let moved = if a == b {
            // Two windows of one program are seated by handle, so their visible
            // order is the order of `seat_hwnds`; swapping the name would be a
            // no-op for them.
            self.settings.swap_seat_windows(a_hwnd, b_hwnd)
        } else {
            // The order the picture shows is the only thing the user is talking
            // about. Rewriting the remembered order from it moves exactly the two
            // slices that were dragged together, even when one of the two had no
            // place in the order to begin with.
            self.rewrite_seat_order(a_hwnd, b_hwnd, shown)
        };
        if !moved {
            crate::log!("seat: drag {a_hwnd:x} → {b_hwnd:x} moved nothing");
            return;
        }
        self.settings.save();
        crate::wheel::seed_seats(
            &self.settings.seat_order,
            &self.settings.bound,
            &self.settings.parked,
        );
        crate::wheel::seed_seat_windows(&self.settings.seat_hwnds);
        crate::settings_ui::refresh_rows();
        crate::log!("seat: swapped {a_hwnd:x} and {b_hwnd:x} in the dial's order");
    }

    /// Rebuilds the remembered order from the seats the picture is showing, with
    /// the two dragged seats exchanged. Everything the picture does not show keeps
    /// its relative place behind them, so a drag never forgets a window.
    fn rewrite_seat_order(
        &mut self,
        a_hwnd: isize,
        b_hwnd: isize,
        shown: &[(String, isize)],
    ) -> bool {
        let at = |hwnd: isize| shown.iter().position(|(_, held)| *held == hwnd);
        let (Some(ia), Some(ib)) = (at(a_hwnd), at(b_hwnd)) else {
            return false;
        };
        if ia == ib {
            return false;
        }
        let mut order: Vec<String> = shown.iter().map(|(mark, _)| mark.clone()).collect();
        order.swap(ia, ib);
        for seat in &self.settings.seat_order {
            if !order.contains(seat) {
                order.push(seat.clone());
            }
        }
        // A favourite is seated by its rank, not by its place in `seat_order`, so
        // a drag that moves one has to carry the rank with it or the builder puts
        // the sector straight back where it was. Re-ranking the favourites in the
        // order the picture now shows keeps the membership exactly as it was and
        // moves only the place.
        let favourites: Vec<String> = order
            .iter()
            .filter(|mark| self.settings.favorites.contains(mark))
            .cloned()
            .collect();
        if !favourites.is_empty() {
            let mut rest: Vec<String> = self
                .settings
                .favorites
                .iter()
                .filter(|mark| !favourites.contains(mark))
                .cloned()
                .collect();
            let mut ranked = favourites;
            ranked.append(&mut rest);
            if ranked != self.settings.favorites {
                self.settings.favorites = ranked;
            }
        }
        self.settings.seat_order = order;
        true
    }

    /// Opens the label editor for one window, sitting under the window it names.
    /// `fingerprint` is what gets written, so the caller has already decided which
    /// window this is about; `hwnd` is only where the popover goes. `fallback` is
    /// the name the field starts with when nobody has named this window yet, so a
    /// window just put on the dial can be saved without typing anything.
    pub(crate) fn open_tag_for_window(
        &mut self,
        fingerprint: &str,
        hwnd: isize,
        heading: &'static str,
        cue: &'static str,
        fallback: Option<&str>,
    ) {
        let initial = self
            .settings
            .tag_for(fingerprint)
            .map(str::to_string)
            .or_else(|| fallback.map(settings::clean_tag))
            .unwrap_or_default();
        crate::tag_ui::commit_open();
        let target = crate::tag_ui::Target {
            fingerprint: fingerprint.to_string(),
            heading,
            cue,
        };
        self.tag_open = Some(target.clone());
        // The box belongs to the window it names, so it opens on that window's
        // screen and in the middle of that screen's work area. Anchoring it under
        // the window instead put it on the floor of the desktop whenever the
        // window sat near the bottom of the screen, which is where a maximised or
        // taskbar-hugging window always sits.
        let monitor = shaiwin_platform_win::monitor::for_window(hwnd);
        let app = self as *mut App;
        crate::tag_ui::open(self.hwnd, app, target, &initial, monitor, None, None);
    }

    /// The `Ctrl+2` slot. One press does two things, in the order the second one
    /// cannot undo the first: the window in front takes a seat on the dial, and
    /// the label editor opens on it carrying the window's own name. The seat is
    /// written before the box appears, so leaving the box with Escape - which
    /// means only "not labelled yet" - keeps it.
    ///
    /// A favourite on its own only reserves a seat: a window that was taken off
    /// the dial, excluded by application, or pushed out by the limit still would
    /// not appear. So the press also gives it a place in the remembered order and
    /// lifts those refusals. That is the whole point of the shortcut.
    fn favourite_foreground(&mut self) {
        let Some(hwnd) = window::foreground() else {
            crate::log!("favourite: nothing was in front to put on the dial");
            return;
        };
        let raw = window::raw_of(hwnd);
        if raw == self.hwnd.0 as isize || raw == self.settings_wnd.0 as isize {
            // Our own page is not a window the dial seats.
            crate::tray::balloon_remembered(
                crate::i18n::t("app_name"),
                crate::i18n::t("tag_no_window"),
                true,
            );
            return;
        }
        let Ok(info) = window::check(hwnd, std::process::id()) else {
            crate::log!("favourite: the window in front is not one the dial seats");
            crate::tray::balloon_remembered(
                crate::i18n::t("app_name"),
                crate::i18n::t("tag_no_window"),
                true,
            );
            return;
        };
        let mark = crate::wheel::fingerprint(&info);
        // Asking for this window by name lifts the two decisions that would
        // otherwise keep it off the dial even after it became a favourite.
        if self.settings.is_excluded(&info.exe) {
            self.set_excluded(&info.exe, false);
        }
        let added = self.settings.add_favorite(&mark);
        if added {
            self.settings.save();
            crate::wheel::seed_favorites(&self.settings.favorites);
        }
        // Seat it now. `restore_to_dial` also clears this handle's `off_dial`
        // mark, so one press really does put the window on the dial.
        self.restore_to_dial(&mark, raw);
        crate::log!(
            "favourite: {} - {mark}",
            if added {
                "joined the dial"
            } else {
                "already on the dial"
            }
        );
        let heading = crate::i18n::t(if added { "fav_added" } else { "fav_already" });
        let name = settings::default_tag(&info.title, &info.app_name());
        self.open_tag_for_window(&mark, raw, heading, "fav_prompt", Some(&name));
    }

    /// What a click on the tray icon does: the page where the dial itself is
    /// drawn, since that is the part of the settings a user reaches for without
    /// knowing the way around the rest of it. `Ctrl+T` asks for the same page.
    pub(crate) fn open_dial(&mut self) {
        self.open_settings_at(settings_ui::SECTION_DIAL);
    }

    /// Every window the dial could seat, front to back. Only windows that are
    /// running get a row: a favourite whose application has shut keeps its seat
    /// reservation in the file, but showing it in a list of windows to pick from
    /// would be offering a choice nobody can make.
    ///
    /// Each row carries the icon the window itself answers with. It is read here,
    /// with the rest of the row, and never kept: the handle belongs to the process
    /// that made it, so the only honest time to hold it is the moment the list is.
    pub(crate) fn window_rows(&self) -> Vec<settings_ui::Row> {
        self.window_rows_from(
            &window::list_switchable(std::process::id()),
            &self.settings,
        )
    }

    /// The rows of a desk that has already been read, seated under the settings
    /// the caller is showing. The settings page passes its draft rather than the
    /// file so that the rows beside the picture and the picture itself are cut
    /// from one set of numbers: a limit or order changed on the page must not
    /// move a window between the two views.
    pub(crate) fn window_rows_from(
        &self,
        live: &[window::WindowInfo],
        settings: &Settings,
    ) -> Vec<settings_ui::Row> {
        let mut rows = Vec::new();
        // Which of them the dial is actually holding, so a row can say so and
        // offer the one gesture that takes it off. Read from the same list the
        // rows are built from, so a row cannot disagree with the picture.
        let seated = crate::wheel::seated_hwnds(settings, live);
        for info in live {
            let mark = crate::wheel::fingerprint(info);
            let (head, app) = crate::i18n::wheel_parts(&info.title, &info.app_name());
            let hwnd = window::hwnd_of(info.hwnd);
            rows.push(settings_ui::Row {
                hwnd: info.hwnd,
                tag: settings.tag_for(&mark).unwrap_or_default().to_string(),
                favourite: settings.favorite_rank(&mark).is_some(),
                on_dial: seated.contains(&info.hwnd),
                off_dial: settings.is_off_dial(info.hwnd),
                icon: window::icon_of(hwnd).unwrap_or_default(),
                excluded: settings.is_excluded(&info.exe),
                fingerprint: mark.clone(),
                exe: info.exe.clone(),
                app,
                title: head,
            });
        }
        rows
    }

    /// Takes a window off the dial. Not the same decision as striking the
    /// application off: this window keeps its place in the order and can be put
    /// back by dragging it in or by the row's own button, landing in the sector
    /// it left. The handle joins `off_dial` so nothing seats it again on its own;
    /// its look-alikes are untouched.
    pub(crate) fn take_off_dial(&mut self, fingerprint: &str, hwnd: isize) {
        let was_parked = self.settings.is_parked(fingerprint);
        // A window the user named by hand holds a seat of its own: taking it off
        // drops only that window's seat, and its look-alikes keep theirs. The
        // fingerprint is parked as well so the dial does not promote another
        // window into the gap - a down-dial leaves an empty sector, not a
        // substitute.
        let unseated = self.settings.unseat_window(hwnd);
        self.settings.seat_park(fingerprint);
        let marked = self.settings.mark_off_dial(hwnd);
        if !marked && was_parked && !unseated {
            return;
        }
        forget_closed_off_dial(&mut self.settings.off_dial);
        self.settings.save();
        crate::wheel::seed_seats(
            &self.settings.seat_order,
            &self.settings.bound,
            &self.settings.parked,
        );
        crate::wheel::seed_off_dial(&self.settings.off_dial);
        crate::wheel::seed_seat_windows(&self.settings.seat_hwnds);
        settings_ui::refresh_rows();
        crate::log!("dial: {fingerprint} left the dial (hwnd {hwnd})");
    }

    /// Gives a window a place on the dial, or returns it to the place it kept
    /// after being taken off. Called by `Ctrl+2`, by the row's double-click, by
    /// the right-click menu and by a drag onto the picture. Nothing here chooses
    /// a bearing: the sector comes from the remembered order, so the same window
    /// is in the same place every time the dial opens.
    pub(crate) fn restore_to_dial(&mut self, mark: &str, hwnd: isize) {
        if hwnd == 0 {
            return;
        }
        let cleared = self.settings.clear_off_dial(hwnd);
        // The gesture named this exact window. Seating the handle is what makes
        // a second window of one application possible: the fingerprint cannot
        // tell the twins apart, the handle can.
        let named = self.settings.seat_window(hwnd);
        // The fingerprint also takes a place in the order, because handles do
        // not survive a restart while the remembered order does - that is what
        // brings a window of this application back after a relaunch. An
        // explicit add also lifts a parked fingerprint, which is why this
        // window was in the "not on the dial" list.
        let placed = self.settings.seat_add(mark);
        if !named && !placed && !cleared {
            return;
        }
        self.settings.save();
        crate::wheel::seed_seats(
            &self.settings.seat_order,
            &self.settings.bound,
            &self.settings.parked,
        );
        crate::wheel::seed_off_dial(&self.settings.off_dial);
        crate::wheel::seed_seat_windows(&self.settings.seat_hwnds);
        settings_ui::refresh_rows();
        crate::log!("dial: {mark} on the dial (hwnd {hwnd})");
    }

    /// Undoes a [`App::restore_to_dial`] that turned out not to seat the
    /// window. Not the same as taking the window off: that is a decision and
    /// parks the application, while this only puts back what the attempt
    /// changed - the named handle, the off-dial mark and the parked place.
    pub(crate) fn undo_restore_to_dial(
        &mut self,
        mark: &str,
        hwnd: isize,
        was_off: bool,
        was_parked: bool,
    ) {
        self.settings.unseat_window(hwnd);
        if was_off {
            self.settings.mark_off_dial(hwnd);
        }
        if was_parked {
            self.settings.seat_park(mark);
        }
        self.settings.save();
        crate::wheel::seed_seats(
            &self.settings.seat_order,
            &self.settings.bound,
            &self.settings.parked,
        );
        crate::wheel::seed_off_dial(&self.settings.off_dial);
        crate::wheel::seed_seat_windows(&self.settings.seat_hwnds);
        settings_ui::refresh_rows();
    }

    /// Strikes an application off the dial, or puts it back. Keyed by the
    /// executable, not the window, because that is the grain the user is thinking
    /// in when they say they never want to see this one again.
    pub(crate) fn set_excluded(&mut self, exe: &str, want: bool) {
        if want {
            if self.settings.is_excluded(exe) {
                return;
            }
            self.settings.excluded.push(exe.to_string());
        } else if !self.settings.excluded.iter().any(|e| e == exe) {
            return;
        } else {
            self.settings.excluded.retain(|e| e != exe);
        }
        self.settings.save();
        crate::wheel::seed_excluded(&self.settings.excluded);
        settings_ui::refresh_rows();
        crate::log!(
            "dial: {exe} is {}",
            if want { "off the dial" } else { "back on it" }
        );
    }

    /// The label editor, centred on the settings window rather than hung under
    /// a desktop window. The settings page is where the label was clicked, so that
    /// is where the box belongs; the old anchor made it jump to whichever corner
    /// of the screen the window happened to be in.
    pub(crate) fn name_window_centered(&mut self, fingerprint: &str, parent: HWND) {
        let initial = self
            .settings
            .tag_for(fingerprint)
            .unwrap_or_default()
            .to_string();
        crate::tag_ui::commit_open();
        let target = crate::tag_ui::Target {
            fingerprint: fingerprint.to_string(),
            heading: "tag_title",
            cue: "tag_placeholder",
        };
        self.tag_open = Some(target.clone());
        let center = unsafe {
            let mut client = RECT::default();
            let mut origin = POINT::default();
            if GetClientRect(parent, &mut client).is_ok()
                && ClientToScreen(parent, &mut origin).as_bool()
            {
                Some(WRect {
                    left: origin.x + client.left,
                    top: origin.y + client.top,
                    right: origin.x + client.right,
                    bottom: origin.y + client.bottom,
                })
            } else {
                None
            }
        };
        // The page may have been dragged to another display since the wheel last
        // opened, so the box is scaled and clamped against the monitor the
        // settings window is actually on, not the one the cursor last picked.
        let monitor = shaiwin_platform_win::monitor::for_window(parent.0 as isize);
        let app = self as *mut App;
        crate::tag_ui::open(self.hwnd, app, target, &initial, monitor, None, center);
    }

    /// The editor closed. The dial reads the new label on its own next frame; this
    /// is only the app letting go of the promise that one is open.
    pub(crate) fn tag_dialog_closed(&mut self) {
        self.tag_open = None;
    }

    // MARK: Settings window

    /// The settings page, on the section somebody asked for. A page that is
    /// already open is brought forward and turned to that section - the key that
    /// names a row works the second time exactly as it did the first.
    fn open_settings_at(&mut self, section: usize) {
        if !self.settings_wnd.is_invalid() && unsafe { IsWindow(Some(self.settings_wnd)).as_bool() }
        {
            // A closed page is a hidden page now, not a destroyed one. `SW_RESTORE`
            // brings a hidden window onto the screen as well as restoring a
            // minimized one, so the same call covers both states the X and the
            // taskbar can leave it in.
            // The page is created on one virtual desktop and stays bound to
            // it; after the user switches desktops, showing it here would only
            // paint on the desktop it was left on. Carrying it to the current
            // one is what makes the tray click work everywhere.
            if !window::move_to_current_desktop(self.settings_wnd) {
                crate::log!("settings: could not move the page to the current virtual desktop");
            }
            unsafe {
                let _ = ShowWindow(self.settings_wnd, SW_RESTORE);
                let _ = BringWindowToTop(self.settings_wnd);
                let _ = SetForegroundWindow(self.settings_wnd);
            }
            crate::log!("settings: brought forward on section {section}");
            settings_ui::show_section(self.settings_wnd, section);
            return;
        }
        let app = self as *mut App;
        self.settings_wnd = settings_ui::open(
            self.window_icon,
            self.tray_icon,
            &self.settings,
            self.monitor,
            app,
            section,
        )
        .unwrap_or_default();
    }

    pub(crate) fn settings_window_closed(&mut self) {
        self.settings_wnd = HWND::default();
    }

    /// Called by the settings window every time a row moves, with the whole draft
    /// rather than the one row. The hotkeys are re-registered first: if one is
    /// taken, nothing here changes at all, and the window pulls the row back and
    /// says why.
    ///
    /// Whole because the rows are not independent. A dial that is on but has no
    /// trigger, or a shortcut another application already owns, is a desktop that
    /// behaves like neither the old settings nor the new ones - which is worse
    /// than the change nobody asked for.
    pub(crate) fn apply_settings(&mut self, draft: settings::Draft) -> Result<(), String> {
        let settings::Draft {
            fav,
            panel,
            wheel_radius,
            wheel_dead,
            trigger_mods,
            trigger_vk,
            trigger_button,
            wheel_theme,
            wheel_accent,
            wheel_rotate,
            colour_scheme,
            wheel_open_ms,
            wheel_limit,
            dial_order,
            activate_pos,
            reduced_motion,
            lang,
        } = draft;
        // Only the shortcuts themselves need the machine. Re-registering two
        // combinations on every notch of the radius stepper was both wasteful
        // and a way for one refused registration to put a modal box in front of
        // a window full of buttons that then looked dead.
        let hotkeys_changed = (self.settings.fav_mods, self.settings.fav_vk) != fav
            || (self.settings.panel_mods, self.settings.panel_vk) != panel;
        if hotkeys_changed {
            self.apply_hotkeys(fav.0, fav.1, panel.0, panel.1)?;
        }
        self.settings.fav_mods = fav.0;
        self.settings.fav_vk = fav.1;
        self.settings.panel_mods = panel.0;
        self.settings.panel_vk = panel.1;
        self.settings.wheel_limit = wheel_limit;
        self.settings.dial_order = dial_order;
        self.settings.activate_pos = activate_pos;
        // The dial takes the same numbers the moment the row moves. What it does
        // not take is a half-applied change: the trigger is handed to the
        // machine, which keeps a gesture already in flight on the trigger it
        // started with, so a key changed mid-drag cannot strand a swallowed
        // button.
        self.settings.wheel = true;
        self.settings.wheel_radius = wheel_radius;
        self.settings.wheel_dead = wheel_dead;
        self.settings.trigger_mods = trigger_mods;
        self.settings.trigger_vk = trigger_vk;
        self.settings.trigger_button = trigger_button;
        self.settings.wheel_theme = wheel_theme;
        self.settings.wheel_accent = wheel_accent;
        // The seam is not a setting any more; the dial's turn has no row either,
        // and reaches this draft only from the drag around the picture.
        self.settings.wheel_seam = settings::WHEEL_SEAM_DEFAULT;
        self.settings.wheel_rotate = wheel_rotate;
        self.settings.colour_scheme = colour_scheme;
        self.settings.wheel_open_ms = wheel_open_ms;
        self.settings.reduced_motion = reduced_motion;
        // Applied here and not at the next start: the tray and the dial read the
        // same table, and a setting that waits until you restart is a setting
        // nobody believes.
        let lang = crate::i18n::lang_tag(&lang);
        let was_lang = std::mem::replace(&mut self.settings.lang, lang.clone());
        self.settings.save();
        crate::wheel::reconfigure(&self.settings);
        if was_lang != lang {
            crate::i18n::activate(&lang);
            crate::wheel::relanguage();
            self.tray_tip = t("app_name").to_string();
            if !self.tray_icon.is_invalid() {
                tray::set_tip(self.hwnd, &self.tray_tip);
            }
            crate::log!("language is now {lang}");
        }
        crate::log!(
            "settings applied: favourite={} panel={} radius={:.0} dead={:.2} \
             trigger_mods={trigger_mods} trigger_key={trigger_vk} button={trigger_button} motion={}",
            self.settings.fav_label(),
            self.settings.panel_label(),
            wheel_radius,
            wheel_dead,
            if reduced_motion { "reduced" } else { "full" }
        );
        Ok(())
    }

    /// The settings the desktop is running with right now, which after a refusal
    /// is not the same as what the panel was showing a moment ago. The window
    /// reads this to put a refused row back; nothing else needs it, and the
    /// private field stays private.
    pub(crate) fn applied_settings(&self) -> settings::Settings {
        self.settings.clone()
    }

    /// How big the management panel was when the user last let go of its corner,
    /// in logical pixels. Written through the same file as every other decision
    /// and read only by `settings_ui::open`, so the shape someone chose is the
    /// shape it comes back in. A drag that ends where it began is not a new
    /// decision and does not write the file.
    pub(crate) fn remember_panel_size(&mut self, w: f32, h: f32) {
        if (self.settings.panel_w - w).abs() < 0.5 && (self.settings.panel_h - h).abs() < 0.5 {
            return;
        }
        self.settings.panel_w = w;
        self.settings.panel_h = h;
        self.settings.save();
        crate::log!("panel size remembered: {w:.0}x{h:.0} logical");
    }

    fn apply_hotkeys(
        &mut self,
        fav_mods: u32,
        fav_vk: u32,
        panel_mods: u32,
        panel_vk: u32,
    ) -> Result<(), String> {
        let old = (
            self.settings.fav_mods,
            self.settings.fav_vk,
            self.settings.panel_mods,
            self.settings.panel_vk,
        );
        unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_FAV);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PANEL);
        }
        let mut failed = Vec::new();
        let switch = |id: i32, mods: u32, vk: u32, failed: &mut Vec<String>| {
            if vk == 0 {
                return;
            }
            let effective = HOT_KEY_MODIFIERS(mods) | MOD_NOREPEAT;
            unsafe {
                if RegisterHotKey(Some(self.hwnd), id, effective, vk).is_err() {
                    failed.push(settings::hotkey_label(mods, vk));
                }
            }
        };
        switch(HOTKEY_FAV, fav_mods, fav_vk, &mut failed);
        switch(HOTKEY_PANEL, panel_mods, panel_vk, &mut failed);
        if !failed.is_empty() {
            unsafe {
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_FAV);
                let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_PANEL);
                if old.1 != 0 {
                    let mods = HOT_KEY_MODIFIERS(old.0) | MOD_NOREPEAT;
                    let _ = RegisterHotKey(Some(self.hwnd), HOTKEY_FAV, mods, old.1);
                }
                if old.3 != 0 {
                    let mods = HOT_KEY_MODIFIERS(old.2) | MOD_NOREPEAT;
                    let _ = RegisterHotKey(Some(self.hwnd), HOTKEY_PANEL, mods, old.3);
                }
            }
            return Err(format!(
                "{}: {}",
                t("settings_hotkey_failed"),
                failed.join(", ")
            ));
        }
        crate::log!(
            "hotkeys now {} / {}",
            settings::hotkey_label(fav_mods, fav_vk),
            settings::hotkey_label(panel_mods, panel_vk),
        );
        Ok(())
    }
}

/// Draws a canvas onto one of the app's layered windows.
///
/// The dial's two layers and nothing else: this is the only pixel the program
/// puts on the desktop by itself, and the only place the RGBA-to-BGRA swap and
/// the recycled DIB live.
pub(crate) fn present_layer(
    hwnd: HWND,
    canvas: &Pixmap,
    dibs: &mut Option<Dib>,
    origin: (i32, i32),
    failed: &mut bool,
) {
    let (w, h) = (canvas.width() as i32, canvas.height() as i32);
    // Grow the DIB only when the frame no longer fits in it. The disc's canvas is
    // the wheel's extent at whatever scale the monitor under it answers with, so
    // one gesture after a change of screen or of radius needs a bigger bitmap than
    // the last one did; allocating the GDI bitmap every frame is a cost the frame
    // rate pays for no visible reason.
    let reuse = dibs.as_ref().is_some_and(|d| {
        let area = (d.w * d.h) as i64;
        let want = (w * h) as i64;
        d.w >= w && d.h >= h && area <= want * 5 / 2
    });
    if !reuse {
        *dibs = Dib::new(w, h);
    }
    let Some(dib) = dibs.as_ref() else { return };
    let stride = (dib.w * 4) as usize;
    let row = (w * 4) as usize;
    unsafe {
        let dst = std::slice::from_raw_parts_mut(dib.bits, stride * dib.h as usize);
        // tiny-skia speaks RGBA, the layered window wants BGRA. One 32-bit
        // swap per pixel, a row at a time, because the DIB may be wider
        // than this frame.
        for (y, src) in canvas.data().chunks_exact(row).enumerate() {
            let out = &mut dst[y * stride..y * stride + row];
            for (to, from) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                let v = u32::from_ne_bytes([from[0], from[1], from[2], from[3]]);
                let swapped = (v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                to.copy_from_slice(&swapped.to_ne_bytes());
            }
        }
    }
    let origin = POINT {
        x: origin.0,
        y: origin.1,
    };
    let size = SIZE { cx: w, cy: h };
    let source = POINT::default();
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    unsafe {
        if let Err(error) = UpdateLayeredWindow(
            hwnd,
            None,
            Some(&origin),
            Some(&size),
            Some(dib.hdc),
            Some(&source),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        ) {
            if !*failed {
                *failed = true;
                crate::log!("UpdateLayeredWindow failed (nothing will show): {error}");
            }
        }
    }
}

thread_local! {
    /// The scratch bitmap every [`blit_pixmap`] copy goes through. The settings
    /// page draws seven thumbnails per paint and the switch draws its art every
    /// 16ms of a slide; making and destroying a DIB section for each of those
    /// was an allocation and a compatible DC the frame paid for every time.
    static BLIT_SCRATCH: RefCell<Option<Dib>> = const { RefCell::new(None) };
}

/// Copies a premultiplied RGBA raster onto `hdc`, swapped into the blue-first
/// order a 32-bit DIB wants, and blended. `dw`/`dh` stretch the copy.
pub(crate) fn blit_pixmap(
    hdc: HDC,
    x: i32,
    y: i32,
    pixels: &[u8],
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
) {
    if sw == 0
        || sh == 0
        || dw == 0
        || dh == 0
        || pixels.len() < (sw as usize * sh as usize * 4)
    {
        return;
    }
    BLIT_SCRATCH.with(|slot| {
        let mut slot = slot.borrow_mut();
        let reuse = slot
            .as_ref()
            .is_some_and(|d| d.w >= sw as i32 && d.h >= sh as i32);
        if !reuse {
            *slot = Dib::new(sw as i32, sh as i32);
        }
        let Some(dib) = slot.as_ref() else { return };
        let stride = (dib.w * 4) as usize;
        let row = (sw * 4) as usize;
        unsafe {
            let dst = std::slice::from_raw_parts_mut(dib.bits, stride * dib.h as usize);
            for (y, src) in pixels.chunks_exact(row).enumerate() {
                let out = &mut dst[y * stride..y * stride + row];
                // tiny-skia speaks RGBA, the DIB wants BGRA. One 32-bit swap
                // per pixel; the row may be narrower than the bitmap.
                for (to, from) in out.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    let v = u32::from_ne_bytes([from[0], from[1], from[2], from[3]]);
                    let swapped = (v & 0xFF00_FF00) | ((v & 0xFF) << 16) | ((v >> 16) & 0xFF);
                    to.copy_from_slice(&swapped.to_ne_bytes());
                }
            }
        }
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            let _ = AlphaBlend(
                hdc,
                x,
                y,
                dw as i32,
                dh as i32,
                dib.hdc,
                0,
                0,
                sw as i32,
                sh as i32,
                blend,
            );
        }
    });
}

fn taskbar_created() -> u32 {
    use std::sync::OnceLock;
    static MSG: OnceLock<u32> = OnceLock::new();
    *MSG.get_or_init(|| unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let (result, deferred) = (*ptr).on_message(msg, wp, lp);
        match deferred {
            Deferred::None => result,
            Deferred::Menu => {
                if let Some(id) = (*ptr).finish_menu() {
                    (*ptr).command(id);
                }
                LRESULT(0)
            }
        }
    }
}
