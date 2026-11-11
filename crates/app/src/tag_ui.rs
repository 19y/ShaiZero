//! The little "name this window" label editor.
//!
//! A tag is one short line the user types, so the field is a stock `EDIT`:
//! IME, selection, clipboard and every keyboard the user already knows come for
//! free. Everything around it — the paper, the live colour, the buttons — is
//! painted here, so the editor can sit under the window it belongs to instead
//! of opening a second, grey window in the middle of the screen.
//!
//! It is a popover, not a modal: the paper repaints in the colour the tag is
//! about to have on every keystroke, Enter saves, Escape puts
//! the old tag back, and clicking anywhere else saves. That is also why the
//! window is anchored under the window it names and sized like a piece of it.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use shaiwin_core::theme::{tag_color, tag_ink};
use shaiwin_platform_win::monitor::Monitor;
use shaiwin_platform_win::Rect as WRect;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateRoundRectRgn, CreateSolidBrush, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetDC, GetStockObject, GetTextExtentPoint32W, GetTextMetricsW,
    InvalidateRect, ReleaseDC, RoundRect, SelectObject, SetBkColor, SetBkMode, SetTextColor,
    SetWindowRgn, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DEFAULT_PITCH, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD, HBRUSH, HDC, HFONT, HGDIOBJ,
    OUT_DEFAULT_PRECIS, PS_SOLID, TEXTMETRICW, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Controls::{EM_SETCUEBANNER, WM_MOUSELEAVE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, ReleaseCapture, SetActiveWindow, SetCapture, SetFocus, TrackMouseEvent,
    TME_LEAVE, TRACKMOUSEEVENT, VK_CONTROL, VK_ESCAPE, VK_RETURN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetCaretBlinkTime,
    GetCaretPos, GetClientRect, GetForegroundWindow, GetParent, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, HideCaret, IsWindow, KillTimer, LoadCursorW,
    RegisterClassExW, SendMessageW, SetCursor, SetForegroundWindow, SetTimer, SetWindowLongPtrW,
    SetWindowPos, SetWindowTextW, ShowWindow, CS_DROPSHADOW, EN_CHANGE, GWLP_USERDATA,
    GWLP_WNDPROC, HICON, HWND_TOPMOST, IDC_ARROW, IDC_HAND, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_SHOW, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_CLOSE, WM_COMMAND,
    WM_CHAR, WM_CTLCOLOREDIT, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCDESTROY, WM_PAINT, WM_SETCURSOR, WM_SETFOCUS, WM_SETFONT,
    WM_TIMER, WNDCLASSEXW, WNDPROC,
    WS_CHILD, WS_CLIPCHILDREN, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_TABSTOP, WS_VISIBLE,
};

use crate::app::App;
use crate::i18n::t;

const CLASS: PCWSTR = w!("PanWen.Tag");
const OPEN_FAILED: isize = 0;

/// The field is a plain edit control: the app talks to it with these three
/// messages, none of which has a friendly wrapper in this binding.
const EM_SETLIMITTEXT: u32 = 0x00C5;
const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_LEFT: u32 = 0x0000;
const EM_SETSEL: u32 = 0x00B1;
const EM_SCROLLCARET: u32 = 0x00B7;
/// Drives the field's caret between its lit and dark halves; see `draw_caret`.
/// The popover owns no other timer, so any id will do.
const CARET_TIMER: usize = 0x7A11;

/// Logical geometry. The width is deliberately small: a 300 px popover under a
/// sector a few centimetres wide would read as a separate window.
const WIDTH: f32 = 286.0;
const HEIGHT: f32 = 136.0;
const PAD: f32 = 14.0;
const TITLE_H: f32 = 16.0;
const FIELD_GAP: f32 = 8.0;
/// Tall enough for one line of the field's font plus even air above and below.
/// A taller box does not make the field easier to hit; it just leaves the text
/// stranded in the middle of a lot of nothing.
const FIELD_H: f32 = 34.0;
const BUTTON_H: f32 = 30.0;
const BUTTON_GAP: f32 = 8.0;
/// Room inside a button between its edge and the word on it.
const BUTTON_X: f32 = 12.0;
/// The least the row of three buttons may have between the ghost on the left and
/// the pair on the right; below this the popover itself grows.
const ROW_FLEX: f32 = 24.0;
const SAVE_W: f32 = 76.0;
const CANCEL_W: f32 = 62.0;
const CLEAR_W: f32 = 72.0;
const RADIUS: f32 = 12.0;

const PAPER: (u8, u8, u8) = (252, 252, 254);
const INK: (u8, u8, u8) = (34, 34, 42);
const MUTED: (u8, u8, u8) = (118, 122, 134);
const FIELD: (u8, u8, u8) = (246, 247, 250);
const EDGE: (u8, u8, u8) = (222, 225, 232);
const GHOST_HOT: (u8, u8, u8) = (238, 240, 245);
const SLATE: (u8, u8, u8) = (44, 49, 60);

static OPEN: AtomicIsize = AtomicIsize::new(OPEN_FAILED);

struct Fonts {
    title: HFONT,
    edit: HFONT,
    button: HFONT,
    ghost: HFONT,
}

