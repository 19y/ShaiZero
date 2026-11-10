//! Tray icon, its menu, balloons, and the "start with Windows" entry.

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconGetRect, Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP,
    NIF_TIP, NIIF_INFO, NIIF_WARNING, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NOTIFYICONDATAW, NOTIFYICONDATAW_0, NOTIFYICONIDENTIFIER, NOTIFYICON_VERSION_4,
};
use windows::Win32::UI::WindowsAndMessaging::{HICON, WM_APP};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

pub const WM_TRAY: u32 = WM_APP + 2;
pub const TRAY_ID: u32 = 1;

/// The tray's clicks under `NOTIFYICON_VERSION_4`: the menu comes as
/// `WM_CONTEXTMENU`, a click as `NIN_SELECT` (or `NIN_KEYSELECT` when it came
/// from the keyboard). They replace the version-0 `WM_*BUTTONUP` pairs, so both
/// spellings are answered below.
pub const NIN_SELECT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER;
pub const NIN_KEYSELECT: u32 = windows::Win32::UI::WindowsAndMessaging::WM_USER + 1;
pub const WM_CONTEXTMENU: u32 = 0x007B;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "ShaiZero";
/// The value the same switch wrote under earlier names. A rename must not leave
/// the old entry behind, still starting an exe that no longer exists, nor lose
/// the fact that the switch was on.
const LEGACY_RUN_VALUES: [&str; 3] = ["SpinWin", "ShaiWin", "PanWen"];

fn nid(hwnd: HWND, icon: Option<HICON>) -> NOTIFYICONDATAW {
    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: TRAY_ID,
        uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY,
        Anonymous: NOTIFYICONDATAW_0 {
            uVersion: NOTIFYICON_VERSION_4,
        },
        ..Default::default()
    };
    if let Some(icon) = icon {
        data.hIcon = icon;
    }
    data
}

fn set_text(data: &mut NOTIFYICONDATAW, slot: &mut [u16; 128], text: &str) {
    let wide: Vec<u16> = text.encode_utf16().take(127).collect();
    slot[..wide.len()].copy_from_slice(&wide);
    data.szTip = *slot;
}

/// The icon the tray accepted, kept so a balloon can be raised from code that
/// does not hold a borrow of the app (the wheel).
static TRAY_OWNER: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
static TRAY_ICON: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

pub fn add(hwnd: HWND, icon: HICON, tip: &str) -> bool {
    unsafe {
        let mut data = nid(hwnd, Some(icon));
        let mut text = [0u16; 128];
        set_text(&mut data, &mut text, tip);
        let added = Shell_NotifyIconW(NIM_ADD, &data).as_bool();
        if added {
            // Version 4 is what tells the shell this is a modern notification
            // icon rather than a version-0 one: it is what gives the icon a real
            // place on the taskbar (and a real rectangle from `GetRect`) instead
            // of the overflow, and it is what makes a click arrive as
            // `NIN_SELECT` / `WM_CONTEXTMENU`. The version is stated in the
            // `NIM_ADD` data too, because a shell that reads it there places the
            // icon right away rather than moving it on the next redrawn.
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);
            TRAY_OWNER.store(hwnd.0 as isize, std::sync::atomic::Ordering::Relaxed);
            TRAY_ICON.store(icon.0 as isize, std::sync::atomic::Ordering::Relaxed);
        }
        added
    }
}

/// A balloon for something the wheel noticed while the settings page is out of
/// the way. Falls back to the journal when there is no tray icon to speak from.
pub fn balloon_remembered(title: &str, body: &str, warning: bool) {
    let owner = TRAY_OWNER.load(std::sync::atomic::Ordering::Relaxed);
    let icon = TRAY_ICON.load(std::sync::atomic::Ordering::Relaxed);
    if owner == 0 || icon == 0 {
        crate::log!("tray: {title}: {body}");
        return;
    }
    balloon(
        HWND(owner as *mut core::ffi::c_void),
        HICON(icon as *mut core::ffi::c_void),
        title,
        body,
        warning,
    );
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let data = nid(hwnd, None);
        let _ = Shell_NotifyIconW(NIM_DELETE, &data);
    }
}

/// Where the shell says the icon is. This is the one answer that separates the
/// three states a "the icon is missing" report can mean: the notification area
/// itself (a rectangle on the taskbar), the overflow (registered, but Windows
/// hides it until the user drags it out), or nowhere (the add never took).
/// Only the shell knows, so it is asked rather than guessed from the add's
/// return value.
pub fn place(hwnd: HWND) -> String {
    unsafe {
        let id = NOTIFYICONIDENTIFIER {
            cbSize: std::mem::size_of::<NOTIFYICONIDENTIFIER>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            ..Default::default()
        };
        match Shell_NotifyIconGetRect(&id) {
            // A rectangle only proves the icon is registered. Windows 11 answers
            // with the overflow button's rectangle when the icon itself is parked
            // behind the chevron, so the log names the rectangle rather than
            // calling it the notification area.
            Ok(area) => format!(
                "registered, the shell reports the rectangle ({}, {})-({}, {})",
                area.left, area.top, area.right, area.bottom
            ),
            // `ERROR_NOT_FOUND` is the other answer for an icon the shell has no
            // place for.
            Err(error) if error.code() == windows::Win32::Foundation::ERROR_NOT_FOUND.to_hresult() => {
                "registered but hidden in the overflow".to_string()
            }
            Err(error) => format!("not registered ({error})"),
        }
    }
}

