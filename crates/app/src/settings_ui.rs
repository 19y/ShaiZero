//! The settings window.
//!
//! A paper panel, hand-painted: two columns. The left one names five sections -
//! trigger, shortcuts, the windows on the dial, language, about - and the right
//! one is a stack of cards holding the section that was clicked. One section is
//! on screen at a time, so the names never move under the pointer and the window
//! never needs a scrollbar.
//!
//! A card is the row this page already used: its name over one line of what it is
//! for, the control to the right of both on the same vertical line. Nothing on the
//! right is new geometry - the cards are the width the single page measured its
//! controls at, and the navigation column is that same width turned into a stack.
//!
//! The windows section keeps the card and gives it a different contents: one
//! window per card, its own small icon and its title on the first line, and the
//! three things that can be done to it - a tag, a seat, a strike-through - as
//! cells along the second. The cells are the same cells the trigger
//! section divides its control column into, so the pressed look still means
//! "this is the answer"; the tag cell shows the tag itself, because naming the
//! window is the reason anybody comes to this section. There are four cards in
//! the frame whatever the list holds, and the wheel over them moves which window
//! sits in which card - the page does not grow, and the bottom edge stays where
//! the click left it.
//!
//! The dial's two sizes are steppers rather than sliders. A slider makes people
//! aim at a pixel of a 224-pixel band to move a radius by ten, and the answer
//! they are judging is a disc on the desktop, not a number; `−`/`+` says which
//! way it goes and keeps the number honest. The picture above them is redrawn on
//! every change, because a theme the user cannot see before they leave the page
//! is a setting they discover by accident tomorrow.
//!
//! The shortcut boxes listen for a combination and paint the result, because
//! the stock one brings its own grey box, its own font and a blank field that
//! says nothing about what to do with it. Reading the modifiers off the keyboard
//! at the moment a key arrives is the whole trick, and `Key::held_mods` does it.
//!
//! Nothing in the window is left to a stock control's own drawing, not even the
//! buttons: the built-in ones set their own colours and would not sit on this
//! paper. No row has a Save beside it - each one applies itself the moment it
//! moves, and the ones that would strand the desktop go back where they came
//! from with a word about why.
//!
//! The window is created on demand and destroyed when it closes; only one may
//! exist at a time. It talks back to [`App`] through the pointer handed to it
//! by `open`, so saving is a direct call, not a file round-trip.

use std::collections::hash_map::DefaultHasher;
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use shaiwin_platform_win::hook;
use shaiwin_platform_win::monitor::Monitor;
use shaiwin_platform_win::window::WindowInfo;
use tiny_skia::{Color, LineCap, Paint, PathBuilder, Pixmap, Stroke, Transform};
use windows::core::{w, BOOL, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    AlphaBlend, BeginPaint, BitBlt, CreateCompatibleDC, CreateDIBSection, CreateFontW,
    CreatePen, CreateSolidBrush, ClientToScreen, DeleteDC, DeleteObject, DrawTextW, Ellipse, EndPaint, FillRect,
    GetStockObject, GetTextExtentPoint32W, InvalidateRect, IntersectClipRect, LineTo, MoveToEx,
    Polygon, Polyline, RoundRect, ScreenToClient,
    SelectClipRgn, SelectObject, SetBkColor, SetBkMode, SetTextColor, SRCCOPY,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, BLENDFUNCTION, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_WINDOW, DEFAULT_CHARSET,
    DEFAULT_GUI_FONT, DEFAULT_PITCH, DIB_RGB_COLORS, DRAW_TEXT_FORMAT, DT_CENTER, DT_END_ELLIPSIS,
    DT_LEFT, DT_NOPREFIX, DT_RIGHT, DT_SINGLELINE, DT_TOP, DT_VCENTER, FW_NORMAL, FW_SEMIBOLD,
    HBRUSH, HDC, HFONT, HGDIOBJ, NULL_BRUSH, OUT_DEFAULT_PRECIS, PS_SOLID,
    TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::WM_MOUSELEAVE;
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, GetDoubleClickTime, GetFocus, ReleaseCapture, SetCapture, SetFocus,
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT, VIRTUAL_KEY, VK_BACK, VK_CONTROL, VK_DELETE,
    VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DestroyWindow,
    DrawIconEx, GetClientRect, GetWindowLongPtrW, GetWindowTextW, IsDialogMessageW,
    IsWindowVisible, KillTimer, LoadCursorW, MessageBoxW, PostMessageW,
    RegisterClassExW, SendMessageW, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos,
    SetWindowTextW, ShowWindow, TrackPopupMenu, BM_CLICK, BN_CLICKED, DI_NORMAL, DLGC_BUTTON,
    DLGC_WANTALLKEYS, DLGC_WANTARROWS, DLGC_WANTCHARS, DM_GETDEFID, GWLP_USERDATA, HICON, HMENU,
    ICON_BIG, ICON_SMALL, IDC_ARROW, MB_ICONWARNING, MB_OK, MF_STRING,
    MINMAXINFO, MSG, SWP_NOACTIVATE, SWP_NOCOPYBITS, SWP_NOZORDER, SW_HIDE, SW_SHOWNA, SW_SHOWNORMAL,
    TPM_RIGHTBUTTON, TPM_RETURNCMD, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND,
    WM_CTLCOLOREDIT, WM_DESTROY, WM_ERASEBKGND, WM_EXITSIZEMOVE, WM_GETDLGCODE, WM_GETMINMAXINFO,
    WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCDESTROY, WM_NULL, WM_PAINT, WM_RBUTTONUP, WM_SETFOCUS, WM_SETFONT, WM_SETICON, WM_SIZE, WM_TIMER,
    WNDCLASSEXW, WNDCLASS_STYLES, WNDPROC, WS_CAPTION, WS_CHILD, WS_CLIPCHILDREN,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_THICKFRAME,
    WS_VISIBLE,
};

use crate::app::App;
use crate::i18n::t;
use crate::settings;

const CLASS: PCWSTR = w!("PanWen.Settings");
const SWITCH_CLASS: PCWSTR = w!("PanWen.Settings.Switch");
const KEY_CLASS: PCWSTR = w!("PanWen.Settings.Key");
const BUTTON_CLASS: PCWSTR = w!("PanWen.Settings.Button");

/// No button carries this id. It is `IDCANCEL`: what the dialog manager sends up
/// as a bare `WM_COMMAND` when Escape is pressed, which is how Escape closes a
/// window that has nothing left to cancel.
const ESCAPE_ID: i32 = 2;
/// The quiet button that opens the folder the language files live in. It is a
/// door, not a decision: nothing it opens can be undone by pressing it.
const FOLDER_ID: i32 = 3;
const FAV_ID: i32 = 101;
const PANEL_ID: i32 = 102;
/// The search field above 轮盘's window list. It is a plain edit control, so the
/// caret, selection, clipboard and IME all behave the way the rest of Windows
/// does; the page only reads its text and filters the list.
const SEARCH_ID: i32 = 733;
/// The trigger recorder, one field where four modifier cells and two button
/// cells used to be.
const MOD_BASE: i32 = 110;
/// The language buttons are numbered by their place in the list on offer, so a
/// click reports which one without the control having to remember anything.
const LANG_BASE: i32 = 201;
const NAV_BASE: i32 = 301;
/// Which sections the left column shows.
const NAV_SECTIONS: [usize; 4] = [SEC_KEYS, SEC_DIAL, SEC_LANG, SEC_ABOUT];
/// The switch in 关于 and the door to the log beside it.
const MOTION_ID: i32 = 311;
const LOG_DOOR_ID: i32 = 312;
/// 轮盘's cells are numbered by group: the accent row, the animation length and
/// the order of the limited list, so a click reports both the group and the
/// choice. The six themes are not cells: they are a filmstrip painted on the
/// picture card, where each thumbnail shows the look instead of naming it. The
/// steppers that follow use their own base.
const DIAL_CELL_BASE: i32 = 700;
const DIAL_THEMES: usize = 6;
const DIAL_ACCENT_CELLS: usize = 3;
const DIAL_ANIM_CELLS: usize = 4;
/// The order the limited list arrives in: newest first, or favourites first.
const DIAL_ORDER_CELLS: usize = 2;
/// Where a picked window lands: where it was, or centred on the pointer's
/// screen. A decision with two answers, so two cells.
const DIAL_POSITION_CELLS: usize = 2;
const DIAL_CELLS: usize =
    DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + DIAL_ORDER_CELLS + DIAL_POSITION_CELLS;
/// The filmstrip's cell height, in logical pixels: a square thumbnail, the
/// theme's name under it, and the air around both.
const DIAL_THEME_H: f32 = 92.0;
const DIAL_THEME_GAP: f32 = 6.0;
/// One short sentence in the note band over the picture, then the timer takes
/// it back: a press that changed the dial is answered in words, not only by the
/// list silently reordering itself.
const DIAL_TOAST_TIMER: usize = 0x52;
const DIAL_TOAST_MS: u32 = 1600;
const DIAL_STEP_BASE: i32 = 720;
/// The six named looks. Each is a whole treatment - plate, fill, rim, glow,
/// shadow - not a tweak of the previous one, so a click is visible at once.
/// The filmstrip paints these as thumbnails rather than as words.
const DIAL_THEME_NAME: [&str; DIAL_THEMES] = [
    "dial_theme_outline",
    "dial_theme_glass",
    "dial_theme_neon",
    "dial_theme_card",
    "dial_theme_neon2",
    "dial_theme_paper",
];
/// How loud the sectors are. Only the sector colour moves; the filmstrip in the
/// picture card shows the same theme at the chosen loudness.
const DIAL_ACCENT_NAME: [&str; DIAL_ACCENT_CELLS] = [
    "dial_accent_low",
    "dial_accent_medium",
    "dial_accent_high",
];
const DIAL_ORDER_NAME: [&str; DIAL_ORDER_CELLS] = [
    "settings_order_recent",
    "settings_order_favourite",
];
const DIAL_POSITION_NAME: [&str; DIAL_POSITION_CELLS] = [
    "settings_dial_position_original",
    "settings_dial_position_center",
];

const DIAL_ANIM_MS: [u32; DIAL_ANIM_CELLS] = [0, 120, 180, 240];
const DIAL_ANIM_NAME: [&str; DIAL_ANIM_CELLS] = [
    "dial_anim_instant",
    "dial_anim_quick",
    "dial_anim_normal",
    "dial_anim_slow",
];
/// A row moved; the window answers one message later. See `State::moved`.
const WM_ROW_MOVED: u32 = WM_APP + 5;
/// The list of windows on the dial is somebody else's business - the shortcut
/// writes it, the dial changes a pin, the label editor commits a tag - and this
/// is how the page hears that the rows it is showing are no longer the rows
/// there are. Posted, not sent: the writer must not wait on a repaint.
const WM_ROWS_CHANGED: u32 = WM_APP + 6;
/// Turn an already open page to another section. `Ctrl+T` twice is one key pressed
/// twice, and the answer is the row the pointer is on its way to rather than a
/// second window on top of the first.
const WM_SHOW_SECTION: u32 = WM_APP + 7;
/// The dial just opened over this page. Whatever the pointer was doing here -
/// dragging a sector, dragging a window from the lift - is no longer the thing
/// being aimed at, and the capture must go back to the desktop so the dial can
/// hear the release.
const WM_WHEEL_OPENED: u32 = WM_APP + 8;

// --- the panel -----------------------------------------------------------------
//
// One warm accent, and it belongs to whatever is live: the switch's track and the
// focus ring. Everything else is ink on paper with a rule between the groups,
// because a group of settings is a sequence, not a grid of boxes.
const PAPER: (u8, u8, u8) = (0xF7, 0xF8, 0xFA);
const FIELD: (u8, u8, u8) = (0xFF, 0xFF, 0xFF);
const INK: (u8, u8, u8) = (0x1A, 0x1D, 0x23);
const MUTED: (u8, u8, u8) = (0x6C, 0x74, 0x80);
/// 琥珀: the colour a combination that another program is likely to want too is
/// written in. It is a warning, not a refusal - the user may know better.
const CONFLICT: (u8, u8, u8) = (0xB4, 0x6A, 0x0A);
const RULE_COLOUR: (u8, u8, u8) = (0xE2, 0xE5, 0xEA);
const EDGE: (u8, u8, u8) = (0xD3, 0xD8, 0xDF);
const SLATE: (u8, u8, u8) = (0x26, 0x2C, 0x38);
const FLAX: (u8, u8, u8) = (0xDD, 0x9A, 0x4E);
const SWITCH_OFF: (u8, u8, u8) = (0xDD, 0xE1, 0xE6);
/// The licence as the LICENSE file spells it, which is also as the card says it.
/// Not `CARGO_PKG_LICENSE`: cargo has to carry a `LicenseRef-` name because the
/// Commons Clause is not an SPDX identifier, and that is not a string to put on
/// a card somebody has to read.
const LICENSE_SHOWN: &str = "MIT + Commons Clause";

// --- the measure ---------------------------------------------------------------
//
// Two columns: the left names the sections, the right is a stack of cards holding
// the one that was clicked. Every constant here is added into `logical_h` and
// walked again by `measure`, which is how the window is never a pixel too tall for
// its own rows.
//
// Nothing in the right column is new geometry. It is the width the single page
// already had (520 - 2*24 = 472), so every control keeps the size it was measured
// at. The navigation column is its own narrow stack of names - narrower than
// `FIELD_W`, and every pixel it gives up is a pixel the cards get - so the cards
// begin on a line the panel already drew, and the controls end on one too: the
// page's right padding, then the field. The designed page is 760 logical pixels,
// which is the width the sentences were cut to; the panel opens wider because the
// dial's picture is the thing that needs the room.
/// The width the page was designed at. The panel opens wider than this now -
/// the dial's picture is what needs the room - so this survives as the width the
/// measurements below are asserted against, not as the window's size.
#[cfg(test)]
const LOGICAL_W: f32 = 760.0;
/// The smallest whole-window the resize handle may be dragged to, in logical
/// pixels. The floor is now low enough to be dragged on a 150% screen without
/// eating the whole width: the name column shrinks with the window, the dial's
/// desk folds into one column under [`DIAL_DESK_MIN`], and whatever no longer
/// fits the glass is reached by scrolling the paper under it.
const MIN_CLIENT_W: f32 = 620.0;
/// How big the panel opens before anybody has resized it, in logical pixels.
/// The dial is the subject of this page, so the opening size is the one at which
/// its picture clears 460 logical pixels of diameter with room for a list of
/// eight windows beside it - not the folded-up single column the page used to be.
const LOGICAL_W_PANEL: f32 = 1120.0;
/// Tall enough for the picture to be a picture rather than a thumbnail, and for
/// the list to show its eight rows without a scroll. The page's floor is shorter;
/// this is what makes the dial readable on the first open.
const LOGICAL_H_PANEL: f32 = 780.0;
/// The height floor. The dial cannot promise a 260-pixel picture and a
/// three-row list inside 440, so at this size its zones fold into one column
/// and the page scrolls - the floor is a floor for the handle, not for the paper.
const MIN_CLIENT_H: f32 = 440.0;

/// The smallest whole-window the resize handle may be dragged to, in device
/// pixels: [`MIN_CLIENT_W`] × [`MIN_CLIENT_H`] logical, plus the caption and thick
/// frame the window carries. The DPI is read from the window each time
/// `WM_GETMINMAXINFO` arrives rather than kept from the size it was opened at, so
/// a page that has been dragged onto a monitor with another scale still stops at
/// the size its own layout was measured for, instead of at a floor computed for
/// the monitor it left.
fn min_track_size(dpi: u32) -> (i32, i32) {
    let dpi = dpi.max(96);
    let scale = dpi as f32 / 96.0;
    let style = WINDOW_STYLE(
        WS_OVERLAPPED.0
            | WS_CAPTION.0
            | WS_SYSMENU.0
            | WS_THICKFRAME.0
            | WS_MAXIMIZEBOX.0
            | WS_MINIMIZEBOX.0,
    );
    let mut frame = RECT {
        left: 0,
        top: 0,
        right: (MIN_CLIENT_W * scale).round() as i32,
        bottom: (MIN_CLIENT_H * scale).round() as i32,
    };
    let _ = unsafe { AdjustWindowRectExForDpi(&mut frame, style, false, WINDOW_EX_STYLE(0), dpi) };
    (frame.right - frame.left, frame.bottom - frame.top)
}

const PAD: f32 = 24.0;
const PAD_BOTTOM: f32 = 22.0;
/// The navigation column. It holds one short name per row, so it is narrower
/// than the control column it faces: every pixel it gives up is a pixel the
/// cards and the dial get back.
const NAV_W: f32 = 144.0;
/// The air between the columns: the gap the groups used to keep from each other.
const GUTTER: f32 = GROUP_GAP;
/// A card is the row the dial's switch already was - a title over one line of
/// explanation with the control to the right of both - laid on the shape the panel
/// already uses for a field.
const CARD_H: f32 = ITEM_H + ITEM_GAP * 2.0;
const CARD_GAP: f32 = ITEM_GAP;
/// The air inside a card, and the distance a label already kept from its control.
const CARD_PAD: f32 = GAP_LABEL;
const GROUP_GAP: f32 = 16.0;
const NOTE_H: f32 = 18.0;
const ITEM_H: f32 = 40.0;
const TITLE_H: f32 = 22.0;
const ITEM_GAP: f32 = 10.0;
/// The control column, and the column the four trigger cells divide into four.
/// Four is the constraint: at 208 a cell came to 46 logical pixels, and `Shift` -
/// which is painted in the cell's own font, in every language, because it is the
/// word on the key - measured 32 of those with only 7 left over for each side of
/// an 8-pixel corner. 224 keeps the last letter out of the curve.
const FIELD_W: f32 = 224.0;
const FIELD_H: f32 = 28.0;
/// The row of language buttons. Four to a row because that is as narrow as a
/// name stays readable at 13 logical pixels in a 472-pixel column, and three
/// rows because a panel taller than the screen is a panel you cannot save from.
const SEG_H: f32 = 32.0;
const SEG_GAP: f32 = 8.0;
const SEG_ROW_GAP: f32 = 6.0;
const SEG_PER_ROW: usize = 4;
const MAX_LANG_ROW: usize = 3;
/// The air between the last row of language buttons and the line that says how
/// to add one more.
const LANG_NOTE_GAP: f32 = 8.0;
/// The box the × of a shortcut field lives in: wide enough to hit without
/// aiming, narrow enough that the combination still reads.
const CLEAR_W: f32 = 26.0;
const SWITCH_W: f32 = 40.0;
const SWITCH_H: f32 = 22.0;
/// The switch's hit box is bigger than the track it shows. 40x22 is a small
/// thing to aim at twice a day; the extra is padding around the same shape, not
/// a second shape.
const SWITCH_BOX_W: f32 = 48.0;
const SWITCH_BOX_H: f32 = 26.0;
const GAP_LABEL: f32 = 14.0;
/// The two ends of a stepper. 40 logical pixels is what `−` and `+` need to be
/// hittable without aiming; the number they move lives in the gap between them,
/// painted by the panel rather than by a control.
const STEP_W: f32 = 40.0;
/// Air between the cells of a segmented row (the four modifiers, the two mouse
/// buttons). Narrower than the language row's gap because these are four in a
/// 224-pixel column, not four in a 472-pixel one.
const CELL_GAP: f32 = 4.0;

// --- the sections ---------------------------------------------------------------
//
// One section on screen at a time, named by the column on the left. Scrolling a
// whole page and keeping a highlight in step with the scroll position is the other
// design; this one spends a click to buy that sync away.
const SEC_KEYS: usize = 0;
/// 轮盘: the live preview and the paint controls.
const SEC_DIAL: usize = 1;
const SEC_LANG: usize = 2;
const SEC_ABOUT: usize = 3;
const SEC_COUNT: usize = 4;
/// The one section [`App`] can ask for by name: the dial's shortcut opens the
/// page on the dial itself, which is also where a plain open lands.
pub const SECTION_DIAL: usize = SEC_DIAL;
/// The most cards any one section lays out. 轮盘 holds a preview and seven rows.
const MAX_CARDS: usize = 8;
/// The card in 语言 that holds the row of names rather than one control.
const LANG_GRID_ROW: usize = 0;
/// The dial's picture is the one thing on the page that grows with the window.
/// These are the three zones of its desk: the name column on the left, the
/// picture in the middle, and the window list with the compact parameter strip
/// on the right. The right column takes a share of the content width, bounded so
/// a row of the list still reads as a row and the cells beside it still say their
/// words.
const DIAL_COL_SHARE: f32 = 0.38;
const DIAL_COL_MIN: f32 = 260.0;
/// The list column stops widening here; past it the picture is what should grow.
const DIAL_COL_MAX: f32 = 440.0;
/// The list card's own inset. Tighter than a card's because its first band is a
/// field rather than a sentence.
const DIAL_LIST_PAD: f32 = 8.0;
/// Air between the search field, the two tuning rows and the list. Six keeps
/// the list on the field's own floor, so the height a search box used to spend
/// on blank paper is height the list now shows rows in.
const DIAL_LIST_GAP: f32 = 6.0;
/// The magnifier at the search field's left edge, and the strip it lives in.
/// The edit control starts after this strip so the icon is drawn on the page
/// rather than under the child, and the text never runs into it.
const DIAL_SEARCH_ICON: f32 = 20.0;
/// The frame drawn around the edit control: a hairline border, and the two
/// pixels between it and the text the control paints.
const DIAL_SEARCH_INSET: f32 = 2.0;
/// The air between the search field and the refresh button beside it. The
/// button is a square the height of the field, so the two read as one band.
const DIAL_REFRESH_GAP: f32 = 6.0;
/// The compact strip's own inset. The strip is a tool band, not a card with a
/// sentence under its name, so it spends four pixels less on its frame and gives
/// that height to the list beside it.
const DIAL_PARAM_PAD: f32 = 10.0;
/// One parameter row of the compact strip: a short name on the left and its
/// cells or its stepper on the right. Twenty-eight is the field's own height, so
/// the strip is as tall as the controls it holds and keeps its air between rows.
const DIAL_ROW_H: f32 = 28.0;
const DIAL_ROW_GAP: f32 = 4.0;
const DIAL_ROW_LABEL_W: f32 = 56.0;
/// The name column at the narrow floor, where the eight pixels it gives back
/// are what keeps the four effect words on one row.
const DIAL_ROW_LABEL_NARROW: f32 = 44.0;
/// Under this width the four-cell groups fold into a 2x2 grid rather than
/// squeezing four words into cells that no longer carry them. 126 is four
/// cells of just over thirty pixels: a two-character word still fits, and the
/// strip keeps one row, which keeps the list's fourth row on the paper.
const DIAL_ROW_WIDE: f32 = 126.0;
/// Below this content width the three zones no longer fit: the picture's
/// 260-pixel floor plus the list's 180-pixel column plus the gutter and both
/// insets need 472, so the desk folds into one column and the page scrolls.
const DIAL_DESK_MIN: f32 = 480.0;
/// The folded preview card's height: its two insets (14 each), the two text
/// bands (22 and 18) and their air (2 and 2), the 8 pixels above the filmstrip,
/// the 92-pixel strip itself, and the 260-pixel picture floor. 440 leaves the
/// square 268, so a folded page still shows a picture rather than a thumbnail.
const DIAL_STACK_PREVIEW_H: f32 = 440.0;
/// A row of the window list. The step is what the wheel moves and the height is
/// the band the row paints in; the difference is the air between two rows.
const POOL_ROW_STEP: f32 = 32.0;
const POOL_ROW_H: f32 = 30.0;
/// The picture card's nominal height. The dial page measures its own card from
/// the client instead; this is what [`section_h`] adds up so the page can still
/// be compared with the single-column sections.
const DIAL_PREVIEW_H: f32 = 300.0;
const DIAL_PREVIEW_ROW: usize = 0;
const DIAL_ACCENT_ROW: usize = 1;
const DIAL_RADIUS_ROW: usize = 2;
const DIAL_SEAM_ROW: usize = 3;
const DIAL_ANIM_ROW: usize = 4;
/// How many windows make the cut, and the order the ones that do arrive in.
const DIAL_LIMIT_ROW: usize = 5;
const DIAL_ORDER_ROW: usize = 6;
/// Where a picked window lands, under the order it was picked in.
const DIAL_POSITION_ROW: usize = 7;

/// The navigation's words. `settings_group_keys` is the name the old page already
/// gave the shortcuts; the others are new because the page came apart into
/// sections.
const SEC_NAME: [&str; SEC_COUNT] = [
    "settings_group_keys",
    "settings_nav_dial",
    "settings_nav_lang",
    "settings_nav_about",
];

/// A card is two strings and a control, so the strings are the page's inventory:
/// the painter draws what is listed here, the cutter checks these, and a section's
/// height is a count away. The note is one line of what the row is for - never a
/// number, never a threshold.
#[derive(Clone, Copy)]
struct Card {
    title: &'static str,
    note: &'static str,
}

const CARDS: [&[Card]; SEC_COUNT] = [
    &[
        Card {
            title: "settings_wheel_trigger",
            note: "",
        },
        Card {
            title: "settings_fav",
            note: "",
        },
        Card {
            title: "settings_panel_key",
            note: "",
        },
    ],
    &[
        Card {
            title: "settings_dial_preview",
            note: "settings_dial_preview_note",
        },
        Card {
            title: "settings_dial_accent_short",
            note: "settings_dial_accent_note",
        },
        Card {
            title: "settings_dial_radius_short",
            note: "settings_dial_radius_note",
        },
        Card {
            title: "settings_dial_seam_short",
            note: "settings_dial_seam_note",
        },
        Card {
            title: "settings_dial_anim_short",
            note: "settings_dial_anim_note",
        },
        Card {
            title: "settings_dial_limit_short",
            note: "settings_dial_limit_note",
        },
        Card {
            title: "settings_dial_order_short",
            note: "settings_dial_order_note",
        },
        Card {
            title: "settings_dial_position_short",
            note: "settings_dial_position_note",
        },
    ],
    &[
        Card {
            title: "settings_lang",
            note: "settings_lang_note",
        },
        Card {
            title: "settings_lang_folder",
            note: "settings_note_folder",
        },
    ],
    &[
        Card {
            title: "settings_about_version",
            note: "settings_note_version",
        },
        // The note band of this card carries the path rather than a sentence: a
        // file you are pointed at is named, not described.
        Card {
            title: "settings_about_log",
            note: "",
        },
        Card {
            title: "settings_about_motion",
            note: "settings_note_motion",
        },
        // The licence names itself on the right; the band under it used to say
        // how the licence was given, which told nobody anything the value did
        // not already say.
        Card {
            title: "settings_about_license",
            note: "",
        },
    ],
];

fn cards_of(section: usize) -> &'static [Card] {
    CARDS[section]
}

/// How many cards a section lays out: as many as `CARDS` names for it.
fn card_count(section: usize) -> usize {
    cards_of(section).len()
}

/// Rows of language buttons for `langs` choices, counting the "follow the
/// system" one.
fn lang_rows(langs: usize) -> usize {
    langs.div_ceil(SEG_PER_ROW).clamp(1, MAX_LANG_ROW)
}

/// The height of one card. Two are not the row height: the language grid's card
/// grows by what the folder is offering today, and the dial's preview by its
/// own picture. Neither squeezes what it is standing among.
fn card_h(section: usize, at: usize, rows: usize) -> f32 {
    if section == SEC_DIAL && at == DIAL_PREVIEW_ROW {
        DIAL_PREVIEW_H
    } else if section == SEC_LANG && at == LANG_GRID_ROW {
        CARD_PAD * 2.0
            + TITLE_H
            + LANG_NOTE_GAP
            + rows as f32 * SEG_H
            + SEG_ROW_GAP * (rows - 1) as f32
            + LANG_NOTE_GAP
            + NOTE_H
    } else {
        CARD_H
    }
}

/// The height the single-column sections want, padding and bottom margin
/// included. The page compares it with the glass to decide whether the wheel
/// has anything to do; [`logical_h`] takes the taller of these for the tests.
fn section_h(section: usize, rows: usize) -> f32 {
    let count = card_count(section);
    PAD
        + (0..count).map(|at| card_h(section, at, rows)).sum::<f32>()
        + CARD_GAP * (count - 1) as f32
        + PAD_BOTTOM
}

/// The height at which every card gets its designed size: the tallest section,
/// which is the dial's. The window itself opens at [`LOGICAL_H_PANEL`] and may
/// be dragged down to [`MIN_CLIENT_H`]; this height is what the measurements are
/// asserted against, so it is only needed by the tests now.
#[cfg(test)]
fn logical_h(rows: usize) -> f32 {
    (0..SEC_COUNT)
        .map(|section| section_h(section, rows))
        .fold(0.0f32, f32::max)
}

/// What the two steppers will not go past. The radius is the ruler's own answer -
/// `core::wheel::Geometry` refuses a dial outside this band whatever the file
/// says - and the starting distance is the gesture core's: under two pixels is a
/// shiver, past forty is a chore on a touchpad.
const RADIUS_MIN: f32 = shaiwin_core::wheel::RADIUS_MIN;
const RADIUS_MAX: f32 = shaiwin_core::wheel::RADIUS_MAX;
const RADIUS_STEP: f32 = 10.0;

/// The three faces on this paper. The heading face went with the group titles:
/// a card's own name is written in the same weight as the button next to it,
/// which is the one thing that makes the two columns read as one document.
const F_TITLE: usize = 0;
const F_NOTE: usize = 1;
const F_BUTTON: usize = 2;

/// Set while a settings window exists, for `IsDialogMessage` in the message
/// loop (Tab navigation, Enter and Escape).
static SETTINGS_HWND: AtomicIsize = AtomicIsize::new(0);

/// The floor last written into `WM_GETMINMAXINFO`, so a drag that asks for it on
/// every step writes one line rather than one per pixel.
static FLOOR_LOGGED: AtomicIsize = AtomicIsize::new(0);

/// One window the dial could seat, and what the page already knows about it.
/// [`App`] builds these from the desktop and the settings file, so the row cannot
/// disagree with either; the icon is the handle the window itself answers with,
/// good for as long as that window is - which is why it is read with the rest of
/// the row rather than kept.
#[derive(Clone)]
pub(crate) struct Row {
    /// The window handle this row stands for; 0 for a favourite that is not
    /// running, which has a seat reserved but no window to ask anything.
    pub hwnd: isize,
    pub fingerprint: String,
    /// Lower-cased executable name - the thing the exclude list is keyed by.
    pub exe: String,
    /// The application's name as a person would write it.
    pub app: String,
    pub title: String,
    pub tag: String,
    pub favourite: bool,
    /// Whether this window holds a sector right now - favourited or seated by
    /// recency. The row shows a filled mark for these; the others are candidates
    /// to drag onto the dial.
    pub on_dial: bool,
    /// Whether the user took this window off the dial by name. Such a row goes
    /// back on a double-click, so the decision is reversible without dragging.
    pub off_dial: bool,
    pub excluded: bool,
    pub icon: HICON,
}

/// One line of the dial's list: a group heading, or a window under one of them.
/// The list is two sections - the windows the dial is holding, and the ones it
/// is not - and both headings are always drawn, so the list never reads as one
/// undifferentiated column and an empty section never disappears. The heading
/// takes a row's height, which keeps the geometry a simple multiple of
/// `POOL_ROW_STEP`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PoolEntry {
    Group { on: bool, count: usize },
    Row { at: usize },
}

/// Splits a filtered list into the two sections. `at` is the index into the
/// rows, kept so the painter, the hit test and the buttons all speak the same
/// row number they did before. A pure function so the section arithmetic can be
/// tested without a window.
fn build_pool_view(rows: &[Row]) -> Vec<PoolEntry> {
    let on_count = rows.iter().filter(|row| row.on_dial).count();
    let mut view = Vec::with_capacity(rows.len() + 2);
    view.push(PoolEntry::Group {
        on: true,
        count: on_count,
    });
    for (at, row) in rows.iter().enumerate() {
        if row.on_dial {
            view.push(PoolEntry::Row { at });
        }
    }
    view.push(PoolEntry::Group {
        on: false,
        count: rows.len() - on_count,
    });
    for (at, row) in rows.iter().enumerate() {
        if !row.on_dial {
            view.push(PoolEntry::Row { at });
        }
    }
    view
}