/// What the popover is naming: a window the dial seats, by its fingerprint.
///
/// The label lives in `settings.txt` and outlives both the window and the
/// application, which is why the box writes the fingerprint and nothing writes
/// the handle. `heading` is the line above the field and `cue` the word inside an
/// empty field, both i18n keys: the shortcut says what the press did, because one
/// key did two things, and the panel says only what the box is for. The caller
/// knows which, and the box does not.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) fingerprint: String,
    pub(crate) heading: &'static str,
    pub(crate) cue: &'static str,
}

struct Dialog {
    app: *mut App,
    target: Target,
    edit: HWND,
    scale: f32,
    /// The popover's width and the three buttons' widths, both measured from the
    /// words the current language puts on them. `layout` is a pure function of
    /// these, so the paint, the hit-test and the edit control cannot disagree.
    width: i32,
    row: [i32; 3],
    fonts: Fonts,
    field: HBRUSH,
    hover: Hit,
    pressed: Hit,
    /// False between creation and the first activation; a deactivation in that
    /// gap is setup noise, not the user clicking away.
    ready: bool,
    /// True once the window is on its way out, so a deactivation fired by
    /// `DestroyWindow` cannot commit a cancelled edit.
    closing: bool,
    /// Whether the next paint of the field draws the caret bar. The blink is
    /// ours, on [`CARET_TIMER`], so it cannot be stopped by the system clock
    /// going quiet; see [`draw_caret`].
    caret_lit: bool,
    /// How tall the bar is: one line of the field's font, in device pixels.
    caret_h: i32,
}

/// The field is the stock control plus two keys. The IME, the selection and the
/// text stay entirely the control's; only the caret bar is painted by us, over
/// the text it belongs to (see `draw_caret`), because the control's own stopped
/// blinking a few seconds into the popover's life.
struct Edit {
    old: WNDPROC,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Hit {
    None,
    Clear,
    Cancel,
    Save,
}

struct Layout {
    title: RECT,
    dot: RECT,
    field: RECT,
    clear: RECT,
    cancel: RECT,
    save: RECT,
}

pub fn register_class(hinst: HINSTANCE, big: HICON, small: HICON) {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_DROPSHADOW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: big,
            hIconSm: small,
            lpszClassName: CLASS,
            ..Default::default()
        };
        let _ = RegisterClassExW(&class);
    }
}

/// Where the popover goes, in screen pixels: the middle of the work area when
/// nothing says otherwise, the middle of `center_in` when a page has a frame of
/// its own, or under `anchor` when the caller wants it under something. Whatever
/// that works out to is then held inside the work area, so a box can never land
/// off the screen or straddle two.
///
/// The middle of the screen is the answer when the caller names a window but no
/// frame: a window that sits on the floor of the desktop - maximised, or parked
/// against the taskbar - would otherwise take the box down with it, and a box on
/// the last line of the screen is the one place it cannot be read.
fn place(
    width: i32,
    height: i32,
    work: WRect,
    anchor: Option<WRect>,
    center_in: Option<WRect>,
    s: f32,
) -> (i32, i32) {
    let (want_x, want_y) = match center_in {
        Some(rect) => (
            (rect.left + rect.right) / 2 - width / 2,
            (rect.top + rect.bottom) / 2 - height / 2,
        ),
        None => match anchor {
            Some(rect) => {
                let cx = (rect.left + rect.right) / 2;
                (cx - width / 2, rect.bottom + (10.0 * s).round() as i32)
            }
            None => (
                work.left + (work.width() - width) / 2,
                work.top + (work.height() - height) / 2,
            ),
        },
    };
    let max_x = (work.right - width).max(work.left);
    let max_y = (work.bottom - height).max(work.top);
    (want_x.clamp(work.left, max_x), want_y.clamp(work.top, max_y))
}