/// Changes only the tooltip. The other flags are deliberately left out: a
/// NIM_MODIFY that carries NIF_ICON re-publishes the icon, and the shell
/// redraws the notification area every time it happens, which reads as the
/// taskbar flickering while the line is being used.
pub fn set_tip(hwnd: HWND, tip: &str) {
    unsafe {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: TRAY_ID,
            uFlags: NIF_TIP | NIF_SHOWTIP,
            ..Default::default()
        };
        let mut text = [0u16; 128];
        set_text(&mut data, &mut text, tip);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// Shows a tray balloon. Used for errors that happen while the line is hidden.
/// `NOTIFYICONDATAW` carries the balloon text in fixed arrays, and Windows cuts
/// whatever does not fit at the end of the array - which is how a lesson in a
/// language with longer words than yours used to stop in the middle of one.
pub const BALLOON_MAX: usize = 255;
const TITLE_MAX: usize = 63;

pub fn balloon(hwnd: HWND, icon: HICON, title: &str, body: &str, warning: bool) {
    unsafe {
        let mut data = nid(hwnd, Some(icon));
        data.uFlags = NIF_INFO | NIF_ICON;
        data.dwInfoFlags = if warning { NIIF_WARNING } else { NIIF_INFO };
        let title_wide = clip(title, TITLE_MAX);
        data.szInfoTitle[..title_wide.len()].copy_from_slice(&title_wide);
        let body_wide = clip(body, BALLOON_MAX);
        data.szInfo[..body_wide.len()].copy_from_slice(&body_wide);
        let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
    }
}

/// `text` as UTF-16 for one of those arrays, at most `max` units. When it does
/// not fit, the cut comes at the last whole word that does, and the reader is
/// told the sentence went on.
fn clip(text: &str, max: usize) -> Vec<u16> {
    let mut keep = text.to_string();
    if keep.chars().count() > max {
        let head: String = keep.chars().take(max.saturating_sub(1)).collect();
        match head.rfind(char::is_whitespace) {
            // A language with no spaces in it at all is not a reason to show
            // nothing: take the characters.
            Some(at) if at > max / 2 => keep = head[..at].trim_end().to_string(),
            _ => keep = head,
        }
        keep.push('…');
    }
    keep.encode_utf16().take(max).collect()
}

pub fn start_at_login() -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ) else {
        return false;
    };
    std::iter::once(RUN_VALUE)
        .chain(LEGACY_RUN_VALUES)
        .any(|name| key.get_value::<String, _>(name).is_ok())
}

pub fn set_start_at_login(on: bool) -> bool {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let Ok(key) = hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE) else {
        return false;
    };
    if on {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        // `--tray` tells the copy started by the login entry to stay in the
        // notification area. A double click has no such argument and opens the
        // settings page, because a silent launch is indistinguishable from one
        // that failed.
        let command = format!("\"{}\" --tray", exe.display());
        if key.set_value(RUN_VALUE, &command).is_err() {
            return false;
        }
        // The new entry is in place; the old one now points at an exe that no
        // longer exists, so the next login would start nothing.
        for old in LEGACY_RUN_VALUES {
            let _ = key.delete_value(old);
        }
        true
    } else {
        let _ = key.delete_value(RUN_VALUE);
        for old in LEGACY_RUN_VALUES {
            let _ = key.delete_value(old);
        }
        true
    }
}

/// Screen position of the mouse, for popup menus.
pub fn cursor() -> POINT {
    let mut p = POINT::default();
    unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::GetCursorPos(&mut p);
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(text: &str, max: usize) -> String {
        String::from_utf16(&clip(text, max)).expect("the clip is valid text")
    }

    #[test]
    fn a_short_balloon_is_left_alone() {
        assert_eq!(shown("Quit ShaiZero", 63), "Quit ShaiZero");
    }

    /// The cut is where a translator meets the API, so it has to land somewhere
    /// a person can read: at a word, with the tail of the sentence admitted.
    #[test]
    fn a_cut_balloon_ends_at_a_word() {
        let long = "one two three four five six seven eight nine ten";
        let cut = shown(long, 21);
        assert_eq!(cut, "one two three four…");
        assert!(cut.chars().count() <= 21);
        // A language that writes the whole lesson without a space still gets the
        // characters it has room for instead of an empty balloon.
        let packed = "一二三四五六七八九十";
        assert_eq!(shown(packed, 4), "一二三…");
    }
}