/// The rows a search keeps: a case-folded substring over the three names a person
/// might remember a window by - the title, the executable and the application
/// name. An empty field keeps every row. It is a free function so the matching
/// rule can be tested without a window and a desk.
fn filter_rows(all: &[Row], query: &str) -> Vec<Row> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return all.to_vec();
    }
    all.iter()
        .filter(|row| {
            row.title.to_lowercase().contains(&query)
                || row.exe.to_lowercase().contains(&query)
                || row.app.to_lowercase().contains(&query)
        })
        .cloned()
        .collect()
}

/// The client size the panel opens at, in logical pixels. The remembered size
/// wins when there is one; otherwise the preferred size does, and the monitor's
/// work area is the ceiling over both. The DPI conversion happens exactly here,
/// once, so this is also the place where a 125% screen is accounted for: a
/// 990-logical page is 1238 device pixels and will not fit on a 1080p screen's
/// 1040-pixel work area, so it is clamped rather than hung off the bottom.
fn panel_open_size(
    remembered: (f32, f32),
    scale: f32,
    work_w: i32,
    work_h: i32,
    min_h: f32,
) -> (f32, f32) {
    let room_w = (work_w as f32 / scale - 24.0).max(MIN_CLIENT_W);
    let room_h = (work_h as f32 / scale - 16.0).max(min_h);
    let wanted = if remembered.0 > 0.0 && remembered.1 > 0.0 {
        remembered
    } else {
        (LOGICAL_W_PANEL, LOGICAL_H_PANEL)
    };
    (
        wanted.0.min(room_w).max(MIN_CLIENT_W),
        wanted.1.min(room_h).max(min_h),
    )
}

struct State {
    app: *mut App,
    /// Borrowed from [`App`], which owns and frees them: the large one for Alt-Tab
    /// and the small one for the title bar.
    window_icon: HICON,
    tray_icon: HICON,
    scale: f32,
    fonts: [HFONT; 3],
    layout: Layout,
    /// The panel itself, so an error can be shown against the window the user is
    /// looking at rather than against whichever control happened to be first.
    hwnd: HWND,
    /// Which of the four sections is on the paper right now. One at a time, so
    /// the window is as tall as its tallest section and a click on the left
    /// column never moves the bottom edge - or the control the pointer was on
    /// its way to.
    section: usize,
    /// The left column. A button like every other word on this window, so it
    /// comes with the focus ring, the pressed look and the Tab stop for free.
    nav_buttons: [HWND; SEC_COUNT],
    /// Every control that belongs to one section rather than to all of them, in
    /// reading order within each. Switching is a pass over this list, which is
    /// why the instant-apply chain never hears about it: the controls are made
    /// once, and only their visibility moves.
    slots: Vec<(usize, HWND)>,
    fav: HWND,
    panel_key: HWND,
    /// 关于: the switch that stops the dial unfurling, and the door to the file
    /// the program writes what it did into.
    motion_switch: HWND,
    log_door: HWND,
    /// The one field that records the trigger, like a game's key bind.
    trigger_field: HWND,
    /// Why the last recording was refused, shown under the recorder.
    trigger_notice: Option<settings::TriggerProblem>,
    /// The language buttons, in the order `lang_tags` lists them.
    lang_buttons: Vec<HWND>,
    /// `auto`, then each language the program and the folder offer, with the name
    /// the pack gives itself. A button is the row it stands for, so the two are
    /// always the same length.
    lang_tags: Vec<(String, String)>,
    /// The choice in force: `auto` or a tag. It is the row that looks pressed,
    /// and the value every later change carries along with it - a language is
    /// never applied on its own, because [`App`] takes the whole draft or none
    /// of it.
    lang: String,
    /// The language every word currently on this window was read from. A draft
    /// that lands with a different one means the whole panel has to be said
    /// again, and this is the only way to know it did.
    applied_lang: String,
    /// The buttons that carry a translated word, with the key they carry it
    /// under. Everything else paints its own text out of `t()` at the moment it
    /// is asked to, so only these have to be told a language changed.
    captions: Vec<(HWND, &'static str)>,
    folder: HWND,
    /// The dial's rows, carried in from the settings this panel opened with and
    /// edited here. They are what the app is running with: a control moves one of
    /// them and the move is applied on the spot, so these are only ever a step
    /// ahead of [`App`]'s own settings while that step is being tried.
    trigger_mods: u32,
    trigger_vk: u32,
    trigger_button: u8,
    wheel_radius: f32,
    /// The hub has no row here: a fraction of the radius is not a thing to argue
    /// with two arrows about. It is carried in and written back untouched, so
    /// applying a row cannot quietly reset a hand-edited file.
    wheel_dead: f32,
    /// The dial's painted look: the named theme, how loud the accents are, the
    /// seam and the arrival time. Edited in 轮盘 and read by the preview on the
    /// same page.
    wheel_theme: u32,
    wheel_accent: u32,
    wheel_seam: f32,
    wheel_open_ms: u32,
    /// The dial's window limit and the order its windows arrive in. Both are
    /// applied at the next open as well as fed to the preview below.
    wheel_limit: u32,
    dial_order: u32,
    /// Where the next pick lands: `0` in place, `1` centred on the screen under
    /// the pointer. Saved with the rest of the draft.
    activate_pos: u32,
    /// 轮盘's cells and steppers, made once with the window like every other
    /// control, so switching sections only moves their visibility.
    dial_cells: [HWND; DIAL_CELLS],
    dial_steps: [HWND; 6],
    /// The first few windows, mirrored beside the picture so one can be dragged
    /// onto a sector without leaving the page.
    dial_pool: Vec<Row>,
    /// The search field above the list, and the text in it. The field is a real
    /// edit control so the caret and the IME work; the page only listens for the
    /// text changing.
    dial_search: HWND,
    /// Every window before the filter, and the filter's text, lower-cased. The
    /// list beside the picture is `dial_pool`, which is a view of `pool_all`;
    /// dragging and the × act on the view, so they keep working while a filter is
    /// typing.
    pool_all: Vec<Row>,
    pool_query: String,
    /// The filtered rows cut into the two sections the list draws - the windows
    /// on the dial, then the rest - rebuilt wherever `dial_pool` is.
    pool_view: Vec<PoolEntry>,
    /// A press on a row's name, held until the release says whether it is a
    /// click or the second half of a double-click. Kept out of `dial_press` so a
    /// press on a row never reaches the preview's own click path.
    row_press: Option<(usize, (i32, i32))>,
    /// The last single click on a row and when it happened. The second click on
    /// the same row inside the system's double-click time is the seat gesture.
    row_click: Option<RowClick>,
    /// The paper-coloured brush under the search field, so the edit control
    /// paints on the page rather than as a white sticker on it.
    search_brush: HBRUSH,
    /// The field's placeholder, and whether the field is showing it right now.
    /// The system's cue banner is not available without a version-six common
    /// controls manifest, and it answers FALSE here; the placeholder is instead
    /// the field's own text, grey while untouched and cleared on the first
    /// focus. The filter reads it as an empty box, never as a query.
    search_hint: String,
    search_show_hint: bool,
    /// Whether the field has the focus, told by the edit's own notifications.
    /// The frame's edge is drawn from this rather than a `GetFocus` at paint
    /// time, so a repaint that arrives before the focus settles still knows.
    search_focused: bool,
    /// Whether the pointer is resting on the list's refresh button, so the
    /// button can brighten under it the way the search frame does under focus.
    refresh_hot: bool,
    /// A still picture of the dial, rebuilt when the settings it is made from
    /// change or when the section is entered. The desk is read to build it, so it
    /// is never rebuilt per paint: a window list is a slow thing to ask for.
    preview: Option<DialPreview>,
    /// The six looks at filmstrip size, made through the same renderer as the
    /// picture and the live dial. Cached against the desk and the accent row:
    /// clicking a thumbnail must not rebuild six pictures to show it was taken.
    theme_thumbs: Option<ThemeThumbs>,
    theme_thumbs_key: (u32, u32, u32, usize, u32),
    /// When a thumbnail was clicked and which theme it asked for. The paint that
    /// lands the new picture reads it and logs how long the switch took; a click
    /// that never reaches the screen would leave it standing for the next paint.
    theme_switch_at: Option<(Instant, u32)>,
    /// The desk the picture is built from, read once and reused. A stepper changes
    /// the dial's own numbers - the radius, the air, the limit - and none of them
    /// changes which windows exist, so rebuilding from this copy is the difference
    /// between a click that answers at once and one that waits on the desktop.
    preview_desk: Option<(Vec<WindowInfo>, Option<isize>)>,
    /// Where the pointer went down on the picture, whatever it landed on. The
    /// release point decides a click by its distance from this point, and the
    /// click itself is resolved here rather than under the cursor.
    dial_press: Option<(i32, i32)>,
    /// A sentence shown in the note band for a moment after a row was taken off
    /// the dial or put back, so the change is said as well as drawn.
    dial_toast: Option<(String, Instant)>,
    /// The off-screen buffer the preview card is composed in. Reused so a
    /// repaint allocates nothing.
    canvas: PreviewCanvas,
    /// The off-screen buffer the whole page is composed in, so the window only
    /// ever receives a finished frame through one blit.
    page: PageCanvas,
    /// Where the full window list beside the picture is scrolled to. The list is
    /// every window the panel knows, not the first three.
    pool_scroll: usize,
    /// Where the page itself is scrolled to, in device pixels. Zero until the
    /// window is dragged below the height its section's cards want; the paper
    /// then runs under the glass and the wheel moves it.
    page_scroll: f32,
}

/// The row a click landed on and when it did, so the next click can be told to
/// be on the same row and inside the same measure. The window class has no
/// `CS_DBLCLKS` - adding it would swallow the second `WM_LBUTTONDOWN` that the
/// theme strip, the steppers and the picture all count on - so the
/// double-click is measured here rather than declared to the system.
struct RowClick {
    at: usize,
    when: Instant,
}

/// The parts of `client` that lie outside `cell`: four bands, no overlap. The
/// page is papered here and the cell itself is left for the one blit that owns it.
fn outside(client: RECT, cell: RECT) -> [RECT; 4] {
    let cell = RECT {
        left: cell.left.max(client.left),
        top: cell.top.max(client.top),
        right: cell.right.min(client.right),
        bottom: cell.bottom.min(client.bottom),
    };
    [
        RECT {
            left: client.left,
            top: client.top,
            right: client.right,
            bottom: cell.top,
        },
        RECT {
            left: client.left,
            top: cell.bottom,
            right: client.right,
            bottom: client.bottom,
        },
        RECT {
            left: client.left,
            top: cell.top,
            right: cell.left,
            bottom: cell.bottom,
        },
        RECT {
            left: cell.right,
            top: cell.top,
            right: client.right,
            bottom: cell.bottom,
        },
    ]
}

/// Device pixels a press has to travel before it counts as a click rather than
/// a shaky click. Six logical pixels: the four a mouse is allowed to shake with,
/// plus the two a hand moving a little before it commits adds.
const SEAT_DRAG_SLOP: f32 = 6.0;

/// The preview card composed off screen: one memory DC holds the paper and the
/// frame, and the page receives it in a single `BitBlt`. Two passes onto the
/// window - paper, then picture - leave a window in which the card is still
/// paper.
struct PreviewCanvas {
    mem: HDC,
    mem_bitmap: Option<HGDIOBJ>,
    mem_old: Option<HGDIOBJ>,
    src: HDC,
    src_bitmap: Option<HGDIOBJ>,
    src_old: Option<HGDIOBJ>,
    src_bits: *mut c_void,
    side: u32,
}

impl Default for PreviewCanvas {
    fn default() -> Self {
        Self {
            mem: HDC::default(),
            mem_bitmap: None,
            mem_old: None,
            src: HDC::default(),
            src_bitmap: None,
            src_old: None,
            src_bits: std::ptr::null_mut(),
            side: 0,
        }
    }
}

impl PreviewCanvas {
    /// A square of `side` pixels, or true when one of that size is already here.
    fn ensure(&mut self, side: u32) -> bool {
        unsafe {
            if self.side == side
                && !self.mem.is_invalid()
                && !self.src.is_invalid()
                && self.side > 0
            {
                return true;
            }
            self.release();
            let info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: side as i32,
                    biHeight: -(side as i32),
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mem = CreateCompatibleDC(None);
            if mem.is_invalid() {
                return false;
            }
            let mut bits: *mut c_void = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(mem);
                return false;
            };
            if bits.is_null() {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(mem);
                return false;
            }
            let mem_old = SelectObject(mem, bitmap.into());
            let src = CreateCompatibleDC(None);
            if src.is_invalid() {
                let _ = SelectObject(mem, mem_old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(mem);
                return false;
            }
            let mut src_bits: *mut c_void = std::ptr::null_mut();
            let Ok(src_bitmap) =
                CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut src_bits, None, 0)
            else {
                let _ = DeleteDC(src);
                let _ = SelectObject(mem, mem_old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(mem);
                return false;
            };
            if src_bits.is_null() {
                let _ = DeleteObject(src_bitmap.into());
                let _ = DeleteDC(src);
                let _ = SelectObject(mem, mem_old);
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(mem);
                return false;
            }
            let src_old = SelectObject(src, src_bitmap.into());
            self.mem = mem;
            self.mem_bitmap = Some(bitmap.into());
            self.mem_old = Some(mem_old);
            self.src = src;
            self.src_bitmap = Some(src_bitmap.into());
            self.src_old = Some(src_old);
            self.src_bits = src_bits;
            self.side = side;
            true
        }
    }

    /// Frees the pair. Called with the window.
    fn clear(&mut self) {
        unsafe {
            self.release();
        }
    }

    unsafe fn release(&mut self) {
        if !self.mem.is_invalid() {
            if let (Some(old), Some(bitmap)) = (self.mem_old, self.mem_bitmap) {
                let _ = SelectObject(self.mem, old);
                let _ = DeleteObject(bitmap);
            }
            let _ = DeleteDC(self.mem);
        }
        if !self.src.is_invalid() {
            if let (Some(old), Some(bitmap)) = (self.src_old, self.src_bitmap) {
                let _ = SelectObject(self.src, old);
                let _ = DeleteObject(bitmap);
            }
            let _ = DeleteDC(self.src);
        }
        self.mem = HDC::default();
        self.mem_bitmap = None;
        self.mem_old = None;
        self.src = HDC::default();
        self.src_bitmap = None;
        self.src_old = None;
        self.src_bits = std::ptr::null_mut();
        self.side = 0;
    }
}

/// The whole page composed off screen: ground, cards, words and the preview
/// card all land in this bitmap first, and the window receives them in one
/// `BitBlt` of the update rectangle. Painting straight onto the window surface
/// leaves the paper visible for however long the rest of the page takes, and a
/// click or a drag frame can be composed by the desktop in the middle of that -
/// the white blink. One blit has no middle.
#[derive(Default)]
struct PageCanvas {
    mem: HDC,
    bitmap: Option<HGDIOBJ>,
    old: Option<HGDIOBJ>,
    w: i32,
    h: i32,
}

impl PageCanvas {
    /// A page of `w` by `h` pixels, or true when one that size is already here.
    fn ensure(&mut self, w: i32, h: i32) -> bool {
        unsafe {
            if self.w == w && self.h == h && !self.mem.is_invalid() && w > 0 && h > 0 {
                return true;
            }
            self.clear();
            if w <= 0 || h <= 0 {
                return false;
            }
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
            let mem = CreateCompatibleDC(None);
            if mem.is_invalid() {
                return false;
            }
            let mut bits: *mut c_void = std::ptr::null_mut();
            let Ok(bitmap) = CreateDIBSection(None, &info, DIB_RGB_COLORS, &mut bits, None, 0)
            else {
                let _ = DeleteDC(mem);
                return false;
            };
            if bits.is_null() {
                let _ = DeleteObject(bitmap.into());
                let _ = DeleteDC(mem);
                return false;
            }
            let old = SelectObject(mem, bitmap.into());
            self.mem = mem;
            self.bitmap = Some(bitmap.into());
            self.old = Some(old);
            self.w = w;
            self.h = h;
            true
        }
    }

    fn clear(&mut self) {
        unsafe {
            if !self.mem.is_invalid() {
                if let (Some(old), Some(bitmap)) = (self.old, self.bitmap) {
                    let _ = SelectObject(self.mem, old);
                    let _ = DeleteObject(bitmap);
                }
                let _ = DeleteDC(self.mem);
            }
            self.mem = HDC::default();
            self.bitmap = None;
            self.old = None;
            self.w = 0;
            self.h = 0;
        }
    }
}

/// The dial as 轮盘 draws it: the pixels and the seating plan that says which
/// window each click landed on.
struct DialPreview {
    /// The full still picture, base plus seats at rest. What is shown.
    pixmap: Pixmap,
    snapshot: crate::wheel::DialSnapshot,
    /// Where the picture sits, in device pixels on the page.
    x: i32,
    y: i32,
    side: u32,
    scale: f32,
    /// The dial settings this picture was made from, and a hash of the seats it
    /// was made from. A picture that only remembered its rectangle would keep
    /// showing the old theme after a click that changed nothing about the size;
    /// a key without the seats would keep showing a sector that has left the
    /// dial, because the limit and the desk size can stay the same while a row
    /// moves between the two sections.
    key: (u32, u32, u32, u32, u32, u32, u32, usize, u64),
}

/// The filmstrip's six pictures: one theme each, at the strip's size. A slot is
/// `None` when the renderer refused that theme, and the strip then draws the
/// empty card instead of failing the page.
struct ThemeThumbs {
    side: u32,
    pixmaps: [Option<Pixmap>; DIAL_THEMES],
}

/// Every text band and control slot of ONE section, in device pixels. `measure`
/// fills it in; the painter and the child windows read it, so nothing is
/// positioned twice.
#[derive(Clone, Copy, Default)]
struct Layout {
    /// Which section this is the layout of, so the painter knows what it is
    /// drawing and a relanguage knows which words are on screen.
    section: usize,
    /// The navigation column: one button per section, in `SEC_NAME` order. Filled
    /// for every section, because it is always on screen.
    nav: [RECT; SEC_COUNT],
    /// The cards of this section, in reading order, and the two lines of text each
    /// one holds. `card` is the surface, `title` and `note` the words inside it.
    card: [RECT; MAX_CARDS],
    title: [RECT; MAX_CARDS],
    note: [RECT; MAX_CARDS],
    row_count: usize,
    // --- the controls, hung on the rows `CARDS` names ---------------------------
    /// 快捷键: the recorder field and the two shortcut boxes.
    trigger_field: RECT,
    field_fav: RECT,
    field_panel: RECT,
    /// 轮盘: the picture, its filmstrip, then the cells and the two steppers
    /// under it.
    dial_preview: RECT,
    /// The filmstrip under the picture: six thumbnails, each drawn through the
    /// same renderer the dial itself uses. Clicking one is the choice.
    dial_strip: RECT,
    dial_themes: [RECT; DIAL_THEMES],
    /// The list card on the right: the search field, the two tuning rows and the
    /// scrolling list of every window all live on this one surface.
    dial_list_card: RECT,
    /// The compact parameter card under the list. One surface holds all six
    /// rows, so the strip reads as a band rather than six cards.
    dial_param_card: RECT,
    dial_cells: [RECT; DIAL_CELLS],
    dial_steps: [RECT; 6],
    dial_values: [RECT; 3],
    /// The scrolling list of every window, beside the picture. Rows are counted
    /// from this rectangle, so the height the picture leaves decides how many are
    /// on paper; the rest are a wheel-turn away.
    dial_list: RECT,
    /// The field above the list that filters it by title, executable or
    /// application, so a desktop with forty windows does not have to be scrolled
    /// to be searched. This is the frame's rectangle, icon included.
    dial_search: RECT,
    /// The edit control inside that frame: the frame's box with the magnifier's
    /// strip taken off the left and the border's inset off every edge.
    dial_search_input: RECT,
    /// The square button at the search field's right hand: re-reads the running
    /// windows so a window opened while the page was up appears without closing
    /// and reopening the settings.
    dial_refresh: RECT,
    /// 语言: the buttons, filled from the front for as many as the folder offers.
    /// Slots past `lang_count` are empty bands. Then the door to the files.
    lang: [RECT; SEG_PER_ROW * MAX_LANG_ROW],
    lang_count: usize,
    folder: RECT,
    /// 关于: two right-aligned values, the door to the log, and the switch for
    /// reduced motion.
    value_version: RECT,
    value_license: RECT,
    log_door: RECT,
    switch_motion: RECT,
    /// How far the paper under the page is taller than the glass, in device
    /// pixels. Zero means everything is on screen; anything else means the
    /// section's bands were shifted up by that much so the rest can be reached.
    scroll_max: f32,
}

fn band(x0: f32, x1: f32, y: f32, h: f32) -> RECT {
    RECT {
        left: x0.round() as i32,
        top: y.round() as i32,
        right: x1.round() as i32,
        bottom: (y + h).round() as i32,
    }
}

/// Put one child window on one measured band. Zero-size bands leave the control
/// alone: a language button that the folder no longer offers is hidden, and a
/// hidden button parked at 0x0 would still be in the Tab order.
///
/// `SWP_NOCOPYBITS` is what keeps a resize from leaving ghosts. `MoveWindow`
/// copies the control's old pixels back at its new origin, and a class without
/// `CS_HREDRAW` is then asked to paint only the edge the resize uncovered - so
/// the copied pixels stay on screen, and a cell that moved right by half its own
/// width shows its old face shifted under the new one. Discarding the bits makes
/// the whole client new, and the control paints itself once, cleanly.
unsafe fn move_child(hwnd: HWND, r: RECT) {
    unsafe {
        let w = r.right - r.left;
        let h = r.bottom - r.top;
        if hwnd.is_invalid() || w <= 0 || h <= 0 {
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            r.left,
            r.top,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOCOPYBITS,
        );
    }
}

/// 轮盘's own walk: a three-zone desk rather than a column of cards.
///
/// The name column is already measured, and its edge is where the content
/// begins. From there the page divides in two: the picture's card takes every
/// pixel the window list does not need, and the list's card takes a bounded
/// share on the right. Both cards run the height of the client, so the picture
/// is the one thing on the page that grows with the window. The six parameters
/// are a compact strip under the list instead of six full-width rows under the
/// picture - the rows the user could not see past.
///
/// Under [`DIAL_DESK_MIN`] the share is not an answer at all: the picture's
/// 260-pixel floor plus the list's 180-pixel column plus the gutter and both
/// insets need more width than the glass has. The desk then folds into one
/// column - picture, list, strip - and the page scrolls to the rest.
fn dial_layout(
    l: &mut Layout,
    client_w: i32,
    client_h: i32,
    s: f32,
    nav_x1: f32,
    scroll: f32,
) {
    let content_x0 = nav_x1 + GUTTER * s;
    let content_x1 = (client_w as f32 - PAD * s).max(content_x0 + 200.0 * s);
    let content_w = content_x1 - content_x0;
    let avail_h = (client_h as f32 - (PAD + PAD_BOTTOM) * s).max(200.0 * s);
    if content_w < DIAL_DESK_MIN * s {
        dial_stack(l, content_x0, content_x1, avail_h, s, scroll);
        return;
    }
    // The desk is as tall as the glass by design: the picture grows with it,
    // so there is nothing below the fold to scroll to.
    let content_top = PAD * s;
    let content_h = avail_h;

    // The right column. It keeps its own share so a row of the list reads as a
    // row and not as a ruler; the picture gets every pixel that is left.
    let col_share = content_w * DIAL_COL_SHARE;
    // At the narrow end of the desk the card's own padding gives six pixels
    // back to each side as well: the dial must still measure its floor, the list
    // must still hold rows, and the two are only both true if the paper between
    // them is a little thinner.
    let narrow = col_share < DIAL_COL_MIN * s;
    let preview_pad = if content_w < (DIAL_COL_MIN + GUTTER + 300.0 + 2.0 * CARD_PAD) * s {
        8.0
    } else {
        CARD_PAD
    };
    let picture_card = (300.0 + preview_pad * 2.0) * s;
    let col_room = content_w - GUTTER * s - picture_card;
    let col_w = if narrow {
        col_room.clamp(180.0 * s, DIAL_COL_MIN * s)
    } else {
        col_share
            .clamp(DIAL_COL_MIN * s, DIAL_COL_MAX * s)
            .min(col_room.max(180.0 * s))
    };
    let col_x = content_x1 - col_w;
    let dial_x1 = (col_x - GUTTER * s).max(content_x0 + 160.0 * s);

    let params = dial_param_metrics(col_x, content_x1, s);
    let params_h = params.h;
    let params_top = content_top + content_h - params_h;

    // --- the picture ----------------------------------------------------------
    l.card[DIAL_PREVIEW_ROW] = band(content_x0, dial_x1, content_top, content_h);
    let card_in_l = content_x0 + preview_pad * s;
    let card_in_r = dial_x1 - preview_pad * s;
    l.title[DIAL_PREVIEW_ROW] = band(card_in_l, card_in_r, content_top + preview_pad * s, TITLE_H * s);
    l.note[DIAL_PREVIEW_ROW] = band(
        card_in_l,
        card_in_r,
        content_top + (preview_pad + TITLE_H + 2.0) * s,
        NOTE_H * s,
    );
    let body_top = content_top + (preview_pad + TITLE_H + 2.0 + NOTE_H + 2.0) * s;
    let body_bottom = (content_top + content_h - preview_pad * s).max(body_top + 110.0 * s);
    // The filmstrip sits on the card's own floor: six thumbnails in a row, each
    // showing the look it would give the dial. The square above it gives up the
    // height the strip needs, and no more.
    let strip_top = (body_bottom - DIAL_THEME_H * s).max(body_top + 110.0 * s);
    let body_h = (strip_top - 8.0 * s - body_top).max(110.0 * s);
    let body_w = (card_in_r - card_in_l).max(110.0 * s);
    // A square, centred in what is left: the dial is the subject, so it takes
    // the smaller of the two dimensions and sits in the middle of the other.
    let side = body_w.min(body_h).max(110.0 * s);
    let px = card_in_l + (body_w - side) * 0.5;
    let py = body_top + (body_h - side) * 0.5;
    // Rounded once, on all four edges: a square has to stay square to the pixel
    // because the buffer is built from this rectangle.
    let side = side.round() as i32;
    let px = px.round() as i32;
    let py = py.round() as i32;
    l.dial_preview = RECT {
        left: px,
        top: py,
        right: px + side,
        bottom: py + side,
    };
    l.dial_strip = band(card_in_l, card_in_r, strip_top, DIAL_THEME_H * s);
    let strip_w = card_in_r - card_in_l;
    let cell_step = strip_w / DIAL_THEMES as f32;
    for i in 0..DIAL_THEMES {
        let x = card_in_l + i as f32 * cell_step;
        l.dial_themes[i] = band(x, x + cell_step - DIAL_THEME_GAP * s, strip_top, DIAL_THEME_H * s);
    }

    // --- the list ------------------------------------------------------------
    let list_bottom = (params_top - CARD_GAP * s).max(content_top + 120.0 * s);
    l.dial_list_card = band(col_x, content_x1, content_top, list_bottom - content_top);
    let list_in_l = col_x + DIAL_LIST_PAD * s;
    let list_in_r = content_x1 - DIAL_LIST_PAD * s;
    let refresh_w = FIELD_H * s;
    let search_y = content_top + DIAL_LIST_PAD * s;
    l.dial_refresh = band(list_in_r - refresh_w, list_in_r, search_y, FIELD_H * s);
    let search_r = l.dial_refresh.left as f32 - DIAL_REFRESH_GAP * s;
    l.dial_search = band(list_in_l, search_r, search_y, FIELD_H * s);
    // The edit sits inside the frame the page draws: the magnifier's strip is
    // taken off the left, and the border's two pixels off every edge, so the
    // field reads as a box rather than as a line of text on the card.
    let search_icon = DIAL_SEARCH_ICON * s;
    let search_inset = DIAL_SEARCH_INSET * s;
    l.dial_search_input = band(
        list_in_l + search_icon + search_inset,
        search_r - search_inset,
        search_y + search_inset,
        FIELD_H * s - search_inset * 2.0,
    );
    // The list starts on the search field's own floor.
    let list_top = l.dial_search.bottom as f32 + DIAL_LIST_GAP * s;
    let list_bottom =
        (l.dial_list_card.bottom as f32 - DIAL_LIST_PAD * s).max(list_top + POOL_ROW_STEP * s);
    l.dial_list = band(list_in_l, list_in_r, list_top, list_bottom - list_top);

    // --- the parameter strip -------------------------------------------------
    dial_params(l, &params, params_top, s);
    l.row_count = card_count(SEC_DIAL);
}

/// The parameter strip's own measurements: how tall it is, and where its name
/// column and control column sit across it. Both the desk and the folded page
/// take their strip from here, so the two can never disagree about a row.
struct ParamStrip {
    card_l: f32,
    card_r: f32,
    in_l: f32,
    in_r: f32,
    h: f32,
    label_x1: f32,
    ctrl_x0: f32,
    ctrl_x1: f32,
    ctrl_w: f32,
    step_w: f32,
    wide: bool,
    group_rows: usize,
}

/// The strip's width-driven numbers. At the narrow end the name column gives
/// eight pixels back to the controls: the names are one or two characters, and
/// the four effect words need every pixel the strip can find.
fn dial_param_metrics(card_l: f32, card_r: f32, s: f32) -> ParamStrip {
    let in_l = card_l + DIAL_PARAM_PAD * s;
    let in_r = card_r - DIAL_PARAM_PAD * s;
    let inner_w = in_r - in_l;
    let label_w = if inner_w < 220.0 * s {
        DIAL_ROW_LABEL_NARROW
    } else {
        DIAL_ROW_LABEL_W
    };
    let label_x1 = in_l + label_w * s;
    let ctrl_x0 = label_x1 + GAP_LABEL * s;
    let ctrl_x1 = in_r;
    let ctrl_w = (ctrl_x1 - ctrl_x0).max(1.0);
    // The stepper's ends keep the number between them readable: at the narrow
    // floor the arrows give up four pixels each before the value does.
    let step_w = (STEP_W * s).min(((ctrl_w - 44.0 * s) / 2.0).max(28.0 * s));
    // Four words in the width a cell needs, or two rows of two when the column
    // is too narrow for them. The strip is the only group of controls on the
    // page that may fold, and it folds downwards, not into a scroll.
    let wide = ctrl_w >= DIAL_ROW_WIDE * s;
    let group_rows = if wide { 1 } else { 2 };
    let group_units = 5.0 + 2.0 * group_rows as f32;
    let group_gaps = 6.0 + 2.0 * (group_rows as f32 - 1.0);
    let h = DIAL_PARAM_PAD * 2.0 * s
        + group_units * DIAL_ROW_H * s
        + group_gaps * DIAL_ROW_GAP * s;
    ParamStrip {
        card_l,
        card_r,
        in_l,
        in_r,
        h,
        label_x1,
        ctrl_x0,
        ctrl_x1,
        ctrl_w,
        step_w,
        wide,
        group_rows,
    }
}

/// Lay the strip's seven rows on its card. `top` is the card's own top, so the
/// caller decides whether the strip hangs under the list or under the glass.
fn dial_params(l: &mut Layout, m: &ParamStrip, top: f32, s: f32) {
    l.dial_param_card = band(m.card_l, m.card_r, top, m.h);
    let row_step = DIAL_ROW_H + DIAL_ROW_GAP;
    let mut py = top + DIAL_PARAM_PAD * s;
    for at in 1..=DIAL_POSITION_ROW {
        let rows_here = if at == DIAL_ANIM_ROW { m.group_rows } else { 1 };
        let h = rows_here as f32 * DIAL_ROW_H * s
            + (rows_here - 1) as f32 * DIAL_ROW_GAP * s;
        l.card[at] = band(m.in_l, m.in_r, py, h);
        // The name sits in the left column of the strip, centred over the row
        // or the two rows of cells it names.
        l.title[at] = band(m.in_l, m.label_x1, py, h);
        let cell_step = m.ctrl_w / if m.wide { 4.0 } else { 2.0 };
        let cell_w = (cell_step - CELL_GAP * s).max(1.0);
        let cell_row = |row: usize| {
            py + row as f32 * row_step * s + (DIAL_ROW_H - FIELD_H) * 0.5 * s
        };
        match at {
            DIAL_ACCENT_ROW => {
                let step = m.ctrl_w / DIAL_ACCENT_CELLS as f32;
                let w = (step - CELL_GAP * s).max(1.0);
                for i in 0..DIAL_ACCENT_CELLS {
                    let x = m.ctrl_x0 + i as f32 * step;
                    l.dial_cells[i] = band(x, x + w, cell_row(0), FIELD_H * s);
                }
            }
            DIAL_ANIM_ROW => {
                for i in 0..DIAL_ANIM_CELLS {
                    let (row, column) = if m.wide { (0, i) } else { (i / 2, i % 2) };
                    let x = m.ctrl_x0 + column as f32 * cell_step;
                    l.dial_cells[DIAL_ACCENT_CELLS + i] =
                        band(x, x + cell_w, cell_row(row), FIELD_H * s);
                }
            }
            DIAL_ORDER_ROW => {
                let step = m.ctrl_w / DIAL_ORDER_CELLS as f32;
                let w = (step - CELL_GAP * s).max(1.0);
                for i in 0..DIAL_ORDER_CELLS {
                    let x = m.ctrl_x0 + i as f32 * step;
                    l.dial_cells[DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + i] =
                        band(x, x + w, cell_row(0), FIELD_H * s);
                }
            }
            DIAL_POSITION_ROW => {
                let step = m.ctrl_w / DIAL_POSITION_CELLS as f32;
                let w = (step - CELL_GAP * s).max(1.0);
                for i in 0..DIAL_POSITION_CELLS {
                    let x = m.ctrl_x0 + i as f32 * step;
                    l.dial_cells
                        [DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + DIAL_ORDER_CELLS + i] =
                        band(x, x + w, cell_row(0), FIELD_H * s);
                }
            }
            _ => {
                // radius, spacing and the limit: two arrows around the number
                // they move, in the same band the cells use.
                let (value, first) = match at {
                    DIAL_RADIUS_ROW => (0, 0),
                    DIAL_SEAM_ROW => (1, 2),
                    _ => (2, 4),
                };
                let cy = cell_row(0);
                l.dial_steps[first] = band(m.ctrl_x0, m.ctrl_x0 + m.step_w, cy, FIELD_H * s);
                l.dial_steps[first + 1] =
                    band(m.ctrl_x1 - m.step_w, m.ctrl_x1, cy, FIELD_H * s);
                l.dial_values[value] =
                    band(m.ctrl_x0 + m.step_w, m.ctrl_x1 - m.step_w, cy, FIELD_H * s);
            }
        }
        py += h + DIAL_ROW_GAP * s;
    }
}

/// The desk folded into one column: the picture, the list, then the strip, one
/// under the other. This is what keeps both promises at every width - a picture
/// never smaller than 260 logical pixels and a list never shorter than three
/// rows - by letting the paper grow past the glass instead of squeezing them.
///
/// The picture's card is fixed at [`DIAL_STACK_PREVIEW_H`], the list at the
/// height its search field, its two tuning rows and both headings plus three
/// window rows need, and the strip at whatever its own row count asks for. The
/// page then scrolls to everything below the first card.
fn dial_stack(
    l: &mut Layout,
    x0: f32,
    x1: f32,
    avail_h: f32,
    s: f32,
    scroll: f32,
) {
    let preview_h = DIAL_STACK_PREVIEW_H * s;
    // Three window rows under the two headings the list always draws: five
    // lines of 32, the field's 28 and its two 8-pixel pads, the air under the
    // field and the air over the last row.
    let list_h = 210.0 * s;
    let params = dial_param_metrics(x0, x1, s);
    let paper_h = preview_h + CARD_GAP * s + list_h + CARD_GAP * s + params.h;
    l.scroll_max = (paper_h - avail_h).max(0.0);
    let top = PAD * s - scroll.clamp(0.0, l.scroll_max);

    // --- the picture ----------------------------------------------------------
    l.card[DIAL_PREVIEW_ROW] = band(x0, x1, top, preview_h);
    let preview_pad = CARD_PAD;
    let card_in_l = x0 + preview_pad * s;
    let card_in_r = x1 - preview_pad * s;
    l.title[DIAL_PREVIEW_ROW] =
        band(card_in_l, card_in_r, top + preview_pad * s, TITLE_H * s);
    l.note[DIAL_PREVIEW_ROW] = band(
        card_in_l,
        card_in_r,
        top + (preview_pad + TITLE_H + 2.0) * s,
        NOTE_H * s,
    );
    let body_top = top + (preview_pad + TITLE_H + 2.0 + NOTE_H + 2.0) * s;
    let body_bottom = top + preview_h - preview_pad * s;
    // The filmstrip sits on the card's own floor, the same as on the desk.
    let strip_top = body_bottom - DIAL_THEME_H * s;
    let body_h = (strip_top - 8.0 * s - body_top).max(110.0 * s);
    let body_w = (card_in_r - card_in_l).max(110.0 * s);
    let side = body_w.min(body_h).max(110.0 * s);
    let px = (card_in_l + (body_w - side) * 0.5).round() as i32;
    let py = (body_top + (body_h - side) * 0.5).round() as i32;
    let side = side.round() as i32;
    l.dial_preview = RECT {
        left: px,
        top: py,
        right: px + side,
        bottom: py + side,
    };
    l.dial_strip = band(card_in_l, card_in_r, strip_top, DIAL_THEME_H * s);
    let strip_w = card_in_r - card_in_l;
    let cell_step = strip_w / DIAL_THEMES as f32;
    for i in 0..DIAL_THEMES {
        let x = card_in_l + i as f32 * cell_step;
        l.dial_themes[i] = band(x, x + cell_step - DIAL_THEME_GAP * s, strip_top, DIAL_THEME_H * s);
    }

    // --- the list ------------------------------------------------------------
    let list_top = top + preview_h + CARD_GAP * s;
    l.dial_list_card = band(x0, x1, list_top, list_h);
    let in_l = x0 + DIAL_LIST_PAD * s;
    let in_r = x1 - DIAL_LIST_PAD * s;
    let refresh_w = FIELD_H * s;
    let search_y = list_top + DIAL_LIST_PAD * s;
    l.dial_refresh = band(in_r - refresh_w, in_r, search_y, FIELD_H * s);
    let search_r = l.dial_refresh.left as f32 - DIAL_REFRESH_GAP * s;
    l.dial_search = band(in_l, search_r, search_y, FIELD_H * s);
    let search_icon = DIAL_SEARCH_ICON * s;
    let search_inset = DIAL_SEARCH_INSET * s;
    l.dial_search_input = band(
        in_l + search_icon + search_inset,
        search_r - search_inset,
        search_y + search_inset,
        FIELD_H * s - search_inset * 2.0,
    );
    let rows_top = l.dial_search.bottom as f32 + DIAL_LIST_GAP * s;
    let rows_bottom = list_top + list_h - DIAL_LIST_PAD * s;
    l.dial_list = band(in_l, in_r, rows_top, (rows_bottom - rows_top).max(POOL_ROW_STEP * s));

    // --- the parameter strip -------------------------------------------------
    let params_top = list_top + list_h + CARD_GAP * s;
    dial_params(l, &params, params_top, s);
    l.row_count = card_count(SEC_DIAL);
}

/// The one place the rows are laid out, top to bottom, for one section.
/// [`logical_h`] is this walk with the same constants, and the test below says so:
/// the window is exactly as tall as the tallest section it can show.
#[allow(clippy::field_reassign_with_default)] // each band needs the running `y`
fn measure(section: usize, client_w: i32, client_h: i32, s: f32, langs: usize) -> Layout {
    measure_tuned(section, client_w, client_h, s, langs, 0.0)
}

/// [`measure`] with the page's scroll offset said out loud.
///
/// `scroll` is how far the page has been pushed up. The bands are measured on a
/// paper that may be taller than the glass, then shifted by that offset, so the
/// painter, the hit tests and the child windows all keep reading one rectangle
/// each. [`Layout::scroll_max`] is what the wheel clamps against.
#[allow(clippy::field_reassign_with_default)]
fn measure_tuned(
    section: usize,
    client_w: i32,
    client_h: i32,
    s: f32,
    langs: usize,
    scroll: f32,
) -> Layout {
    let mut l = Layout::default();
    l.section = section;
    let offered = langs.min(SEG_PER_ROW * MAX_LANG_ROW);
    let rows = lang_rows(offered);

    // The navigation column first, because the content column begins at its edge.
    // It is the same measurement in every section, which is the point of it: the
    // names do not move when one of them is clicked. A narrow window gives the
    // names less room rather than giving the controls none: the column may take
    // at most a quarter of the client, so the fields keep their full width.
    let nav_w = NAV_W.min(client_w as f32 / s * 0.30);
    let nav_x1 = PAD * s + nav_w * s;
    let mut y = PAD * s;
    for (slot, at) in NAV_SECTIONS.iter().copied().enumerate() {
        l.nav[at] = band(PAD * s, nav_x1, y, SEG_H * s);
        if slot + 1 < NAV_SECTIONS.len() {
            y += (SEG_H + SEG_GAP) * s;
        }
    }

    // 轮盘 is not a column of cards: it is a three-zone desk, measured by its
    // own walk. Everything below this line is the single-column sections.
    if section == SEC_DIAL {
        dial_layout(&mut l, client_w, client_h, s, nav_x1, scroll);
        return l;
    }

    let x0 = nav_x1 + GUTTER * s;
    // A card keeps its text and its control a padding away from its own edge, so
    // the control column sits one padding left of the single page's and keeps the
    // width every control was measured with.
    let x1 = client_w as f32 - PAD * s - CARD_PAD * s;
    let field_x = x1 - FIELD_W * s;
    let text_x0 = x0 + CARD_PAD * s;
    let text_x1 = field_x - GAP_LABEL * s;

    let write_h = |at: usize| card_h(section, at, rows) * s;
    // One card writes its own second band: the language grid's row of names. It
    // takes the width of the card rather than the width of a sentence, because
    // what it holds is controls, not prose.
    let writes_its_own_note = |at: usize| section == SEC_LANG && at == LANG_GRID_ROW;
    let count = card_count(section);
    // The cards are as tall as they were designed to be. A window below that
    // scrolls instead of squeezing them: shrinking is allowed, starving a row of
    // its control is not. The navigation column stays where it is - it is the one
    // thing that must be reachable without a wheel.
    let content_h = section_h(section, rows) * s - (PAD + PAD_BOTTOM) * s;
    let avail_h = (client_h as f32 - (PAD + PAD_BOTTOM) * s).max(0.0);
    l.scroll_max = (content_h - avail_h).max(0.0);
    let mut tops = [0.0f32; MAX_CARDS];
    let mut heights = [0.0f32; MAX_CARDS];
    let mut y = PAD * s - scroll.clamp(0.0, l.scroll_max);
    for (at, top) in tops.iter_mut().enumerate().take(count) {
        let h = write_h(at);
        heights[at] = h;
        *top = y;
        l.card[at] = band(x0, x1 + CARD_PAD * s, y, h);
        l.title[at] = band(text_x0, text_x1, y + CARD_PAD * s, TITLE_H * s);
        if !writes_its_own_note(at) {
            l.note[at] = band(text_x0, text_x1, y + (CARD_PAD + TITLE_H) * s, NOTE_H * s);
        }
        y += h + CARD_GAP * s;
    }
    l.row_count = count;

    // The slots a control sits in: the field column, centred in its card.
    let ctrl = |at: usize| {
        band(
            field_x,
            x1,
            tops[at] + (heights[at] - FIELD_H * s) / 2.0,
            FIELD_H * s,
        )
    };
    // The number a pair of arrows moves, and the two values 关于 right-aligns.
    let value = |at: usize| {
        band(
            field_x,
            x1,
            tops[at] + CARD_PAD * s,
            heights[at] - CARD_PAD * 2.0 * s,
        )
    };
    let toggle = |at: usize| {
        band(
            x1 - SWITCH_BOX_W * s,
            x1,
            tops[at] + (heights[at] - SWITCH_BOX_H * s) / 2.0,
            SWITCH_BOX_H * s,
        )
    };

    match section {
        SEC_KEYS => {
            // The recorder takes the first row whole: click it, then press the
            // combination you want, the way a game asks for a bind.
            l.trigger_field = ctrl(0);
            l.field_fav = ctrl(1);
            l.field_panel = ctrl(2);
        }
        SEC_DIAL => {
            // Measured by `dial_layout` above, which the early return in
            // `measure` reaches before this walk begins.
        }
        SEC_LANG => {
            // The names share their row's width, so three languages are not one
            // wide button and two narrow ones, and they are inset by the card the
            // way everything else in it is.
            let inner = x1 - text_x0;
            let grid_top = tops[0] + (CARD_PAD + TITLE_H + LANG_NOTE_GAP) * s;
            for row in 0..rows {
                let first = row * SEG_PER_ROW;
                let in_row = (offered - first).min(SEG_PER_ROW);
                let step = inner / in_row as f32;
                let ry = grid_top + row as f32 * (SEG_H + SEG_ROW_GAP) * s;
                for column in 0..in_row {
                    let at = first + column;
                    let sx0 = text_x0 + column as f32 * step;
                    l.lang[at] = band(sx0, sx0 + step - SEG_GAP * s, ry, SEG_H * s);
                }
            }
            l.lang_count = offered;
            // The line under the last row of names, which is the card's second
            // sentence and runs the width of the card rather than of a label.
            l.note[0] = band(
                text_x0,
                x1,
                grid_top
                    + (rows as f32 * SEG_H + SEG_ROW_GAP * (rows - 1) as f32 + LANG_NOTE_GAP) * s,
                NOTE_H * s,
            );
            l.folder = ctrl(1);
        }
        SEC_ABOUT => {
            l.value_version = value(0);
            l.log_door = ctrl(1);
            l.switch_motion = toggle(2);
            l.value_license = value(3);
        }
        _ => {}
    }
    l
}

// --- colour helpers -------------------------------------------------------------

/// GDI keeps its colours blue first.
const fn rgb(c: (u8, u8, u8)) -> COLORREF {
    COLORREF(c.0 as u32 | ((c.1 as u32) << 8) | ((c.2 as u32) << 16))
}

fn solid(c: (u8, u8, u8)) -> HBRUSH {
    unsafe { CreateSolidBrush(rgb(c)) }
}

fn paint(c: (u8, u8, u8), a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0, c.1, c.2, a);
    p.anti_alias = true;
    p
}