/// Opens the popover under `anchor` (screen pixels) for one window. Only
/// one may exist; opening another commits the first, which is what clicking away
/// means.
pub fn open(
    parent: HWND,
    app: *mut App,
    target: Target,
    initial: &str,
    monitor: Monitor,
    anchor: Option<WRect>,
    center_in: Option<WRect>,
) {
    unsafe {
        let existing = OPEN.load(Ordering::Relaxed);
        if existing != OPEN_FAILED {
            let hwnd = HWND(existing as *mut c_void);
            if IsWindow(Some(hwnd)).as_bool() {
                save(hwnd);
            } else {
                OPEN.store(OPEN_FAILED, Ordering::Relaxed);
            }
        }
        let Some(hinst) = GetModuleHandleW(None).ok().map(HINSTANCE::from) else {
            return;
        };
        let s = monitor.scale.max(1.0);
        // The same face the rest of the app writes in, chosen by the language
        // rather than by which one the developer speaks. It is needed before the
        // window is made, because the words on the buttons decide how wide it is.
        let face = crate::i18n::ui_face();
        let fonts = Fonts {
            title: font(s, 11.5, FW_NORMAL.0 as i32, face),
            edit: font(s, 13.5, FW_NORMAL.0 as i32, face),
            button: font(s, 12.5, FW_SEMIBOLD.0 as i32, face),
            ghost: font(s, 12.0, FW_NORMAL.0 as i32, face),
        };
        let row = button_row(fonts.button, s);
        let width = row_width(s, &row);
        if width > (WIDTH * s).round() as i32 {
            crate::log!("the tag popover grew to {width}px to hold the words of this language");
        }
        let height = (HEIGHT * s).round() as i32;
        let (x, y) = place(width, height, monitor.work, anchor, center_in, s);

        let empty = wstr("");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0),
            CLASS,
            PCWSTR(empty.as_ptr()),
            WINDOW_STYLE(WS_POPUP.0 | WS_CLIPCHILDREN.0),
            x,
            y,
            width,
            height,
            Some(parent),
            None,
            Some(hinst),
            None,
        );

        let Ok(hwnd) = hwnd else {
            return;
        };

        // Windows 11 rounds top-level windows itself; ask for it, and keep the
        // region as the fallback for older builds and for the clipped corners.
        let preference = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&preference as *const _) as *const c_void,
            std::mem::size_of_val(&preference) as u32,
        );
        let radius = (RADIUS * 2.0 * s).round() as i32;
        let region = CreateRoundRectRgn(0, 0, width + 1, height + 1, radius, radius);
        if !region.is_invalid() {
            let _ = SetWindowRgn(hwnd, Some(region), true);
        }

        let field = CreateSolidBrush(rgb(FIELD));

        let layout = layout(s, width, &row);
        let edit_x = layout.field.left + (12.0 * s).round() as i32;
        let edit_w = (layout.field.right - layout.field.left) - (24.0 * s).round() as i32;
        // A stock single-line edit draws its line from the top of its client
        // area: the caret sits at y = 0 and the box below the text is empty.
        // So the box is cut to one line, and it is the *text* that gets
        // centred in the field, not the box. A box sized by eye is what made
        // the label ride high in its own field.
        let field_h = layout.field.bottom - layout.field.top;
        let line_h = text_height(fonts.edit, s);
        let slack = (2.0 * s).round() as i32;
        let edit_h = (line_h + slack).min(field_h);
        // Glyphs do not fill the line box. A font keeps a band below the
        // baseline for descenders, and a line of CJK or of lower-case Latin
        // never reaches into it, so a box centred on the field still leaves the
        // words sitting high with a gap under them. Half that band is what puts
        // the ink, rather than the box, on the field's centre line.
        let nudge = ((line_h as f32) * 0.075).round() as i32;
        let edit_y =
            (layout.field.top + (field_h - edit_h) / 2 + nudge).min(layout.field.bottom - edit_h);
        let initial_wide = wstr(initial);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(initial_wide.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | ES_LEFT | ES_AUTOHSCROLL),
            edit_x,
            edit_y,
            edit_w,
            edit_h,
            Some(hwnd),
            None,
            Some(hinst),
            None,
        )
        .unwrap_or_default();
        if !edit.is_invalid() {
            let _ = SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(fonts.edit.0 as usize)),
                Some(LPARAM(1)),
            );
            let _ = SendMessageW(edit, EM_SETLIMITTEXT, Some(WPARAM(24)), Some(LPARAM(0)));
            // A window about to hold a seat is named for that reason, and the
            // empty field says what to put in it; a row in the panel is only
            // being labelled. The caller knows which, and says so with the key.
            let cue = wstr(t(target.cue));
            let _ = SendMessageW(
                edit,
                EM_SETCUEBANNER,
                Some(WPARAM(1)),
                Some(LPARAM(cue.as_ptr() as isize)),
            );
        }

        let state = Box::into_raw(Box::new(Dialog {
            app,
            target,
            edit,
            scale: s,
            width,
            row,
            fonts,
            field,
            hover: Hit::None,
            pressed: Hit::None,
            ready: false,
            closing: false,
            // The field opens lit, the way a caret that just started blinking
            // does; the first tick is then the dark half.
            caret_lit: true,
            caret_h: line_h,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, state as isize);
        OPEN.store(hwnd.0 as isize, Ordering::Relaxed);

        // Enter and Escape belong to the field; every other key, including the
        // IME's and the clipboard's, is left exactly as the edit control wants.
        if !edit.is_invalid() {
            let old = std::mem::transmute::<isize, WNDPROC>(GetWindowLongPtrW(edit, GWLP_WNDPROC));
            let boxed = Box::into_raw(Box::new(Edit { old }));
            SetWindowLongPtrW(edit, GWLP_USERDATA, boxed as isize);
            SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        take_focus(hwnd, edit);
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        if !edit.is_invalid() {
            let _ = SetFocus(Some(edit));
            // A rename starts with the old tag selected, so the first keystroke
            // replaces it rather than landing in front of it. The caret then
            // sits at the end of that selection, where the edit puts it.
            let _ = SendMessageW(edit, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            let _ = SendMessageW(edit, EM_SCROLLCARET, Some(WPARAM(0)), Some(LPARAM(0)));
            // The bar is painted here (see `draw_caret`); the stock caret is
            // kept hidden so the field never shows two.
            let _ = HideCaret(Some(edit));
            let _ = InvalidateRect(Some(edit), None, false);
            let _ = InvalidateRect(Some(hwnd), None, false);
            let _ = SetTimer(Some(hwnd), CARET_TIMER, blink_ms(), None);
            crate::log!(
                "tag caret: opened lit line_h={line_h}px blink={}ms focus={}",
                blink_ms(),
                if GetFocus() == edit { "edit" } else { "elsewhere" }
            );
        }
        (*state).ready = true;
    }
}

/// How often the caret changes phase: one blink period, so it is lit for one
/// and dark for the next - the rhythm `GetCaretBlinkTime` names.
fn blink_ms() -> u32 {
    let blink = unsafe { GetCaretBlinkTime() };
    // 0xFFFF_FFFF means the user asked for no blinking at all; 530 ms is the
    // system default.
    if (200..=1200).contains(&blink) {
        blink
    } else {
        530
    }
}

/// The bar's rectangle in the field's client area, or `None` when there is no
/// caret to ask. `GetCaretPos` still answers while the caret is hidden, which is
/// the position the stock control keeps moving as the user types.
unsafe fn caret_bar(edit: HWND, state: &Dialog) -> Option<RECT> {
    unsafe {
        let mut at = POINT::default();
        if GetCaretPos(&mut at).is_err() {
            return None;
        }
        let mut client = RECT::default();
        GetClientRect(edit, &mut client).ok()?;
        let width = (2.0 * state.scale).round().max(1.0) as i32;
        let y = at.y.clamp(0, (client.bottom - state.caret_h).max(0));
        Some(RECT {
            left: at.x,
            top: y,
            right: at.x + width,
            bottom: y + state.caret_h,
        })
    }
}

/// Paints the field's caret when the blink is in its lit half.
///
/// Windows' own caret is hidden (see `open` and `edit_proc`) because its blink
/// clock goes quiet a few seconds after the popover opens, leaving a still bar
/// in a field that is perfectly ready to be typed into. Painting the bar on our
/// own timer keeps it flashing for as long as the box is up, at the rate the
/// system asks for, and it is drawn after the text so it sits over the glyphs
/// exactly where the hidden caret is.
unsafe fn draw_caret(edit: HWND, state: &Dialog) {
    unsafe {
        if !state.caret_lit || state.closing || GetFocus() != edit {
            return;
        }
        let Some(bar) = caret_bar(edit, state) else {
            return;
        };
        let dc = GetDC(Some(edit));
        if dc.is_invalid() {
            return;
        }
        let brush = CreateSolidBrush(rgb(INK));
        let _ = FillRect(dc, &bar, brush);
        let _ = DeleteObject(brush.into());
        let _ = ReleaseDC(Some(edit), dc);
    }
}

/// Makes the popover the foreground window so the field gets the keyboard.
///
/// A click on the window list does not make our process foreground (the list
/// is shown without activation), and the shortcut fires while some other
/// app holds it, so `SetForegroundWindow` can be refused by the rules that
/// stop background apps from stealing focus. Attaching to the foreground
/// thread for the call is the documented way through; the field is given the
/// keyboard inside that attachment, because `SetFocus` is refused to a thread
/// that does not own the foreground window. It is detached right away, before
/// any of our message handling runs.
unsafe fn take_focus(hwnd: HWND, edit: HWND) {
    unsafe {
        let foreground = GetForegroundWindow();
        let me = GetCurrentThreadId();
        let owner = if foreground.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(foreground, None)
        };
        let attached = owner != 0 && owner != me && AttachThreadInput(me, owner, true).as_bool();
        let _ = SetForegroundWindow(hwnd);
        let _ = SetActiveWindow(hwnd);
        if !edit.is_invalid() {
            let _ = SetFocus(Some(edit));
        }
        if attached {
            let _ = AttachThreadInput(me, owner, false);
        }
    }
}

/// Saves what is in the field and closes. Used when the user goes back to the
/// window list while the editor is open: the list is shown without activation,
/// so the ordinary deactivation path never runs.
pub fn commit_open() {
    let raw = OPEN.load(Ordering::Relaxed);
    if raw == OPEN_FAILED {
        return;
    }
    let hwnd = HWND(raw as *mut c_void);
    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
        save(hwnd);
    }
}