/// Rounded rectangles everywhere agree on the radius, in logical pixels: the
/// tile, the buttons and the switch track are the same family of shape.
const RADIUS: f32 = 8.0;

// --- registration ---------------------------------------------------------------

pub fn register_class(hinst: HINSTANCE, big: HICON, small: HICON) {
    unsafe {
        // No common control is created here, so no InitCommonControlsEx: the
        // panel is four classes of our own.
        register(CLASS, hinst, big, small, Some(wndproc), WNDCLASS_STYLES(0));
        register(
            SWITCH_CLASS,
            hinst,
            big,
            small,
            Some(switch_wndproc),
            WNDCLASS_STYLES(0),
        );
        register(
            KEY_CLASS,
            hinst,
            big,
            small,
            Some(key_wndproc),
            WNDCLASS_STYLES(0),
        );
        register(
            BUTTON_CLASS,
            hinst,
            big,
            small,
            Some(button_wndproc),
            WNDCLASS_STYLES(0),
        );
    }
}

unsafe fn register(
    name: PCWSTR,
    hinst: HINSTANCE,
    big: HICON,
    small: HICON,
    proc: WNDPROC,
    style: WNDCLASS_STYLES,
) {
    unsafe {
        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style,
            lpfnWndProc: proc,
            hInstance: hinst,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: big,
            hIconSm: small,
            // Every window here paints its own background; the class brush is
            // only what would show for the instant before the first WM_PAINT.
            hbrBackground: HBRUSH((COLOR_WINDOW.0 + 1) as *mut c_void),
            lpszClassName: name,
            ..Default::default()
        };
        let _ = RegisterClassExW(&class);
    }
}

/// The buttons on the language row: "follow the system" first, then every
/// language the program ships or the folder offers.
///
/// A stored tag nobody offers - a file deleted since - is kept anyway. It is the
/// user's own choice, and a row that silently moved to another button would say
/// the panel had decided something it had not.
fn offered_langs(stored: &str) -> Vec<(String, String)> {
    let mut langs = vec![(crate::i18n::AUTO.to_string(), t("lang_auto").to_string())];
    langs.extend(
        crate::i18n::languages()
            .into_iter()
            .map(|lang| (lang.tag, lang.name)),
    );
    if !langs.iter().any(|(tag, _)| tag == stored) {
        let stored = stored.to_string();
        langs.push((stored.clone(), stored));
    }
    langs.truncate(SEG_PER_ROW * MAX_LANG_ROW);
    langs
}

/// Opens the settings window on one section. The caller keeps the returned
/// handle to focus an existing window instead of making a second one.
pub fn open(
    window_icon: HICON,
    tray_icon: HICON,
    settings: &settings::Settings,
    monitor: Monitor,
    app: *mut App,
    section: usize,
) -> Option<HWND> {
    unsafe {
        let hinst: HINSTANCE = GetModuleHandleW(None).ok()?.into();
        let scale = monitor.scale.max(1.0);
        let langs = offered_langs(&settings.lang);
        // The size the window opens at is the remembered one, clamped by the
        // monitor; the floor only matters when the monitor is shorter than the
        // page, in which case the picture gives its height up rather than the
        // window hanging off the screen.
        let min_h = MIN_CLIENT_H;
        let (open_w, open_h) = panel_open_size(
            (settings.panel_w, settings.panel_h),
            scale,
            monitor.work.width(),
            monitor.work.height(),
            min_h,
        );
        let mut frame = RECT {
            left: 0,
            top: 0,
            right: (open_w * scale).round() as i32,
            bottom: (open_h * scale).round() as i32,
        };
        // WS_CLIPCHILDREN: the page paints a whole off-screen frame and blits it
        // back, and the wheel page does that twice on the way up. Without this
        // the second blit covers the children the first one had already put on
        // screen, which reads as an empty left column and blank value cells.
        let style = WS_OVERLAPPED
            | WS_CAPTION
            | WS_SYSMENU
            | WS_THICKFRAME
            | WS_MAXIMIZEBOX
            | WS_MINIMIZEBOX
            | WS_CLIPCHILDREN;
        let _ = AdjustWindowRectExForDpi(
            &mut frame,
            style,
            false,
            WINDOW_EX_STYLE(0),
            (96.0 * scale).round() as u32,
        );
        let width = frame.right - frame.left;
        let height = frame.bottom - frame.top;
        let work = monitor.work;
        let x = work.left + ((work.width() - width) / 2).max(0);
        let y = work.top + ((work.height() - height) / 2).max(0);

        let title = wstr(t("settings_title"));
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            CLASS,
            PCWSTR(title.as_ptr()),
            style,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(hinst),
            None,
        )
        .ok()?;

        // The floor is one line in the log at every open: the size the handle
        // stops at, and the logical floor it was converted from.
        let dpi_now = GetDpiForWindow(hwnd);
        let (floor_w, floor_h) = min_track_size(dpi_now);
        crate::log!(
            "window: the floor is {}x{} for {} dpi ({}x{} logical)",
            floor_w,
            floor_h,
            dpi_now,
            MIN_CLIENT_W,
            MIN_CLIENT_H
        );

        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let section = section.min(SEC_COUNT - 1);
        // `AdjustWindowRectExForDpi` counts less frame than this system actually
        // puts on a caption window: the border it treats as nothing is eleven
        // pixels to a side here, so the client comes back sixteen logical pixels
        // narrower than the page was measured for. `measure` lays the columns out
        // in the client it is handed, and the first thing those sixteen pixels
        // take is off the end of a sentence. Take them back before anybody sees
        // the window, and shift it by half of what it grows so it stays centred.
        let want = SIZE {
            cx: (open_w * scale).round() as i32,
            cy: (open_h * scale).round() as i32,
        };
        if client.right != want.cx || client.bottom != want.cy {
            let _ = SetWindowPos(
                hwnd,
                None,
                x - (want.cx - client.right) / 2,
                y - (want.cy - client.bottom) / 2,
                width + want.cx - client.right,
                height + want.cy - client.bottom,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let _ = GetClientRect(hwnd, &mut client);
        }
        let ptr = Box::into_raw(Box::new(State {
            app,
            window_icon,
            tray_icon,
            scale,
            fonts: make_fonts(scale),
            section,
            layout: measure(section, client.right, client.bottom, scale, langs.len()),
            hwnd,
            nav_buttons: [HWND::default(); SEC_COUNT],
            slots: Vec::new(),
            fav: HWND::default(),
            panel_key: HWND::default(),
            motion_switch: HWND::default(),
            log_door: HWND::default(),
            trigger_field: HWND::default(),
            trigger_notice: None,
            lang_buttons: Vec::new(),
            lang_tags: langs,
            lang: settings.lang.clone(),
            applied_lang: settings.lang.clone(),
            captions: Vec::new(),
            folder: HWND::default(),
            trigger_mods: settings.trigger_mods,
            trigger_vk: settings.trigger_vk,
            trigger_button: settings.trigger_button,
            wheel_radius: settings.wheel_radius,
            wheel_dead: settings.wheel_dead,
            wheel_theme: settings.wheel_theme,
            wheel_accent: settings.wheel_accent,
            wheel_seam: settings.wheel_seam,
            wheel_open_ms: settings.wheel_open_ms,
            wheel_limit: settings.wheel_limit,
            dial_order: settings.dial_order,
            activate_pos: settings.activate_pos,
            dial_cells: [HWND::default(); DIAL_CELLS],
            dial_steps: [HWND::default(); 6],
            dial_pool: Vec::new(),
            dial_search: HWND::default(),
            pool_all: Vec::new(),
            pool_query: String::new(),
            pool_view: Vec::new(),
            row_press: None,
            row_click: None,
            search_brush: HBRUSH::default(),
            search_hint: String::new(),
            search_show_hint: false,
            search_focused: false,
            refresh_hot: false,
            preview: None,
            theme_thumbs: None,
            theme_thumbs_key: (0, 0, 0, 0, 0),
            theme_switch_at: None,
            dial_press: None,
            dial_toast: None,
            canvas: PreviewCanvas::default(),
            page: PageCanvas::default(),
            pool_scroll: 0,
            page_scroll: 0.0,
            preview_desk: None,
        }));
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr as isize);
        (*ptr).build(hwnd, settings);
        warn_cut(&(*ptr));
        // Building the page lays out the dial's controls but reads no windows:
        // `apply_section` is what walks the desk, and it only runs on a turn
        // from another section. Opening straight onto 轮盘 skipped it, so the
        // list sat empty until the refresh button was pressed. Turning to the
        // section the page is already on is the same ask as the key that named
        // it - it re-reads the list, which here is the first read.
        if section == SEC_DIAL {
            (*ptr).apply_section(SEC_DIAL);
        }
        SETTINGS_HWND.store(hwnd.0 as isize, Ordering::Relaxed);
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);
        // Arriving somewhere is a request about that section, so the name that
        // was clicked - which is on paper whatever the list underneath it holds -
        // takes the focus.
        let _ = SetFocus(Some((*ptr).nav_buttons[section]));
        Some(hwnd)
    }
}