/// The three buttons, as wide as the words they carry. A popover that cuts
/// "Убрать метку" down to "Убрать мет…" has thrown away the thing the button is
/// for, so the words set the width and not the other way round.
unsafe fn button_row(font: HFONT, s: f32) -> [i32; 3] {
    let inset = (BUTTON_X * s).round() as i32;
    let min = |logical: f32| (logical * s).round() as i32;
    let word = |key: &str| text_width(font, t(key)) + 2 * inset;
    [
        min(CLEAR_W).max(word("tag_clear")),
        min(CANCEL_W).max(word("tag_cancel")),
        min(SAVE_W).max(word("tag_ok")),
    ]
}

/// The popover: the width it ships with, or wider when a language
/// needs the room for its three words.
fn row_width(s: f32, row: &[i32; 3]) -> i32 {
    let pad = (PAD * s).round() as i32;
    let gap = (BUTTON_GAP * s).round() as i32;
    let flex = (ROW_FLEX * s).round() as i32;
    let needed = pad * 2 + row[0] + flex + row[1] + gap + row[2];
    ((WIDTH * s).round() as i32).max(needed)
}

/// How wide `text` comes out in `font`, in device pixels.
unsafe fn text_width(font: HFONT, text: &str) -> i32 {
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return (text.chars().count() as f32 * 7.0) as i32;
        }
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        let ok = GetTextExtentPoint32W(dc, &wide, &mut size).as_bool();
        let _ = SelectObject(dc, old);
        let _ = ReleaseDC(None, dc);
        if ok {
            size.cx
        } else {
            (wide.len() as f32 * 7.0) as i32
        }
    }
}