/// Tell the page that the dial's list is not the list it is showing. The shortcut
/// takes a seat, the label editor commits a word, the wheel gains or loses a
/// window - none of them own this window, and none of them may paint into it from
/// the thread they were called on. Posted, not sent: the writer must not wait on a
/// repaint, and a page that is not open has nothing to answer.
pub(crate) fn refresh_rows() {
    let raw = SETTINGS_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = PostMessageW(
            Some(HWND(raw as *mut c_void)),
            WM_ROWS_CHANGED,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// The dial opened. Posted for the same reason as the rows: the caller is the
/// hook's thread, and no window may be painted from there. A page that is not
/// open has nothing to release.
pub(crate) fn wheel_opened() {
    let raw = SETTINGS_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    unsafe {
        let _ = PostMessageW(
            Some(HWND(raw as *mut c_void)),
            WM_WHEEL_OPENED,
            WPARAM(0),
            LPARAM(0),
        );
    }
}

/// Turn a page that is already open to one section, so the key that names a
/// section lands on it the second time as well as the first.
pub(crate) fn show_section(hwnd: HWND, section: usize) {
    unsafe {
        let _ = PostMessageW(Some(hwnd), WM_SHOW_SECTION, WPARAM(section), LPARAM(0));
    }
}

/// Called from the message loop before Translate/Dispatch. Returns true when
/// the dialog manager handled the key (Tab, Enter, Escape, arrows).
pub fn is_dialog_message(msg: &MSG) -> bool {
    let raw = SETTINGS_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return false;
    }
    unsafe { IsDialogMessageW(HWND(raw as *mut c_void), msg).as_bool() }
}

impl State {
    /// The window's whole inventory, made once: the four names down the left and
    /// every card of every section on the right. Which of them are on screen is
    /// [`State::show_only`]'s business, and nothing else has to know it changed -
    /// least of all the chain that applies a row the moment it moves.
    ///
    /// Tab order follows reading order, so the children are created in the order
    /// they are meant to be walked: Windows hands focus to the next control it
    /// created, and a control that is not shown is skipped.
    unsafe fn build(&mut self, parent: HWND, settings: &settings::Settings) {
        unsafe {
            let mut client = RECT::default();
            let _ = GetClientRect(parent, &mut client);
            let offered = self.lang_tags.len();

            // --- the left column -----------------------------------------------
            // The same four bands in every section, which is the point of a
            // column: the names do not move when one of them is clicked.
            for &at in &NAV_SECTIONS {
                let key = SEC_NAME[at];
                let button = self.button(
                    parent,
                    t(key),
                    NAV_BASE + at as i32,
                    self.layout.nav[at],
                    at == self.section,
                    FIELD,
                );
                self.nav_buttons[at] = button;
                self.captions.push((button, key));
            }

            // --- the shortcuts -------------------------------------------------
            let l = measure(SEC_KEYS, client.right, client.bottom, self.scale, offered);
            // The trigger is two rows of cells rather than a sentence with two
            // blanks in it: one modifier, and one button. A cell that looks
            // pressed is the answer, and pressing it is the whole edit. Only one
            // modifier is ever pressed: a chord is not a gesture a hand can hold
            // while dragging.
            self.trigger_field = self.key_trigger(
                parent,
                l.trigger_field,
                MOD_BASE,
                settings.trigger_mods,
                settings.trigger_vk,
                settings.trigger_button,
            );
            self.slots.push((SEC_KEYS, self.trigger_field));
            self.fav = self.key(
                parent,
                l.field_fav,
                FAV_ID,
                settings.fav_mods,
                settings.fav_vk,
            );
            self.slots.push((SEC_KEYS, self.fav));
            self.panel_key = self.key(
                parent,
                l.field_panel,
                PANEL_ID,
                settings.panel_mods,
                settings.panel_vk,
            );
            self.slots.push((SEC_KEYS, self.panel_key));

            // --- the dial's picture and its paint controls -------------------------
            // The preview is painted by the panel itself from a snapshot of the
            // dial; the cells and the two steppers are ordinary controls. The
            // selected cell is the stored value, so the page's own state cannot
            // drift from the settings it claims to be showing.
            let l = measure(SEC_DIAL, client.right, client.bottom, self.scale, offered);
            // The six themes are not controls: the filmstrip is painted on the
            // picture card by `paint_theme_strip`, and the press under it is
            // resolved against `l.dial_themes`. Only the accent row is a cell.
            for (at, key) in DIAL_ACCENT_NAME.iter().enumerate() {
                let cell = self.button(
                    parent,
                    t(key),
                    DIAL_CELL_BASE + at as i32,
                    l.dial_cells[at],
                    settings.wheel_accent == at as u32,
                    PAPER,
                );
                self.dial_cells[at] = cell;
                self.captions.push((cell, *key));
                self.slots.push((SEC_DIAL, cell));
            }
            for (at, key) in DIAL_ANIM_NAME.iter().enumerate() {
                let index = DIAL_ACCENT_CELLS + at;
                let cell = self.button(
                    parent,
                    t(key),
                    DIAL_CELL_BASE + index as i32,
                    l.dial_cells[index],
                    settings.wheel_open_ms == DIAL_ANIM_MS[at],
                    PAPER,
                );
                self.dial_cells[index] = cell;
                self.captions.push((cell, *key));
                self.slots.push((SEC_DIAL, cell));
            }
            for (at, key) in DIAL_ORDER_NAME.iter().enumerate() {
                let index = DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + at;
                let cell = self.button(
                    parent,
                    t(key),
                    DIAL_CELL_BASE + index as i32,
                    l.dial_cells[index],
                    settings.dial_order == at as u32,
                    PAPER,
                );
                self.dial_cells[index] = cell;
                self.captions.push((cell, *key));
                self.slots.push((SEC_DIAL, cell));
            }
            for (at, key) in DIAL_POSITION_NAME.iter().enumerate() {
                let index = DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + DIAL_ORDER_CELLS + at;
                let cell = self.button(
                    parent,
                    t(key),
                    DIAL_CELL_BASE + index as i32,
                    l.dial_cells[index],
                    settings.activate_pos == at as u32,
                    PAPER,
                );
                self.dial_cells[index] = cell;
                self.captions.push((cell, *key));
                self.slots.push((SEC_DIAL, cell));
            }
            for (at, glyph) in ["−", "+", "−", "+", "−", "+"].iter().enumerate() {
                let step = self.button(
                    parent,
                    glyph,
                    DIAL_STEP_BASE + at as i32,
                    l.dial_steps[at],
                    false,
                    PAPER,
                );
                self.dial_steps[at] = step;
                self.slots.push((SEC_DIAL, step));
            }
            // An end already sitting on its floor or ceiling starts greyed, so
            // the first click on a full dial cannot land on a dead arrow.
            self.refresh_steppers();
            // The search field above the list. A real edit control rather than a
            // painted box: the caret, Ctrl+A, the clipboard and the IME are
            // Windows' own, and this page only watches the text change. `ES_LEFT
            // | ES_AUTOHSCROLL` is 0x0080; `EM_SETCUEBANNER` is 0x1501.
            let search_style = WINDOW_STYLE(0x0080);
            let search = self.child(
                parent,
                w!("EDIT"),
                l.dial_search_input,
                SEARCH_ID,
                search_style,
            );
            let _ = SendMessageW(
                search,
                WM_SETFONT,
                Some(WPARAM(self.fonts[1].0 as usize)),
                Some(LPARAM(1)),
            );
            // A little air between the caret and the frame, so the first letter
            // does not sit on the border. `EM_SETMARGINS` with `EC_LEFTMARGIN |
            // EC_RIGHTMARGIN` is 0x00D3.
            let margins = (6.0 * self.scale) as isize;
            let _ = SendMessageW(
                search,
                0x00D3,
                Some(WPARAM(0x0001 | 0x0002)),
                Some(LPARAM(margins | (margins << 16))),
            );
            // The placeholder is the field's own text rather than the system's
            // cue banner: the banner answers FALSE without a version-six
            // common-controls manifest, while text is always painted. It is
            // grey until the first focus and read as an empty box by the filter.
            self.dial_search = search;
            self.search_brush = CreateSolidBrush(rgb(FIELD));
            self.search_hint = t("settings_dial_search").to_string();
            self.search_show_hint = true;
            let hint = wstr(&self.search_hint);
            let _ = SetWindowTextW(search, PCWSTR(hint.as_ptr()));
            self.slots.push((SEC_DIAL, search));
            // --- the languages ----------------------------------------------------
            // One button per language on offer, and the one being used is the one
            // that looks pressed. They are ordinary buttons rather than radio
            // buttons because a name you can read is the whole control.
            let l = measure(SEC_LANG, client.right, client.bottom, self.scale, offered);
            for (at, (tag, name)) in self.lang_tags.clone().iter().enumerate() {
                let selected = *tag == self.lang;
                let button = self.button(parent, name, LANG_BASE + at as i32, l.lang[at], selected, PAPER);
                self.lang_buttons.push(button);
                self.slots.push((SEC_LANG, button));
            }
            self.folder = self.button(parent, t("lang_folder"), FOLDER_ID, l.folder, false, PAPER);
            self.captions.push((self.folder, "lang_folder"));
            self.slots.push((SEC_LANG, self.folder));

            // --- what it is, and what it did ---------------------------------------
            let l = measure(SEC_ABOUT, client.right, client.bottom, self.scale, offered);
            self.log_door = self.button(
                parent,
                t("settings_open_log"),
                LOG_DOOR_ID,
                l.log_door,
                false,
                PAPER,
            );
            self.captions.push((self.log_door, "settings_open_log"));
            self.slots.push((SEC_ABOUT, self.log_door));
            // The knob of this switch obeys the setting its own row changes, which
            // is the only way a row about motion can be believed.
            self.motion_switch = self.switch(
                parent,
                l.switch_motion,
                MOTION_ID,
                settings.reduced_motion,
                settings.reduced_motion,
            );
            self.slots.push((SEC_ABOUT, self.motion_switch));

            self.show_only(self.section);

            if !self.window_icon.is_invalid() {
                let _ = send(
                    parent,
                    WM_SETICON,
                    ICON_BIG as usize,
                    self.window_icon.0 as isize,
                );
            }
            if !self.tray_icon.is_invalid() {
                let _ = send(
                    parent,
                    WM_SETICON,
                    ICON_SMALL as usize,
                    self.tray_icon.0 as isize,
                );
            }
        }
    }

    /// Put every other section's controls away. `SW_HIDE`, not a move off the
    /// paper: a control that is not a window on screen cannot take a click, a
    /// Tab stop, or a screen reader's attention, which is the whole reason the
    /// hidden rows do not have to be un-made when a section changes.
    unsafe fn show_only(&self, section: usize) {
        unsafe {
            for (at, hwnd) in self.slots.iter() {
                let cmd = if *at == section { SW_SHOWNA } else { SW_HIDE };
                let _ = ShowWindow(*hwnd, cmd);
            }
        }
    }

    /// Put every child back on the bands the new client size measured. Called
    /// when the window is resized and when a section is entered, because the
    /// hidden sections were placed at whatever size the window had the last time
    /// they were measured.
    unsafe fn place_children(&self, l: &Layout) {
        unsafe {
            for &at in &NAV_SECTIONS {
                move_child(self.nav_buttons[at], l.nav[at]);
            }
            match l.section {
                SEC_KEYS => {
                    move_child(self.trigger_field, l.trigger_field);
                    move_child(self.fav, l.field_fav);
                    move_child(self.panel_key, l.field_panel);
                }
                SEC_DIAL => {
                    for (cell, rect) in self.dial_cells.iter().zip(l.dial_cells) {
                        move_child(*cell, rect);
                    }
                    for (cell, rect) in self.dial_steps.iter().zip(l.dial_steps) {
                        move_child(*cell, rect);
                    }
                    move_child(self.dial_search, l.dial_search_input);
                }
                SEC_LANG => {
                    for (at, cell) in self.lang_buttons.iter().enumerate() {
                        if at < l.lang_count {
                            move_child(*cell, l.lang[at]);
                        }
                    }
                    move_child(self.folder, l.folder);
                }
                SEC_ABOUT => {
                    move_child(self.log_door, l.log_door);
                    move_child(self.motion_switch, l.switch_motion);
                }
                _ => {}
            }
        }
    }

    /// Turn to another section. The window does not resize - it is as tall as
    /// its tallest section precisely so that this cannot move the bottom edge
    /// out from under the pointer - and no row is applied, because a click on a
    /// name decides nothing about the desktop.
    unsafe fn apply_section(&mut self, section: usize) {
        unsafe {
            if section >= SEC_COUNT {
                return;
            }
            // The list is read on the way in rather than kept: it is the desktop's
            // business, and a page that has been out of sight for ten minutes has
            // no business claiming to know what is on the dial. Coming back to the
            // page that is already on screen is still coming in - the dial's
            // shortcut is how a person asks what is on the dial *now* - so it is
            // not swallowed by the same-section return below.
            if section == self.section {
                if section == SEC_DIAL {
                    // Coming back to 轮盘 refreshes the short lift, but leaves the
                    // picture alone: entering a section is not the same as asking
                    // for it again.
                    self.refresh_dial_pool();
                    let _ = InvalidateRect(Some(self.hwnd), None, false);
                }
                return;
            }
            self.section = section;
            let mut client = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut client);
            // A new section starts at its top. The paper under the previous
            // section has nothing to do with this one, and a page that opened
            // halfway down would hide the very band the nav just asked for.
            self.page_scroll = 0.0;
            self.layout = measure_tuned(
                section,
                client.right,
                client.bottom,
                self.scale,
                self.lang_tags.len(),
                self.page_scroll,
            );
            let placed = self.layout;
            self.place_children(&placed);
            self.show_only(section);
            if section == SEC_DIAL {
                // Entering the page is the moment to photograph the dial afresh:
                // windows have come and gone since the last visit. The short lift
                // beside the picture is read at the same moment so it lists the
                // same windows the panel would.
                self.preview = None;
                self.refresh_dial_pool();
                self.log_dial_layout();
            }
            for (at, button) in self.nav_buttons.iter().enumerate() {
                self.press(*button, at == section);
            }
            // The focus was on a row that has just gone away. Left to itself,
            // Windows looks for a window that can take it and does not stay
            // inside this one, so the name that was clicked keeps it.
            let gone = GetFocus();
            let fallback_focus = self.nav_buttons[section];
            if !self.stops().contains(&gone) {
                let _ = SetFocus(Some(fallback_focus));
            }
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            // Different sections are different sentences, and a word that does
            // not fit in the one this click brought up is a line in the log.
            warn_cut(self);
        }
    }

    unsafe fn child(
        &self,
        parent: HWND,
        class: PCWSTR,
        rect: RECT,
        id: i32,
        style: WINDOW_STYLE,
    ) -> HWND {
        unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR::null(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP | style,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                Some(parent),
                Some(HMENU(id as usize as *mut c_void)),
                None,
                None,
            )
            .unwrap_or_default()
        }
    }

    unsafe fn switch(
        &mut self,
        parent: HWND,
        rect: RECT,
        id: i32,
        on: bool,
        reduced: bool,
    ) -> HWND {
        unsafe {
            let hwnd = self.child(parent, SWITCH_CLASS, rect, id, WINDOW_STYLE(0));
            let ctrl = Box::into_raw(Box::new(Switch {
                hwnd,
                state: self as *mut State,
                on,
                pressed: false,
                hover: false,
                focused: false,
                anim: if on { 1.0 } else { 0.0 },
                reduced,
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctrl as isize);
            hwnd
        }
    }

    unsafe fn key(&mut self, parent: HWND, rect: RECT, id: i32, mods: u32, vk: u32) -> HWND {
        unsafe {
            let hwnd = self.child(parent, KEY_CLASS, rect, id, WINDOW_STYLE(0));
            let ctrl = Box::into_raw(Box::new(Key {
                hwnd,
                state: self as *mut State,
                mods,
                vk,
                button: 0,
                kind: KeyKind::Hotkey,
                capturing: false,
                pending: 0,
                focused: false,
                await_release: false,
                before: (mods, vk),
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctrl as isize);
            hwnd
        }
    }

    /// The trigger recorder. It is the same box as a shortcut field, but it
    /// records whichever comes first - a key or a mouse button - so a binding
    /// can be `F6`, `Ctrl+Shift+Left` or `Alt+Middle`.
    unsafe fn key_trigger(
        &mut self,
        parent: HWND,
        rect: RECT,
        id: i32,
        mods: u32,
        vk: u32,
        button: u8,
    ) -> HWND {
        unsafe {
            let hwnd = self.child(parent, KEY_CLASS, rect, id, WINDOW_STYLE(0));
            let ctrl = Box::into_raw(Box::new(Key {
                hwnd,
                state: self as *mut State,
                mods,
                vk,
                button,
                kind: KeyKind::Trigger,
                capturing: false,
                pending: 0,
                focused: false,
                await_release: false,
                before: (mods, vk),
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctrl as isize);
            hwnd
        }
    }

    unsafe fn button(
        &mut self,
        parent: HWND,
        text: &str,
        id: i32,
        rect: RECT,
        primary: bool,
        behind: (u8, u8, u8),
    ) -> HWND {
        unsafe {
            let hwnd = self.child(parent, BUTTON_CLASS, rect, id, WINDOW_STYLE(0));
            // The word is painted by hand, but it is also the window text: a
            // caption nobody can read out of the window is a control a screen
            // reader - or a test - has to guess at.
            let wide = wstr(text);
            let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
            let ctrl = Box::into_raw(Box::new(Button {
                hwnd,
                parent,
                state: self as *mut State,
                id,
                text: text.to_string(),
                primary,
                pressed: false,
                focused: false,
                enabled: true,
                behind,
            }));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, ctrl as isize);
            hwnd
        }
    }

    // --- painting the panel -----------------------------------------------------

    /// The paper: a white ground, the column of names the children draw for
    /// themselves, and the cards of the section that is turned to. Every word in
    /// a card comes from [`CARDS`], so the painter and the cutter of
    /// [`warn_cut`] read the same inventory and neither can be told a lie about
    /// what is on screen.
    unsafe fn paint(&mut self, hwnd: HWND) {
        unsafe {
            let mut ps = std::mem::zeroed();
            let screen = BeginPaint(hwnd, &mut ps);
            if screen.is_invalid() {
                return;
            }
            let started = Instant::now();
            // What this paint was asked to cover. The steppers invalidate one
            // number band at a time; a full-client answer here would mean the
            // click reached something that repainted the page.
            let asked_w = ps.rcPaint.right - ps.rcPaint.left;
            let asked_h = ps.rcPaint.bottom - ps.rcPaint.top;
            let asked = ps.rcPaint;
            if asked_w <= 0 || asked_h <= 0 {
                let _ = EndPaint(hwnd, &ps);
                return;
            }
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            // The page is composed in an off-screen bitmap and reaches the
            // window in one blit of the update rectangle at the bottom. The
            // surface never shows paper with the rest of the page still being
            // drawn on it, which is what a click or a drag frame used to catch.
            let offscreen = self.page.ensure(client.right, client.bottom);
            let hdc = if offscreen { self.page.mem } else { screen };
            // The page is only copied back over the rectangle this paint was
            // asked for, so nothing outside it can change; clipping the
            // off-screen DC to the same rectangle keeps the painter from
            // composing cards, rows and thumbnails nobody is about to see.
            if offscreen {
                let _ = IntersectClipRect(hdc, asked.left, asked.top, asked.right, asked.bottom);
            }
            let l = self.layout;
            // The frame is built before the page is cleared. Rebuilding takes
            // a couple of dozen milliseconds on a busy desk, and a page that
            // has already put white paper under the preview when that starts
            // shows a white blink in the one spot the click was about. Building
            // first leaves yesterday's pixels in place until the new ones can
            // be blitted in the same paint.
            let rebuilt = if l.section == SEC_DIAL {
                let fresh = self.ensure_preview(&l);
                // The filmstrip is rebuilt after the picture, from the same desk
                // copy, so the six thumbnails and the big picture agree about
                // which windows are on the dial.
                let thumbs = self.ensure_theme_thumbs(&l);
                fresh || thumbs
            } else {
                false
            };
            let ground = solid(FIELD);
            // The picture's own cell is never papered. The card is composed off
            // screen and lands in a single blit, so there is no moment in which
            // the preview is bare paper - which is the white blink a drag used
            // to leave behind.
            let cell = if l.section == SEC_DIAL {
                self.preview.as_ref().map(|p| RECT {
                    left: p.x,
                    top: p.y,
                    right: p.x + p.side as i32,
                    bottom: p.y + p.side as i32,
                })
            } else {
                None
            };
            if let Some(cell) = cell {
                for band in outside(client, cell) {
                    FillRect(hdc, &band, ground);
                }
            } else {
                FillRect(hdc, &client, ground);
            }
            // The page is papered whole only when there is no picture cell.
            if l.section == SEC_DIAL {
                // Whatever is under a new frame has already been built above,
                // so the paper goes down and the picture lands in the same pass.
                let ready = started.elapsed().as_secs_f32() * 1000.0;
                crate::log!("paint dial: cleared after {ready:.2}ms rebuilt={rebuilt}");
            }

            if l.section == SEC_DIAL {
                self.paint_dial(hdc, &l);
            } else {
                let cards = cards_of(l.section);
                for (at, item) in cards.iter().enumerate().take(l.row_count) {
                    self.card(hdc, l.card[at]);
                    self.text(
                        hdc,
                        t(item.title),
                        F_BUTTON,
                        INK,
                        l.title[at],
                        DT_TOP | DT_LEFT,
                    );
                    let mut note = self.note_text(l.section, at);
                    let mut colour = MUTED;
                    // The recorder's own card carries the sentence about the
                    // chord it just refused, because the 校验 row that used to
                    // repeat it in the abstract is gone.
                    if l.section == SEC_KEYS && at == 0 {
                        if let Some((text, warn)) = self.trigger_notice_text() {
                            note = text.to_string();
                            if warn {
                                colour = CONFLICT;
                            }
                        }
                    }
                    self.text(hdc, &note, F_NOTE, colour, l.note[at], DT_TOP | DT_LEFT);
                }
            }

            if l.section == SEC_DIAL {
                // A paint that does not touch the picture's own cell - the note
                // band, a stepper - leaves it alone: the pixels under it did not
                // change, and composing them again would only cost the frame its
                // time.
                let touched = self.preview.as_ref().is_some_and(|p| {
                    asked.right > p.x
                        && asked.left < p.x + p.side as i32
                        && asked.bottom > p.y
                        && asked.top < p.y + p.side as i32
                });
                if touched {
                    self.paint_preview(hdc, asked);
                }
            }

            // The numbers and names the panel owns - painted here rather than by a
            // control, so there is one thing to keep in step with the draft.
            match l.section {
                SEC_DIAL => {
                    self.text(
                        hdc,
                        &format!("{} px", self.wheel_radius.round()),
                        F_TITLE,
                        INK,
                        l.dial_values[0],
                        DT_VCENTER | DT_CENTER,
                    );
                    self.text(
                        hdc,
                        &format!("{:.0} px", self.wheel_seam),
                        F_TITLE,
                        INK,
                        l.dial_values[1],
                        DT_VCENTER | DT_CENTER,
                    );
                    self.text(
                        hdc,
                        &format!("{}", self.wheel_limit),
                        F_TITLE,
                        INK,
                        l.dial_values[2],
                        DT_VCENTER | DT_CENTER,
                    );
                    // The search field is a system EDIT so typing and IME come
                    // free; the frame and the magnifier are drawn here, around
                    // and beside the child, so the field reads as a box on this
                    // paper instead of a line of text that happens to sit above
                    // the list. The border brightens while it has the focus.
                    let focused = self.search_focused;
                    self.search_frame(hdc, l.dial_search, focused);
                    self.refresh_button(hdc, l.dial_refresh, self.refresh_hot);
                    // Every window the panel knows, not a sample, cut into the
                    // two sections the headings name. The dots say on/off row by
                    // row; the headings and the rule between them say it once,
                    // and both headings are drawn even when a section is empty.
                    let icon = (16.0 * self.scale).round().max(1.0) as i32;
                    for entry in 0..self.pool_view.len() {
                        let Some(rect) = self.pool_entry_rect(entry) else {
                            continue;
                        };
                        match self.pool_view[entry] {
                            PoolEntry::Group { on, count } => {
                                if !on {
                                    // The rule belongs to the second heading: one
                                    // hairline across the column, so the two
                                    // halves read as two lists and not one long
                                    // one that happens to be sorted.
                                    let line = RECT {
                                        left: rect.left,
                                        top: rect.top,
                                        right: rect.right,
                                        bottom: rect.top + 1,
                                    };
                                    let edge = solid(EDGE);
                                    let _ = FillRect(hdc, &line, edge);
                                }
                                let label = t(if on {
                                    "settings_dial_group_on"
                                } else {
                                    "settings_dial_group_off"
                                });
                                // The rows carry no button any more, so the
                                // gesture is the only way between the two
                                // halves; the line beside each heading says it
                                // once, small enough not to cost a row.
                                let hint = t("settings_dial_double_hint");
                                let hint_w = self.text_width(hdc, hint, F_NOTE);
                                let air = (8.0 * self.scale).round() as i32;
                                self.text(
                                    hdc,
                                    &format!("{label} \u{00b7} {count}"),
                                    F_NOTE,
                                    INK,
                                    RECT {
                                        left: rect.left,
                                        top: rect.top + (4.0 * self.scale) as i32,
                                        right: (rect.right - hint_w - air).max(rect.left),
                                        bottom: rect.bottom,
                                    },
                                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                                );
                                self.text(
                                    hdc,
                                    hint,
                                    F_NOTE,
                                    MUTED,
                                    RECT {
                                        left: (rect.right - hint_w).max(rect.left),
                                        top: rect.top + (4.0 * self.scale) as i32,
                                        right: rect.right,
                                        bottom: rect.bottom,
                                    },
                                    DT_RIGHT | DT_VCENTER | DT_SINGLELINE,
                                );
                            }
                            PoolEntry::Row { at } => {
                                let Some(row) = self.dial_pool.get(at) else {
                                    continue;
                                };
                                let head = if !row.tag.is_empty() {
                                    &row.tag
                                } else if !row.title.is_empty() {
                                    &row.title
                                } else {
                                    &row.app
                                };
                                let colour = if row.on_dial { INK } else { MUTED };
                                let mark = if row.on_dial { "\u{25cf}" } else { "\u{25cb}" };
                                self.text(
                                    hdc,
                                    mark,
                                    F_NOTE,
                                    colour,
                                    RECT {
                                        left: rect.left,
                                        top: rect.top,
                                        right: rect.left + (14.0 * self.scale) as i32,
                                        bottom: rect.bottom,
                                    },
                                    DT_LEFT | DT_VCENTER | DT_SINGLELINE,
                                );
                                let _ = DrawIconEx(
                                    hdc,
                                    rect.left + (16.0 * self.scale) as i32,
                                    rect.top + (rect.bottom - rect.top - icon) / 2,
                                    row.icon,
                                    icon,
                                    icon,
                                    0,
                                    None,
                                    DI_NORMAL,
                                );
                                let name_left =
                                    rect.left + (22.0 * self.scale) as i32 + icon;
                                self.text(
                                    hdc,
                                    head,
                                    F_NOTE,
                                    colour,
                                    RECT {
                                        left: name_left,
                                        top: rect.top,
                                        right: rect.right,
                                        bottom: rect.bottom,
                                    },
                                    DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                                );
                            }
                        }
                    }
                    // Both headings are still drawn when a search matches
                    // nothing, so the two sections never vanish; this line says
                    // why they are both empty, instead of leaving the paper
                    // below them blank.
                    if self.dial_pool.is_empty() && !self.pool_query.trim().is_empty() {
                        let step = (POOL_ROW_STEP * self.scale).round().max(1.0) as i32;
                        let hint = RECT {
                            left: l.dial_list.left,
                            top: l.dial_list.top + 2 * step,
                            right: l.dial_list.right,
                            bottom: l.dial_list.bottom,
                        };
                        self.text(
                            hdc,
                            t("settings_dial_no_match"),
                            F_NOTE,
                            MUTED,
                            hint,
                            DT_CENTER | DT_TOP | DT_SINGLELINE | DT_END_ELLIPSIS,
                        );
                    }
                }
                SEC_ABOUT => {
                    self.text(
                        hdc,
                        env!("CARGO_PKG_VERSION"),
                        F_TITLE,
                        INK,
                        l.value_version,
                        DT_VCENTER | DT_RIGHT,
                    );
                    self.text(
                        hdc,
                        LICENSE_SHOWN,
                        F_TITLE,
                        INK,
                        l.value_license,
                        DT_VCENTER | DT_RIGHT,
                    );
                }
                _ => {}
            }

            // One blit, only the rectangle this paint was asked to cover. The
            // window surface goes from yesterday's frame to the finished one,
            // with no paper in between.
            if offscreen {
                let _ = BitBlt(
                    screen, asked.left, asked.top, asked_w, asked_h, Some(hdc), asked.left,
                    asked.top, SRCCOPY,
                );
                // The page outlives this paint, so the clip must not: the next
                // full repaint would otherwise be cut to yesterday's rectangle.
                let _ = SelectClipRgn(hdc, None);
            }
            let _ = DeleteObject(ground.into());
            let _ = EndPaint(hwnd, &ps);
            if l.section == SEC_DIAL {
                // Evidence for the "no white flash" promise: a stepper click
                // asks for one band, the picture is ready before the paper is
                // cleared, and the whole paint stays well under a frame.
                let taken = started.elapsed().as_secs_f32() * 1000.0;
                crate::log!(
                    "paint dial: asked={asked_w}x{asked_h}@{},{} client={}x{} rebuilt={rebuilt} took={taken:.2}ms",
                    asked.left,
                    asked.top,
                    client.right,
                    client.bottom
                );
                // A thumbnail click is timed from the press that chose it to the
                // blit that put the new look on screen: the interval the eye
                // actually waits through.
                if let Some((at, theme)) = self.theme_switch_at.take() {
                    let ms = at.elapsed().as_secs_f32() * 1000.0;
                    crate::log!("dial theme: {theme} took={ms:.2}ms");
                }
            }
        }
    }

    /// The search field's frame: a white box on the card's paper, a hairline
    /// border that warms to the accent while the field has the focus, and the
    /// magnifier in the strip the edit control leaves at its left. The box is
    /// what makes the field readable as an input at a glance - the card alone
    /// gave it the same paper as the page behind it.
    unsafe fn search_frame(&self, hdc: HDC, r: RECT, focused: bool) {
        unsafe {
            let brush = solid(FIELD);
            let border = if focused { FLAX } else { EDGE };
            let pen = CreatePen(PS_SOLID, 1, rgb(border));
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, pen.into());
            let radius = (RADIUS * 1.4 * self.scale) as i32;
            let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
            // The magnifier: a lens and a handle, drawn in the strip before the
            // child so it never has to be re-painted by the edit control.
            let ink = if focused { FLAX } else { MUTED };
            let weight = (self.scale.round() as i32).max(1);
            let glyph = CreatePen(PS_SOLID, weight, rgb(ink));
            let old_glyph = SelectObject(hdc, glyph.into());
            let icon = (DIAL_SEARCH_ICON * self.scale) as i32;
            let cx = r.left + icon / 2;
            let cy = (r.top + r.bottom) / 2 - self.scale.round() as i32;
            let rad = (5.0 * self.scale).round() as i32;
            let _ = Ellipse(hdc, cx - rad, cy - rad, cx + rad, cy + rad);
            let corner = (3.6 * self.scale).round() as i32;
            let _ = MoveToEx(hdc, cx + corner, cy + corner, None);
            let tip = (4.0 * self.scale).round() as i32;
            let _ = LineTo(hdc, cx + corner + tip, cy + corner + tip);
            let _ = SelectObject(hdc, old_glyph);
            let _ = DeleteObject(glyph.into());
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());
        }
    }

    /// The list's refresh button: a square the height of the search field, with
    /// a circular arrow on it. Pressing it re-reads the running windows, so a
    /// window opened while the page was up shows up without a close-and-reopen.
    /// It brightens under the pointer the way the search frame does under focus.
    unsafe fn refresh_button(&self, hdc: HDC, r: RECT, hot: bool) {
        unsafe {
            if r.right <= r.left || r.bottom <= r.top {
                return;
            }
            let brush = solid(FIELD);
            let border = if hot { FLAX } else { EDGE };
            let pen = CreatePen(PS_SOLID, 1, rgb(border));
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, pen.into());
            let radius = (RADIUS * 1.4 * self.scale) as i32;
            let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
            // The arrow: a three-quarter circle drawn as a polyline, with a
            // filled head at the end pointing the way the circle travels.
            let ink = if hot { FLAX } else { MUTED };
            let weight = (self.scale.round() as i32).max(1);
            let glyph = CreatePen(PS_SOLID, weight, rgb(ink));
            let head_brush = solid(ink);
            let old_glyph = SelectObject(hdc, glyph.into());
            let cx = (r.left + r.right) as f32 / 2.0;
            let cy = (r.top + r.bottom) as f32 / 2.0;
            let rad = 6.5 * self.scale;
            let start = -60.0_f32.to_radians();
            let sweep = 300.0_f32.to_radians();
            let steps = 28;
            let mut arc = Vec::with_capacity(steps + 1);
            for i in 0..=steps {
                let ang = start + sweep * i as f32 / steps as f32;
                arc.push(POINT {
                    x: (cx + rad * ang.cos()).round() as i32,
                    y: (cy + rad * ang.sin()).round() as i32,
                });
            }
            let _ = Polyline(hdc, &arc);
            // The head sits at the arc's end and aims along the tangent.
            let end = start + sweep;
            let tip = POINT {
                x: (cx + (rad + 3.0 * self.scale) * end.cos()).round() as i32,
                y: (cy + (rad + 3.0 * self.scale) * end.sin()).round() as i32,
            };
            let base = POINT {
                x: (cx + (rad - 3.0 * self.scale) * end.cos()).round() as i32,
                y: (cy + (rad - 3.0 * self.scale) * end.sin()).round() as i32,
            };
            let side = 3.0 * self.scale;
            let left = POINT {
                x: (base.x as f32 - side * end.sin()).round() as i32,
                y: (base.y as f32 + side * end.cos()).round() as i32,
            };
            let right = POINT {
                x: (base.x as f32 + side * end.sin()).round() as i32,
                y: (base.y as f32 - side * end.cos()).round() as i32,
            };
            let old_head = SelectObject(hdc, head_brush.into());
            let _ = Polygon(hdc, &[tip, left, right]);
            let _ = SelectObject(hdc, old_head);
            let _ = SelectObject(hdc, old_glyph);
            let _ = DeleteObject(glyph.into());
            let _ = DeleteObject(head_brush.into());
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());
        }
    }

    /// One card: the warm paper the panel used to be printed on, lifted off a
    /// ground that has taken the colour the controls used to be. The border is
    /// the softest line in the window, so the layering reads from the inside
    /// out - a control's `EDGE`, then its card, then the ground - which is the
    /// old page's sentence with the two halves swapped.
    unsafe fn card(&self, hdc: HDC, r: RECT) {
        unsafe {
            let brush = solid(PAPER);
            let pen = CreatePen(PS_SOLID, 1, rgb(RULE_COLOUR));
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, pen.into());
            let radius = (RADIUS * 2.0 * self.scale) as i32;
            let _ = RoundRect(hdc, r.left, r.top, r.right, r.bottom, radius, radius);
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());
        }
    }

    /// The dial page's own painting: one picture card on the left, the list card
    /// on the right, and the compact strip of parameters under the list. The
    /// strip is one surface with six short names down its left edge; its cells
    /// and steppers are children and say their own words.
    unsafe fn paint_dial(&self, hdc: HDC, l: &Layout) {
        unsafe {
            self.card(hdc, l.card[DIAL_PREVIEW_ROW]);
            self.text(
                hdc,
                t(cards_of(SEC_DIAL)[DIAL_PREVIEW_ROW].title),
                F_BUTTON,
                INK,
                l.title[DIAL_PREVIEW_ROW],
                DT_TOP | DT_LEFT,
            );
            let note = self.note_text(SEC_DIAL, DIAL_PREVIEW_ROW);
            self.text(
                hdc,
                &note,
                F_NOTE,
                MUTED,
                l.note[DIAL_PREVIEW_ROW],
                DT_TOP | DT_LEFT,
            );
            self.card(hdc, l.dial_list_card);
            self.card(hdc, l.dial_param_card);
            self.paint_theme_strip(hdc, l);
            for at in 1..cards_of(SEC_DIAL).len().min(l.row_count) {
                self.text(
                    hdc,
                    t(cards_of(SEC_DIAL)[at].title),
                    F_NOTE,
                    MUTED,
                    l.title[at],
                    DT_LEFT | DT_VCENTER,
                );
            }
        }
    }

    /// The filmstrip: six cards under the picture, each carrying the look it
    /// would give the dial. The selected one takes an ink ring and its name in
    /// ink; the others keep the card's soft edge, so the row reads as six frames
    /// of the same film rather than six buttons.
    unsafe fn paint_theme_strip(&self, hdc: HDC, l: &Layout) {
        unsafe {
            let Some(thumbs) = self.theme_thumbs.as_ref() else {
                return;
            };
            let side = thumbs.side as i32;
            for (i, cell) in l.dial_themes.iter().take(DIAL_THEMES).enumerate() {
                let cell = *cell;
                if cell.right <= cell.left || cell.bottom <= cell.top {
                    continue;
                }
                self.card(hdc, cell);
                if let Some(pixmap) = thumbs.pixmaps[i].as_ref() {
                    let tx = cell.left + (cell.right - cell.left - side) / 2;
                    let ty = cell.top + ((2.0 * self.scale).round() as i32);
                    blit(
                        hdc,
                        tx,
                        ty,
                        pixmap.data(),
                        pixmap.width(),
                        pixmap.height(),
                    );
                }
                let selected = self.wheel_theme as usize == i;
                if selected {
                    let pen = CreatePen(
                        PS_SOLID,
                        (2.0 * self.scale).round().max(1.0) as i32,
                        rgb(INK),
                    );
                    let old_pen = SelectObject(hdc, pen.into());
                    let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
                    let radius = (RADIUS * 1.6 * self.scale) as i32;
                    let _ = RoundRect(
                        hdc,
                        cell.left,
                        cell.top,
                        cell.right,
                        cell.bottom,
                        radius,
                        radius,
                    );
                    let _ = SelectObject(hdc, old_pen);
                    let _ = SelectObject(hdc, old_brush);
                    let _ = DeleteObject(pen.into());
                }
                let mut name = RECT {
                    left: cell.left + (4.0 * self.scale).round() as i32,
                    right: cell.right - (4.0 * self.scale).round() as i32,
                    top: cell.top + side + (4.0 * self.scale).round() as i32,
                    bottom: cell.bottom - (2.0 * self.scale).round() as i32,
                };
                if name.bottom <= name.top {
                    name.top = (cell.bottom - (14.0 * self.scale) as i32).max(cell.top);
                    name.bottom = cell.bottom;
                }
                self.text(
                    hdc,
                    t(DIAL_THEME_NAME[i]),
                    F_NOTE,
                    if selected { INK } else { MUTED },
                    name,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS,
                );
            }
        }
    }

    /// How wide a word is in one of the panel's faces, on the DC it will be drawn
    /// on. Two words share a line in more than one place, and the only way the
    /// first is not cut by the second is to ask the second what it costs.
    unsafe fn text_width(&self, hdc: HDC, text: &str, font: usize) -> i32 {
        unsafe {
            let wide: Vec<u16> = text.encode_utf16().collect();
            let old = SelectObject(hdc, HGDIOBJ(self.fonts[font].0));
            let mut size = SIZE::default();
            let measured = GetTextExtentPoint32W(hdc, &wide, &mut size).as_bool();
            SelectObject(hdc, old);
            if measured {
                size.cx
            } else {
                0
            }
        }
    }

    /// The second line of a card. One card has something other than a sentence
    /// there: the log's is a path, because a file you are pointed at is named
    /// rather than described, and the door under the row is how you get to it.
    fn note_text(&self, section: usize, at: usize) -> String {
        let card = cards_of(section)[at];
        if section == SEC_DIAL && at == DIAL_PREVIEW_ROW {
            if let Some((line, since)) = self.dial_toast.as_ref() {
                if since.elapsed().as_millis() < DIAL_TOAST_MS as u128 {
                    return line.clone();
                }
            }
            // The card's own sentence. There are no tuning rows under the search
            // field any more, so nothing has to be explained about arrows.
            return t(card.note).to_string();
        }
        if section == SEC_DIAL && at == DIAL_LIMIT_ROW {
            let mut note = t(card.note).to_string();
            if self.wheel_limit >= settings::WHEEL_LIMIT_MAX {
                let extra = t("settings_dial_limit_ceiling")
                    .replace("{n}", &settings::WHEEL_LIMIT_MAX.to_string());
                note = format!("{note} · {extra}");
            }
            return note;
        }
        if !card.note.is_empty() {
            return t(card.note).to_string();
        }
        // The log card writes its own path into the note band; a card with no
        // sentence - the licence, whose value says everything - writes nothing.
        if section == SEC_ABOUT && at == 1 {
            log_path_shown()
        } else {
            String::new()
        }
    }

    /// The warning the second card shows after a recording. A risky binding
    /// still saves; the note is amber so the reason a press does nothing
    /// elsewhere is on the page before it is discovered by trial.
    fn trigger_notice_text(&self) -> Option<(&'static str, bool)> {
        match self.trigger_notice? {
            settings::TriggerProblem::Invalid => Some((t("settings_trigger_invalid"), true)),
            settings::TriggerProblem::Conflict => Some((t("settings_trigger_conflict"), true)),
            settings::TriggerProblem::Risky => Some((t("settings_trigger_risky"), true)),
        }
    }

    unsafe fn text(
        &self,
        hdc: HDC,
        text: &str,
        font: usize,
        colour: (u8, u8, u8),
        rect: RECT,
        align: DRAW_TEXT_FORMAT,
    ) {
        unsafe {
            if text.is_empty() || rect.right <= rect.left {
                return;
            }
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            let old_font = SelectObject(hdc, HGDIOBJ(self.fonts[font].0));
            let _ = SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, rgb(colour));
            let mut r = rect;
            DrawTextW(
                hdc,
                &mut wide,
                &mut r,
                align | DT_NOPREFIX | DT_END_ELLIPSIS | DT_SINGLELINE,
            );
            let _ = SelectObject(hdc, old_font);
        }
    }

    // --- results ----------------------------------------------------------------

    fn checked(&self, hwnd: HWND) -> bool {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Switch;
            !ptr.is_null() && (*ptr).on
        }
    }

    /// The combination a shortcut box is holding, whether or not the user ever
    /// clicked it. A box still listening when Save is pressed has not chosen
    /// anything, so it reports the value it came in with.
    fn key_mods_vk(&self, hwnd: HWND) -> (u32, u32) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Key;
            if ptr.is_null() {
                return (0, 0);
            }
            ((*ptr).mods, (*ptr).vk)
        }
    }

    /// Every control that takes focus, in the order they are read down the
    /// section that is on screen. The names in the left column come first because
    /// they were created first, and a section's rows follow in theirs; a control
    /// belonging to a section nobody is looking at is not a stop, which is what
    /// keeps Tab inside the paper rather than sending it hunting through the
    /// hidden half of the window. The same list is what a relanguage repaints.
    fn stops(&self) -> Vec<HWND> {
        let mut stops: Vec<HWND> = self.nav_buttons.to_vec();
        stops.extend(
            self.slots
                .iter()
                .filter_map(|(at, hwnd)| (*at == self.section).then_some(*hwnd)),
        );
        stops.retain(|hwnd| !hwnd.is_invalid() && unsafe { IsWindowVisible(*hwnd).as_bool() });
        stops
    }

    /// Tab and Shift-Tab out of a shortcut box. While a box is listening it takes
    /// every key, so the dialog manager never gets the Tab that ends the listening.
    fn focus_stop(&self, current: HWND, backward: bool) {
        let stops = self.stops();
        let Some(at) = stops.iter().position(|stop| *stop == current) else {
            return;
        };
        let next = if backward {
            stops[(at + stops.len() - 1) % stops.len()]
        } else {
            stops[(at + 1) % stops.len()]
        };
        if !next.is_invalid() {
            unsafe {
                let _ = SetFocus(Some(next));
            }
        }
    }

    /// Move the pressed look onto whichever cells the draft says are on. The
    /// language row does the same thing for the same reason: the click answers
    /// the question, and the look follows the answer rather than the other way.
    unsafe fn press(&self, hwnd: HWND, on: bool) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Button;
            if !ptr.is_null() {
                (*ptr).primary = on;
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
    }

    /// A recording came back from the field. A conflict or an unusable chord is
    /// put back to the binding in force and explained on the page; a risky one
    /// saves but leaves the note amber, because the user asked for it.
    unsafe fn try_trigger(&mut self, mods: u32, vk: u32, button: u8) {
        let current = (*self.app).applied_settings();
        let problem = settings::trigger_problem(mods, vk, button, &current);
        self.trigger_notice = problem;
        let accepted = matches!(problem, None | Some(settings::TriggerProblem::Risky));
        if accepted {
            self.trigger_mods = mods;
            self.trigger_vk = vk;
            self.trigger_button = button;
        } else {
            self.trigger_mods = current.trigger_mods;
            self.trigger_vk = current.trigger_vk;
            self.trigger_button = current.trigger_button;
        }
        self.refresh_trigger();
        self.moved();
        // The second card's note is painted by the panel, not by the field.
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// Put the draft's binding into the field.
    unsafe fn refresh_trigger(&self) {
        self.set_trigger(
            self.trigger_field,
            self.trigger_mods,
            self.trigger_vk,
            self.trigger_button,
        );
    }

    unsafe fn set_trigger(&self, hwnd: HWND, mods: u32, vk: u32, button: u8) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Key;
            if !ptr.is_null() {
                (*ptr).set_trigger_combo(mods, vk, button);
            }
        }
    }

    /// A cell in 轮盘: one of the three accent loudnesses, an animation length,
    /// or the order the limited list arrives in.
    unsafe fn choose_dial(&mut self, group: usize, at: usize) {
        let seats_changed = group == 2;
        match group {
            0 => self.wheel_accent = at as u32,
            1 => self.wheel_open_ms = DIAL_ANIM_MS[at],
            2 => self.dial_order = at as u32,
            _ => self.activate_pos = at as u32,
        }
        for (index, cell) in self.dial_cells.iter().enumerate() {
            let selected = if index < DIAL_ACCENT_CELLS {
                self.wheel_accent == index as u32
            } else if index < DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS {
                self.wheel_open_ms == DIAL_ANIM_MS[index - DIAL_ACCENT_CELLS]
            } else if index < DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + DIAL_ORDER_CELLS {
                self.dial_order == (index - DIAL_ACCENT_CELLS - DIAL_ANIM_CELLS) as u32
            } else {
                self.activate_pos
                    == (index - DIAL_ACCENT_CELLS - DIAL_ANIM_CELLS - DIAL_ORDER_CELLS) as u32
            };
            self.press(*cell, selected);
        }
        self.preview = None;
        // Only the accent stays in the picture's key, but a new loudness means a
        // new picture and a new filmstrip too.
        if group == 0 {
            self.theme_thumbs = None;
            let strip = self.layout.dial_strip;
            let _ = InvalidateRect(Some(self.hwnd), Some(&strip), false);
        }
        if seats_changed {
            self.sync_pool_seats();
            let _ = InvalidateRect(Some(self.hwnd), Some(&self.layout.dial_list), false);
        }
        unsafe { self.moved() };
    }

    /// The filmstrip's click. A look is chosen the moment the button goes down -
    /// there is nothing to confirm and no second step - and only the picture and
    /// the strip are invalidated: the page's paper is already composed off screen
    /// and not a pixel of it is cleared to white.
    unsafe fn choose_theme(&mut self, id: u32) {
        if self.wheel_theme == id {
            return;
        }
        self.wheel_theme = id;
        self.theme_switch_at = Some((Instant::now(), id));
        self.preview = None;
        // The thumbnails themselves do not change with the theme: each one is a
        // different look already, and only the ring around the chosen cell moves.
        // The ring's pen straddles the cell edge, so grow the strip by one pen
        // width or the old ring's outer pixel would be left on the page.
        let mut strip = self.layout.dial_strip;
        let pen = (2.0 * self.scale).round().max(1.0) as i32;
        strip = RECT {
            left: strip.left - pen,
            top: strip.top - pen,
            right: strip.right + pen,
            bottom: strip.bottom + pen,
        };
        let preview = self.layout.dial_preview;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), Some(&strip), false);
            let _ = InvalidateRect(Some(self.hwnd), Some(&preview), false);
            self.moved();
        }
    }

    /// The two steppers: dial radius, then the air between sectors. Both change
    /// what the picture beside them holds, so the picture is thrown away and made
    /// again on the next paint.
    unsafe fn step_dial(&mut self, at: usize) {
        let mut seats_changed = false;
        unsafe {
            let l = self.layout;
            match at {
                0 | 1 => {
                    let d = if at == 0 { -RADIUS_STEP } else { RADIUS_STEP };
                    self.wheel_radius = (self.wheel_radius + d).clamp(RADIUS_MIN, RADIUS_MAX);
                    let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_values[0]), false);
                }
                _ => {
                    // Every pair is minus first, plus second - the same order
                    // the two buttons are laid out in.
                    let d: i64 = if at % 2 == 0 { -1 } else { 1 };
                    if at >= 4 {
                        self.wheel_limit = (self.wheel_limit as i64 + d).clamp(
                            settings::WHEEL_LIMIT_MIN as i64,
                            settings::WHEEL_LIMIT_MAX as i64,
                        ) as u32;
                        seats_changed = true;
                        let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_values[2]), false);
                        // The sentence explains what the number means for the
                        // windows the user bound by hand, so it has to be as new
                        // as the number itself.
                        let _ = InvalidateRect(
                            Some(self.hwnd),
                            Some(&l.note[DIAL_LIMIT_ROW]),
                            false,
                        );
                    } else {
                        self.wheel_seam =
                            (self.wheel_seam + d as f32).clamp(0.0, settings::WHEEL_SEAM_MAX);
                        let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_values[1]), false);
                    }
                }
            }
            self.preview = None;
            // Only the picture and the band this arrow owns are new. The card
            // titles and the static rows do not depend on this number. The list
            // of windows does: its rows say "on the dial" or not, and the number
            // moves windows across that line - so that column is redrawn when
            // the seats changed, from the copy already in hand.
            let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_preview), false);
            if seats_changed {
                self.sync_pool_seats();
                let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_list), false);
            }
            self.refresh_steppers();
            self.moved();
        }
    }

    /// Say what just happened to a window in the note band over the picture,
    /// and start the clock that takes the line back.
    unsafe fn say(&mut self, key: &str, name: &str) {
        let limit = (*self.app).applied_settings().wheel_limit.to_string();
        let line = t(key)
            .replace("{name}", name)
            .replace("{limit}", &limit);
        self.dial_toast = Some((line, Instant::now()));
        let _ = SetTimer(Some(self.hwnd), DIAL_TOAST_TIMER, DIAL_TOAST_MS, None);
        let rect = self.layout.note[DIAL_PREVIEW_ROW];
        let _ = InvalidateRect(Some(self.hwnd), Some(&rect), false);
    }

    /// The double-click's two answers: a window that holds a sector leaves the
    /// dial, one that does not asks the dial for a seat. A seat is a request, not
    /// a promise - the application may be struck off or the window may have
    /// closed - so the answer is read back from the same seat list the rows were
    /// built from, and a refusal puts the row back where it came from and says
    /// why.
    unsafe fn toggle_pool_row(&mut self, at: usize) {
        let Some(row) = self.dial_pool.get(at) else {
            return;
        };
        if row.on_dial {
            self.act_on_row(at);
            return;
        }
        let fingerprint = row.fingerprint.clone();
        let hwnd = row.hwnd;
        let excluded = row.excluded;
        let exe = row.exe.clone();
        let was_off = row.off_dial;
        let was_parked = (*self.app).applied_settings().is_parked(&fingerprint);
        let name = if row.tag.trim().is_empty() {
            row.title.clone()
        } else {
            row.tag.clone()
        };
        if hwnd == 0 {
            self.say("settings_dial_seat_not_running", &name);
            crate::log!("dial seat: fp={fingerprint} refused reason=not-running");
            return;
        }
        if excluded {
            // Double-clicking a row is the user asking for this window in so
            // many words. An application-wide exclusion is a stronger "never
            // show this" that used to win here and answer with a refusal the
            // user did not ask for; the explicit gesture now lifts it for the
            // one application whose row was clicked and then seats the window.
            (*self.app).set_excluded(&exe, false);
            crate::log!("dial seat: fp={fingerprint} cleared exclusion for {exe}");
        }
        (*self.app).restore_to_dial(&fingerprint, hwnd);
        // Read the seat back through the same pipeline the rows come from. The
        // question is not "was the pin written" but "does the dial hold this
        // handle", and only the seat list can answer that.
        let seated = (*self.app)
            .window_rows()
            .iter()
            .any(|row| row.hwnd == hwnd && row.on_dial);
        let limit = (*self.app).applied_settings().wheel_limit;
        if seated {
            self.say("settings_dial_back_msg", &name);
            crate::log!("dial seat: fp={fingerprint} ok limit={limit}");
        } else {
            let problem = crate::wheel::seat_problem(
                &(*self.app).applied_settings(),
                &fingerprint,
                hwnd,
            );
            // Undo the attempt: a window that was off goes back off so the row
            // does not claim a seat it never got. This only puts back what the
            // attempt changed; taking the window off is not the same gesture.
            if was_off {
                (*self.app).undo_restore_to_dial(&fingerprint, hwnd, was_off, was_parked);
            }
            let key = match problem {
                Some(crate::wheel::SeatProblem::Excluded) => "settings_dial_seat_excluded",
                Some(crate::wheel::SeatProblem::NotRunning) => "settings_dial_seat_not_running",
                _ => "settings_dial_seat_failed",
            };
            self.say(key, &name);
            crate::log!(
                "dial seat: fp={fingerprint} failed reason={problem:?} limit={limit}"
            );
        }
        self.preview = None;
        self.refresh_dial_pool();
        let l = self.layout;
        let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_list), false);
        let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_preview), false);
        let _ = InvalidateRect(Some(self.hwnd), Some(&l.note[DIAL_PREVIEW_ROW]), false);
    }

    /// Take a row off the dial, or put it back. The word on the row and its
    /// right-click menu both come through here, so the two entries cannot drift
    /// apart, and the answer is said in the note band as well as drawn in the
    /// list and the picture.
    unsafe fn act_on_row(&mut self, at: usize) -> bool {
        let Some(row) = self.dial_pool.get(at) else {
            return false;
        };
        if !(row.on_dial || row.off_dial) {
            return false;
        }
        let fingerprint = row.fingerprint.clone();
        let hwnd = row.hwnd;
        let name = if row.tag.trim().is_empty() {
            row.title.clone()
        } else {
            row.tag.clone()
        };
        let was_on = row.on_dial;
        if was_on {
            (*self.app).take_off_dial(&fingerprint, hwnd);
        } else {
            (*self.app).restore_to_dial(&fingerprint, hwnd);
        }
        let key = if was_on {
            "settings_dial_removed_msg"
        } else {
            "settings_dial_back_msg"
        };
        self.say(key, &name);
        self.preview = None;
        self.refresh_dial_pool();
        let l = self.layout;
        let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_list), false);
        true
    }

    /// A row change touches the picture, the list beside it and the count
    /// sentence - not the rest of the page.
    unsafe fn invalidate_dial_rows(&self, hwnd: HWND) {
        unsafe {
            let l = self.layout;
            let _ = InvalidateRect(Some(hwnd), Some(&l.dial_preview), false);
            let _ = InvalidateRect(Some(hwnd), Some(&l.dial_list), false);
            let _ = InvalidateRect(Some(hwnd), Some(&l.note[DIAL_PREVIEW_ROW]), false);
        }
    }

    /// Make the picture again if it is missing. The desk is read here only when
    /// the settings it shows have changed, not on every repaint: a still picture
    /// of a list of windows is not something to ask for sixty times a second.
    unsafe fn ensure_preview(&mut self, l: &Layout) -> bool {
        let started = Instant::now();
        let side = (l.dial_preview.right - l.dial_preview.left) as u32;
        // The picture is made from the pages's OWN values, not from the file:
        // clicking a theme has to redraw the dial before anything is written
        // back, or the control and the picture disagree for a frame.
        let mut settings = (*self.app).applied_settings();
        settings.wheel_theme = self.wheel_theme;
        settings.wheel_accent = self.wheel_accent;
        settings.wheel_seam = self.wheel_seam;
        settings.wheel_radius = self.wheel_radius;
        settings.wheel_dead = self.wheel_dead;
        settings.wheel_open_ms = self.wheel_open_ms;
        settings.wheel_limit = self.wheel_limit;
        settings.dial_order = self.dial_order;
        let snapshot = match self.preview_desk.as_ref() {
            Some((list, front)) => crate::wheel::snapshot_of(&settings, list.clone(), *front),
            None => {
                let read = Instant::now();
                let (list, front) = crate::wheel::desk(&settings);
                let read_ms = read.elapsed().as_secs_f32() * 1000.0;
                self.preview_desk = Some((list.clone(), front));
                let snapshot = crate::wheel::snapshot_of(&settings, list, front);
                crate::log!(
                    "dial rebuild: desk read {read_ms:.2}ms, {} seats, limit={}",
                    snapshot.seats.len(),
                    settings.wheel_limit
                );
                snapshot
            }
        };
        // The key has to name the seats themselves, not only the numbers that
        // produce them. A row can move between "on the dial" and "not" while the
        // radius, the limit and the window count stay exactly the same - and a
        // key that missed that is precisely how the list came to call a row on
        // the dial over a picture that had no sector for it.
        let seats_key = {
            let mut hasher = DefaultHasher::new();
            for seat in &snapshot.seats {
                seat.fingerprint.hash(&mut hasher);
                seat.bearing.to_bits().hash(&mut hasher);
                seat.ring.hash(&mut hasher);
                seat.span.to_bits().hash(&mut hasher);
            }
            hasher.finish()
        };
        let key = (
            self.wheel_theme,
            self.wheel_accent,
            self.wheel_radius.to_bits(),
            self.wheel_seam.to_bits(),
            self.wheel_open_ms,
            self.wheel_limit,
            self.dial_order,
            snapshot.bounds.len(),
            seats_key,
        );
        if self.preview.as_ref().is_some_and(|p| {
            p.side == side
                && p.x == l.dial_preview.left
                && p.y == l.dial_preview.top
                && p.key == key
        }) {
            return false;
        }
        let rings = snapshot.bounds.len().max(1);
        let extent = snapshot.geometry.extent(rings).max(1.0);
        // The dial is drawn at the size the card has room for rather than at the
        // user's radius: the preview is a picture of the dial's proportions, and a
        // 300-pixel radius would simply fall outside its own card.
        let scale = side as f32 / (2.0 * extent);
        let Some(base) = crate::wheel::render_snapshot_base(&snapshot, side, scale) else {
            return false;
        };
        let mut pixmap = base.clone();
        crate::wheel::render_snapshot_seats(
            &base,
            &snapshot,
            &snapshot.seats,
            side,
            scale,
            &mut pixmap,
        );
        let render_ms = started.elapsed().as_secs_f32() * 1000.0;
        crate::log!(
            "dial rebuild: side={side} seats={} limit={} render={render_ms:.2}ms",
            snapshot.seats.len(),
            settings.wheel_limit
        );
        self.preview = Some(DialPreview {
            pixmap,
            snapshot,
            x: l.dial_preview.left,
            y: l.dial_preview.top,
            side,
            scale,
            key,
        });
        true
    }

    /// Make the filmstrip's six pictures again if the accent row or the desk
    /// changed. Each one is `snapshot_of` with the theme overridden and drawn
    /// through the same `render_thumb` the live dial's renderer feeds, so a
    /// thumbnail is a promise the dial can keep.
    unsafe fn ensure_theme_thumbs(&mut self, l: &Layout) -> bool {
        let Some(preview) = self.preview.as_ref() else {
            return false;
        };
        let cell = l.dial_themes[0];
        let cell_w = (cell.right - cell.left) as f32;
        let side = ((cell_w - 8.0 * self.scale)
            .min(DIAL_THEME_H * self.scale - 22.0 * self.scale))
        .round()
        .max(8.0) as u32;
        let rings = preview.snapshot.bounds.len() as u32;
        let seats = preview.snapshot.seats.len();
        // A different desk with the same seat count is still a different picture,
        // so the windows themselves are part of the key.
        let desk = preview.snapshot.seats.iter().fold(0u32, |hash, seat| {
            hash.wrapping_mul(31)
                .wrapping_add(seat.hwnd as u32)
                .wrapping_add(seat.fingerprint.len() as u32)
        });
        let key = (self.wheel_accent, side, rings, seats, desk);
        if self.theme_thumbs_key == key && self.theme_thumbs.as_ref().is_some_and(|t| t.side == side)
        {
            return false;
        }
        let Some((list, front)) = self.preview_desk.as_ref() else {
            return false;
        };
        let started = Instant::now();
        let mut settings = (*self.app).applied_settings();
        settings.wheel_accent = self.wheel_accent;
        settings.wheel_seam = self.wheel_seam;
        settings.wheel_radius = self.wheel_radius;
        settings.wheel_dead = self.wheel_dead;
        settings.wheel_open_ms = self.wheel_open_ms;
        settings.wheel_limit = self.wheel_limit;
        settings.dial_order = self.dial_order;
        let list = list.clone();
        let mut pixmaps: [Option<Pixmap>; DIAL_THEMES] = std::array::from_fn(|_| None);
        for theme in 0..DIAL_THEMES as u32 {
            settings.wheel_theme = theme;
            let snapshot = crate::wheel::snapshot_of(&settings, list.clone(), *front);
            pixmaps[theme as usize] =
                crate::wheel::render_thumb(&snapshot, theme, self.wheel_accent, side);
        }
        let taken = started.elapsed().as_secs_f32() * 1000.0;
        let made = pixmaps.iter().filter(|p| p.is_some()).count();
        crate::log!("dial thumbs: {made}/{DIAL_THEMES} x {side}px in {taken:.2}ms");
        crate::log!(
            "dial strip: rect={},{} {}x{} step={:.2} scale={:.2}",
            l.dial_strip.left,
            l.dial_strip.top,
            l.dial_strip.right - l.dial_strip.left,
            l.dial_strip.bottom - l.dial_strip.top,
            (l.dial_strip.right - l.dial_strip.left) as f32 / DIAL_THEMES as f32,
            self.scale
        );
        self.theme_thumbs_key = key;
        self.theme_thumbs = Some(ThemeThumbs { side, pixmaps });
        true
    }

    fn preview_fingerprint(&self, x: i32, y: i32) -> Option<String> {
        let l = self.layout;
        let preview = self.preview.as_ref()?;
        if x < l.dial_preview.left
            || x >= l.dial_preview.right
            || y < l.dial_preview.top
            || y >= l.dial_preview.bottom
        {
            return None;
        }
        let px = (x - l.dial_preview.left) as f32;
        let py = (y - l.dial_preview.top) as f32;
        preview
            .snapshot
            .hit(px, py, preview.scale, preview.side)
            .map(|seat| seat.fingerprint.clone())
    }

    /// The picture, composed in one buffer and put on the page with one blit.
    /// The paper under the card and the frame itself exist off screen first;
    /// only the finished pixels reach the window.
    unsafe fn paint_preview(&mut self, hdc: HDC, asked: RECT) {
        let Some(preview) = self.preview.as_ref() else {
            return;
        };
        let (x, y, side) = (preview.x, preview.y, preview.side);
        if !self.canvas.ensure(side) {
            return;
        }
        let (mem, src, src_bits) = (self.canvas.mem, self.canvas.src, self.canvas.src_bits);
        unsafe {
            // Only the asked-for part of the card is composed and put down. A
            // stepper that asks for one band must not pay for the whole picture.
            let local = RECT {
                left: asked.left.max(x) - x,
                top: asked.top.max(y) - y,
                right: asked.right.min(x + side as i32) - x,
                bottom: asked.bottom.min(y + side as i32) - y,
            };
            if local.right <= local.left || local.bottom <= local.top {
                return;
            }
            let (w, h) = (local.right - local.left, local.bottom - local.top);
            let ground = solid(FIELD);
            let _ = FillRect(mem, &local, ground);
            {
                let preview = self.preview.as_ref().expect("checked above");
                copy_region(
                    src_bits,
                    preview.pixmap.data(),
                    side as usize,
                    local.left as usize,
                    local.top as usize,
                    w as usize,
                    h as usize,
                );
            }
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let _ = AlphaBlend(
                mem, local.left, local.top, w, h, src, local.left, local.top, w, h, blend,
            );
            let _ = BitBlt(hdc, x + local.left, y + local.top, w, h, Some(mem), local.left, local.top, SRCCOPY);
        }
    }

    /// A click on the preview is a click on the window it landed on: the same
    /// sector the wheel would have hovered opens its label editor. Anything the
    /// pointer hits outside the disc - or in the hub - is left alone.
    unsafe fn click_preview(&mut self, x: i32, y: i32) {
        let Some(fingerprint) = self.preview_fingerprint(x, y) else {
            return;
        };
        (*self.app).name_window_centered(&fingerprint, self.hwnd);
    }

    /// The one way anything leaves this window: the whole draft is offered to the
    /// app, and either all of it lands or none does.
    ///
    /// Offered whole because the rows are not independent - a shortcut another
    /// application already owns is refused, and a change that stopped halfway
    /// would leave the paper showing one setting and the desktop running another.
    /// A refusal is answered by moving the control back and saying why out loud,
    /// because a row that silently returns to where it was reads as a broken row.
    unsafe fn apply(&mut self) {
        let draft = unsafe { self.draft() };
        let became_a_different_language = draft.lang != self.applied_lang;
        let outcome = match refuse(&draft) {
            Some(key) => Err(t(key).to_string()),
            None => (*self.app).apply_settings(draft.clone()),
        };
        match outcome {
            Ok(()) => {
                self.applied_lang = draft.lang.clone();
                self.set_motion(draft.reduced_motion);
                self.lang = draft.lang;
                // The picture is rebuilt from the values that were just applied.
                // Which parts of the page are new depends on what this save did:
                // a theme or a size changes the dial and nothing else, so only
                // the picture and the number beside it are invalidated. A language
                // change still repaints the whole page, because every word on it
                // is new.
                self.preview = None;
                let l = self.layout;
                if became_a_different_language {
                    let _ = InvalidateRect(Some(self.hwnd), None, false);
                    self.relabel();
                } else if self.section == SEC_DIAL {
                    // The picture, and that is all: the value band this click
                    // changed was already invalidated by its own stepper. Asking
                    // for the other two bands here unioned three far-apart rects
                    // with the picture into one tall band, which is the "the page
                    // refreshes on every click" the user saw.
                    let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_preview), false);
                }
                // The keys and about pages carry nothing that this save changed
                // on the paper: the boxes and the switch are child controls and
                // repaint themselves. Invalidating the page for them is what made
                // a single click look like a refresh.
            }
            Err(reason) => {
                // Back before the words: `message` runs a nested loop, and the
                // window paints in it. A row that repaints still showing the
                // value that was refused would be a lie on the paper for as long
                // as the box is up.
                self.rollback();
                message(self.hwnd, &reason);
            }
        }
    }

    /// The panel's whole answer as one value, taken from the controls rather than
    /// from a memory of what they were built with: a shortcut box that is still
    /// listening has not chosen anything, so it reports the combination it came in
    /// with.
    unsafe fn draft(&self) -> settings::Draft {
        let (fav_mods, fav_vk) = self.key_mods_vk(self.fav);
        let (panel_mods, panel_vk) = self.key_mods_vk(self.panel_key);
        settings::Draft {
            fav: (fav_mods, fav_vk),
            panel: (panel_mods, panel_vk),
            wheel_radius: self.wheel_radius,
            wheel_dead: self.wheel_dead,
            trigger_mods: self.trigger_mods,
            trigger_vk: self.trigger_vk,
            trigger_button: self.trigger_button,
            wheel_theme: self.wheel_theme,
            wheel_accent: self.wheel_accent,
            wheel_seam: self.wheel_seam,
            wheel_open_ms: self.wheel_open_ms,
            wheel_limit: self.wheel_limit,
            dial_order: self.dial_order,
            activate_pos: self.activate_pos,
            reduced_motion: self.checked(self.motion_switch),
            lang: self.lang.clone(),
        }
    }

    /// Pull every control back to the settings the app is actually running with,
    /// after a change it refused.
    unsafe fn rollback(&mut self) {
        let s = (*self.app).applied_settings();
        self.trigger_mods = s.trigger_mods;
        self.trigger_vk = s.trigger_vk;
        self.trigger_button = s.trigger_button;
        self.trigger_notice = None;
        self.wheel_radius = s.wheel_radius;
        self.wheel_dead = s.wheel_dead;
        self.wheel_theme = s.wheel_theme;
        self.wheel_accent = s.wheel_accent;
        self.wheel_seam = s.wheel_seam;
        self.wheel_open_ms = s.wheel_open_ms;
        // The limit is part of what an apply either took or refused. Leaving it
        // at the refused number made the arrows keep counting up a dial that
        // never got the seats, and the page could not be talked out of it.
        self.wheel_limit = s.wheel_limit;
        self.dial_order = s.dial_order;
        self.preview = None;
        self.lang = s.lang.clone();
        self.refresh_trigger();
        self.set_switch(self.motion_switch, s.reduced_motion);
        self.set_motion(s.reduced_motion);
        self.set_key(self.fav, s.fav_mods, s.fav_vk);
        self.set_key(self.panel_key, s.panel_mods, s.panel_vk);
        for (index, button) in self.lang_buttons.iter().enumerate() {
            self.press(*button, self.lang_tags[index].0 == s.lang);
        }
        let l = self.layout;
        unsafe {
            // 轮盘's numbers and picture were showing the refused draft; a
            // refusal is rare enough to pay for the whole page.
            if l.section == SEC_DIAL {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
            self.refresh_steppers();
        }
    }

    /// Say the window again in the language that is now in force: the panel's own
    /// text, the captions the buttons carry, the face they are drawn in, and the
    /// title the window manager shows.
    unsafe fn relabel(&mut self) {
        unsafe {
            // The "follow the system" row is the one name on this window that is
            // a sentence rather than what a pack calls itself.
            for (tag, name) in &mut self.lang_tags {
                if *tag == crate::i18n::AUTO {
                    *name = t("lang_auto").to_string();
                }
            }
            for (button, key) in &self.captions {
                self.set_caption(*button, t(key));
            }
            for (button, (_, name)) in self.lang_buttons.iter().zip(self.lang_tags.iter()) {
                self.set_caption(*button, name);
            }
            // The fonts hold the old face; the words are in the new one.
            for font in self.fonts.iter() {
                if !font.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(font.0));
                }
            }
            self.fonts = make_fonts(self.scale);
            let title = wstr(t("settings_title"));
            let _ = SetWindowTextW(self.hwnd, PCWSTR(title.as_ptr()));
            for hwnd in self.stops() {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            let _ = InvalidateRect(Some(self.hwnd), None, false);
            // A different language is different widths, and a word that no longer
            // fits is a line in the log rather than a mystery on the paper.
            warn_cut(self);
        }
    }

    /// A row was moved. The change is not applied from inside the control that
    /// moved it: the window is told and answers one message later, when no click
    /// is still in progress and a refused row can be put back without reaching
    /// into a control that is still drawing itself.
    unsafe fn moved(&self) {
        unsafe {
            let _ = PostMessageW(Some(self.hwnd), WM_ROW_MOVED, WPARAM(0), LPARAM(0));
        }
    }

    unsafe fn set_caption(&self, hwnd: HWND, text: &str) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Button;
            if !ptr.is_null() {
                (*ptr).set_text(text);
            }
        }
    }

    /// Grey out a stepper end that has reached its floor or ceiling. The page's
    /// own numbers are clamped, so an end that cannot move is the truth and not
    /// a broken button.
    unsafe fn set_enabled(&self, hwnd: HWND, on: bool) {
        unsafe {
            if hwnd.is_invalid() {
                return;
            }
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Button;
            if !ptr.is_null() {
                (*ptr).set_enabled(on);
            }
        }
    }

    /// Which of the six dial arrows have room to move: floor first, ceiling
    /// second, for the three rows they belong to.
    unsafe fn refresh_steppers(&self) {
        let room = [
            self.wheel_radius > RADIUS_MIN,
            self.wheel_radius < RADIUS_MAX,
            self.wheel_seam > 0.0,
            self.wheel_seam < settings::WHEEL_SEAM_MAX,
            self.wheel_limit > settings::WHEEL_LIMIT_MIN,
            self.wheel_limit < settings::WHEEL_LIMIT_MAX,
        ];
        for (at, hwnd) in self.dial_steps.iter().enumerate() {
            if let Some(on) = room.get(at) {
                unsafe { self.set_enabled(*hwnd, *on) };
            }
        }
    }

    unsafe fn set_switch(&self, hwnd: HWND, on: bool) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Switch;
            if !ptr.is_null() {
                (*ptr).set(on);
            }
        }
    }

    /// Hand the new motion setting to the two knobs that slide. A switch about
    /// motion which goes on sliding after the desktop was told to stop is
    /// arguing with the row that just changed, and there is no other way for the
    /// row to be seen to do anything at all.
    unsafe fn set_motion(&self, reduced: bool) {
        unsafe {
            let ptr = GetWindowLongPtrW(self.motion_switch, GWLP_USERDATA) as *mut Switch;
            if !ptr.is_null() {
                (*ptr).reduced = reduced;
            }
        }
    }

    unsafe fn set_key(&self, hwnd: HWND, mods: u32, vk: u32) {
        unsafe {
            let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Key;
            if !ptr.is_null() {
                (*ptr).set_combo(mods, vk);
            }
        }
    }

    /// The row answers a click by moving the app: a language changes every word on
    /// screen, so the words change with it.
    fn choose_lang(&mut self, at: usize) {
        self.lang = self.lang_tags[at].0.clone();
        for (index, button) in self.lang_buttons.iter().enumerate() {
            unsafe {
                let ptr = GetWindowLongPtrW(*button, GWLP_USERDATA) as *mut Button;
                if !ptr.is_null() {
                    (*ptr).primary = index == at;
                    let _ = InvalidateRect(Some(*button), None, false);
                }
            }
        }
        unsafe { self.moved() };
    }

    /// The language files are text somebody has to edit, so the door to them is
    /// part of the setting. Making sure the folder exists first is what puts a
    /// finished, fill-in-the-blanks copy inside it.
    unsafe fn open_language_folder(&self) {
        unsafe {
            crate::i18n::export_templates();
            let path = crate::i18n::languages_dir().display().to_string();
            let wide = wstr(&path);
            let _ = ShellExecuteW(
                Some(self.hwnd),
                w!("open"),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                None,
                SW_SHOWNORMAL,
            );
        }
    }

    // --- the windows on the dial --------------------------------------------------

    /// The refresh button's act: walk the desk again so a window opened while the
    /// page was up appears in the list and the picture, without closing and
    /// reopening the settings. The rows and the preview are rebuilt from one read
    /// (see [`Self::refresh_dial_pool`]) and the two surfaces are redrawn.
    unsafe fn press_refresh(&mut self) {
        self.refresh_dial_pool();
        self.preview = None;
        let l = self.layout;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_list), false);
            let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_preview), false);
            let _ = InvalidateRect(Some(self.hwnd), Some(&l.dial_refresh), false);
        }
        crate::log!("dial refresh: button pressed, desk re-read");
    }

    /// Re-read the dial's list and lay it out again under the cards, in its two
    /// sections. Every window the panel can name is here, not a sample: the ones
    /// the dial holds first, then the rest, because a window that is not on the
    /// dial yet is the one most likely to be put on it.
    fn refresh_dial_pool(&mut self) {
        // One walk of the desk, shared by the rows and the picture: the same
        // moments of the same windows decide both what the list calls "on the
        // dial" and what the wheel draws. Two reads could disagree - and did,
        // which is how a row claimed a seat over an empty picture.
        let live = shaiwin_platform_win::window::list_switchable(std::process::id());
        // The page's own values, exactly as `ensure_preview` reads them. A limit
        // or order the page has changed but not yet written to the file still
        // decides which windows get seats, so the rows have to be built from the
        // same numbers as the picture or one of them can name a different desk.
        let mut settings = unsafe { (*self.app).applied_settings() };
        settings.wheel_limit = self.wheel_limit;
        settings.dial_order = self.dial_order;
        let desk: Vec<WindowInfo> = live
            .iter()
            .filter(|info| !settings.is_excluded(&info.exe))
            .cloned()
            .collect();
        self.preview_desk = Some((
            desk,
            shaiwin_platform_win::window::foreground().map(shaiwin_platform_win::window::raw_of),
        ));
        self.preview = None;
        self.row_press = None;
        self.row_click = None;
        self.pool_all = unsafe { (*self.app).window_rows_from(&live, &settings) };
        self.sort_pool();
        self.refilter_pool();
        // One line per refresh so the order can be checked from the log: `*`
        // marks a row the wheel is holding, which must all come first.
        let head: Vec<String> = self
            .dial_pool
            .iter()
            .take(8)
            .map(|row| {
                let name = if row.tag.is_empty() {
                    if row.title.is_empty() {
                        &row.app
                    } else {
                        &row.title
                    }
                } else {
                    &row.tag
                };
                format!("{}{name}", if row.on_dial { "*" } else { "-" })
            })
            .collect();
        crate::log!(
            "dial pool: {} rows on={} off={} on-dial-first=[{}]",
            self.dial_pool.len(),
            self.dial_pool.iter().filter(|row| row.on_dial).count(),
            self.dial_pool.iter().filter(|row| !row.on_dial).count(),
            head.join(" | ")
        );
    }

    /// The dial's list reads top-down in the order a person thinks about it:
    /// the windows that are on the wheel first, then the rest. "On the wheel"
    /// is the line that has to stay put - it is the answer to "what is on my
    /// dial", which is why the list exists next to the picture.
    fn sort_pool(&mut self) {
        // Stable on purpose: the rows arrive in the desk's own order, front
        // first, and that is the order a person looks for a window in. The keys
        // only decide which of the two sections a row belongs to; they must not
        // shuffle the names inside one, or the list moves under the pointer
        // every time a double-click changes a seat.
        self.pool_all.sort_by_key(|row| (!row.on_dial, !row.favourite));
    }

    /// Re-marks the rows the dial is holding after the number or the order
    /// changed. The list part of a row is "on the dial" or "not", and both the
    /// limit and the order move windows across that line - so the rows have to
    /// be redrawn even though no window was read. Using the desk list the preview
    /// already holds keeps a stepper click free of a walk over every window; the
    /// fallback only runs when there is no copy to use.
    unsafe fn sync_pool_seats(&mut self) {
        let Some((list, _front)) = self.preview_desk.as_ref() else {
            self.refresh_dial_pool();
            return;
        };
        let mut settings = (*self.app).applied_settings();
        settings.wheel_limit = self.wheel_limit;
        settings.dial_order = self.dial_order;
        let seated = crate::wheel::seated_hwnds(&settings, list);
        for row in &mut self.pool_all {
            if row.hwnd != 0 {
                row.on_dial = seated.contains(&row.hwnd);
            }
        }
        // A window that just took a seat, or lost one, moves across the line
        // now - not on the next refresh, which may be a keystroke away.
        self.sort_pool();
        self.refilter_pool();
    }

    /// Keep the rows the search field's text selects. The match is a case-folded
    /// substring over the three names a person might remember a window by - the
    /// title, the executable and the application. An empty field is every window.
    /// The rows the page draws and the rows it drags are the same vector, so a
    /// filter narrows the view without breaking the drag: it only changes which
    /// row is under the pointer.
    fn refilter_pool(&mut self) {
        self.dial_pool = filter_rows(&self.pool_all, &self.pool_query);
        self.pool_view = build_pool_view(&self.dial_pool);
        let visible = self.visible_pool_entries().max(1);
        let last = self.pool_view.len().saturating_sub(visible);
        self.pool_scroll = self.pool_scroll.min(last);
        // The two sections survive a search and count what survived it: one
        // line, so a report of a half-empty list can be checked against the
        // words in the box rather than guessed at.
        crate::log!(
            "dial filter: query={:?} rows={} on={} off={}",
            self.pool_query,
            self.dial_pool.len(),
            self.dial_pool.iter().filter(|row| row.on_dial).count(),
            self.dial_pool.iter().filter(|row| !row.on_dial).count()
        );
    }

    /// One line for the screenshots: where the list and the picture ended up and
    /// how many rows the height shows. Written when the page opens and again
    /// after a size change, so a layout claim can be checked from the log rather
    /// than from a ruler on the image.
    fn log_dial_layout(&self) {
        if self.section != SEC_DIAL {
            return;
        }
        let l = self.layout;
        crate::log!(
            "dial layout: list={},{} {}x{} preview={},{} {}x{} scale={:.3} nav={} rows={} scroll={:.1}/{:.1}",
            l.dial_list.left,
            l.dial_list.top,
            l.dial_list.right - l.dial_list.left,
            l.dial_list.bottom - l.dial_list.top,
            l.dial_preview.left,
            l.dial_preview.top,
            l.dial_preview.right - l.dial_preview.left,
            l.dial_preview.bottom - l.dial_preview.top,
            self.scale,
            (NAV_W * self.scale) as i32,
            self.visible_pool_entries(),
            self.page_scroll,
            l.scroll_max
        );
    }

    /// How many lines of the list fit in the rectangle the page gave it. The
    /// headings take a row's height each, so the count is over the entries and
    /// not only over the windows. The height is whatever the picture leaves, so
    /// a tall window shows more of the list rather than a fixed three.
    fn visible_pool_entries(&self) -> usize {
        let list = self.layout.dial_list;
        let step = (POOL_ROW_STEP * self.scale).round().max(1.0) as i32;
        let height = (list.bottom - list.top).max(0);
        (height / step) as usize
    }

    /// The rectangle of one line of the list - heading or window - in page
    /// pixels, or `None` when the scroll has taken it out of the viewport.
    fn pool_entry_rect(&self, entry: usize) -> Option<RECT> {
        let list = self.layout.dial_list;
        let step = (POOL_ROW_STEP * self.scale).round().max(1.0) as i32;
        let height = (POOL_ROW_H * self.scale).round().max(1.0) as i32;
        let y = list.top + (entry as i32 - self.pool_scroll as i32) * step;
        if y < list.top || y + height > list.bottom {
            return None;
        }
        Some(RECT {
            left: list.left,
            top: y,
            right: list.right,
            bottom: y + height,
        })
    }

    /// Which row of the list a page point is on. A heading occupies a line of
    /// the list but is not a window, so a press on one finds nothing.
    fn pool_row_at(&self, x: i32, y: i32) -> Option<usize> {
        for entry in 0..self.pool_view.len() {
            let PoolEntry::Row { at } = self.pool_view[entry] else {
                continue;
            };
            if self.pool_entry_rect(entry).is_some_and(|rect| {
                x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
            }) {
                return Some(at);
            }
        }
        None
    }

    /// The wheel over the dial's list moves it one row, the same grain the list
    /// itself uses. The list is the only thing on this page that can overflow, so
    /// it is the only thing that answers the wheel.
    unsafe fn scroll_dial_pool(&mut self, delta: i32) {
        let visible = self.visible_pool_entries().max(1);
        let last = self.pool_view.len().saturating_sub(visible);
        let next = if delta < 0 {
            (self.pool_scroll + 1).min(last)
        } else {
            self.pool_scroll.saturating_sub(1)
        };
        if next == self.pool_scroll {
            return;
        }
        self.pool_scroll = next;
        let rect = self.layout.dial_list;
        let _ = unsafe { InvalidateRect(Some(self.hwnd), Some(&rect), false) };
    }

    /// The wheel over the page moves the paper under the glass by a row and a
    /// half. Only a window dragged below its section's design height has any
    /// paper to move; everywhere else this does nothing and the message falls
    /// through to the default, which is how a desktop keeps its wheel.
    unsafe fn scroll_page(&mut self, delta: i32) {
        let max = self.layout.scroll_max;
        if max <= 0.0 {
            return;
        }
        let step = (POOL_ROW_STEP * 2.0 * self.scale).round();
        let next = (self.page_scroll - (delta as f32 / 120.0) * step).clamp(0.0, max);
        if (next - self.page_scroll).abs() < 0.5 {
            return;
        }
        self.page_scroll = next;
        let mut client = RECT::default();
        unsafe {
            let _ = GetClientRect(self.hwnd, &mut client);
        }
        self.layout = measure_tuned(
            self.section,
            client.right,
            client.bottom,
            self.scale,
            self.lang_tags.len(),
            self.page_scroll,
        );
        let placed = self.layout;
        unsafe {
            self.place_children(&placed);
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// The log is the only way anybody can see what the program did when nobody
    /// was watching, so the row that names it has a door. Opening the file rather
    /// than the folder is the whole point: the thing you wanted is in it.
    unsafe fn open_log(&self) {
        unsafe {
            let path = log_path().display().to_string();
            let wide = wstr(&path);
            let _ = ShellExecuteW(
                Some(self.hwnd),
                w!("open"),
                PCWSTR(wide.as_ptr()),
                PCWSTR::null(),
                None,
                SW_SHOWNORMAL,
            );
        }
    }
}

/// The file the log is written to. [`crate::log`] opens it at startup and keeps
/// the handle, so this is the same path worked out again rather than a second
/// copy of anything - the folder is one name, and the file inside it is another.
fn log_path() -> std::path::PathBuf {
    crate::log::path()
}

/// The log's name the way a person would recognise it. `data_dir` is one of two
/// known folders, and writing the front of the path out in full is how a line
/// under a row became a line nobody could read the end of: `%LOCALAPPDATA%\…` is
/// thirty-four characters, and the band under a card holds twenty-six of them.
/// The door on that row opens the file, so the line's job is to say *which* file,
/// not to be typed into an address box - which leaves the front as three dots and
/// the part that identifies it in full.
fn log_path_shown() -> String {
    let path = log_path().display().to_string();
    for var in ["LOCALAPPDATA", "APPDATA"] {
        let Some(base) = std::env::var_os(var) else {
            continue;
        };
        let base = std::path::PathBuf::from(base).display().to_string();
        if let Some(rest) = path.strip_prefix(&base) {
            return format!("…{rest}");
        }
    }
    path
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut State;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let state = &mut *ptr;
        match msg {
            WM_CTLCOLOREDIT => {
                // The search field's own painting, in the page's colours: the
                // field's white behind it and ink in front. The white is what
                // makes it read as a box on the card rather than as a line of
                // text that happens to sit above the list. While the placeholder
                // is in the field, it is grey, the way a hint should read.
                let hdc = HDC(wp.0 as *mut c_void);
                let _ = SetBkColor(hdc, rgb(FIELD));
                let text = if state.search_show_hint { MUTED } else { INK };
                let _ = SetTextColor(hdc, rgb(text));
                LRESULT(state.search_brush.0 as isize)
            }
            WM_COMMAND => {
                let id = (wp.0 & 0xFFFF) as i32;
                let notification = ((wp.0 >> 16) & 0xFFFF) as u32;
                // Escape reaches the parent as a bare IDCANCEL, with no click
                // notification at all. It closes the window; there is nothing
                // left in it to undo, because every row applied itself the
                // moment it was moved.
                if id == ESCAPE_ID && (notification == 0 || notification == BN_CLICKED) {
                    let _ = DestroyWindow(hwnd);
                    return LRESULT(0);
                }
                if id == SEARCH_ID && notification == 0x0100 {
                    // EN_SETFOCUS: the placeholder leaves before the caret
                    // arrives, and the frame's edge brightens, so the box says
                    // where the keys are going.
                    if state.search_show_hint {
                        state.search_show_hint = false;
                        let empty = wstr("");
                        let _ = SetWindowTextW(state.dial_search, PCWSTR(empty.as_ptr()));
                    }
                    state.search_focused = true;
                    let frame = state.layout.dial_search;
                    let _ = InvalidateRect(Some(hwnd), Some(&frame), false);
                    return LRESULT(0);
                }
                if id == SEARCH_ID && notification == 0x0200 {
                    // EN_KILLFOCUS: an empty box gets its placeholder back, so
                    // it never reads as a field someone has emptied on purpose.
                    let mut text = [0u16; 256];
                    let len = GetWindowTextW(state.dial_search, &mut text) as usize;
                    if len == 0 {
                        state.search_show_hint = true;
                        let hint = wstr(&state.search_hint);
                        let _ = SetWindowTextW(state.dial_search, PCWSTR(hint.as_ptr()));
                    }
                    state.search_focused = false;
                    let frame = state.layout.dial_search;
                    let _ = InvalidateRect(Some(hwnd), Some(&frame), false);
                    return LRESULT(0);
                }
                if id == SEARCH_ID && notification == 0x0300 {
                    // EN_CHANGE: the field's text is the filter. The rows are
                    // re-cut from the copy of the desk the page already holds, so
                    // typing costs a re-sort of a vector, not a walk of the desktop.
                    // The placeholder is not a query: while it is showing, the
                    // list is every window.
                    let mut text = [0u16; 256];
                    let len = GetWindowTextW(state.dial_search, &mut text) as usize;
                    let typed = String::from_utf16_lossy(&text[..len.min(text.len())]);
                    state.search_show_hint = state.search_show_hint && typed == state.search_hint;
                    state.pool_query = if state.search_show_hint {
                        String::new()
                    } else {
                        typed
                    };
                    state.refilter_pool();
                    let list = state.layout.dial_list;
                    let _ = InvalidateRect(Some(hwnd), Some(&list), false);
                    return LRESULT(0);
                }
                if id == FOLDER_ID && notification == BN_CLICKED {
                    state.open_language_folder();
                    return LRESULT(0);
                }
                if id == LOG_DOOR_ID && notification == BN_CLICKED {
                    state.open_log();
                    return LRESULT(0);
                }
                // Turning to another section is not a change to the desktop, so
                // it never goes near `apply`: the rows are made once and only
                // which of them are on paper moves.
                if (NAV_BASE..NAV_BASE + SEC_COUNT as i32).contains(&id)
                    && notification == BN_CLICKED
                {
                    let at = (id - NAV_BASE) as usize;
                    if NAV_SECTIONS.contains(&at) {
                        state.apply_section(at);
                    }
                    return LRESULT(0);
                }
                if notification == BN_CLICKED {
                    if (DIAL_CELL_BASE..DIAL_CELL_BASE + DIAL_CELLS as i32).contains(&id) {
                        let at = (id - DIAL_CELL_BASE) as usize;
                        let (group, index) = if at < DIAL_ACCENT_CELLS {
                            (0, at)
                        } else if at < DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS {
                            (1, at - DIAL_ACCENT_CELLS)
                        } else if at < DIAL_ACCENT_CELLS + DIAL_ANIM_CELLS + DIAL_ORDER_CELLS {
                            (2, at - DIAL_ACCENT_CELLS - DIAL_ANIM_CELLS)
                        } else {
                            (3, at - DIAL_ACCENT_CELLS - DIAL_ANIM_CELLS - DIAL_ORDER_CELLS)
                        };
                        state.choose_dial(group, index);
                        return LRESULT(0);
                    }
                    if (DIAL_STEP_BASE..DIAL_STEP_BASE + 6).contains(&id) {
                        state.step_dial((id - DIAL_STEP_BASE) as usize);
                    }
                }
                if id >= LANG_BASE && notification == BN_CLICKED {
                    let at = (id - LANG_BASE) as usize;
                    if at < state.lang_tags.len() {
                        state.choose_lang(at);
                    }
                    return LRESULT(0);
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if state.section == SEC_DIAL {
                    let x = (lp.0 as i16) as i32;
                    let y = ((lp.0 >> 16) as i16) as i32;
                    let rb = state.layout.dial_refresh;
                    let hot = x >= rb.left && x < rb.right && y >= rb.top && y < rb.bottom;
                    if hot != state.refresh_hot {
                        state.refresh_hot = hot;
                        let _ = InvalidateRect(Some(hwnd), Some(&rb), false);
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
                if state.refresh_hot {
                    state.refresh_hot = false;
                    let rb = state.layout.dial_refresh;
                    let _ = InvalidateRect(Some(hwnd), Some(&rb), false);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                // The picture is painted by this window, so a press on a sector
                // arrives here. The release is the click that renames it.
                if state.section == SEC_DIAL {
                    let x = (lp.0 as i16) as i32;
                    let y = ((lp.0 >> 16) as i16) as i32;
                    // The refresh button is painted by this window, so its press
                    // arrives here. Like the filmstrip, it answers on the way
                    // down: re-read the desk so newly opened windows appear.
                    let rb = state.layout.dial_refresh;
                    if x >= rb.left && x < rb.right && y >= rb.top && y < rb.bottom {
                        state.press_refresh();
                        return LRESULT(0);
                    }
                    // The filmstrip is painted by this window, so a press on a
                    // thumbnail arrives here rather than at a child. It is a
                    // choice on the way down: no preview step, no OK button.
                    for at in 0..DIAL_THEMES {
                        let cell = state.layout.dial_themes[at];
                        if x >= cell.left && x < cell.right && y >= cell.top && y < cell.bottom {
                            state.dial_press = None;
                            state.choose_theme(at as u32);
                            crate::log!("dial strip: press={at} x={x} y={y}");
                            return LRESULT(0);
                        }
                    }
                    // The press point is the click's own address. A drag that
                    // starts in the seam between rings and ends on a sector
                    // would otherwise be read as a click under the release
                    // point, and the label box would open for a gesture that
                    // was aimed somewhere else entirely.
                    state.dial_press = Some((x, y));
                    state.row_press = None;
                    // A row is the double-click's target: the press is only
                    // remembered here, and the release decides whether it was
                    // the first half of a double-click.
                    if let Some(at) = state.pool_row_at(x, y) {
                        state.dial_press = None;
                        state.row_press = Some((at, (x, y)));
                        return LRESULT(0);
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if state.section == SEC_DIAL {
                    let x = (lp.0 as i16) as i32;
                    let y = ((lp.0 >> 16) as i16) as i32;
                    let press = state.dial_press.take();
                    if let Some((at, (px, py))) = state.row_press.take() {
                        // A press on a row's name. It has no drag of its own any
                        // more, so a release that travelled is simply dropped; a
                        // release that stayed put is the first click, or the
                        // second when the last click was this same row inside
                        // the system's own double-click time. The measure is
                        // read from Windows rather than guessed, so the gesture
                        // matches the one the rest of the desktop teaches.
                        let slop = SEAT_DRAG_SLOP * state.scale;
                        let dx = (x - px) as f32;
                        let dy = (y - py) as f32;
                        if dx * dx + dy * dy < slop * slop {
                            let measured = state.row_click.as_ref().is_some_and(|click| {
                                click.at == at
                                    && click.when.elapsed().as_millis()
                                        <= GetDoubleClickTime() as u128
                            });
                            if measured {
                                state.row_click = None;
                                state.toggle_pool_row(at);
                            } else {
                                state.row_click = Some(RowClick {
                                    at,
                                    when: Instant::now(),
                                });
                            }
                        }
                    } else if let Some((px, py)) = press {
                        // Nothing grabbed the press: it began on a gap, the hub,
                        // or outside the disc. Only a release that stayed put is
                        // a click, and it is answered at the press point - a
                        // gesture that travelled is never asked to open a label
                        // box just because it happened to end on a sector.
                        let slop = SEAT_DRAG_SLOP * state.scale;
                        let dx = (x - px) as f32;
                        let dy = (y - py) as f32;
                        if dx * dx + dy * dy < slop * slop {
                            state.click_preview(px, py);
                        }
                    } else {
                        state.click_preview(x, y);
                    }
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                // A right click on a sector offers one decision: take the window
                // off the dial. The list rows answer the same button, in whichever
                // direction the row is currently pointing.
                crate::log!(
                    "sector menu: right button up at {},{} section={}",
                    (lp.0 as i16) as i32,
                    ((lp.0 >> 16) as i16) as i32,
                    state.section
                );
                if state.section == SEC_DIAL {
                    let x = (lp.0 as i16) as i32;
                    let y = ((lp.0 >> 16) as i16) as i32;
                    // The list answers the right button too: a row that is on
                    // the dial offers "remove", one that is off offers "put
                    // back". The same guarded track call as the sectors - a
                    // menu that cannot own the foreground is skipped, not stranded.
                    if let Some(row_at) = state.pool_row_at(x, y) {
                        let (on_dial, off_dial) = {
                            let row = &state.dial_pool[row_at];
                            (row.on_dial, row.off_dial)
                        };
                        if on_dial || off_dial {
                            let word = if on_dial {
                                crate::i18n::t("settings_dial_remove")
                            } else {
                                crate::i18n::t("settings_dial_restore")
                            };
                            let wide = wstr(word);
                            let menu = CreatePopupMenu().unwrap_or_default();
                            if !menu.is_invalid() {
                                let _ =
                                    AppendMenuW(menu, MF_STRING, 1, PCWSTR(wide.as_ptr()));
                                let mut spot = POINT { x, y };
                                let _ = ClientToScreen(hwnd, &mut spot);
                                let owned = SetForegroundWindow(hwnd).as_bool();
                                let picked = if owned {
                                    TrackPopupMenu(
                                        menu,
                                        TPM_RIGHTBUTTON | TPM_RETURNCMD,
                                        spot.x,
                                        spot.y,
                                        Some(0),
                                        hwnd,
                                        None,
                                    )
                                } else {
                                    crate::log!("row menu: no foreground, menu skipped");
                                    BOOL(0)
                                };
                                let _ = DestroyMenu(menu);
                                let _ = PostMessageW(
                                    Some(hwnd),
                                    WM_NULL,
                                    WPARAM(0),
                                    LPARAM(0),
                                );
                                crate::log!(
                                    "row menu: at={row_at} on_dial={on_dial} picked={}",
                                    picked.0
                                );
                                if picked.0 == 1 {
                                    state.act_on_row(row_at);
                                }
                            }
                            return LRESULT(0);
                        }
                    }
                    let seat = state.preview_fingerprint(x, y).and_then(|fingerprint| {
                        state
                            .preview
                            .as_ref()
                            .and_then(|preview| {
                                preview
                                    .snapshot
                                    .seats
                                    .iter()
                                    .find(|seat| seat.fingerprint == fingerprint)
                            })
                            .cloned()
                    });
                    let probe = {
                        let l = state.layout;
                        match state.preview.as_ref() {
                            Some(p) => {
                                let c = p.side as f32 / 2.0;
                                let px = (x - l.dial_preview.left) as f32;
                                let py = (y - l.dial_preview.top) as f32;
                                let d = ((px - c).powi(2) + (py - c).powi(2)).sqrt() / p.scale;
                                format!(
                                    "rect={},{} {}x{} side={} scale={:.2} d={d:.2} hub={:.2} rings={}",
                                    l.dial_preview.left,
                                    l.dial_preview.top,
                                    l.dial_preview.right - l.dial_preview.left,
                                    l.dial_preview.bottom - l.dial_preview.top,
                                    p.side,
                                    p.scale,
                                    p.snapshot.geometry.hub(),
                                    p.snapshot.bounds.len()
                                )
                            }
                            None => "no preview".to_string(),
                        }
                    };
                    crate::log!(
                        "sector menu: click at {x},{y} seat={} probe={}",
                        seat.as_ref()
                            .map(|seat| seat.fingerprint.as_str())
                            .unwrap_or("-"),
                        probe
                    );
                    if let Some(seat) = seat {
                        let word = wstr(crate::i18n::t("menu_sector_remove"));
                        let menu = CreatePopupMenu().unwrap_or_default();
                            if !menu.is_invalid() {
                                let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR(word.as_ptr()));
                                let mut at = POINT { x, y };
                                let _ = ClientToScreen(hwnd, &mut at);
                                // A native menu only runs its own loop when this
                                // window can take the foreground. Without it the
                                // track call can sit waiting forever and the page
                                // stops answering, so it is asked first and the
                                // menu is skipped rather than stranded.
                                let owned = SetForegroundWindow(hwnd).as_bool();
                                let picked = if owned {
                                    TrackPopupMenu(
                                        menu,
                                        TPM_RIGHTBUTTON | TPM_RETURNCMD,
                                        at.x,
                                        at.y,
                                        Some(0),
                                        hwnd,
                                        None,
                                    )
                                } else {
                                    crate::log!("sector menu: no foreground, menu skipped");
                                    BOOL(0)
                                };
                                let _ = DestroyMenu(menu);
                                let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                                crate::log!("sector menu: picked {}", picked.0);
                                if picked.0 == 1 {
                                    (*state.app).take_off_dial(&seat.fingerprint, seat.hwnd);
                                    state.preview = None;
                                    state.refresh_dial_pool();
                                    state.invalidate_dial_rows(hwnd);
                                }
                            }
                    }
                }
                LRESULT(0)
            }
            WM_TIMER if wp.0 == DIAL_TOAST_TIMER => {
                // The sentence had its moment; the band goes back to saying
                // what the page is for.
                let _ = KillTimer(Some(hwnd), DIAL_TOAST_TIMER);
                state.dial_toast = None;
                let rect = state.layout.note[DIAL_PREVIEW_ROW];
                let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
                LRESULT(0)
            }
            DM_GETDEFID => {
                // Enter looks for a default button by asking the parent this.
                // There is none: a settings window that applies every row as it
                // is moved has nothing left to press, and inventing an invisible
                // default would be a decision nobody clicked.
                LRESULT(0)
            }
            WM_ROW_MOVED => {
                // Applied here, in the parent's own queue, rather than by the
                // control that moved: a refusal puts a modal box on this window,
                // and while that box is up the window underneath is disabled, so
                // no second row can move into the middle of the first one's
                // rollback.
                state.apply();
                LRESULT(0)
            }
            WM_WHEEL_OPENED => {
                // The dial is the thing being aimed at now, so the still picture
                // underneath is stale: a page nobody is looking at will build a
                // fresh one when it is next entered.
                state.preview = None;
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_ROWS_CHANGED => {
                // Somebody else changed the dial's list - the shortcut took a
                // seat, the editor committed a label, a window closed. The dial
                // preview is thrown away either way: a label the editor just
                // committed, or a direction that moved, is drawn into that still
                // picture, and a page nobody is looking at will build a fresh one
                // when it is next entered.
                state.preview = None;
                if state.section == SEC_DIAL {
                    // The dial page's list is the other view of the same rows:
                    // a window taken off or put back has to leave or enter it
                    // now, not on the next visit. Only the picture, the list and
                    // the count beside the dial changed - the rest of the page
                    // stays on the surface untouched.
                    state.refresh_dial_pool();
                    state.invalidate_dial_rows(hwnd);
                }
                LRESULT(0)
            }
            WM_SHOW_SECTION => {
                if wp.0 < SEC_COUNT {
                    state.apply_section(wp.0);
                }
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                // A child that does not want the wheel hands it to its parent, so
                // this is where the dial's list arrives. Over the list the wheel
                // moves the list; anywhere else it moves the page's own paper,
                // which only exists when the window is shorter than the section.
                // With neither of those to do the message goes to the default,
                // so a page with nothing to scroll does not eat it in silence.
                // The placeholder arm below is removed.
                let mut at = POINT {
                    x: (lp.0 & 0xFFFF) as i16 as i32,
                    y: ((lp.0 >> 16) & 0xFFFF) as i16 as i32,
                };
                let _ = ScreenToClient(hwnd, &mut at);
                let over_list = state.section == SEC_DIAL
                    && state.layout.dial_list.right > state.layout.dial_list.left
                    && at.x >= state.layout.dial_list.left
                    && at.x < state.layout.dial_list.right
                    && at.y >= state.layout.dial_list.top
                    && at.y < state.layout.dial_list.bottom;
                if over_list {
                    state.scroll_dial_pool(((wp.0 >> 16) as i16) as i32);
                    LRESULT(0)
                } else if state.layout.scroll_max > 0.0 {
                    state.scroll_page(((wp.0 >> 16) as i16) as i32);
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_GETMINMAXINFO => {
                // A floor for the resize handle in logical pixels, converted to a
                // whole-window size through the same frame the window was made
                // with. Below this the folded dial cannot keep its promises - a
                // 260-pixel picture and three window rows - and the paper scrolls
                // under the glass instead, so the page never lays itself out into
                // a shape it cannot keep. The DPI is the one the window is on now
                // - the page may have been dragged to another monitor since it was
                // opened, and the floor has to follow the layout, not the monitor
                // it left.
                let info = &mut *(lp.0 as *mut MINMAXINFO);
                let dpi = GetDpiForWindow(hwnd);
                let (min_w, min_h) = min_track_size(if dpi == 0 {
                    (96.0 * state.scale).round() as u32
                } else {
                    dpi
                });
                info.ptMinTrackSize.x = min_w;
                info.ptMinTrackSize.y = min_h;
                let stamp = ((min_w as isize) << 32) | (min_h as isize & 0xFFFF_FFFF);
                if FLOOR_LOGGED.swap(stamp, Ordering::Relaxed) != stamp {
                    crate::log!(
                        "window: the floor is {}x{} for {} dpi ({}x{} logical)",
                        min_w,
                        min_h,
                        GetDpiForWindow(hwnd),
                        MIN_CLIENT_W,
                        MIN_CLIENT_H
                    );
                }
                LRESULT(0)
            }
            WM_SIZE => {
                // The page lays itself out again in the new client, and the
                // picture is remade at the size its card now has. Both cards are
                // the height of the client, so a bigger window is a bigger dial
                // and a longer list, not a taller row.
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                state.layout = measure_tuned(
                    state.section,
                    client.right,
                    client.bottom,
                    state.scale,
                    state.lang_tags.len(),
                    state.page_scroll,
                );
                // A window dragged taller can leave the page scrolled past the
                // end of its own paper; the clamp puts it back on the floor.
                state.page_scroll = state.page_scroll.min(state.layout.scroll_max);
                let placed = state.layout;
                state.place_children(&placed);
                state.preview = None;
                state.log_dial_layout();
                // The layout changed, so the whole page is new - but the erase
                // flag stays off: the page paints every pixel of its own client
                // and `WM_ERASEBKGND` already answers with one, so asking for an
                // erase only buys a frame of the class brush.
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_EXITSIZEMOVE => {
                // The corner was let go. Read the client in device pixels and
                // store the logical size, because the next open may be on a
                // monitor with a different scale.
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let w = client.right as f32 / state.scale;
                let h = client.bottom as f32 / state.scale;
                (*state.app).remember_panel_size(w, h);
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                state.paint(hwnd);
                LRESULT(0)
            }
            WM_CLOSE => {
                // The page is the app's face, not the app. Closing it puts it away
                // in the notification area: the process stays, the shortcuts stay
                // armed, and the dial still opens on the gesture. The only way out
                // is the tray's 退出, where the shutdown path can unhook, unhotkey
                // and remove the icon in an order that leaves nothing behind.
                // Hiding rather than destroying also keeps the page's size, the
                // chosen section and the typed query for the next opening.
                if IsWindowVisible(hwnd).as_bool() {
                    let _ = ShowWindow(hwnd, SW_HIDE);
                    crate::log!("settings: closed to the notification area; the process stays");
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                SETTINGS_HWND.store(0, Ordering::Relaxed);
                (*state.app).settings_window_closed();
                for font in state.fonts.iter() {
                    if !font.is_invalid() {
                        let _ = DeleteObject(HGDIOBJ(font.0));
                    }
                }
                state.canvas.clear();
                state.page.clear();
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

// --- the switch ------------------------------------------------------------------

/// The on/off control. A checkbox is a square with a tick in it, which reads as
/// "a thing on a list"; this is a state of the app, and a track with a knob on
/// one side or the other says which state in one glance.
///
/// Every pixel of it is rasterised by tiny-skia. GDI's `Ellipse` has no
/// anti-aliasing, and a stair-stepped circle sitting next to the smooth ones the
/// panel draws was most of what made the old switch look cheap.
struct Switch {
    hwnd: HWND,
    state: *mut State,
    on: bool,
    pressed: bool,
    hover: bool,
    focused: bool,
    /// Knob position: 0 is the left, 1 the right. It slides, because a control
    /// that teleports does not tell you which side it just moved to - unless the
    /// panel was opened with `reduced_motion`, in which case it teleports.
    anim: f32,
    reduced: bool,
}

/// One timer per switch, and only while the knob is moving.
const SWITCH_TIMER: usize = 0x5f_11;

unsafe extern "system" fn switch_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Switch;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let ctrl = &mut *ptr;
        match msg {
            WM_PAINT => {
                ctrl.paint();
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_GETDLGCODE => LRESULT((DLGC_BUTTON | DLGC_WANTCHARS) as isize),
            WM_LBUTTONDOWN => {
                let _ = SetFocus(Some(hwnd));
                ctrl.pressed = true;
                let _ = SetCapture(hwnd);
                ctrl.refresh();
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                ctrl.pressed = false;
                if hit(lp, hwnd) {
                    ctrl.flip();
                }
                ctrl.refresh();
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if !ctrl.hover {
                    ctrl.hover = true;
                    ctrl.refresh();
                }
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                LRESULT(0)
            }
            WM_MOUSELEAVE => {
                ctrl.hover = false;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_TIMER => {
                if wp.0 == SWITCH_TIMER && !ctrl.slide() {
                    let _ = KillTimer(Some(hwnd), SWITCH_TIMER);
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if wp.0 as u32 == VK_SPACE.0 as u32 {
                    ctrl.flip();
                    ctrl.refresh();
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_SETFOCUS => {
                ctrl.focused = true;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                ctrl.focused = false;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let _ = KillTimer(Some(hwnd), SWITCH_TIMER);
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

fn hit(lp: LPARAM, hwnd: HWND) -> bool {
    unsafe {
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let x = lp.0 as i16 as i32;
        let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
        x >= 0 && x < r.right && y >= 0 && y < r.bottom
    }
}

impl Switch {
    fn refresh(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// The answer came from somewhere other than a click - a change the app
    /// refused, a row that has to be put back - and the knob goes straight to it.
    /// Sliding there would be the panel arguing with itself.
    fn set(&mut self, on: bool) {
        self.on = on;
        self.anim = if on { 1.0 } else { 0.0 };
        self.refresh();
    }

    /// The only thing that turns the state into a new value: everything else in
    /// this file ends up here, so a click, a space bar and a row the app refused
    /// all move the knob the same way.
    fn flip(&mut self) {
        self.on = !self.on;
        unsafe {
            // The panel is told, not called: see [`State::moved`].
            (*self.state).moved();
        }
        if self.reduced {
            self.anim = if self.on { 1.0 } else { 0.0 };
            self.refresh();
            return;
        }
        unsafe {
            let _ = SetTimer(Some(self.hwnd), SWITCH_TIMER, 16, None);
        }
    }

    /// One frame of the slide. `false` means it has arrived.
    fn slide(&mut self) -> bool {
        let target = if self.on { 1.0 } else { 0.0 };
        let d = target - self.anim;
        if d.abs() < 0.02 {
            self.anim = target;
            self.refresh();
            return false;
        }
        // Exponential, not linear: it starts with the click and settles, which is
        // what a thing with mass does.
        self.anim += d * 0.35;
        self.refresh();
        true
    }

    unsafe fn paint(&self) {
        unsafe {
            let mut ps = std::mem::zeroed();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            if hdc.is_invalid() {
                return;
            }
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let scale = (*self.state).scale;
            let paper = solid(PAPER);
            FillRect(hdc, &r, paper);
            let _ = DeleteObject(paper.into());

            let w = (r.right - r.left).max(0) as u32;
            let h = (r.bottom - r.top).max(0) as u32;
            if let Some((bytes, bw, bh)) = switch_art(
                w,
                h,
                scale,
                self.anim,
                self.pressed,
                self.hover,
                self.focused,
            ) {
                blit(hdc, 0, 0, &bytes, bw, bh);
            }
            let _ = EndPaint(self.hwnd, &ps);
        }
    }
}

/// The switch, anti-aliased. The track carries the state and the knob stays the
/// same object on either end of it, so `on` and `off` weigh the same; the old
/// one swapped a pale pill for a block of ink, which made the enabled rows shout.
fn switch_art(
    w: u32,
    h: u32,
    scale: f32,
    anim: f32,
    pressed: bool,
    hover: bool,
    focused: bool,
) -> Option<(Vec<u8>, u32, u32)> {
    let mut pix = Pixmap::new(w, h)?;
    pix.fill(Color::TRANSPARENT);
    let s = scale;
    let tw = SWITCH_W * s;
    let th = SWITCH_H * s;
    // The track hugs the right edge of the box so it lines up with the field
    // column; the padding is all on the side the label is on.
    let right = w as f32;
    let left = (right - tw).max(0.0);
    let top = (h as f32 - th) / 2.0;
    let inset = 0.5 * s;

    let fill = mix(SWITCH_OFF, FLAX, anim.clamp(0.0, 1.0));
    let edge = mix(fill, INK, if hover || focused { 0.45 } else { 0.18 });
    let track = rrect(
        left + inset,
        top + inset,
        tw - inset * 2.0,
        th - inset * 2.0,
        th / 2.0 - inset,
    )?;
    pix.fill_path(
        &track,
        &paint(fill, if pressed { 235 } else { 255 }),
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    stroke(
        &mut pix,
        &track,
        paint(edge, 255),
        1.0 * s,
        Transform::identity(),
    );

    let radius = (th / 2.0 - 3.0 * s) * if pressed { 0.88 } else { 1.0 };
    let cx = left + th / 2.0 + anim * (tw - th);
    let cy = top + th / 2.0;
    let knob = circle_path(cx, cy, radius)?;
    pix.fill_path(
        &knob,
        &paint(FIELD, 255),
        tiny_skia::FillRule::Winding,
        Transform::identity(),
        None,
    );
    stroke(
        &mut pix,
        &knob,
        paint((0, 0, 0), 36),
        1.0 * s,
        Transform::identity(),
    );

    if focused {
        let o = 2.5 * s;
        if let Some(ring) = rrect(left - o, top - o, tw + o * 2.0, th + o * 2.0, th / 2.0 + o) {
            stroke(
                &mut pix,
                &ring,
                paint(FLAX, 255),
                1.4 * s,
                Transform::identity(),
            );
        }
    }

    Some((pix.data().to_vec(), w, h))
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> (u8, u8, u8) {
    let t = t.clamp(0.0, 1.0);
    let at = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    (at(a.0, b.0), at(a.1, b.1), at(a.2, b.2))
}

fn stroke(
    canvas: &mut Pixmap,
    path: &tiny_skia::Path,
    colour: Paint<'static>,
    width: f32,
    transform: Transform,
) {
    let stroke = Stroke {
        width,
        line_cap: LineCap::Round,
        ..Default::default()
    };
    canvas.stroke_path(path, &colour, &stroke, transform, None);
}

fn rrect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w, y, x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h, x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x, y + h, x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y, x, y, x + r, y);
    pb.close();
    pb.finish()
}

fn circle_path(cx: f32, cy: f32, r: f32) -> Option<tiny_skia::Path> {
    let mut pb = PathBuilder::new();
    let k = 0.552_284_7 * r;
    pb.move_to(cx - r, cy);
    pb.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    pb.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    pb.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    pb.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    pb.close();
    pb.finish()
}

/// Copy one rectangle of a premultiplied RGBA raster into a blue-first 32-bit
/// DIB of the same stride.
unsafe fn copy_region(
    dest: *mut c_void,
    pixels: &[u8],
    stride_px: usize,
    left: usize,
    top: usize,
    w: usize,
    h: usize,
) {
    if dest.is_null() || w == 0 || h == 0 {
        return;
    }
    let stride = stride_px * 4;
    if pixels.len() < stride * (top + h) || left + w > stride_px {
        return;
    }
    unsafe {
        let target = dest as *mut u8;
        for row in top..top + h {
            let from = row * stride + left * 4;
            std::ptr::copy_nonoverlapping(pixels.as_ptr().add(from), target.add(from), w * 4);
        }
        for row in top..top + h {
            let base = row * stride + left * 4;
            for px in 0..w {
                let at = base + px * 4;
                let blue = *target.add(at);
                let red = *target.add(at + 2);
                *target.add(at) = red;
                *target.add(at + 2) = blue;
            }
        }
    }
}

/// Copy a premultiplied RGBA raster onto the window. GDI's 32-bit DIB sections
/// are blue-first, so the pairs swap on the way in.
unsafe fn blit(hdc: HDC, x: i32, y: i32, pixels: &[u8], w: u32, h: u32) {
    unsafe {
        blit_scaled(hdc, x, y, pixels, w, h, w, h);
    }
}

/// The same copy, from a raster of `sw` by `sh` onto a destination of `dw` by
/// `dh`. A live drag frame is composed smaller than the card and stretched here
/// in one pass, which is what buys the frame its sixty-per-second footing.
unsafe fn blit_scaled(
    hdc: HDC,
    x: i32,
    y: i32,
    pixels: &[u8],
    sw: u32,
    sh: u32,
    dw: u32,
    dh: u32,
) {
    crate::app::blit_pixmap(hdc, x, y, pixels, sw, sh, dw, dh);
}

/// The keyboard's answer to a hover: the same warm accent the dial uses, on the
/// shape the control already has. It goes inside the edge, not outside it — a
/// control's window is a rectangle, so a ring drawn beyond its rounded plate
/// lands in the corner notches and reads as damage rather than focus.
unsafe fn focus_ring(hdc: HDC, r: &RECT, radius: i32, scale: f32) {
    unsafe {
        let pen = CreatePen(PS_SOLID, 1.max((1.4 * scale) as i32), rgb(FLAX));
        let old_pen = SelectObject(hdc, pen.into());
        let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
        let o = (2.0 * scale).round() as i32;
        let _ = RoundRect(
            hdc,
            o,
            o,
            r.right - o,
            r.bottom - o,
            (radius - o * 2).max(1),
            (radius - o * 2).max(1),
        );
        let _ = SelectObject(hdc, old_pen);
        let _ = SelectObject(hdc, old_brush);
        let _ = DeleteObject(pen.into());
    }
}

// --- the shortcut box ---------------------------------------------------------------

/// A combination the app can register as a hotkey. The box is only listening
/// while it is showing the keys you are holding; anything else is the value that
/// was already there, so clicking it and walking away changes nothing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    /// A shortcut: keys only, with the × band that clears it.
    Hotkey,
    /// The trigger binder: a key or a mouse button, and no × - a trigger always
    /// has a value.
    Trigger,
}

struct Key {
    hwnd: HWND,
    state: *mut State,
    mods: u32,
    vk: u32,
    /// The mouse button recorded by a trigger field (0 left, 1 right, 2 middle).
    button: u8,
    kind: KeyKind,
    capturing: bool,
    /// True for the click that started the recorder, so that the same click is
    /// not recorded as the answer.
    await_release: bool,
    /// The modifiers held at this moment while listening.
    pending: u32,
    focused: bool,
    /// What was in the box when it started listening, for Escape and Tab.
    before: (u32, u32),
}

unsafe extern "system" fn key_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Key;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let ctrl = &mut *ptr;
        match msg {
            WM_PAINT => {
                ctrl.paint();
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_GETDLGCODE => LRESULT(if ctrl.capturing {
                // Everything, including the keys the dialog manager would
                // otherwise spend on navigation - while this box is open,
                // Ctrl+Right is a candidate shortcut, not a way out.
                DLGC_WANTALLKEYS | DLGC_WANTARROWS | DLGC_WANTCHARS
            } else {
                DLGC_BUTTON | DLGC_WANTCHARS
            } as isize),
            // A focused button is clicked by the space bar by whoever runs the
            // dialog manager, so the keyboard and the mouse come through here.
            BM_CLICK => {
                ctrl.begin();
                ctrl.await_release = false;
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let _ = SetFocus(Some(hwnd));
                if ctrl.kind == KeyKind::Hotkey && ctrl.vk != 0 && ctrl.clear_hit(lp) {
                    // The × is the whole of "how do I switch this shortcut off".
                    ctrl.mods = 0;
                    ctrl.vk = 0;
                    ctrl.settled();
                } else if !ctrl.capturing {
                    ctrl.begin();
                    ctrl.await_release = ctrl.kind == KeyKind::Trigger;
                } else if ctrl.kind == KeyKind::Trigger && !ctrl.await_release {
                    // The recorder is already listening, so this click is the
                    // answer: a left-button binding.
                    ctrl.record_mouse(0);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if ctrl.kind == KeyKind::Trigger && ctrl.capturing {
                    // The click that opened the box has come back up; from now
                    // on a click is an answer.
                    ctrl.await_release = false;
                    ctrl.refresh();
                }
                LRESULT(0)
            }
            // 0x0204 / 0x0207: the right and middle button going down.
            0x0204 | 0x0207 => {
                if ctrl.kind == KeyKind::Trigger && ctrl.capturing && !ctrl.await_release {
                    let button = if msg == 0x0204 { 1 } else { 2 };
                    ctrl.record_mouse(button);
                }
                LRESULT(0)
            }
            0x0205 | 0x0208 => LRESULT(0),
            WM_KEYDOWN => {
                let key = wp.0 as u32;
                if ctrl.capturing {
                    ctrl.key_down(key);
                    LRESULT(0)
                } else if key == VK_SPACE.0 as u32 {
                    ctrl.begin();
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_KEYUP => {
                // Letting go of Ctrl takes the "Ctrl …" back off the box.
                if ctrl.capturing && is_modifier(wp.0 as u32) {
                    ctrl.pending = held_mods();
                    ctrl.refresh();
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_SETFOCUS => {
                ctrl.focused = true;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                ctrl.focused = false;
                if !ctrl.capturing {
                    ctrl.refresh();
                } else {
                    // Clicking away is not choosing a shortcut.
                    ctrl.restore();
                }
                LRESULT(0)
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                // A box destroyed mid-capture is the one way the hook could be
                // left standing down: closing the window is not an answer to the
                // question the box asked.
                if ctrl.capturing {
                    hook::resume();
                    crate::log!("hook: the desktop's again (the box asking was destroyed)");
                }
                drop(Box::from_raw(ptr));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

impl Key {
    /// Start listening. The modifiers already under the fingers are shown at
    /// once, so a click held through Ctrl reads as "Ctrl, and then?".
    unsafe fn begin(&mut self) {
        self.before = (self.mods, self.vk);
        self.capturing = true;
        self.pending = held_mods();
        // The next key is the answer to this box's question, so the app's own
        // hook stands down while the box listens: no gesture may start around
        // the click, and no key may be taken as an Escape from a dial. Said in
        // the log because "the box suspended the hook" is otherwise a claim
        // about a flag nobody can read from outside.
        hook::suspend();
        crate::log!("hook: stood down while a shortcut box listens");
        self.refresh();
    }

    fn leave(&mut self) {
        if self.capturing {
            hook::resume();
            crate::log!("hook: the desktop's again");
        }
        self.capturing = false;
        self.pending = 0;
        self.refresh();
    }

    /// The box is done with a value it changed itself, so the app is told. Not
    /// from here - `State::moved` defers it one message, the way every other row
    /// does, because this box is still inside its own key handling.
    unsafe fn settled(&mut self) {
        self.leave();
        match self.kind {
            KeyKind::Hotkey => (*self.state).moved(),
            KeyKind::Trigger => {
                (*self.state).try_trigger(self.mods, self.vk, self.button);
            }
        }
    }

    unsafe fn record_mouse(&mut self, button: u8) {
        self.mods = held_mods();
        self.vk = 0;
        self.button = button;
        self.settled();
    }

    fn set_trigger_combo(&mut self, mods: u32, vk: u32, button: u8) {
        self.mods = mods;
        self.vk = vk;
        self.button = button;
        self.before = (mods, vk);
        self.leave();
    }

    /// Show a combination that was decided somewhere else: after the app refused
    /// this box's change, the settings it is actually running with go back into
    /// the field. Nothing is left listening - a box mid-flight has not chosen the
    /// value it is holding, so it cannot keep holding one that was refused.
    fn set_combo(&mut self, mods: u32, vk: u32) {
        self.mods = mods;
        self.vk = vk;
        self.before = (mods, vk);
        self.leave();
    }

    fn restore(&mut self) {
        let (mods, vk) = self.before;
        self.mods = mods;
        self.vk = vk;
        self.leave();
    }

    unsafe fn key_down(&mut self, key: u32) {
        if self.kind == KeyKind::Trigger {
            // A trigger is whichever key or button comes first; Tab is a key
            // like any other here, and Escape still walks away.
            if key == VK_ESCAPE.0 as u32 {
                self.restore();
            } else if is_modifier(key) {
                self.pending = held_mods();
                self.refresh();
            } else {
                self.mods = held_mods();
                self.vk = key;
                self.button = 0;
                self.settled();
            }
            return;
        }
        if key == VK_ESCAPE.0 as u32 {
            self.restore();
        } else if key == VK_TAB.0 as u32 {
            // Tab stays navigation, so a box never traps the keyboard; the price
            // is that Ctrl+Tab cannot be recorded here.
            let backward = held_mods() & settings::MOD_SHIFT != 0;
            self.restore();
            (*self.state).focus_stop(self.hwnd, backward);
        } else if key == VK_BACK.0 as u32 || key == VK_DELETE.0 as u32 {
            self.mods = 0;
            self.vk = 0;
            self.settled();
        } else if is_modifier(key) {
            self.pending = held_mods();
            self.refresh();
        } else {
            self.mods = held_mods();
            self.vk = key;
            self.settled();
        }
    }

    /// The × band at the right end of the field.
    unsafe fn clear_hit(&self, lp: LPARAM) -> bool {
        unsafe {
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let x = lp.0 as i16 as i32;
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
            x >= r.right - (CLEAR_W * (*self.state).scale).round() as i32
                && y >= r.top
                && y < r.bottom
        }
    }

    fn refresh(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    unsafe fn paint(&self) {
        unsafe {
            let mut ps = std::mem::zeroed();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            if hdc.is_invalid() {
                return;
            }
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let scale = (*self.state).scale;

            // The rounded plate sits in a rectangular window the parent's clip
            // excludes from its own repaint, so the corner notches are this
            // paint's to fill: a key box sits on a card, and a notch left to
            // yesterday's pixels reads as a black bite after a maximize.
            let behind = solid(PAPER);
            FillRect(hdc, &r, behind);
            let _ = DeleteObject(behind.into());

            let field = solid(FIELD);
            let edge = CreatePen(
                PS_SOLID,
                1,
                rgb(if self.capturing || self.focused {
                    FLAX
                } else {
                    EDGE
                }),
            );
            let old_brush = SelectObject(hdc, field.into());
            let old_pen = SelectObject(hdc, edge.into());
            let radius = (RADIUS * 2.0 * scale) as i32;
            let _ = RoundRect(hdc, 0, 0, r.right, r.bottom, radius, radius);
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
            let _ = DeleteObject(field.into());
            let _ = DeleteObject(edge.into());

            let trigger = self.kind == KeyKind::Trigger;
            let clear = !trigger && self.vk != 0 && !self.capturing;
            let shown = if trigger {
                if self.capturing {
                    let mods = mod_names(self.pending).join("+");
                    if mods.is_empty() {
                        t("settings_key_capture").to_string()
                    } else {
                        format!("{mods}+…")
                    }
                } else {
                    trigger_text(self.mods, self.vk, self.button)
                }
            } else {
                key_text(self.capturing, self.pending, self.mods, self.vk)
            };
            let chosen = trigger || self.vk != 0 || (self.capturing && self.pending != 0);
            let text_box = RECT {
                left: (9.0 * scale).round() as i32,
                top: r.top,
                right: r.right
                    - if clear {
                        (CLEAR_W * scale).round() as i32
                    } else {
                        (9.0 * scale).round() as i32
                    },
                bottom: r.bottom,
            };
            (*self.state).text(
                hdc,
                &shown,
                F_TITLE,
                if chosen { INK } else { MUTED },
                text_box,
                DT_VCENTER | DT_LEFT,
            );

            if clear {
                // Drawn, not typed: a multiply of a font's × is a different shape
                // at every scale, and this one is two strokes.
                let arm = 3.5 * scale;
                let cx = (r.right as f32 - CLEAR_W * scale / 2.0).round();
                let cy = (r.bottom as f32 / 2.0).round();
                let cross = CreatePen(PS_SOLID, 1.max((1.3 * scale) as i32), rgb(MUTED));
                let old_pen = SelectObject(hdc, cross.into());
                for dy in [-arm, arm] {
                    let _ = MoveToEx(
                        hdc,
                        (cx - arm).round() as i32,
                        (cy + dy).round() as i32,
                        None,
                    );
                    let _ = LineTo(hdc, (cx + arm).round() as i32, (cy - dy).round() as i32);
                }
                let _ = SelectObject(hdc, old_pen);
                let _ = DeleteObject(cross.into());
            }
            if self.focused {
                focus_ring(hdc, &r, radius, scale);
            }
            let _ = EndPaint(self.hwnd, &ps);
        }
    }
}

/// The modifiers, in the order a label reads them.
fn mod_names(mods: u32) -> Vec<&'static str> {
    let mut names = Vec::new();
    if mods & settings::MOD_CONTROL != 0 {
        names.push("Ctrl");
    }
    if mods & settings::MOD_ALT != 0 {
        names.push("Alt");
    }
    if mods & settings::MOD_SHIFT != 0 {
        names.push("Shift");
    }
    if mods & settings::MOD_WIN != 0 {
        names.push("Win");
    }
    names
}

/// What the trigger's field says. A chord that ends in a mouse button names the
/// button in full: `Alt+Left` reads as the arrow key, and the field has the room.
fn trigger_text(mods: u32, vk: u32, button: u8) -> String {
    if vk != 0 {
        return settings::trigger_label(mods, vk, button);
    }
    let mut parts = mod_names(mods);
    parts.push(match button {
        1 => t("settings_wheel_right"),
        2 => t("settings_wheel_middle"),
        _ => t("settings_wheel_left"),
    });
    parts.join("+")
}

/// What the box says: the combination under the fingers, then the one that is
/// set, then what it takes to set one.
fn key_text(capturing: bool, pending: u32, mods: u32, vk: u32) -> String {
    if capturing {
        let names = mod_names(pending);
        if names.is_empty() {
            t("settings_key_capture").to_string()
        } else {
            format!("{} …", names.join("+"))
        }
    } else if vk == 0 {
        t("settings_key_unset").to_string()
    } else {
        settings::hotkey_label(mods, vk)
    }
}

/// The keys that are parts of a combination rather than one of their own.
fn is_modifier(key: u32) -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|vk| vk.0 as u32 == key)
}

/// The modifiers as they physically are, read off the keyboard instead of off
/// the message queue. A queue can be a key behind, and a shortcut recorded one
/// modifier short is a shortcut that does not do what its own label says.
unsafe fn held_mods() -> u32 {
    let mut mods = 0;
    if is_down(VK_CONTROL) {
        mods |= settings::MOD_CONTROL;
    }
    if is_down(VK_MENU) {
        mods |= settings::MOD_ALT;
    }
    if is_down(VK_SHIFT) {
        mods |= settings::MOD_SHIFT;
    }
    if is_down(VK_LWIN) || is_down(VK_RWIN) {
        mods |= settings::MOD_WIN;
    }
    mods
}

/// The high bit of the key state is the answer to "is it down now?".
fn is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) < 0 }
}

// --- the button ---------------------------------------------------------------------

/// Every word-shaped control on this window: a modifier cell, a button choice, a
/// stepper end, a language, a door. A stock button would do almost the same job,
/// but an owner-drawn one paints through its parent's `WM_DRAWITEM`, which a
/// print pass never asks for: a screenshot of this window would show blank
/// rectangles and tell nobody whether the panel is right.
struct Button {
    hwnd: HWND,
    parent: HWND,
    state: *mut State,
    id: i32,
    text: String,
    primary: bool,
    pressed: bool,
    focused: bool,
    /// A stepper end that has nothing left to give: the number is already at
    /// its floor or ceiling. It stays on the page so the row keeps its shape,
    /// but it says so in grey rather than pretending a click would move it.
    enabled: bool,
    /// The colour of what sits behind the button. The plate is a rounded rect
    /// inside a rectangular window, and the parent clips its repaint to the
    /// whole rectangle, so the four notches at the corners are painted by nobody
    /// else: this button fills them itself, or a maximize/restore leaves the
    /// pixels of whatever stood there before standing in them for good.
    behind: (u8, u8, u8),
}

unsafe extern "system" fn button_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Button;
        if ptr.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        let ctrl = &mut *ptr;
        match msg {
            WM_PAINT => {
                ctrl.paint();
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_GETDLGCODE => LRESULT((DLGC_BUTTON | DLGC_WANTCHARS) as isize),
            // Enter, from whoever answers DM_GETDEFID for the panel.
            BM_CLICK => {
                if ctrl.enabled {
                    ctrl.click();
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if !ctrl.enabled {
                    return LRESULT(0);
                }
                let _ = SetFocus(Some(hwnd));
                ctrl.pressed = true;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                ctrl.pressed = false;
                let inside = {
                    let mut r = RECT::default();
                    let _ = GetClientRect(hwnd, &mut r);
                    let x = lp.0 as i16 as i32;
                    let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
                    x >= 0 && x < r.right && y >= 0 && y < r.bottom
                };
                if inside && ctrl.enabled {
                    ctrl.click();
                } else {
                    ctrl.refresh();
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                // The space bar usually arrives as a click pair instead, from the
                // dialog manager; this is the path for when it does not.
                if wp.0 as u32 == VK_SPACE.0 as u32 {
                    if ctrl.enabled {
                        ctrl.click();
                    }
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wp, lp)
                }
            }
            WM_SETFOCUS => {
                ctrl.focused = true;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_KILLFOCUS => {
                ctrl.focused = false;
                ctrl.pressed = false;
                ctrl.refresh();
                LRESULT(0)
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(ptr));
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

impl Button {
    /// The same `WM_COMMAND` a real button sends, so the panel's answer to a
    /// click does not care which kind of button it came from.
    fn click(&self) {
        if !self.enabled {
            return;
        }
        unsafe {
            let _ = send(
                self.parent,
                WM_COMMAND,
                (BN_CLICKED as usize) << 16 | self.id as usize,
                self.hwnd.0 as isize,
            );
        }
    }

    /// A stepper end at its floor or ceiling: greyed and inert until the number
    /// moves away from the edge.
    fn set_enabled(&mut self, on: bool) {
        if self.enabled == on {
            return;
        }
        self.enabled = on;
        if !on {
            self.pressed = false;
        }
        self.refresh();
    }

    fn refresh(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }

    /// The word on the face, changed under it. The button paints its own text, so
    /// this is the string it will draw; it is also the window text, which is what
    /// a screen reader and the screenshot test read out of the control - so both
    /// go, or the caption is a lie in one of them.
    unsafe fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        let wide = wstr(text);
        let _ = SetWindowTextW(self.hwnd, PCWSTR(wide.as_ptr()));
        self.refresh();
    }

    unsafe fn paint(&self) {
        unsafe {
            let mut ps = std::mem::zeroed();
            let hdc = BeginPaint(self.hwnd, &mut ps);
            if hdc.is_invalid() {
                return;
            }
            let mut r = RECT::default();
            let _ = GetClientRect(self.hwnd, &mut r);
            let scale = (*self.state).scale;
            // The notches first. See `behind`: the parent's clip excludes this
            // whole rectangle, so what the corners hold is what this paint puts
            // there - anything left over from an earlier layout would outlive
            // every repaint that follows.
            let behind = solid(self.behind);
            FillRect(hdc, &r, behind);
            let _ = DeleteObject(behind.into());
            let bg = if self.primary {
                if self.pressed {
                    (0x1B, 0x20, 0x2A)
                } else {
                    SLATE
                }
            } else if self.pressed {
                (0xEC, 0xEF, 0xF2)
            } else {
                FIELD
            };
            let brush = solid(bg);
            let pen = CreatePen(PS_SOLID, 1, rgb(if self.primary { SLATE } else { EDGE }));
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, pen.into());
            let radius = (RADIUS * 2.0 * scale) as i32;
            let _ = RoundRect(hdc, 0, 0, r.right, r.bottom, radius, radius);
            let _ = SelectObject(hdc, old_brush);
            let _ = SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
            let _ = DeleteObject(pen.into());

            (*self.state).text(
                hdc,
                &self.text,
                F_BUTTON,
                if self.primary {
                    (0xFF, 0xFF, 0xFF)
                } else if self.enabled {
                    INK
                } else {
                    // The same grey as the sentence under a card: at the floor or
                    // the ceiling there is nothing this end can do.
                    MUTED
                },
                r,
                DT_CENTER | DT_VCENTER,
            );
            if self.focused {
                // The dotted rectangle Windows draws is invisible on a filled
                // button; a ring on the shape itself reads as focus either way.
                focus_ring(hdc, &r, radius, scale);
            }
            let _ = EndPaint(self.hwnd, &ps);
        }
    }
}

// --- fonts and messages --------------------------------------------------------------

/// `SendMessageW` in these bindings takes optional parameters; every message
/// this window sends has real ones.
unsafe fn send(hwnd: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wp)), Some(LPARAM(lp))).0 }
}

fn make_fonts(scale: f32) -> [HFONT; 3] {
    // The face comes from the language in force, which knows whether its own
    // strings need Chinese glyphs and whether a pack asked for something else.
    let face = crate::i18n::ui_face();
    [
        font(scale, 13.0, FW_NORMAL.0 as i32, face),
        font(scale, 11.5, FW_NORMAL.0 as i32, face),
        font(scale, 13.0, FW_SEMIBOLD.0 as i32, face),
    ]
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

/// One log line per string the section on paper has no room for. A shortened
/// sentence and a truncated one look the same to somebody who does not read the
/// language, so the alternative to guessing is to say which string went over its
/// band. It runs again on every turn of the left column: the words of a section
/// nobody has clicked on yet are words nobody has seen cut in half yet.
fn warn_cut(state: &State) {
    let l = state.layout;
    let cards = cards_of(l.section);
    let mut rows: Vec<(&str, String, RECT, usize)> = Vec::new();
    // The four names down the left, in the band the controls in the cards end on.
    for (at, &key) in SEC_NAME.iter().enumerate() {
        rows.push((key, t(key).to_string(), l.nav[at], F_BUTTON));
    }
    // A card is two lines beside a control. The note is the longer of the two and
    // has the least room: the text column stops where the control column starts.
    for (at, card) in cards.iter().copied().enumerate().take(l.row_count) {
        // The dial's compact strip paints a short name in a narrow column and
        // keeps no sentence under it; measuring it with the card face would
        // report a cut that is not there.
        let strip = l.section == SEC_DIAL && at > 0;
        rows.push((
            card.title,
            t(card.title).to_string(),
            l.title[at],
            if strip { F_NOTE } else { F_BUTTON },
        ));
        if strip {
            continue;
        }
        let note = state.note_text(l.section, at);
        if !note.is_empty() {
            rows.push((
                if card.note.is_empty() {
                    "the log's path"
                } else {
                    card.note
                },
                note,
                l.note[at],
                F_NOTE,
            ));
        }
    }
    // Then the words the controls of this section carry. The recorder shows a
    // chord, which is longer than any single label the page has to fit.
    match l.section {
        SEC_KEYS => {
            rows.push((
                "trigger",
                trigger_text(settings::MOD_CONTROL | settings::MOD_SHIFT, 0, 2),
                l.trigger_field,
                F_BUTTON,
            ));
        }
        SEC_LANG => {
            rows.push((
                "lang_folder",
                t("lang_folder").to_string(),
                l.folder,
                F_BUTTON,
            ));
            for (at, (_, name)) in state.lang_tags.iter().enumerate().take(l.lang_count) {
                rows.push(("_name", name.clone(), l.lang[at], F_BUTTON));
            }
        }
        _ => {
            rows.push((
                "settings_open_log",
                t("settings_open_log").to_string(),
                l.log_door,
                F_BUTTON,
            ));
            rows.push((
                "the version",
                env!("CARGO_PKG_VERSION").to_string(),
                l.value_version,
                F_TITLE,
            ));
            rows.push((
                "the license",
                LICENSE_SHOWN.to_string(),
                l.value_license,
                F_TITLE,
            ));
        }
    }
    let rows = rows;
    // A caption that reaches the rounded corner of a button reads as cut even
    // when the last letter is still there.
    let button_room = (RADIUS * 2.0 * state.scale).round() as i32;
    unsafe {
        let dc = CreateCompatibleDC(None);
        if dc.is_invalid() {
            return;
        }
        for (key, text, rect, font) in rows {
            let wide: Vec<u16> = text.encode_utf16().collect();
            let old = SelectObject(dc, HGDIOBJ(state.fonts[font].0));
            let mut size = SIZE::default();
            let measured = GetTextExtentPoint32W(dc, &wide, &mut size).as_bool();
            SelectObject(dc, old);
            let spare = if font == F_BUTTON { button_room } else { 0 };
            let band = rect.right - rect.left - spare;
            if measured && band > 0 && size.cx > band {
                crate::log!(
                    "the panel has no room for {key}: {cx}px in {band}px",
                    cx = size.cx
                );
            }
        }
        let _ = DeleteDC(dc);
    }
}

/// The two decisions this window will not let anybody make, and the i18n key for
/// what it says when it refuses one. They are checked here rather than inside the
/// app because the answer is a row of this panel going back where it came from,
/// and only this window knows which row.
///
/// A shortcut with no modifier is a bare keypress the app swallows; a dial with
/// no modifier opens on every drag anywhere on the desktop. Both are worse than
/// the change somebody meant, which is why the row is refused instead of clamped.
///
/// The machine falls back to Alt for a hand-written file; a panel does not offer
/// a cell that silently means something else.
fn refuse(draft: &settings::Draft) -> Option<&'static str> {
    // A function key is already its own meaning, so `F13` alone counts as a
    // shortcut; anything else needs a modifier standing next to it.
    let bare = |vk: u32| vk != 0 && !(0x70..=0x87).contains(&vk);
    if (draft.fav.1 != 0 && draft.fav.0 == 0 && bare(draft.fav.1))
        || (draft.panel.1 != 0 && draft.panel.0 == 0 && bare(draft.panel.1))
    {
        return Some("settings_no_mods");
    }
    // Two rows asking for the same combination cannot both be registered: the
    // second loses to the first and the panel would roll the whole draft back
    // with a message about somebody else's application. Saying so here names the
    // actual mistake, and there is only one of it to fix.
    let pairs = [draft.fav, draft.panel];
    for (at, one) in pairs.iter().enumerate() {
        if one.1 == 0 {
            continue;
        }
        if pairs[..at]
            .iter()
            .any(|other| other.1 == one.1 && other.0 == one.0)
        {
            return Some("settings_hotkey_dup");
        }
    }
    if draft.trigger_mods
        & (settings::MOD_CONTROL | settings::MOD_ALT | settings::MOD_SHIFT | settings::MOD_WIN)
        == 0
    {
        return Some("settings_wheel_needs_mod");
    }
    None
}

fn message(parent: HWND, text: &str) {
    let text = wstr(text);
    let title = wstr(t("settings_title"));
    unsafe {
        let _ = MessageBoxW(
            Some(parent),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn wstr(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

// --- what this file promises ---------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The measure is the window: `section_h` is `measure` walked with the same
    /// constants, so no section can outgrow its own frame - and the frame is as
    /// tall as the tallest of them, which is what a click on the left column
    /// cannot move. Checked at the scales a person actually runs.
    #[test]
    fn the_window_is_as_tall_as_its_tallest_section() {
        // Three languages is what the folder offers on a fresh install: the
        // system plus the two the program ships.
        const LANGS: usize = 3;
        let rows = lang_rows(LANGS);
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            let width = (LOGICAL_W * scale).round() as i32;
            for section in 0..SEC_COUNT {
                let l = measure(
                    section,
                    width,
                    (logical_h(rows) * scale).round() as i32,
                    scale,
                    LANGS,
                );
                // 轮盘 measures its own desk: one card as tall as the client, a
                // list card and a strip beside it. `section_h` still describes it
                // for the frame's comparison below, but its cards are not a
                // top-to-bottom column of equal rows any more.
                if section == SEC_DIAL {
                    continue;
                }
                let cards = &l.card[..l.row_count];
                // The walk and the sum are the same content; they can only part
                // company by the rounding of their parts. A card missing from
                // `section_h`, or one the painter stopped drawing, is off by tens.
                let measured = cards[cards.len() - 1].bottom as f32 + PAD_BOTTOM * scale;
                assert!(
                    (measured - section_h(section, rows) * scale).abs() <= 1.0,
                    "section {section} is not the height it claims at scale {scale}: the \
                     cards end at {measured}, the section is {}",
                    section_h(section, rows) * scale
                );
                // Cards go down the page and never run into each other.
                for pair in cards.windows(2) {
                    assert!(
                        pair[1].top >= pair[0].bottom,
                        "cards cross at scale {scale}"
                    );
                }
                // A card holds its two lines: the name below its top edge, the
                // sentence above its bottom, and never one over the other.
                for at in 0..l.row_count {
                    assert!(l.title[at].top >= l.card[at].top);
                    assert!(l.title[at].bottom <= l.note[at].top);
                    assert!(l.note[at].bottom <= l.card[at].bottom);
                }
            }
            // One frame for five sections. The bottom edge does not move when a
            // name is clicked, so nothing the pointer was aimed at drops away.
            for section in 0..SEC_COUNT {
                assert!(
                    section_h(section, rows) <= logical_h(rows),
                    "section {section} does not fit the window"
                );
            }
            // The dial's preview is the tallest thing on the page: it is the one
            // section that draws a picture rather than a row of controls, so it
            // sets today's height. It is a count, not a list: the frame holds the
            // same picture whatever the desktop is actually running.
            assert_eq!(logical_h(rows), section_h(SEC_DIAL, rows));
        }
    }

    /// The two columns are one page: the navigation is as wide as the control
    /// column it faces, the cards start one gutter to its right, and every control
    /// keeps the width it was measured with on the single page this came from.
    #[test]
    fn the_columns_line_up_on_lines_the_panel_already_drew() {
        const LANGS: usize = 3;
        for scale in [1.0f32, 1.5] {
            let width = (LOGICAL_W * scale).round() as i32;
            let height = (logical_h(lang_rows(LANGS)) * scale).round() as i32;
            let keys = measure(SEC_KEYS, width, height, scale, LANGS);
            // The column of names is its own measure now: narrow enough to hand
            // the room it gives up to the cards, wide enough for the longest name.
            let nav_w = keys.nav[0].right - keys.nav[0].left;
            let want_nav = (NAV_W * scale).round() as i32;
            assert!(
                nav_w.abs_diff(want_nav) <= 1,
                "the nav is {nav_w} wide, not {want_nav}"
            );
            // The names start where the page starts, at the same padding the
            // single page kept from its edge.
            assert_eq!(keys.nav[0].left, (PAD * scale).round() as i32);
            assert_eq!(keys.nav[0].top, (PAD * scale).round() as i32);

            // A card is the content column with a padding taken off each end, so
            // the control inside it ends one padding short of the card.
            let inset = (CARD_PAD * scale).round() as i32;
            assert!(keys.card[0].right - keys.trigger_field.right >= inset - 1);
            assert!(keys.trigger_field.left >= keys.title[0].right);
            // The recorder takes the whole content column, like the two shortcut
            // fields under it: one column, three rows.
            assert_eq!(keys.trigger_field.left, keys.field_fav.left);
            assert_eq!(keys.trigger_field.right, keys.field_fav.right);
            assert_eq!(keys.field_fav.left, keys.field_panel.left);
            assert_eq!(keys.field_fav.right, keys.field_panel.right);
            assert!(keys.field_fav.top > keys.trigger_field.bottom);
            assert!(keys.field_panel.top > keys.field_fav.bottom);
            // 关于 reads its values in the same column 快捷键 puts its boxes in.
            let about = measure(SEC_ABOUT, width, height, scale, LANGS);
            assert_eq!(about.value_version.left, about.log_door.left);
            assert!(about.value_license.right <= about.card[3].right);
            assert!(about.switch_motion.right <= about.card[2].right + 1);

            // The window can be dragged down to its 620 floor: the dial's desk
            // folds into one column, keeps its 260-pixel picture and its list,
            // and the paper scrolls to the rest.
            let narrow = (MIN_CLIENT_W * scale).round() as i32;
            let small = measure(
                SEC_DIAL,
                narrow,
                (MIN_CLIENT_H * scale) as i32,
                scale,
                LANGS,
            );
            assert!(
                small.dial_preview.right - small.dial_preview.left
                    >= (260.0 * scale) as i32 - 1,
                "the dial still has room at the narrowest width"
            );
            assert!(
                small.title[DIAL_PREVIEW_ROW].right <= small.card[DIAL_PREVIEW_ROW].right,
                "the sentences run the width of their card"
            );
            assert!(
                small.dial_preview.bottom <= small.dial_list_card.top,
                "the picture and the list keep to their own bands"
            );
            // The square is centred in the folded column rather than pinned to
            // its left edge, so the air on both sides is the same.
            assert!(
                (small.dial_preview.left - small.card[DIAL_PREVIEW_ROW].left)
                    .abs_diff(small.card[DIAL_PREVIEW_ROW].right - small.dial_preview.right)
                    <= 1,
                "the picture is not centred in its column"
            );
            assert!(
                small.dial_preview.bottom <= small.card[DIAL_PREVIEW_ROW].bottom,
                "the picture stays inside its card"
            );
            // A door is centred in its card, which at this height is sixteen
            // logical pixels of air above it - the same air a field has.
            let air = (about.log_door.top - about.card[1].top) as f32 / scale;
            assert!(
                (air - 16.0).abs() <= 0.5,
                "the log door sits {air} pixels into its card"
            );
        }
        // The rows are the same heights the single page walked, so nothing here
        // needed a new number: a card is one row, and a card's two lines are the
        // two lines that row already had.
        assert_eq!(CARD_H, ITEM_H + ITEM_GAP * 2.0);
        assert_eq!(GUTTER, GROUP_GAP);
        assert!(NAV_W < FIELD_W, "the nav gives its surplus width to the cards");
        assert_eq!(CARD_PAD, GAP_LABEL);
        // The width is all spoken for: two page paddings, the two columns, the
        // gutter, a padding inside each card and the air before a control. What
        // is left is the band the sentences are cut to, so widening any column
        // takes words off the page - which is what `warn_cut` is there to hear.
        let text = 760.0
            - (PAD * 2.0 + NAV_W + GUTTER + CARD_PAD * 2.0 + GAP_LABEL + FIELD_W);
        assert!(
            (text - 286.0).abs() < 0.01,
            "the note band is {text} logical pixels, not the 286 the sentences were cut for"
        );
    }

    /// The floor is a size the corner can actually be dragged DOWN to, and it is
    /// converted at the DPI the window is on - not at the one it was opened at.
    /// Both halves of that promise are checked here: the opening height is taller
    /// than the floor, so the handle has somewhere to travel, and the track grows
    /// with the monitor's scale instead of being left at 100%.
    #[test]
    fn the_floor_is_a_size_the_corner_can_be_dragged_down_to() {
        assert_eq!(MIN_CLIENT_W, 620.0);
        assert_eq!(MIN_CLIENT_H, 440.0);
        assert!(
            MIN_CLIENT_H < super::LOGICAL_H_PANEL,
            "the floor and the opening size are the same window"
        );
        let (w, h) = super::min_track_size(96);
        assert!(w >= MIN_CLIENT_W as i32, "the track is {w} wide");
        assert!(h >= MIN_CLIENT_H as i32, "the track is {h} high");
        assert!(
            w <= MIN_CLIENT_W as i32 + 32,
            "the frame adds {} to the width",
            w - MIN_CLIENT_W as i32
        );
        assert!(
            h <= MIN_CLIENT_H as i32 + 56,
            "the frame adds {} to the height",
            h - MIN_CLIENT_H as i32
        );
        let (w150, h150) = super::min_track_size(144);
        assert!(
            w150 >= (MIN_CLIENT_W * 1.5) as i32,
            "{w150} is not 150% of the floor"
        );
        assert!(
            h150 >= (MIN_CLIENT_H * 1.5) as i32,
            "{h150} is not 150% of the floor"
        );
    }

    /// The list starts on its search field's floor: nothing sits between them,
    /// so the card holds window rows rather than blank paper.
    #[test]
    fn the_list_starts_on_its_own_search_field() {
        for scale in [1.0f32, 1.5] {
            let w = (MIN_CLIENT_W * scale).round() as i32;
            let h = (MIN_CLIENT_H * scale).round() as i32;
            let l = measure(SEC_DIAL, w, h, scale, 3);
            let air = (l.dial_list.top - l.dial_search.bottom) as f32 / scale;
            assert!(
                air <= 8.0,
                "the list keeps the field's floor, {air} pixels off"
            );
            let step = (POOL_ROW_STEP * scale).round() as i32;
            let lines = (l.dial_list.bottom - l.dial_list.top).max(0) / step;
            assert!(
                lines >= 5,
                "the list shows {lines} lines, fewer than two headings and three windows"
            );
        }
    }

    /// The refresh button shares the search field's band: a square the field's
    /// height, flush with the list's right edge, a gap to the field's right, and
    /// the edit kept inside the frame the button now shares the row with. Both
    /// the wide desk and the folded page lay it out the same way.
    #[test]
    fn the_refresh_button_sits_beside_the_search_field() {
        for (w, h) in [(1120, 780), (MIN_CLIENT_W as i32, MIN_CLIENT_H as i32)] {
            let scale = 1.0f32;
            let l = measure(SEC_DIAL, w, h, scale, 3);
            let side = (FIELD_H * scale).round() as i32;
            assert_eq!(l.dial_refresh.right - l.dial_refresh.left, side);
            assert_eq!(l.dial_refresh.bottom - l.dial_refresh.top, side);
            assert_eq!(l.dial_refresh.top, l.dial_search.top);
            assert_eq!(l.dial_refresh.right, l.dial_list.right);
            assert!(
                l.dial_search.right < l.dial_refresh.left,
                "the field and the button overlap"
            );
            assert!(
                l.dial_search_input.right <= l.dial_search.right,
                "the edit runs out of its frame"
            );
        }
    }

    /// The floor of the window is the size at which the dial still keeps every
    /// promise it makes: a picture no smaller than 260 logical pixels, both list
    /// headings and three window rows, and the whole parameter strip - by folding
    /// the desk into one column and letting the paper scroll under the glass.
    #[test]
    fn the_floor_is_the_dial_with_its_picture_at_the_picture_floor() {
        for scale in [1.0f32, 1.5] {
            let w = (MIN_CLIENT_W * scale).round() as i32;
            let h = (MIN_CLIENT_H * scale).round() as i32;
            let l = measure(SEC_DIAL, w, h, scale, 3);
            // The paper is taller than the glass at the floor, and that is how
            // the promises are kept: the rest is reached with the wheel.
            assert!(
                l.scroll_max > 0.0,
                "the folded page has nothing to scroll at the floor"
            );
            let preview = l.dial_preview.right - l.dial_preview.left;
            assert!(
                preview >= (260.0 * scale) as i32 - 1,
                "the picture is {preview} wide at the floor"
            );
            assert_eq!(
                l.dial_preview.bottom - l.dial_preview.top,
                preview,
                "the picture stays square"
            );
            assert!(
                l.dial_preview.bottom <= l.card[DIAL_PREVIEW_ROW].bottom,
                "the picture stays inside its card at the floor"
            );
            assert!(
                l.dial_strip.bottom <= l.card[DIAL_PREVIEW_ROW].bottom,
                "the filmstrip stays inside its card at the floor"
            );
            // The list starts on the field's own floor: at most eight logical
            // pixels of air between the box and the first heading.
            let air = (l.dial_list.top - l.dial_search.bottom) as f32 / scale;
            assert!(air <= 8.0, "the list starts {air} pixels under the field");
            // The edit is inside the frame, with the magnifier's strip to its
            // left, so the field reads as a box and the text clears the lens.
            assert!(
                l.dial_search_input.left
                    >= l.dial_search.left + (DIAL_SEARCH_ICON * scale) as i32,
                "the edit starts clear of the magnifier"
            );
            assert!(
                l.dial_search_input.right <= l.dial_search.right,
                "the edit runs outside its frame"
            );
            let row = (POOL_ROW_STEP * scale).round() as i32;
            let lines = (l.dial_list.bottom - l.dial_list.top).max(0) / row;
            // The two section headings are always on the paper, so the rows
            // that can carry a window are what is left after them.
            assert!(
                lines >= 5,
                "the window list shows {} rows at the floor, fewer than three",
                lines.saturating_sub(2)
            );
            // One column: the list sits under the picture, sharing its edges.
            assert_eq!(l.dial_list_card.left, l.card[DIAL_PREVIEW_ROW].left);
            assert_eq!(l.dial_list_card.right, l.card[DIAL_PREVIEW_ROW].right);
            assert!(
                l.dial_list_card.top >= l.card[DIAL_PREVIEW_ROW].bottom,
                "the list runs into the picture"
            );
            assert!(
                l.nav[SEC_COUNT - 1].bottom <= h,
                "the last name ends {} past the client",
                l.nav[SEC_COUNT - 1].bottom - h
            );
            // Scrolled to the end, the whole strip is on the glass.
            let end = measure_tuned(SEC_DIAL, w, h, scale, 3, l.scroll_max);
            assert!(
                end.dial_param_card.top >= 0,
                "the strip starts {} pixels above the glass",
                -end.dial_param_card.top
            );
            assert!(
                end.dial_param_card.bottom <= h,
                "the strip runs {} pixels past the glass",
                end.dial_param_card.bottom - h
            );
        }
    }

    /// The language buttons divide the card they are given, and a folder holding a
    /// new one adds a row inside that card rather than narrowing the names - up to
    /// the three rows the frame can hold.
    #[test]
    fn the_language_row_divides_what_it_is_given() {
        let width = LOGICAL_W.round() as i32;
        let three = measure(SEC_LANG, width, (logical_h(1) * 1.0).round() as i32, 1.0, 3);
        // The grid is inset by the card's own padding, so the first name starts
        // where the card's own name does.
        assert_eq!(three.lang[0].left, three.title[0].left);
        assert!(three.lang[1].left >= three.lang[0].right);
        assert!(three.lang[2].right <= width - PAD as i32 + SEG_GAP as i32);
        // A row holds `SEG_PER_ROW`, so it is the fifth choice - not the fourth -
        // that starts a second row.
        let four = measure(SEC_LANG, width, (logical_h(1) * 1.0).round() as i32, 1.0, 4);
        assert_eq!(lang_rows(4), 1);
        assert_eq!(four.lang[3].top, four.lang[0].top);
        let five = measure(SEC_LANG, width, (logical_h(2) * 1.0).round() as i32, 1.0, 5);
        assert_eq!(lang_rows(5), 2);
        assert_eq!(five.lang[4].top, five.lang[0].bottom + SEG_ROW_GAP as i32);
        // The card grew by exactly the row it added, and the sentence under the
        // names moved with it rather than being pushed out of the card.
        assert_eq!(
            five.note[0].top - three.note[0].top,
            (SEG_H + SEG_ROW_GAP).round() as i32
        );
        assert!(five.note[0].bottom <= five.card[0].bottom);
        for at in 0..4 {
            assert!(four.lang[at].bottom <= four.card[0].bottom);
        }
        // Past the three rows the frame holds, more languages are a narrower grid
        // rather than a taller window - the clamp is the promise.
        assert_eq!(lang_rows(0), 1);
        assert_eq!(lang_rows(3), 1);
        assert_eq!(lang_rows(99), MAX_LANG_ROW);
        assert_eq!(logical_h(MAX_LANG_ROW), section_h(SEC_DIAL, MAX_LANG_ROW));
    }

    /// The box tells "nothing is set" from "I am listening" from "this is what is
    /// set", and while it listens it shows the modifiers under your fingers -
    /// which is the only proof that a shortcut was heard at all.
    #[test]
    fn the_shortcut_box_says_what_it_means() {
        assert_eq!(key_text(false, 0, 0, 0), t("settings_key_unset"));
        assert_eq!(key_text(false, 0, settings::MOD_CONTROL, 0x48), "Ctrl+H");
        assert_eq!(
            key_text(true, 0, settings::MOD_CONTROL, 0x48),
            t("settings_key_capture")
        );
        assert_eq!(
            key_text(true, settings::MOD_CONTROL | settings::MOD_SHIFT, 0, 0),
            "Ctrl+Shift …"
        );
        assert_eq!(
            settings::hotkey_label(settings::MOD_WIN | settings::MOD_ALT, 0x54),
            "Alt+Win+T"
        );
    }

    /// The only two reasons a row of this panel goes back where it came from.
    /// Everything else the app takes and clamps, because a number that is too big
    /// is still the change you meant; these two are not - they leave the desktop
    /// behaving like neither the old settings nor the new ones.
    #[test]
    fn the_two_rows_that_would_strand_the_desktop_are_refused() {
        assert_eq!(refuse(&settings::Draft::default()), None);
        // A bare letter as a global shortcut swallows that key everywhere.
        let bare = settings::Draft {
            fav: (0, 0x41),
            ..settings::Draft::default()
        };
        assert_eq!(refuse(&bare), Some("settings_no_mods"));
        // A function key is already its own meaning, so it may stand alone.
        let function = settings::Draft {
            fav: (0, 0x74),
            ..settings::Draft::default()
        };
        assert_eq!(refuse(&function), None);
        // "No shortcut" is a thing the user may decide, and it steals nothing.
        let unset = settings::Draft {
            fav: (0, 0),
            panel: (0, 0),
            ..settings::Draft::default()
        };
        assert_eq!(refuse(&unset), None);
        // The trigger is the only thing that opens the dial now, so an empty set
        // of modifiers is a dial that can never open: that has to be refused.
        let naked = settings::Draft {
            trigger_mods: 0,
            ..settings::Draft::default()
        };
        assert_eq!(refuse(&naked), Some("settings_wheel_needs_mod"));
        // Any one of the four counts, and there is no separate switch to forget.
        for bit in [
            settings::MOD_CONTROL,
            settings::MOD_ALT,
            settings::MOD_SHIFT,
            settings::MOD_WIN,
        ] {
            let one = settings::Draft {
                trigger_mods: bit,
                ..settings::Draft::default()
            };
            assert_eq!(refuse(&one), None);
        }
    }
}

/// The search field's rule, tested without a window: the list is narrowed to the
/// rows whose title, executable or application contains the text, folded to one
/// case, and an empty field is every row.
#[cfg(test)]
mod sizing_tests {
    use super::{measure, DIAL_PREVIEW_ROW, SEC_DIAL};

    /// The size the panel opens at (1120×780 logical at 100%) has to give the
    /// picture the 460 logical pixels the dial is read at, and the list beside it
    /// eight rows - the complaint that the old page showed one.
    #[test]
    fn the_panel_opens_with_a_picture_wide_enough_to_read() {
        let layout = measure(SEC_DIAL, 1120, 780, 1.0, 3);
        let side = layout.dial_preview.right - layout.dial_preview.left;
        assert!(side >= 460, "the picture opens at {side}, not 460");
        assert_eq!(
            side,
            layout.dial_preview.bottom - layout.dial_preview.top,
            "the picture is square"
        );
        assert!(
            layout.dial_preview.bottom <= layout.card[DIAL_PREVIEW_ROW].bottom,
            "the picture stays inside its card"
        );
        let rows = layout.dial_list.bottom - layout.dial_list.top;
        let step = 32;
        assert!(
            rows >= 8 * step,
            "the list opens {rows} high - fewer than eight rows"
        );
    }

    /// The default size on a 1080p screen at 100% is the size the picture needs;
    /// at 125% the same screen simply has less logical room, so the panel is
    /// clamped instead of hanging off the bottom.
    #[test]
    fn the_panel_opens_at_the_size_the_screen_has_room_for() {
        let (w, h) = super::panel_open_size((0.0, 0.0), 1.0, 1920, 1040, super::MIN_CLIENT_H);
        assert_eq!((w, h), (1120.0, 780.0));
        let (w, h) = super::panel_open_size((0.0, 0.0), 1.25, 1536, 1040, super::MIN_CLIENT_H);
        assert_eq!(w, 1120.0);
        assert!(h <= 1040.0 / 1.25, "the panel is taller than the work area");
        // A remembered size is honoured, and still clamped to the screen. The
        // short screen has 712 logical pixels of work area, so that is the
        // tallest whole page that fits.
        let (w, h) = super::panel_open_size((1200.0, 900.0), 1.0, 1366, 728, super::MIN_CLIENT_H);
        assert_eq!(w, 1200.0, "1200 fits on a 1366-wide work area");
        assert_eq!(h, 712.0, "the page opens at the work area's height");
    }

    /// On a smaller screen the panel is clamped to the work area rather than
    /// hanging off the bottom. The picture is smaller, not clipped.
    #[test]
    fn a_short_screen_keeps_the_whole_page_on_the_paper() {
        let layout = measure(SEC_DIAL, 900, 816, 1.0, 3);
        assert!(layout.dial_preview.right - layout.dial_preview.left >= 110);
        assert!(layout.dial_preview.bottom <= layout.card[DIAL_PREVIEW_ROW].bottom);
    }
}

/// The search field's rule, tested without a window: the list is narrowed to the
/// rows whose title, executable or application contains the text, folded to one
/// case, and an empty field is every row.
#[cfg(test)]
mod search_tests {
    use super::{filter_rows, Row};
    use windows::Win32::UI::WindowsAndMessaging::HICON;

    fn row(title: &str, exe: &str, app: &str) -> Row {
        Row {
            hwnd: 0,
            fingerprint: format!("{exe}:{title}"),
            exe: exe.to_string(),
            app: app.to_string(),
            title: title.to_string(),
            tag: String::new(),
            favourite: false,
            on_dial: false,
            off_dial: false,
            excluded: false,
            icon: HICON::default(),
        }
    }

    #[test]
    fn search_matches_title_executable_and_application_in_any_case() {
        let all = vec![
            row("Outlook - Inbox", "outlook.exe", "Outlook"),
            row("写周报 - 记事本", "notepad.exe", "Notepad"),
            row("Terminal", "WindowsTerminal.exe", "Windows Terminal"),
        ];
        assert_eq!(filter_rows(&all, "INBOX").len(), 1, "title, folded");
        assert_eq!(filter_rows(&all, "notepad").len(), 1, "executable");
        assert_eq!(filter_rows(&all, "windows term").len(), 1, "application");
        assert_eq!(filter_rows(&all, "周报").len(), 1, "a Chinese title");
        assert_eq!(filter_rows(&all, "window").len(), 1, "a row that matches twice is one row");
    }

    #[test]
    fn an_empty_search_keeps_every_window() {
        let all = vec![row("A", "a.exe", "A"), row("B", "b.exe", "B")];
        assert_eq!(filter_rows(&all, "").len(), 2);
        assert_eq!(filter_rows(&all, "   ").len(), 2, "spaces are not a filter");
        assert!(filter_rows(&all, "nothing like this").is_empty());
    }

    /// The list is two sections with a heading each, and the second heading is
    /// there even when nothing is under it: an empty "未上轮盘" is a statement
    /// about the dial, not a reason to hide the word.
    #[test]
    fn the_list_is_two_sections_both_always_named() {
        use super::{build_pool_view, PoolEntry};
        let mut all = vec![row("B", "b.exe", "B"), row("A", "a.exe", "A")];
        all[1].on_dial = true;
        let view = build_pool_view(&all);
        assert_eq!(
            view,
            vec![
                PoolEntry::Group { on: true, count: 1 },
                PoolEntry::Row { at: 1 },
                PoolEntry::Group { on: false, count: 1 },
                PoolEntry::Row { at: 0 },
            ],
            "the heading, its windows, the second heading, the rest"
        );
        // Every window on the dial leaves the second heading with a count of
        // zero - and it is still drawn.
        all[0].on_dial = true;
        let view = build_pool_view(&all);
        assert_eq!(
            view,
            vec![
                PoolEntry::Group { on: true, count: 2 },
                PoolEntry::Row { at: 0 },
                PoolEntry::Row { at: 1 },
                PoolEntry::Group { on: false, count: 0 },
            ]
        );
        // A search that matches nothing still leaves both headings, so the list
        // reads as "nothing here" instead of vanishing.
        let view = build_pool_view(&[]);
        assert_eq!(
            view,
            vec![
                PoolEntry::Group { on: true, count: 0 },
                PoolEntry::Group { on: false, count: 0 },
            ]
        );
    }

    /// Filtering happens before the split, so a search keeps the two sections
    /// and each section counts what survived it.
    #[test]
    fn a_search_keeps_the_two_sections() {
        use super::{build_pool_view, PoolEntry};
        let mut all = vec![
            row("Outlook - Inbox", "outlook.exe", "Outlook"),
            row("Terminal", "WindowsTerminal.exe", "Windows Terminal"),
            row("Notes", "notepad.exe", "Notepad"),
        ];
        all[0].on_dial = true;
        let rows = filter_rows(&all, "outlook");
        let view = build_pool_view(&rows);
        assert_eq!(
            view,
            vec![
                PoolEntry::Group { on: true, count: 1 },
                PoolEntry::Row { at: 0 },
                PoolEntry::Group { on: false, count: 0 },
            ],
            "the match is in the top section, and the bottom one says zero"
        );
    }
}