fn layout(s: f32, width: i32, row: &[i32; 3]) -> Layout {
    let pad = (PAD * s).round() as i32;
    let title_h = (TITLE_H * s).round() as i32;
    let field_top = pad + title_h + (FIELD_GAP * s).round() as i32;
    let field_h = (FIELD_H * s).round() as i32;
    let button_h = (BUTTON_H * s).round() as i32;
    let button_top = (HEIGHT * s).round() as i32 - pad - button_h;
    let right = width - pad;
    let save_w = row[2];
    let cancel_w = row[1];
    let clear_w = row[0];
    let gap = (BUTTON_GAP * s).round() as i32;
    Layout {
        title: RECT {
            left: pad + (14.0 * s).round() as i32,
            top: pad,
            right,
            bottom: pad + title_h,
        },
        dot: RECT {
            left: pad,
            top: pad + (4.0 * s).round() as i32,
            right: pad + (8.0 * s).round() as i32,
            bottom: pad + (12.0 * s).round() as i32,
        },
        field: RECT {
            left: pad,
            top: field_top,
            right,
            bottom: field_top + field_h,
        },
        clear: RECT {
            left: pad,
            top: button_top,
            right: pad + clear_w,
            bottom: button_top + button_h,
        },
        cancel: RECT {
            left: right - save_w - gap - cancel_w,
            top: button_top,
            right: right - save_w - gap,
            bottom: button_top + button_h,
        },
        save: RECT {
            left: right - save_w,
            top: button_top,
            right,
            bottom: button_top + button_h,
        },
    }
}

fn read_edit(edit: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(edit).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let got = GetWindowTextW(edit, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..got.min(len)])
    }
}

fn dialog(hwnd: HWND) -> *mut Dialog {
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Dialog }
}

fn save(hwnd: HWND) {
    unsafe {
        let ptr = dialog(hwnd);
        if ptr.is_null() {
            let _ = DestroyWindow(hwnd);
            return;
        }
        let state = &mut *ptr;
        if state.closing {
            return;
        }
        state.closing = true;
        let text = read_edit(state.edit);
        (*state.app).set_window_tag(&state.target.fingerprint, &text);
        let _ = DestroyWindow(hwnd);
    }
}

fn cancel(hwnd: HWND) {
    unsafe {
        let ptr = dialog(hwnd);
        if ptr.is_null() {
            let _ = DestroyWindow(hwnd);
            return;
        }
        let state = &mut *ptr;
        if state.closing {
            return;
        }
        state.closing = true;
        // The dial is not written until the user says save, so there is nothing
        // to put back here.
        let _ = DestroyWindow(hwnd);
    }
}

fn preview(hwnd: HWND) {
    unsafe {
        if dialog(hwnd).is_null() {
            return;
        }
        // Nothing behind this popover redraws as the user types: the dial is
        // closed while the field has the keyboard. The chip in the corner of
        // the popover is the live answer, and it reads the field on paint.
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

fn clear(hwnd: HWND) {
    unsafe {
        let ptr = dialog(hwnd);
        if ptr.is_null() {
            return;
        }
        let state = &mut *ptr;
        if state.closing {
            return;
        }
        state.closing = true;
        let empty = wstr("");
        let _ = SetWindowTextW(state.edit, PCWSTR(empty.as_ptr()));
        (*state.app).set_window_tag(&state.target.fingerprint, "");
        let _ = DestroyWindow(hwnd);
    }
}

fn hit_test(state: &Dialog, point: (i32, i32)) -> Hit {
    let text = read_edit(state.edit);
    let l = layout(state.scale, state.width, &state.row);
    let inside =
        |r: &RECT| point.0 >= r.left && point.0 < r.right && point.1 >= r.top && point.1 < r.bottom;
    if inside(&l.save) {
        Hit::Save
    } else if inside(&l.cancel) {
        Hit::Cancel
    } else if !text.trim().is_empty() && inside(&l.clear) {
        Hit::Clear
    } else {
        Hit::None
    }
}

fn client_point(lp: LPARAM) -> (i32, i32) {
    let packed = lp.0 as u32;
    let x = (packed & 0xffff) as u16 as i16 as i32;
    let y = ((packed >> 16) & 0xffff) as u16 as i16 as i32;
    (x, y)
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let ptr = dialog(hwnd);
        if ptr.is_null() {
            return;
        }
        let state = &*ptr;
        let mut ps = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        if hdc.is_invalid() {
            return;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let paper = CreateSolidBrush(rgb(PAPER));
        FillRect(hdc, &client, paper);
        let _ = DeleteObject(paper.into());

        let l = layout(state.scale, state.width, &state.row);
        let text = read_edit(state.edit);
        let colour = tag_color(&text);
        let base = colour.as_tuple();
        let ink = tag_ink(colour).as_tuple();
        let empty = text.trim().is_empty();

        // Header: a small colour dot and the word for what this box is.
        round_box(hdc, &l.dot, base, base, state.scale);
        let heading = t(state.target.heading);
        text_out(hdc, heading, state.fonts.title, MUTED, l.title, DT_LEFT);

        // The field around the stock edit. The edit shares its colour, so only
        // the rounded rim is visible.
        round_box(hdc, &l.field, FIELD, EDGE, state.scale);

        // The primary button previews the colour the tag is about to have.
        let save_bg = if empty { SLATE } else { base };
        let save_bg = if state.pressed == Hit::Save {
            mix(save_bg, (0, 0, 0), 0.16)
        } else if state.hover == Hit::Save {
            mix(save_bg, (255, 255, 255), 0.12)
        } else {
            save_bg
        };
        round_box(hdc, &l.save, save_bg, save_bg, state.scale);
        text_out(hdc, t("tag_ok"), state.fonts.button, ink, l.save, DT_CENTER);

        ghost(
            hdc,
            &l.cancel,
            t("tag_cancel"),
            state.fonts.ghost,
            state.hover == Hit::Cancel,
            state.pressed == Hit::Cancel,
            state.scale,
        );
        if !empty {
            ghost(
                hdc,
                &l.clear,
                t("tag_clear"),
                state.fonts.ghost,
                state.hover == Hit::Clear,
                state.pressed == Hit::Clear,
                state.scale,
            );
        }
        let _ = EndPaint(hwnd, &ps);
    }
}

/// A ghost button: paper, a hairline edge, and a wash while the pointer is on
/// it. The primary button uses `round_box` instead.
unsafe fn ghost(
    hdc: HDC,
    rect: &RECT,
    label: &str,
    font: HFONT,
    hot: bool,
    pressed: bool,
    scale: f32,
) {
    unsafe {
        let fill = if pressed {
            (228, 231, 238)
        } else if hot {
            GHOST_HOT
        } else {
            PAPER
        };
        round_box(
            hdc,
            rect,
            fill,
            if hot || pressed { EDGE } else { PAPER },
            scale,
        );
        text_out(
            hdc,
            label,
            font,
            if hot { INK } else { MUTED },
            *rect,
            DT_CENTER,
        );
    }
}

unsafe fn round_box(hdc: HDC, rect: &RECT, fill: (u8, u8, u8), edge: (u8, u8, u8), scale: f32) {
    unsafe {
        let brush = CreateSolidBrush(rgb(fill));
        let pen = CreatePen(PS_SOLID, 1, rgb(edge));
        let old_brush = SelectObject(hdc, brush.into());
        let old_pen = SelectObject(hdc, pen.into());
        let radius = (9.0 * scale).round() as i32 * 2;
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        let _ = SelectObject(hdc, old_brush);
        let _ = SelectObject(hdc, old_pen);
        let _ = DeleteObject(brush.into());
        let _ = DeleteObject(pen.into());
    }
}

unsafe fn text_out(
    hdc: HDC,
    text: &str,
    font: HFONT,
    colour: (u8, u8, u8),
    rect: RECT,
    align: DRAW_TEXT_FORMAT,
) {
    unsafe {
        if text.is_empty() {
            return;
        }
        let mut wide: Vec<u16> = text.encode_utf16().collect();
        let old = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, rgb(colour));
        let mut r = rect;
        DrawTextW(
            hdc,
            &mut wide,
            &mut r,
            align | DT_NOPREFIX | DT_END_ELLIPSIS | DT_SINGLELINE | DT_VCENTER,
        );
        let _ = SelectObject(hdc, old);
    }
}

unsafe extern "system" fn edit_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Edit;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let state = &*ptr;
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            let boxed = Box::from_raw(ptr);
            return CallWindowProcW(boxed.old, hwnd, msg, wp, lp);
        }
        if msg == WM_KEYDOWN {
            if wp.0 == VK_RETURN.0 as usize {
                if let Ok(parent) = GetParent(hwnd) {
                    save(parent);
                }
                return LRESULT(0);
            }
            if wp.0 == VK_ESCAPE.0 as usize {
                if let Ok(parent) = GetParent(hwnd) {
                    cancel(parent);
                }
                return LRESULT(0);
            }
            // The classic single-line edit control has no select-all of its
            // own, so Ctrl+A would only type an "a" over the selection. 'A'
            // with Ctrl down selects everything instead; anything else - plain
            // letters, Ctrl+C/X/V, the IME - is the stock control's.
            if wp.0 == 'A' as usize && GetKeyState(VK_CONTROL.0 as i32) < 0 {
                let _ = SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
                let _ = SendMessageW(hwnd, EM_SCROLLCARET, Some(WPARAM(0)), Some(LPARAM(0)));
                return LRESULT(0);
            }
        }
        // TranslateMessage has already queued the WM_CHAR for Ctrl+A by the
        // time the key-down reaches this proc, so swallow the control byte
        // (0x01) too: the stock edit would otherwise answer it after our
        // EM_SETSEL and put the caret back at one end.
        if msg == WM_CHAR && wp.0 == 1 && GetKeyState(VK_CONTROL.0 as i32) < 0 {
            let _ = SendMessageW(hwnd, EM_SETSEL, Some(WPARAM(0)), Some(LPARAM(-1)));
            return LRESULT(0);
        }

        // Everything else - the selection, the IME, the clipboard, the text -
        // is the stock control's, untouched. The caret alone is ours: the
        // control's own is kept hidden, and ours is painted after its text by
        // `draw_caret`, on the blink timer the popover owns.
        let result = CallWindowProcW(state.old, hwnd, msg, wp, lp);
        match msg {
            WM_PAINT => {
                if let Ok(parent) = GetParent(hwnd) {
                    let ptr = dialog(parent);
                    if !ptr.is_null() {
                        draw_caret(hwnd, &*ptr);
                    }
                }
            }
            WM_SETFOCUS => {
                // The control has just made (or remade) its own caret; hide it
                // and start our blink lit, so the field never looks dead after
                // a click back into it.
                let _ = HideCaret(Some(hwnd));
                if let Ok(parent) = GetParent(hwnd) {
                    let ptr = dialog(parent);
                    if !ptr.is_null() {
                        let state = &mut *ptr;
                        if !state.closing {
                            state.caret_lit = true;
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
            }
            WM_KILLFOCUS => {
                // No focus, no bar: the field must not show an insertion point
                // it will not act on.
                let _ = HideCaret(Some(hwnd));
                let mut client = RECT::default();
                if GetClientRect(hwnd, &mut client).is_ok() {
                    let _ = InvalidateRect(Some(hwnd), Some(&client), false);
                }
            }
            WM_KEYDOWN | WM_LBUTTONDOWN | WM_LBUTTONUP => {
                // Typing and clicking move the insertion point; the bar follows
                // at once instead of waiting for the next tick to catch up.
                if let Ok(parent) = GetParent(hwnd) {
                    let ptr = dialog(parent);
                    if !ptr.is_null() {
                        let state = &mut *ptr;
                        if !state.closing {
                            let _ = HideCaret(Some(hwnd));
                            state.caret_lit = true;
                            let _ = InvalidateRect(Some(hwnd), None, false);
                        }
                    }
                }
            }
            _ => {}
        }
        result
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = dialog(hwnd);
        match msg {
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_CTLCOLOREDIT => {
                if ptr.is_null() {
                    return DefWindowProcW(hwnd, msg, wp, lp);
                }
                let state = &*ptr;
                let hdc = HDC(wp.0 as *mut c_void);
                SetTextColor(hdc, rgb(INK));
                let _ = SetBkColor(hdc, rgb(FIELD));
                LRESULT(state.field.0 as isize)
            }
            WM_COMMAND => {
                if !ptr.is_null() && ((wp.0 >> 16) as u16 as u32) == EN_CHANGE {
                    let state = &*ptr;
                    preview(hwnd);
                    // The save button changes colour and the clear button can
                    // appear or disappear with the text.
                    if !state.closing {
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_ACTIVATE => {
                let state = if ptr.is_null() { None } else { Some(&*ptr) };
                if let Some(state) = state {
                    if (wp.0 & 0xFFFF) as u32 == WA_INACTIVE {
                        if state.ready && !state.closing {
                            save(hwnd);
                        }
                    } else {
                        // Coming back to the popover starts the blink over, lit.
                        let _ = HideCaret(Some(state.edit));
                        (*ptr).caret_lit = true;
                        let _ = InvalidateRect(Some(state.edit), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if !ptr.is_null() {
                    let state = &mut *ptr;
                    if state.closing {
                        return LRESULT(0);
                    }
                    let hit = hit_test(state, client_point(lp));
                    if hit != state.hover {
                        state.hover = hit;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    let mut track = TRACKMOUSEEVENT {
                        cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                        dwFlags: TME_LEAVE,
                        hwndTrack: hwnd,
                        dwHoverTime: 0,
                    };
                    let _ = TrackMouseEvent(&mut track);
                }
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                if !ptr.is_null() {
                    let state = &mut *ptr;
                    if state.hover != Hit::None {
                        state.hover = Hit::None;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if !ptr.is_null() {
                    let state = &mut *ptr;
                    let hit = hit_test(state, client_point(lp));
                    if hit != Hit::None {
                        state.pressed = hit;
                        let _ = SetCapture(hwnd);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    } else {
                        // Anywhere else on the paper: the field keeps the keys.
                        let _ = SetFocus(Some(state.edit));
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if !ptr.is_null() {
                    let state = &mut *ptr;
                    let hit = hit_test(state, client_point(lp));
                    let pressed = std::mem::replace(&mut state.pressed, Hit::None);
                    let _ = ReleaseCapture();
                    let _ = InvalidateRect(Some(hwnd), None, false);
                    if pressed != Hit::None && pressed == hit {
                        match pressed {
                            Hit::Save => save(hwnd),
                            Hit::Cancel => cancel(hwnd),
                            Hit::Clear => clear(hwnd),
                            Hit::None => {}
                        }
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // The field normally has the focus, but a click on the paper can
                // leave the popup itself focused; Enter and Escape still mean
                // what they say.
                if !ptr.is_null() {
                    if wp.0 == VK_RETURN.0 as usize {
                        save(hwnd);
                        return LRESULT(0);
                    }
                    if wp.0 == VK_ESCAPE.0 as usize {
                        cancel(hwnd);
                        return LRESULT(0);
                    }
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_SETCURSOR => {
                if !ptr.is_null() {
                    let state = &*ptr;
                    let icon = if state.hover != Hit::None {
                        IDC_HAND
                    } else {
                        IDC_ARROW
                    };
                    if let Ok(cursor) = LoadCursorW(None, icon) {
                        SetCursor(Some(cursor));
                    }
                    return LRESULT(1);
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_TIMER if wp.0 == CARET_TIMER => {
                if !ptr.is_null() {
                    let state = &mut *ptr;
                    if !state.closing && !state.edit.is_invalid() {
                        // The control may bring its own caret back on typing; a
                        // hide here keeps ours the only bar in the field.
                        let _ = HideCaret(Some(state.edit));
                        state.caret_lit = !state.caret_lit;
                        // The two lines alternating in the log are the proof
                        // that the bar is being painted and lifted; the point
                        // is the one the paint will use, so a caret the field
                        // never gave us a place for would say so here.
                        let bar = caret_bar(state.edit, state);
                        crate::log!(
                            "tag caret: {} {}",
                            if state.caret_lit { "lit" } else { "dark" },
                            match bar {
                                Some(b) => format!("at={},{} h={}", b.left, b.top, b.bottom - b.top),
                                None => "no position".to_string(),
                            }
                        );
                        let _ = InvalidateRect(Some(state.edit), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                cancel(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let _ = KillTimer(Some(hwnd), CARET_TIMER);
                OPEN.store(OPEN_FAILED, Ordering::Relaxed);
                LRESULT(0)
            }
            WM_NCDESTROY => {
                OPEN.store(OPEN_FAILED, Ordering::Relaxed);
                if !ptr.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    let state = Box::from_raw(ptr);
                    if !state.app.is_null() {
                        (*state.app).tag_dialog_closed();
                    }
                    for f in [
                        state.fonts.title,
                        state.fonts.edit,
                        state.fonts.button,
                        state.fonts.ghost,
                    ] {
                        let _ = DeleteObject(f.into());
                    }
                    let _ = DeleteObject(state.field.into());
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// --- painting helpers ---------------------------------------------------------

/// GDI keeps its colours blue first.
const fn rgb(c: (u8, u8, u8)) -> COLORREF {
    COLORREF(c.0 as u32 | ((c.1 as u32) << 8) | ((c.2 as u32) << 16))
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (f(a.0, b.0), f(a.1, b.1), f(a.2, b.2))
}

fn font(scale: f32, size: f32, weight: i32, face: PCWSTR) -> HFONT {
    unsafe {
        let handle = CreateFontW(
            -(size * scale).round() as i32,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            face,
        );
        if handle.is_invalid() {
            HFONT(GetStockObject(DEFAULT_GUI_FONT).0)
        } else {
            handle
        }
    }
}

/// How tall one line of `font` is, in device pixels. The edit, its caret and
/// the field around it all use this one number, so they cannot disagree.
unsafe fn text_height(font: HFONT, scale: f32) -> i32 {
    unsafe {
        let dc = GetDC(None);
        if dc.is_invalid() {
            return (16.0 * scale).round() as i32;
        }
        let old = SelectObject(dc, HGDIOBJ(font.0));
        let mut metrics = TEXTMETRICW::default();
        let height = if GetTextMetricsW(dc, &mut metrics).as_bool() {
            metrics.tmHeight
        } else {
            (16.0 * scale).round() as i32
        };
        let _ = SelectObject(dc, old);
        let _ = ReleaseDC(None, dc);
        height
    }
}

fn wstr(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::place;
    use shaiwin_platform_win::Rect;

    /// The box belongs to the screen the window is on, and to the middle of it.
    /// Nothing about the window's own height is allowed to drag the box down: a
    /// window sitting on the taskbar used to take it to the last line of the
    /// desktop, where it could not be read.
    #[test]
    fn a_named_window_puts_the_box_in_the_middle_of_its_screen() {
        let work = Rect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1392,
        };
        // The window it names is maximised, so its bottom edge is the work area's.
        let floor = Rect {
            left: 0,
            top: 0,
            right: 2560,
            bottom: 1392,
        };
        let (x, y) = place(320, 140, work, None, None, 1.0);
        assert_eq!((x, y), ((2560 - 320) / 2, (1392 - 140) / 2));
        assert!(y + 140 < work.bottom, "the box must not sit on the floor");
        // An anchor is only ever asked for by a caller that wants it; the hotkeys
        // hand in nothing, so the floor-hugging rectangle above changes no part of
        // the answer.
        let (ax, ay) = place(320, 140, work, Some(floor), None, 1.0);
        assert_eq!((ax, ay), ((2560 - 320) / 2, 1392 - 140));
        assert_eq!(place(320, 140, work, None, None, 1.0), (x, y));
    }

    /// A second screen is a work area of its own, and the box lands inside it
    /// rather than being centred on the primary one and clamped back across.
    #[test]
    fn a_second_screen_gets_a_box_of_its_own() {
        let hd = Rect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        let uhd = Rect {
            left: 1920,
            top: 0,
            right: 4480,
            bottom: 1392,
        };
        let (x, y) = place(320, 140, uhd, None, None, 1.0);
        assert_eq!(x, 1920 + (2560 - 320) / 2);
        assert_eq!(y, (1392 - 140) / 2);
        assert!(x >= uhd.left && x + 320 <= uhd.right);
        assert!(x > hd.right, "the box did not stay on its own screen");
    }

    /// A page that wants the box over one of its own frames still has the answer
    /// clamped to the screen, so a narrow window near an edge cannot push it out.
    #[test]
    fn a_frame_near_the_edge_keeps_the_box_on_the_screen() {
        let work = Rect {
            left: 0,
            top: 0,
            right: 1280,
            bottom: 1024,
        };
        let frame = Rect {
            left: 1200,
            top: 0,
            right: 1280,
            bottom: 200,
        };
        let (x, y) = place(320, 140, work, None, Some(frame), 1.0);
        assert_eq!(x, 1280 - 320);
        assert!(y >= work.top && y + 140 <= work.bottom);
    }
}
