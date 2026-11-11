//! User settings, stored as `key=value` lines in
//! `%LOCALAPPDATA%\ShaiZero\settings.txt`. No serde, no registry: the file is
//! meant to be readable and editable by hand.

use std::fs;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use shaiwin_core::gesture::{Button, Mods, Trigger, TriggerInput};
use shaiwin_core::theme::Rgb;

pub const APP_DIR: &str = "ShaiZero";
/// The folders this app wrote under earlier names, newest first. One of them
/// holds the settings a person already has, and a rename must not cost them it.
const LEGACY_DIRS: [&str; 3] = ["SpinWin", "ShaiWin", "PanWen"];

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
const MOD_ALL: u32 = MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN;

pub const VK_2: u32 = 0x32;
pub const VK_T: u32 = 0x54;
/// The one window shortcut: `Ctrl+2` puts the window in front on the dial and
/// opens its label editor with the window's own name already filled in.
/// A collision moves the panel, never this chord.
pub const FAV_MODS_DEFAULT: u32 = MOD_CONTROL;
pub const FAV_VK_DEFAULT: u32 = VK_2;
/// A sector's size is a share of its ring, never a fixed angle: the other
/// sectors take back whatever one of them does not use, so the ring stays whole.
pub const SECTOR_WEIGHT_DEFAULT: f32 = 1.0;
pub const SECTOR_WEIGHT_MIN: f32 = 0.6;
pub const SECTOR_WEIGHT_MAX: f32 = 3.0;

/// The wheel's shipped size, hub share, and how far the cursor has to travel
/// before the dial opens.
///
/// These are the same four numbers `shaiwin_core::wheel` and
/// `shaiwin_core::gesture` start from, restated here rather than imported: this
/// crate's job is to turn a text file into a struct without leaning on the
/// geometry, and a default that outlives a refactor of the ruler is the wrong
/// kind of coupling. A radius outside the core's range is clamped *there*, where
/// it is used, and the live dial additionally scales down to fit the work area
/// of the monitor it opens on, so a hand-edited `wheel_radius=900` cannot make
/// a dial larger than the screen - it is simply a request the ruler overrules.
pub const WHEEL_RADIUS_DEFAULT: f32 = 150.0;
pub const WHEEL_DEAD_DEFAULT: f32 = 0.42;
pub const TRIGGER_THRESHOLD_DEFAULT: i32 = 8;
/// The dial's look: which accents the sectors take, how they are filled, the air
/// between them, and how long the disc takes to arrive. Zero milliseconds means
/// no animation at all, which is also what the system's reduced-motion setting
/// asks for.
pub const WHEEL_PALETTE_DEFAULT: u32 = 0;
pub const WHEEL_PALETTE_MAX: u32 = 3;
pub const WHEEL_STYLE_DEFAULT: u32 = 0;
pub const WHEEL_STYLE_MAX: u32 = 2;
/// The seven named looks the dial can wear: minimal outline, dark glass, glow,
/// tactile card, self-lit neon, paper card and the cyan "Aurora" look from the
/// project page. `6` is the look the dial ships with.
pub const WHEEL_THEME_DEFAULT: u32 = 6;
pub const WHEEL_THEME_MAX: u32 = 6;
/// How loud the sectors are: 0 soft, 1 medium, 2 vivid. Only the accent colour
/// moves - the plate, seams, strokes and shadows belong to the theme.
pub const WHEEL_ACCENT_DEFAULT: u32 = 1;
pub const WHEEL_ACCENT_MAX: u32 = 2;
/// The dial ships with its sectors touching: the seam is the air between them,
/// and zero is the flat, page-like look. It is not a setting any more - the
/// sectors always meet, and the value is only kept so an older file still reads.
pub const WHEEL_SEAM_DEFAULT: f32 = 0.0;
/// The whole dial can be turned so its first sector sits where the hand expects
/// it; degrees clockwise from 12 o'clock.
pub const WHEEL_ROTATE_DEFAULT: f32 = 0.0;
pub const WHEEL_OPEN_MS_DEFAULT: u32 = 180;
pub const WHEEL_OPEN_MS_MAX: u32 = 400;
/// How many windows the dial draws. The wheel is a quick target, not a wall of
/// every window the desktop owns: ten is the shipped answer, and the ceiling is
/// two rings at twelve sectors each.
pub const WHEEL_LIMIT_DEFAULT: u32 = 10;
pub const WHEEL_LIMIT_MIN: u32 = 3;
pub const WHEEL_LIMIT_MAX: u32 = 24;
/// How the windows are chosen when there are more than the limit: `0` the most
/// recently used first, `1` the favourites list first and then recency.
pub const DIAL_ORDER_DEFAULT: u32 = 0;
pub const DIAL_ORDER_MAX: u32 = 1;
/// What the slider will not go past, so a typo in the file has an upper bound to
/// be clamped to rather than a number nobody meant. This matches the core's own
/// ceiling; the dial scales to fit the screen at open time, so a large number
/// here means "as large as this monitor allows", never a wheel that does not fit.
pub const WHEEL_RADIUS_MAX: f32 = 720.0;
/// How many times a refused write is tried again, and how long between tries.
/// The file is small and the reason for a refusal is a third party holding it
/// for milliseconds, so a short wait is worth it; a long one is not worth having
/// the desktop's mouse hook wait behind it.
const WRITE_TRIES: [Duration; 3] = [
    Duration::ZERO,
    Duration::from_millis(50),
    Duration::from_millis(100),
];

pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(APP_DIR)
}

/// One-time move from an older folder: the settings file and the language packs.
/// Only when the new folder has neither yet, so a user who has already run
/// ShaiZero keeps what it wrote.
pub fn migrate_legacy() {
    let Some(base) = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("APPDATA"))
        .map(PathBuf::from)
    else {
        return;
    };
    let new = base.join(APP_DIR);
    let new_settings = new.join("settings.txt");
    // Packs copied from an older name, and packs an earlier launch of this name
    // already copied, both carry the old product name inside their lines: the
    // title bar would read 晒窗设置 even though every string in this build says
    // ShaiZero. The rewrite is idempotent, so it runs on every start.
    for name in LEGACY_DIRS {
        let old = base.join(name);
        let old_settings = old.join("settings.txt");
        if old_settings.is_file() && !new_settings.exists() {
            if let Err(err) = fs::create_dir_all(&new) {
                eprintln!(
                    "settings: could not make {} for the old settings ({err})",
                    new.display()
                );
            } else {
                if let Err(err) = fs::copy(&old_settings, &new_settings) {
                    eprintln!("settings: could not move the old settings ({err})");
                }
                let old_langs = old.join("languages");
                let new_langs = new.join("languages");
                if old_langs.is_dir() && !new_langs.exists() {
                    if let Err(err) = copy_dir(&old_langs, &new_langs) {
                        eprintln!("settings: could not move the old language packs ({err})");
                    }
                }
            }
        }
    }
    rewrite_old_name(&new.join("languages"));
}

/// A user pack is data, not a build artefact, so the new name cannot be baked
/// into it - it has to be substituted after the copy. Only the product name is
/// touched; the rest of the line, including its language, is left alone.
fn rewrite_old_name(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("txt") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let renamed = text
            .replace("晒窗", "ShaiZero")
            .replace("潘问", "ShaiZero")
            .replace("ShaiWin", "ShaiZero")
            .replace("PanWen", "ShaiZero")
            .replace("SpinWin", "ShaiZero");
        if renamed != text {
            let _ = fs::write(&path, renamed);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let name = entry.file_name();
            let _ = fs::copy(&path, to.join(name));
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct Settings {
    /// No fades, no sliding rails, no unfurling dial. Set by the row in 关于
    /// rather than read off the system, and honoured in the two places the
    /// program animates anything: the dial's opening and the switches' knobs.
    pub reduced_motion: bool,
    /// Lower-cased executable names that never get a seat on the dial.
    pub excluded: Vec<String>,
    /// Modifiers and virtual-key code for "give the window in front a seat".
    /// A virtual key of 0 means the shortcut is switched off.
    pub fav_mods: u32,
    pub fav_vk: u32,
    /// The same, for opening the management panel.
    pub panel_mods: u32,
    pub panel_vk: u32,
    /// The wheel is the switcher: hold the trigger, drag out past the threshold,
    /// let go on a sector. Kept for the old settings files; it no longer switches
    /// anything off, because the trigger is now the only thing that decides
    /// whether the wheel can open.
    pub wheel: bool,
    /// The wheel's radius, in logical pixels. Clamped by `core::wheel::Geometry`,
    /// and clamped here too so what the panel reads back is what was asked for.
    pub wheel_radius: f32,
    /// How much of that radius is the hub, as a fraction of it.
    pub wheel_dead: f32,
    /// Physical pixels the cursor has to travel before the dial opens.
    pub drag_threshold: i32,
    /// Modifier flags that open the dial, and which button drags it out.
    /// The flags are the same `MOD_ALT`/`MOD_CONTROL`/`MOD_SHIFT`/`MOD_WIN` bits
    /// the two shortcuts use, because it is the same line of the same file.
    pub trigger_mods: u32,
    /// The key that starts the gesture, as a virtual-key code. `0` means the
    /// trigger is a mouse button instead. A keyboard trigger opens the dial
    /// where the pointer already is.
    pub trigger_vk: u32,
    /// 0 left, 1 right, 2 middle. Only read when `trigger_vk` is `0`.
    pub trigger_button: u8,
    /// How the dial is painted: the accent palette, the fill style, the air
    /// between sectors, and how long the disc takes to arrive. All four are
    /// drawn from here and nothing else reads them.
    pub wheel_palette: u32,
    pub wheel_style: u32,
    /// The named look: 0 minimal outline, 1 dark glass, 2 glow, 3 tactile card,
    /// 4 neon, 5 paper card, 6 Aurora. This is what the renderer reads; the
    /// palette and style indexes stay only so an older settings file still loads.
    pub wheel_theme: u32,
    /// How loud the sectors are, 0 soft to 2 vivid. The structure of the theme -
    /// plate, seams, strokes, shadows - is not touched by it.
    pub wheel_accent: u32,
    /// The air between sectors. Always zero - the dial's slices meet - and kept
    /// only because the file still carries the line.
    pub wheel_seam: f32,
    /// Degrees clockwise the whole dial is turned from its built-in bearing,
    /// written by dragging outside the rim.
    pub wheel_rotate: f32,
    /// The chosen rarity-colour preset. 0 = off (auto palette), 1 = Hawkeye
    /// red/gold/violet/blue/green, 2 = Classic cobalt/teal/violet/orange/rose.
    /// Scheme also gives the top favourite windows a wider slice automatically.
    pub colour_scheme: u32,
    pub wheel_open_ms: u32,
    /// How many places of the remembered order the dial seats by itself. The
    /// rest stay in the management list until the user puts one there by hand -
    /// a bound window outranks the limit, so the shortcut can always add one
    /// more than this number.
    pub wheel_limit: u32,
    /// `0` most recently used first, `1` the favourites list first.
    pub dial_order: u32,
    /// Where a picked window lands: `0` stays where it was and comes to the
    /// front, `1` is moved to the middle of the work area of the monitor under
    /// the pointer. A maximized window stays maximized either way.
    pub activate_pos: u32,
    /// The directions the user pinned, read out of the file at start-up and
    /// written back the moment one of them moves.
    ///
    /// A pin is a direction and a window's fingerprint, never a sector number and
    /// never a handle: handles come and go with the applications that own them,
    /// and sectors move when the count on the ring changes. What survives a
    /// restart is therefore "that window, over there". The fingerprint is the
    /// design's known weak point - two windows of one program with the same first
    /// word in the title carry the same fingerprint, and the pin lands on whichever
    /// the dial finds first. Nothing is silently wrong about that: the wheel draws
    /// the pin mark on the sector it seated, so a pin that jumped to the wrong
    /// window is visible, and pulling it and re-pressing it takes a minute.
    pub pinned_slots: Vec<Pin>,
    /// The windows the user keeps on the dial, as fingerprints in the order they
    /// were added.
    ///
    /// Only the order is meaningful: an entry is "a window with this fingerprint
    /// is worth a sector", and the dial seats these first, innermost ring first.
    /// A favourite whose application is not running holds its place in the list
    /// and claims no sector - it is a seat reservation, not a launcher. The
    /// fingerprint carries the same weakness a pin does, and the management panel
    /// is where a favourite that stopped matching gets cleared.
    pub favorites: Vec<String>,
    /// The order the dial's sectors are held in, as fingerprints. This is the
    /// memory that keeps a window in the same place from one opening of the dial
    /// to the next: the desktop's z-order changes every time a window is used,
    /// so seating by recency used to reshuffle the whole wheel after one pick.
    ///
    /// A window that has closed keeps its entry, so reopening it brings it back
    /// to the same place. New windows are not added by themselves - they wait in
    /// the management list until `Ctrl+2` or a double-click seats them.
    pub seat_order: Vec<String>,
    /// The fingerprints the user put on the dial by hand - `Ctrl+2`, a
    /// double-click, or a row's own button. A bound window is always seated;
    /// `wheel_limit` only decides how many windows the dial takes on by itself
    /// from the remembered order beyond those. That is why the shortcut can put
    /// a ninth window on a dial set to eight.
    pub bound: Vec<String>,
    /// The fingerprints in [`Settings::seat_order`] the user took off the dial.
    /// They keep their place in the order but are skipped when seating, so a
    /// window put back returns to its old sector and no other window slides in
    /// to fill the gap. Unlike `off_dial` this survives the window closing.
    pub parked: Vec<String>,
    /// Windows the user took off the dial by hand, by window handle. A pin or a
    /// favourite is a seat *reservation*; this is the opposite decision - "never
    /// seat this window, however recently it was used" - and it outranks both
    /// until the user puts it back.
    ///
    /// A handle and not a fingerprint: two documents of one program carry the
    /// same fingerprint, and striking one off used to strike the whole group
    /// with it. A handle names exactly one window, and a window that has closed
    /// takes its entry with it the next time the list is written, so the file
    /// does not grow a line per window that was ever struck off.
    pub off_dial: Vec<isize>,
    /// Windows the user seated by hand, by window handle, in the order they were
    /// asked for. A fingerprint names an application, and several windows of one
    /// program share it; a handle names exactly the window the double-click or
    /// `Ctrl+2` was aimed at, so seating a second document of one program seats
    /// that document and not whichever look-alike happens to be in front. Each
    /// entry is one sector, exempt from `wheel_limit`. Handles do not survive a
    /// restart, so these are the session's explicit choices; the fingerprint
    /// order remains the cross-restart reservation.
    pub seat_hwnds: Vec<isize>,
    /// The labels the user typed for their windows. A pair, not a map, because the
    /// file is written in list order and a HashMap would shuffle the tags on every
    /// save; lookups are over a dozen entries at most.
    pub tags: Vec<Label>,
    /// The sector colours the user picked by hand. A window with an entry here
    /// ignores the palette the dial assigns automatically, which is how a slice
    /// that still looks like its neighbour gets a distinct colour on purpose.
    pub colors: Vec<Colour>,
    /// How much of its ring each sector claims, when the user resized it by hand.
    /// `1.0` is the even share the dial starts with; an entry here widens or
    /// narrows one slice without changing how its neighbours remember their own.
    pub weights: Vec<WindowWeight>,
    /// The language the interface is drawn in: `auto` follows the system, and
    /// anything else is the name of a pack in the languages folder. Kept as text
    /// rather than an enum because the folder decides what exists.
    pub lang: String,
    /// The size the management panel was last left at, in logical pixels, so a
    /// window dragged to a comfortable shape comes back that way. `0` is a panel
    /// nobody has resized: it opens at the size the screen has room for.
    pub panel_w: f32,
    pub panel_h: f32,
}

/// One pinned direction: where the pointer was pointing when the user let go with
/// `Ctrl` held, degrees clockwise from twelve o'clock, and the fingerprint of the
/// window that holds it (`exe path | window class | first word of the title`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub bearing: f32,
    /// The ring the user dropped the sector on, when they aimed at one. `None` is
    /// a pin written before rings were kept: it lands on the nearest free sector
    /// at its angle, exactly as it always did.
    pub ring: Option<usize>,
    /// How much room on its ring this sector claims, relative to its neighbours.
    /// `None` is the shipped one, so a file written before sizes were kept loads
    /// unchanged and every sector stays equal.
    pub weight: Option<f32>,
    pub window: String,
}

/// One window's label: the fingerprint it was typed for (`exe path | window class
/// | first word of the title`, the same one a pin uses) and the text the user
/// wants on the sector.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub window: String,
    pub tag: String,
}

/// One window's hand-picked sector colour: the same fingerprint a label uses and
/// the RGB the user chose. Kept apart from [`Settings::tags`] so naming a window
/// and colouring it are independent decisions.
#[derive(Clone, Debug, PartialEq)]
pub struct Colour {
    pub window: String,
    pub rgb: Rgb,
}

/// One window's hand-picked sector size: the same fingerprint a colour uses and
/// the weight it claims on its ring. Kept apart from [`Settings::colors`] so a
/// window can be recoloured without being resized, and vice versa.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowWeight {
    pub window: String,
    pub weight: f32,
}

/// A tag the sector can actually carry. Tabs and line breaks would break the
/// file, and an essay on a sector helps nobody.
pub fn clean_tag(tag: &str) -> String {
    let flat: String = tag
        .chars()
        .map(|c| if c == ',' || c.is_control() { ' ' } else { c })
        .collect();
    flat.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(24)
        .collect()
}

/// The name a new label starts with: the window's title up to the ` - ` that
/// Windows apps put before the application name, or the application's name when
/// there is no title. Clean and short enough for a sector and for the field.
pub fn default_tag(title: &str, app: &str) -> String {
    let head = title.split(" - ").next().unwrap_or_default().trim();
    clean_tag(if head.is_empty() { app } else { head })
}

/// What the settings window is asking for, before any of it is made true.
///
/// The panel drafts ten decisions - two shortcuts and the language, then the
/// dial: whether it is on, its trigger, its button, its size, its hub and how far
/// the pointer has to travel before it opens, and whether anything on screen is
/// allowed to move at all. Either all of them land or none do, because a save that stops at a
/// taken shortcut must leave the rest of the line as it was. They travel as one
/// value so a half-applied save cannot be an accident of the argument list.
#[derive(Clone, Debug)]
pub struct Draft {
    pub fav: (u32, u32),
    pub panel: (u32, u32),
    /// The wheel's radius, in logical pixels. Clamped by `core::wheel::Geometry`,
    /// and clamped here too so what the panel reads back is what was asked for.
    pub wheel_radius: f32,
    /// How much of that radius is the hub, as a fraction of it.
    pub wheel_dead: f32,
    /// Modifier flags that open the dial, and which button drags it out.
    /// The flags are the same `MOD_ALT`/`MOD_CONTROL`/`MOD_SHIFT`/`MOD_WIN` bits
    /// the two shortcuts use, because it is the same line of the same file.
    pub trigger_mods: u32,
    /// The recorded key (`0` when the trigger is a mouse button), and the mouse
    /// button (0 left, 1 right, 2 middle).
    pub trigger_vk: u32,
    pub trigger_button: u8,
    /// The dial's named look, accent loudness and open animation. The named look
    /// the renderer follows: 0 outline, 1 glass, 2 glow, 3 card, 4 neon, 5 paper,
    /// 6 Aurora.
    pub wheel_theme: u32,
    pub wheel_accent: u32,
    /// Degrees clockwise the whole dial is turned. It has no row on the page:
    /// the picture's blank ring past the rim is the handle, and the page carries
    /// the number only so that drag can be saved like any other change.
    pub wheel_rotate: f32,
    pub colour_scheme: u32,
    pub wheel_open_ms: u32,
    /// The dial's window limit and the order the windows arrive in.
    pub wheel_limit: u32,
    pub dial_order: u32,
    /// `0` in place, `1` centred on the screen under the pointer.
    pub activate_pos: u32,
    /// Nothing that slides or unfurls: the dial arrives at full size and the
    /// switches' knobs jump to their ends. The row that moves this is in 关于,
    /// and the two places it is read - `wheel::paint_disc` and the switch
    /// control - are the only animations the program has.
    pub reduced_motion: bool,
    /// A pack tag, or `auto` to follow the system.
    pub lang: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            reduced_motion: false,
            excluded: Vec::new(),
            fav_mods: FAV_MODS_DEFAULT,
            fav_vk: FAV_VK_DEFAULT,
            panel_mods: MOD_CONTROL,
            panel_vk: VK_T,
            wheel: true,
            wheel_radius: WHEEL_RADIUS_DEFAULT,
            wheel_dead: WHEEL_DEAD_DEFAULT,
            drag_threshold: TRIGGER_THRESHOLD_DEFAULT,
            trigger_mods: MOD_ALT,
            trigger_vk: 0,
            trigger_button: 0,
            wheel_palette: WHEEL_PALETTE_DEFAULT,
            wheel_style: WHEEL_STYLE_DEFAULT,
            wheel_theme: WHEEL_THEME_DEFAULT,
            wheel_accent: WHEEL_ACCENT_DEFAULT,
            wheel_seam: WHEEL_SEAM_DEFAULT,
            wheel_rotate: WHEEL_ROTATE_DEFAULT,
            colour_scheme: 0,
            wheel_open_ms: WHEEL_OPEN_MS_DEFAULT,
            wheel_limit: WHEEL_LIMIT_DEFAULT,
            dial_order: DIAL_ORDER_DEFAULT,
            activate_pos: 0,
            pinned_slots: Vec::new(),
            favorites: Vec::new(),
            seat_order: Vec::new(),
            bound: Vec::new(),
            parked: Vec::new(),
            off_dial: Vec::new(),
            seat_hwnds: Vec::new(),
            tags: Vec::new(),
            colors: Vec::new(),
            weights: Vec::new(),
            lang: crate::i18n::AUTO.to_string(),
            panel_w: 0.0,
            panel_h: 0.0,
        }
    }
}

impl Default for Draft {
    /// A panel opened on a machine with no settings file at all.
    fn default() -> Draft {
        let s = Settings::default();
        Draft {
            fav: (s.fav_mods, s.fav_vk),
            panel: (s.panel_mods, s.panel_vk),
            wheel_radius: s.wheel_radius,
            wheel_dead: s.wheel_dead,
            trigger_mods: s.trigger_mods,
            trigger_vk: s.trigger_vk,
            trigger_button: s.trigger_button,
            wheel_theme: s.wheel_theme,
            wheel_accent: s.wheel_accent,
            wheel_rotate: s.wheel_rotate,
            colour_scheme: s.colour_scheme,
            wheel_open_ms: s.wheel_open_ms,
            wheel_limit: s.wheel_limit,
            dial_order: s.dial_order,
            activate_pos: s.activate_pos,
            reduced_motion: s.reduced_motion,
            lang: s.lang,
        }
    }
}

impl Settings {
    fn file() -> PathBuf {
        data_dir().join("settings.txt")
    }

    /// Where a file this app cannot make sense of goes first. The next
    /// unreadable file overwrites it, and that is the whole risk: what this app
    /// did write is a file it can read back, so whatever lands here is damage
    /// rather than a setting somebody meant.
    fn backup_file() -> PathBuf {
        data_dir().join("settings.bak.txt")
    }

    pub fn load() -> Settings {
        let file = Self::file();
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Settings::default(),
            Err(err) => {
                let where_it_went = stash_damaged(&file, &Self::backup_file());
                crate::log!(
                    "settings: {} could not be read ({err}){}",
                    file.display(),
                    match where_it_went {
                        Some(path) => format!(", kept at {}", path.display()),
                        None => String::new(),
                    }
                );
                return Settings::default();
            }
        };
        let (mut s, recognised) = parse(&text);
        if recognised == 0 && !text.trim().is_empty() {
            // A file with lines in it and not one key this app wrote. That is
            // damage, not a fresh install, and the reading that costs nothing is
            // to start from the defaults and leave the evidence where a person
            // can still find it.
            let where_it_went = stash_damaged(&file, &Self::backup_file());
            crate::log!(
                "settings: no line of {} was understood, starting from the defaults{}",
                file.display(),
                match where_it_went {
                    Some(path) => format!("; the file is kept at {}", path.display()),
                    None => "; the file could not be kept".to_string(),
                }
            );
            return Settings::default();
        }
        // A file written before the recorder existed has no `trigger_key`. If it
        // still carries the old shipped Ctrl+right, move it to the binding the
        // new default advertises; a binding the user chose stays untouched.
        let has_recorded_key = text
            .lines()
            .any(|line| line.trim_start().starts_with("trigger_key="));
        if !has_recorded_key
            && s.trigger_vk == 0
            && s.trigger_mods == MOD_CONTROL
            && s.trigger_button == 1
        {
            s.trigger_mods = MOD_ALT;
            s.trigger_button = 0;
        }
        // Files written before explicit seats existed used `parked` for two
        // things: windows the user took off, and the overflow past
        // `wheel_limit` at first run. Only the first is still a decision - the
        // overflow is simply beyond the default roster now - so leaving it
        // parked would make raising the limit show nothing. A file with no
        // `bound_seats` line predates the split.
        let has_bound = text
            .lines()
            .any(|line| line.trim_start().starts_with("bound_seats="));
        s.drop_legacy_overflow(!has_bound);
        // The one place a number is decided, whether it came from the file or
        // from a hand that edited it.
        s.normalize();
        s
    }

    /// Writes what has been decided, and does not leave a half file behind.
    ///
    /// The bytes go to a name next door and move over the real one afterwards,
    /// so the worst an interrupted write can produce is the previous file plus a
    /// stray temporary - never a settings file that stops in the middle of a
    /// line. A refusal to move (something else holding the file) is retried
    /// briefly and, if it stands, logged: the decision lives in this process
    /// until the next write succeeds, so a momentary lock loses nothing.
    pub fn save(&mut self) {
        self.normalize();
        let file = Self::file();
        if let Some(dir) = file.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Err(err) = write_atomic(&file, &self.render()) {
            crate::log!("settings: not written ({err})");
        }
    }

    /// The whole file as this app would write it. Normalised in, normalised out:
    /// what is on disk is exactly what the next start would read back.
    fn render(&self) -> String {
        format!(
            "reduced_motion={}\n\
             fav_mods={}\nfav_vk={}\npanel_mods={}\npanel_vk={}\n\
             lang={}\nexcluded={}\n\
             wheel={}\ntrigger_mods={}\ntrigger_button={}\ntrigger_key={}\n\
             wheel_radius={:.1}\nwheel_dead={:.2}\ndrag_threshold={}\n\
             wheel_palette={}\nwheel_style={}\nwheel_theme={}\nwheel_accent={}\nwheel_seam={:.1}\nwheel_rotate={:.1}\ncolour_scheme={}\nwheel_open_ms={}\n\
             wheel_limit={}\ndial_order={}\nactivate_pos={}\n\
             pinned_slots={}\nfavorite_slots={}\noff_dial={}\nwindow_tags={}\nwindow_colors={}\nwindow_weights={}\n\
             seat_order={}\nbound_seats={}\nparked_seats={}\nseat_hwnds={}\n\
             panel_w={:.0}\npanel_h={:.0}\n",
            u8::from(self.reduced_motion),
            self.fav_mods,
            self.fav_vk,
            self.panel_mods,
            self.panel_vk,
            self.lang,
            self.excluded.join(","),
            u8::from(self.wheel),
            self.trigger_mods,
            self.trigger_button_name(),
            self.trigger_vk,
            self.wheel_radius,
            self.wheel_dead,
            self.drag_threshold,
            self.wheel_palette,
            self.wheel_style,
            self.wheel_theme,
            self.wheel_accent,
            self.wheel_seam,
            self.wheel_rotate,
            self.colour_scheme,
            self.wheel_open_ms,
            self.wheel_limit,
            self.dial_order,
            self.activate_pos,
            format_pins(&self.pinned_slots),
            format_favorites(&self.favorites),
            format_handles(&self.off_dial),
            format_labels(&self.tags),
            format_colors(&self.colors),
            format_weights(&self.weights),
            format_favorites(&self.seat_order),
            format_favorites(&self.bound),
            format_favorites(&self.parked),
            format_handles(&self.seat_hwnds),
            self.panel_w,
            self.panel_h
        )
    }

    /// Every number in range and every list with one entry per window. The one
    /// place this happens: the file, a slider, a stepper and a hand with a text
    /// editor all arrive here, and a rule written twice is a rule two places can
    /// disagree about.
    pub fn normalize(&mut self) {
        self.wheel_radius = clamp_radius(self.wheel_radius);
        self.wheel_dead = clamp_dead(self.wheel_dead);
        self.drag_threshold = clamp_threshold(self.drag_threshold);
        self.wheel_palette = self.wheel_palette.min(WHEEL_PALETTE_MAX);
        self.wheel_style = self.wheel_style.min(WHEEL_STYLE_MAX);
        self.wheel_theme = self.wheel_theme.min(WHEEL_THEME_MAX);
        self.wheel_accent = self.wheel_accent.min(WHEEL_ACCENT_MAX);
        // The air between sectors was a setting once; the dial is one plate now,
        // so a file that still carries a seam is read and put back to zero.
        self.wheel_seam = WHEEL_SEAM_DEFAULT;
        self.wheel_rotate = clamp_rotation(self.wheel_rotate);
        self.colour_scheme = self
            .colour_scheme
            .min(shaiwin_core::theme::SCHEME_MAX as u32);
        self.wheel_open_ms = self.wheel_open_ms.min(WHEEL_OPEN_MS_MAX);
        self.wheel_limit = self
            .wheel_limit
            .clamp(WHEEL_LIMIT_MIN, WHEEL_LIMIT_MAX);
        self.dial_order = self.dial_order.min(DIAL_ORDER_MAX);
        self.activate_pos = self.activate_pos.min(1);
        // A remembered panel size is either a size somebody chose or nothing at
        // all. A number too small to hold the page, or too large for any screen,
        // is not a choice - it is a file edited by hand or a window dragged off
        // the edge of a monitor that is no longer there.
        self.panel_w = clamp_panel_size(self.panel_w);
        self.panel_h = clamp_panel_size(self.panel_h);
        self.fav_mods &= MOD_ALL;
        self.panel_mods &= MOD_ALL;
        self.trigger_mods &= MOD_ALL;
        self.trigger_vk &= 0xFF;
        // A modifier can be part of the chord, never the key it is aimed with.
        if is_modifier_vk(self.trigger_vk) {
            self.trigger_vk = 0;
        }
        self.trigger_button = self.trigger_button.min(4);
        self.fav_vk &= 0xFF;
        self.panel_vk &= 0xFF;
        // One chord, one action. Two shortcuts are registered with the desktop;
        // a file that gives both the same combination would have one of them
        // silently disappear. `Ctrl+2` is the chord the wheel is known by, so a
        // collision moves the *panel* to `Ctrl+Shift+T` instead of quietly
        // relabelling the window shortcut as `Ctrl+3` - which is exactly how a
        // settings page came to advertise a key nobody had chosen.
        if self.fav_vk != 0
            && self.fav_vk == self.panel_vk
            && self.fav_mods == self.panel_mods
        {
            self.panel_mods = MOD_CONTROL | MOD_SHIFT;
            self.panel_vk = VK_T;
            if self.panel_mods == self.fav_mods && self.panel_vk == self.fav_vk {
                self.panel_vk = 0;
            }
        }
        for pin in &mut self.pinned_slots {
            pin.weight = pin
                .weight
                .map(|w| w.clamp(SECTOR_WEIGHT_MIN, SECTOR_WEIGHT_MAX));
        }
        for label in &mut self.tags {
            label.tag = clean_tag(&label.tag);
        }
        self.tags.retain(|label| !label.tag.is_empty());
        dedupe_by(&mut self.pinned_slots, |pin| pin.window.clone());
        dedupe_by(&mut self.tags, |label| label.window.clone());
        dedupe_by(&mut self.colors, |colour| colour.window.clone());
        dedupe_by(&mut self.weights, |w| w.window.clone());
        for w in &mut self.weights {
            w.weight = w.weight.clamp(SECTOR_WEIGHT_MIN, SECTOR_WEIGHT_MAX);
        }
        self.weights
            .retain(|w| (w.weight - SECTOR_WEIGHT_DEFAULT).abs() > 0.01);
        dedupe(&mut self.favorites);
        dedupe(&mut self.seat_order);
        dedupe(&mut self.bound);
        dedupe(&mut self.parked);
        // A parked window that holds no place in the order is a stale line; it
        // must not be resurrected by a later edit of the file.
        let ordered = self.seat_order.clone();
        self.bound.retain(|mark| ordered.contains(mark));
        self.parked.retain(|mark| ordered.contains(mark));
        self.off_dial.retain(|hwnd| *hwnd != 0);
        dedupe_by(&mut self.off_dial, |hwnd| *hwnd);
        self.seat_hwnds.retain(|hwnd| *hwnd != 0);
        dedupe_by(&mut self.seat_hwnds, |hwnd| *hwnd);
        for exe in &mut self.excluded {
            *exe = exe.to_lowercase();
        }
        self.excluded.retain(|exe| !exe.is_empty());
        dedupe(&mut self.excluded);
    }

    /// Drops the overflow the old cap used to write into `parked`. A file older
    /// than `bound_seats` parked everything past `wheel_limit` at first run;
    /// those entries are not a decision, they are just windows waiting for the
    /// default roster to grow, and keeping them parked would make raising the
    /// limit show nothing. A manual park was always inside the old roster, so it
    /// is kept.
    fn drop_legacy_overflow(&mut self, legacy: bool) {
        if !legacy {
            return;
        }
        let limit = self.wheel_limit.clamp(WHEEL_LIMIT_MIN, WHEEL_LIMIT_MAX) as usize;
        let order = self.seat_order.clone();
        self.parked.retain(|mark| {
            order
                .iter()
                .position(|seat| seat == mark)
                .is_some_and(|at| at < limit)
        });
    }

    pub fn is_excluded(&self, exe: &str) -> bool {
        self.excluded.iter().any(|e| e == exe)
    }

    /// The place a window holds in the dial's order, if it has one.
    pub fn seat_index(&self, mark: &str) -> Option<usize> {
        self.seat_order.iter().position(|seat| seat == mark)
    }

    /// The other half of a drag between two sectors that share a fingerprint:
    /// two documents of one program are seated by handle, so their visible order
    /// is the order of [`Settings::seat_hwnds`]. Swapping the marks would be a
    /// no-op for them; swapping the handles is what moves the slices.
    pub fn swap_seat_windows(&mut self, a: isize, b: isize) -> bool {
        if a == b {
            return false;
        }
        let ia = self.seat_hwnds.iter().position(|hwnd| *hwnd == a);
        let ib = self.seat_hwnds.iter().position(|hwnd| *hwnd == b);
        match (ia, ib) {
            (Some(ia), Some(ib)) => {
                self.seat_hwnds.swap(ia, ib);
                true
            }
            _ => false,
        }
    }

    /// Whether the user took this window off the dial. It keeps its place in
    /// [`Settings::seat_order`] but is skipped while seating.
    pub fn is_parked(&self, mark: &str) -> bool {
        self.parked.iter().any(|seat| seat == mark)
    }

    /// Gives a window a place on the dial without moving any other window. A
    /// window that was taken off comes back to the place it kept; a window that
    /// never had a place takes the first place the user emptied, so explicitly
    /// seating a window after a down-dial does not leave the dial permanently
    /// short of a seat. The window also joins [`Settings::bound`]: the explicit
    /// request outranks `wheel_limit`, which only counts the windows the dial
    /// takes on by itself. Answers whether anything moved, so the caller knows
    /// whether the file is worth writing.
    pub fn seat_add(&mut self, mark: &str) -> bool {
        let limit = self.wheel_limit.clamp(WHEEL_LIMIT_MIN, WHEEL_LIMIT_MAX) as usize;
        let was_parked = self.is_parked(mark);
        let at = self.seat_index(mark);
        // An entry already within the default roster, or already bound, keeps
        // the place it holds. One waiting past the roster has no sector to keep,
        // so the explicit add treats it like a newcomer and gives it the place
        // the user emptied first.
        let keeps_place = at.is_some_and(|at| {
            was_parked || at < limit || self.bound.iter().any(|seat| seat == mark)
        });
        let mut changed = false;
        if !keeps_place {
            if let Some(at) = at {
                self.seat_order.remove(at);
            }
            let emptied = self
                .seat_order
                .iter()
                .position(|seat| self.is_parked(seat));
            if let Some(at) = emptied {
                // The mark it replaces was taken off the dial. It keeps its place
                // in the file's history at the tail and stays parked, so it
                // cannot come back on its own; only another explicit add can
                // seat it.
                let displaced = std::mem::replace(&mut self.seat_order[at], mark.to_string());
                self.seat_order.push(displaced);
            } else {
                self.seat_order.push(mark.to_string());
            }
            changed = true;
        }
        if was_parked {
            self.parked.retain(|seat| seat != mark);
            changed = true;
        }
        if !self.bound.iter().any(|seat| seat == mark) {
            self.bound.push(mark.to_string());
            changed = true;
        }
        changed
    }

    /// Takes a window off the dial but keeps its place in the order, so the
    /// sector count shrinks by one and nothing else slides into the gap.
    pub fn seat_park(&mut self, mark: &str) {
        if self.seat_index(mark).is_none() {
            self.seat_order.push(mark.to_string());
        }
        if !self.is_parked(mark) {
            self.parked.push(mark.to_string());
        }
        // Taken off by hand is no longer an explicit request to keep it on.
        self.bound.retain(|seat| seat != mark);
    }

    /// Whether this exact window holds an explicit seat.
    pub fn is_seat_window(&self, hwnd: isize) -> bool {
        hwnd != 0 && self.seat_hwnds.contains(&hwnd)
    }

    /// Seats exactly this window. A handle, not a fingerprint: several windows
    /// of one program share a fingerprint, and the gesture was aimed at one of
    /// them, so that one gets the sector. Answers whether anything moved.
    pub fn seat_window(&mut self, hwnd: isize) -> bool {
        if hwnd == 0 || self.is_seat_window(hwnd) {
            return false;
        }
        self.seat_hwnds.push(hwnd);
        true
    }

    /// Takes this window's explicit seat away. Its look-alikes keep theirs.
    pub fn unseat_window(&mut self, hwnd: isize) -> bool {
        let before = self.seat_hwnds.len();
        self.seat_hwnds.retain(|held| *held != hwnd);
        self.seat_hwnds.len() != before
    }

    /// First run only: the desk as it stands becomes the dial's order. Pins come
    /// first (in the direction they were pinned at), then favourites, then the
    /// rest of the desk in z-order. Nothing is parked: `wheel_limit` is applied
    /// by the seating rule, which takes the first places of this order and
    /// leaves the rest waiting in the management list. Raising the limit later
    /// brings the next windows in without touching the order.
    pub fn seed_seat_order(&mut self, live: &[String]) {
        if !self.seat_order.is_empty() {
            return;
        }
        let mut pins: Vec<(f32, &str)> = self
            .pinned_slots
            .iter()
            .map(|pin| (pin.bearing, pin.window.as_str()))
            .collect();
        pins.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut order: Vec<String> = Vec::new();
        for mark in pins
            .into_iter()
            .map(|(_, mark)| mark)
            .chain(self.favorites.iter().map(String::as_str))
            .chain(live.iter().map(String::as_str))
        {
            if !order.iter().any(|held| held == mark) {
                order.push(mark.to_string());
            }
        }
        self.seat_order = order;
    }

    /// Where this window sits on the favourites list, if it is on it. The dial
    /// seats by that rank, which is why the list is a list.
    pub fn favorite_rank(&self, fingerprint: &str) -> Option<usize> {
        self.favorites.iter().position(|f| f == fingerprint)
    }

    /// Puts a window on the favourites list at the end. Answers `false` when it
    /// was already there, because the caller has to know whether the file is worth
    /// writing and whether to ask for a tag or only offer to change one.
    pub fn add_favorite(&mut self, fingerprint: &str) -> bool {
        if self.favorites.iter().any(|f| f == fingerprint) {
            return false;
        }
        self.favorites.push(fingerprint.to_string());
        true
    }

    /// Whether this window - not every window that looks like it - was taken
    /// off the dial. The application is not touched (see `excluded` for that),
    /// and neither is a second window of the same program. A zero handle is
    /// never a window, so it never matches.
    pub fn is_off_dial(&self, hwnd: isize) -> bool {
        hwnd != 0 && self.off_dial.contains(&hwnd)
    }

    /// Takes a window off the dial. Answers `false` when it already was, so the
    /// caller knows whether the file is worth writing.
    pub fn mark_off_dial(&mut self, hwnd: isize) -> bool {
        if hwnd == 0 || self.is_off_dial(hwnd) {
            return false;
        }
        self.off_dial.push(hwnd);
        true
    }

    /// Puts a window back on the dial. Only the window whose handle this is
    /// moves; the rest of the desk is where it was.
    pub fn clear_off_dial(&mut self, hwnd: isize) -> bool {
        let before = self.off_dial.len();
        self.off_dial.retain(|held| *held != hwnd);
        self.off_dial.len() != before
    }

    pub fn tag_for(&self, fingerprint: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|l| l.window == fingerprint)
            .map(|l| l.tag.as_str())
    }

    /// Writes, replaces or - with an empty tag - removes a label. The tag is
    /// [cleaned](`clean_tag`) here so that however it arrives, from the panel's box
    /// or from a hand-edited file, what lands is something the sector can draw.
    pub fn set_tag(&mut self, fingerprint: &str, tag: &str) {
        let tag = clean_tag(tag);
        if let Some(label) = self.tags.iter_mut().find(|l| l.window == fingerprint) {
            if tag.is_empty() {
                self.tags.retain(|l| l.window != fingerprint);
            } else {
                label.tag = tag;
            }
            return;
        }
        if !tag.is_empty() {
            self.tags.push(Label {
                window: fingerprint.to_string(),
                tag,
            });
        }
    }

    /// The colour the user picked for this window, if any. `None` means "let the
    /// dial assign one", which is what most windows do.
    pub fn colour_for(&self, fingerprint: &str) -> Option<Rgb> {
        self.colors
            .iter()
            .find(|c| c.window == fingerprint)
            .map(|c| c.rgb)
    }

    /// Writes, replaces or - with `None` - removes a hand-picked sector colour.
    /// Answers whether anything changed, so the caller does not write the file
    /// or rebuild the dial after a no-op.
    pub fn set_colour(&mut self, fingerprint: &str, colour: Option<Rgb>) -> bool {
        let at = self.colors.iter().position(|c| c.window == fingerprint);
        match (at, colour) {
            (Some(at), None) => {
                self.colors.remove(at);
                true
            }
            (Some(at), Some(rgb)) => {
                if self.colors[at].rgb == rgb {
                    return false;
                }
                self.colors[at].rgb = rgb;
                true
            }
            (None, Some(rgb)) => {
                self.colors.push(Colour {
                    window: fingerprint.to_string(),
                    rgb,
                });
                true
            }
            (None, None) => false,
        }
    }

    /// How much of its ring this window's sector claims. `1.0` is the even share;
    /// a window the user never resized has no entry and answers with the default,
    /// so callers can treat the answer as the size directly.
    pub fn weight_for(&self, fingerprint: &str) -> f32 {
        self.weights
            .iter()
            .find(|w| w.window == fingerprint)
            .map(|w| w.weight)
            .unwrap_or(SECTOR_WEIGHT_DEFAULT)
    }

    /// The hand-picked weight, if any. Unlike [`Settings::weight_for`] which
    /// always answers with a number (1.0 being the default), this returns `None`
    /// when the user never changed anything so callers can tell "auto" apart
    /// from "pinned".
    pub fn custom_weight_for(&self, fingerprint: &str) -> Option<f32> {
        self.weights
            .iter()
            .find(|w| w.window == fingerprint)
            .map(|w| w.weight)
    }

    /// Writes, replaces or - with `None` - removes a hand-picked sector size. A
    /// value that clamps back to the default is removed rather than stored, so the
    /// file never carries a size that does nothing. Answers whether anything
    /// changed, so the caller skips the save and the repaint after a no-op.
    pub fn set_weight(&mut self, fingerprint: &str, weight: Option<f32>) -> bool {
        let at = self.weights.iter().position(|w| w.window == fingerprint);
        let normalized = |w: f32| {
            let c = w.clamp(SECTOR_WEIGHT_MIN, SECTOR_WEIGHT_MAX);
            if (c - SECTOR_WEIGHT_DEFAULT).abs() > 0.01 {
                Some(c)
            } else {
                None
            }
        };
        match (at, weight.and_then(normalized)) {
            (Some(at), None) => {
                self.weights.remove(at);
                true
            }
            (Some(at), Some(w)) => {
                if (self.weights[at].weight - w).abs() < 0.01 {
                    return false;
                }
                self.weights[at].weight = w;
                true
            }
            (None, Some(w)) => {
                self.weights.push(WindowWeight {
                    window: fingerprint.to_string(),
                    weight: w,
                });
                true
            }
            (None, None) => false,
        }
    }

    pub fn fav_label(&self) -> String {
        hotkey_label(self.fav_mods, self.fav_vk)
    }

    pub fn panel_label(&self) -> String {
        hotkey_label(self.panel_mods, self.panel_vk)
    }

}

/// A human-readable shortcut, e.g. `Ctrl+Alt+H`. `vk == 0` means unbound.
pub fn hotkey_label(mods: u32, vk: u32) -> String {
    if vk == 0 {
        return "—".to_string();
    }
    let mut parts = Vec::new();
    if mods & MOD_CONTROL != 0 {
        parts.push("Ctrl");
    }
    if mods & MOD_ALT != 0 {
        parts.push("Alt");
    }
    if mods & MOD_SHIFT != 0 {
        parts.push("Shift");
    }
    if mods & MOD_WIN != 0 {
        parts.push("Win");
    }
    let key = key_name(vk);
    parts.push(key.as_str());
    parts.join("+")
}

fn key_name(vk: u32) -> String {
    match vk {
        0x30..=0x39 => char::from_u32(vk)
            .map(String::from)
            .unwrap_or_else(|| format!("VK{vk:02X}")),
        0x41..=0x5A => char::from_u32(vk)
            .map(String::from)
            .unwrap_or_else(|| format!("VK{vk:02X}")),
        0x70..=0x87 => format!("F{}", vk - 0x6F),
        0x20 => "Space".to_string(),
        0x25 => "Left".to_string(),
        0x26 => "Up".to_string(),
        0x27 => "Right".to_string(),
        0x28 => "Down".to_string(),
        0x24 => "Home".to_string(),
        0x23 => "End".to_string(),
        0x21 => "PgUp".to_string(),
        0x22 => "PgDn".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        0x09 => "Tab".to_string(),
        0x0D => "Enter".to_string(),
        0x1B => "Esc".to_string(),
        _ => format!("VK{vk:02X}"),
    }
}

fn parse_mods(value: &str) -> u32 {
    value.parse::<u32>().map(|v| v & MOD_ALL).unwrap_or(0)
}

/// One line of the file, read into the settings it changes. Answers `false` for
/// a key this app does not know, which is how the caller can tell an old file
/// from a damaged one.
fn apply_line(s: &mut Settings, key: &str, value: &str) -> bool {
    let value = value.trim();
    match key {
        "reduced_motion" => s.reduced_motion = value == "1",
        "wheel" => s.wheel = value != "0",
        "wheel_radius" => s.wheel_radius = parse_wheel_radius(value),
        "wheel_dead" => s.wheel_dead = parse_wheel_dead(value),
        "wheel_palette" => {
            s.wheel_palette = parse_index(value, WHEEL_PALETTE_MAX, WHEEL_PALETTE_DEFAULT)
        }
        "wheel_style" => s.wheel_style = parse_index(value, WHEEL_STYLE_MAX, WHEEL_STYLE_DEFAULT),
        "wheel_theme" => s.wheel_theme = parse_index(value, WHEEL_THEME_MAX, WHEEL_THEME_DEFAULT),
        "wheel_accent" => s.wheel_accent = parse_index(value, WHEEL_ACCENT_MAX, WHEEL_ACCENT_DEFAULT),
        "colour_scheme" => {
            s.colour_scheme = parse_index(
                value,
                shaiwin_core::theme::SCHEME_MAX as u32,
                0,
            );
        }
        "wheel_seam" => s.wheel_seam = WHEEL_SEAM_DEFAULT,
        "wheel_rotate" => s.wheel_rotate = parse_wheel_rotate(value),
        "wheel_open_ms" => {
            s.wheel_open_ms = parse_index(value, WHEEL_OPEN_MS_MAX, WHEEL_OPEN_MS_DEFAULT)
        }
        "wheel_limit" => {
            s.wheel_limit = parse_index(value, WHEEL_LIMIT_MAX, WHEEL_LIMIT_DEFAULT)
                .max(WHEEL_LIMIT_MIN)
        }
        "dial_order" => {
            s.dial_order = parse_index(value, DIAL_ORDER_MAX, DIAL_ORDER_DEFAULT)
        }
        "activate_pos" => s.activate_pos = parse_index(value, 1, 0),
        "panel_w" => s.panel_w = value.parse::<f32>().unwrap_or(0.0),
        "panel_h" => s.panel_h = value.parse::<f32>().unwrap_or(0.0),
        "drag_threshold" => s.drag_threshold = parse_trigger_threshold(value),
        "trigger_mods" => s.trigger_mods = parse_trigger_mods(value),
        "trigger_button" => {
            s.trigger_button = match value.trim().to_ascii_lowercase().as_str() {
                "right" => 1,
                "middle" => 2,
                "back" | "x1" | "side1" => 3,
                "forward" | "x2" | "side2" => 4,
                _ => 0,
            }
        }
        "trigger_key" => s.trigger_vk = parse_trigger_key(value),
        "pinned_slots" => s.pinned_slots = parse_pins(value),
        "favorite_slots" => s.favorites = parse_favorites(value),
        "seat_order" => s.seat_order = parse_favorites(value),
        "bound_seats" => s.bound = parse_favorites(value),
        "parked_seats" => s.parked = parse_favorites(value),
        "off_dial" => s.off_dial = parse_handles(value),
        "seat_hwnds" => s.seat_hwnds = parse_handles(value),
        "window_tags" => s.tags = parse_labels(value),
        "window_colors" => s.colors = parse_colors(value),
        "window_weights" => s.weights = parse_weights(value),
        "excluded" => {
            s.excluded = value
                .split(',')
                .map(|e| e.trim().to_lowercase())
                .filter(|e| !e.is_empty())
                .collect();
        }
        "fav_mods" => s.fav_mods = parse_mods(value),
        "fav_vk" => s.fav_vk = parse_vk(value),
        "panel_mods" => s.panel_mods = parse_mods(value),
        "panel_vk" => s.panel_vk = parse_vk(value),
        "lang" => s.lang = crate::i18n::lang_tag(value),
        _ => return false,
    }
    true
}

/// The file as text, and how many of its lines carried a key this app wrote. The
/// count is what lets `load` tell a file it should trust from one that only
/// looks like somebody's notes: every line of a settings file is a decision, so
/// a line that means nothing is worth keeping rather than quietly dropping.
fn parse(text: &str) -> (Settings, usize) {
    let mut s = Settings::default();
    let mut recognised = 0;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if apply_line(&mut s, key.trim(), value) {
            recognised += 1;
        }
    }
    (s, recognised)
}

/// The dial's radius as a number: positive, real, and not past what the slider
/// will not go past. Anything else is the shipped size, because a radius nobody
/// could read out of the file is not a dial anyone asked for.
fn clamp_radius(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 {
        value.min(WHEEL_RADIUS_MAX)
    } else {
        WHEEL_RADIUS_DEFAULT
    }
}

/// The hub as a fraction of the radius, which is the one number with no unit to
/// argue about. Outside nought to one it is not a fraction, so the value was
/// never what it says and the shipped answer is the honest reading.
fn clamp_dead(value: f32) -> f32 {
    if value.is_finite() && value > 0.0 && value < 1.0 {
        value
    } else {
        WHEEL_DEAD_DEFAULT
    }
}

/// The whole dial's turn, in degrees clockwise from 12 o'clock. Any finite
/// number is folded into one revolution rather than rejected, because the mouse
/// that moves it can cross either end of the scale.
fn clamp_rotation(value: f32) -> f32 {
    if value.is_finite() {
        value.rem_euclid(360.0)
    } else {
        WHEEL_ROTATE_DEFAULT
    }
}

/// A paired size - the panel's remembered width or height - is either a size
/// somebody chose or a refusal. Below the smallest page the panel can draw it is
/// not a size at all, so it becomes `0` and the panel opens on its own terms.
/// The ceiling is wide enough for the largest monitor and finite, because the
/// arithmetic downstream is in floats and `NaN` is not a window.
fn clamp_panel_size(value: f32) -> f32 {
    if !value.is_finite() || value < 400.0 {
        0.0
    } else {
        value.min(4000.0)
    }
}

/// A numbered choice - a palette, a fill style, an animation length - is read as
/// an index and falls back to the shipped one for anything it cannot hold.
fn parse_index(value: &str, max: u32, fallback: u32) -> u32 {
    match value.trim().parse::<u32>() {
        Ok(v) => v.min(max),
        Err(_) => fallback,
    }
}

/// Physical pixels of travel before the dial opens. The bounds are the gesture
/// core's own: under two is a shiver, past forty is a chore on a touchpad.
fn clamp_threshold(value: i32) -> i32 {
    value.clamp(
        shaiwin_core::gesture::THRESHOLD_MIN,
        shaiwin_core::gesture::THRESHOLD_MAX,
    )
}

/// Copies a file the app cannot use to somewhere a person can still open it.
/// Answers where, or `None` when the copy itself failed - which is reported, not
/// swallowed, because a settings file that vanishes is worse than one that was
/// never readable.
fn stash_damaged(file: &Path, backup: &Path) -> Option<PathBuf> {
    match fs::copy(file, backup) {
        Ok(_) => Some(backup.to_path_buf()),
        Err(_) => None,
    }
}

/// The bytes go to a neighbour and move over the target afterwards. A move that
/// is refused because something else has the target open is tried again twice,
/// then given up on; anything else is not worth waiting for.
fn write_atomic(file: &Path, text: &str) -> io::Result<()> {
    let tmp = temp_beside(file);
    fs::write(&tmp, text)?;
    let mut refused = None;
    for wait in WRITE_TRIES {
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        match fs::rename(&tmp, file) {
            Ok(_) => return Ok(()),
            Err(err) if is_lock(&err) => refused = Some(err),
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                return Err(err);
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    Err(refused.unwrap_or_else(|| io::Error::other("the file would not move into place")))
}

/// The error a busy file answers with. `raw_os_error` is checked as well as the
/// kind because a sharing violation and a lock violation have no `ErrorKind` of
/// their own on Windows, and they are precisely the two that clear on their own.
fn is_lock(err: &io::Error) -> bool {
    matches!(err.kind(), io::ErrorKind::PermissionDenied)
        || matches!(err.raw_os_error(), Some(5) | Some(32) | Some(33))
}

fn temp_beside(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| format!("{}.tmp", n.to_string_lossy()))
        .unwrap_or_else(|| "settings.txt.tmp".to_string());
    file.with_file_name(name)
}

/// Keeps the first entry for each key and drops the rest. A hand-edited file can
/// name one window twice, and the app reads the first match - so the first is
/// the one that survives, and what is on disk says the same thing the dial does.
fn dedupe_by<T, K: PartialEq>(list: &mut Vec<T>, mut key: impl FnMut(&T) -> K) {
    let mut seen: Vec<K> = Vec::with_capacity(list.len());
    let mut kept: Vec<T> = Vec::with_capacity(list.len());
    for item in list.drain(..) {
        let k = key(&item);
        if seen.contains(&k) {
            continue;
        }
        seen.push(k);
        kept.push(item);
    }
    *list = kept;
}

fn dedupe(list: &mut Vec<String>) {
    let mut seen: Vec<String> = Vec::with_capacity(list.len());
    list.retain(|item| {
        if seen.contains(item) {
            return false;
        }
        seen.push(item.clone());
        true
    });
}

fn parse_vk(value: &str) -> u32 {
    value.parse::<u32>().map(|v| v & 0xFF).unwrap_or(0)
}

/// The dial's radius as the file states it, in logical pixels.
///
/// The ceiling here is looser than `core::wheel`'s on purpose: what a person
/// gets away from the ruler is the ruler's answer, and what this answers is
/// "that is not a radius of a window's neighbour", which is a wider question.
/// The number is [`clamp_radius`]'s, so a value typed into the panel and a value
/// read out of the file are held to the same one rule.
pub fn parse_wheel_radius(value: &str) -> f32 {
    clamp_radius(read_number(value))
}

/// The hub as a fraction of the radius, which is the one number with no unit to
/// argue about. Outside nought to one it is not a fraction, so the file said
/// something else and the shipped answer is the honest reading.
pub fn parse_wheel_dead(value: &str) -> f32 {
    clamp_dead(read_number(value))
}

/// The whole dial's turn as the file states it, in degrees clockwise.
pub fn parse_wheel_rotate(value: &str) -> f32 {
    clamp_rotation(read_number(value))
}

/// A number the file could hold. Text that is not one, and a number that is not
/// a place on the ruler, both come back as something no clamp would accept, so
/// the caller's own fallback is what lands.
fn read_number(value: &str) -> f32 {
    value.trim().parse::<f32>().unwrap_or(f32::NAN)
}

/// Physical pixels of travel before the dial opens. The bounds are the gesture
/// core's own: under two is a shiver, past forty is a chore on a touchpad.
pub fn parse_trigger_threshold(value: &str) -> i32 {
    match value.trim().parse::<i32>() {
        Ok(v) => clamp_threshold(v),
        Err(_) => TRIGGER_THRESHOLD_DEFAULT,
    }
}

/// Only the four keys a drag can be asked to carry.
pub fn parse_trigger_mods(value: &str) -> u32 {
    value
        .parse::<u32>()
        .map(|v| v & (MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN))
        .unwrap_or(MOD_CONTROL)
}

/// A virtual-key code the user may bind: the low byte of a Windows VK. Modifier
/// keys are stored as modifiers, never as the key itself.
fn parse_trigger_key(value: &str) -> u32 {
    value
        .trim()
        .parse::<u32>()
        .map(|v| v & 0xFF)
        .unwrap_or(0)
}

fn is_modifier_vk(vk: u32) -> bool {
    matches!(
        vk,
        0x10 | 0x11 | 0x12 // Shift, Ctrl, Alt (generic)
            | 0xA0 | 0xA1 // L/R Shift
            | 0xA2 | 0xA3 // L/R Ctrl
            | 0xA4 | 0xA5 // L/R Alt
            | 0x5B | 0x5C // L/R Win
    )
}

/// The pinned directions, on one line: `bearing:fingerprint` pairs separated by
/// commas.
///
/// The characters a fingerprint can contain that would end a pair early - the
/// comma, which is the separator, the backslash, which is nearly every character
/// of a Windows path, and the `=` the tag line uses - are all written escaped by
/// [`escape_field`] rather than forbidden. The bearing never needs it: it is a
/// rounded number.
pub fn format_pins(pins: &[Pin]) -> String {
    pins.iter()
        .map(|pin| {
            let bearing = pin.bearing.rem_euclid(360.0);
            // `147/2` is the ring-aware form, `147` a direction only, and the
            // trailing `*1.5` adds the sector's share of the ring. The weight is
            // only written when it differs from the shipped one, so a file made
            // before sizes were kept round-trips byte for byte.
            let mut desc = format!("{bearing:.0}");
            if let Some(ring) = pin.ring {
                desc.push('/');
                desc.push_str(&ring.to_string());
            }
            if let Some(weight) = pin.weight {
                if (weight - SECTOR_WEIGHT_DEFAULT).abs() > 0.01 {
                    desc.push('*');
                    desc.push_str(&format!("{weight:.2}"));
                }
            }
            format!("{desc}:{}", escape_field(&pin.window))
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The other half of [`format_pins`]. A pair whose number is not a number is
/// dropped rather than guessed at: the file was edited by a person, and a pin
/// that lands on a direction nobody wrote is worse than one that is missing.
pub fn parse_pins(value: &str) -> Vec<Pin> {
    let mut pins = Vec::new();
    for pair in split_fields(value, ',') {
        let Some((bearing, window)) = pair.split_once(':') else {
            continue;
        };
        // `147/2` is the ring-aware form; a bare `147` is every file written
        // before rings were kept, and it must still load as a direction only.
        // `147/2*1.5` adds the sector's weight; the `*` is optional.
        let (desc, weight) = match bearing.trim().split_once('*') {
            Some((desc, weight)) => (desc, weight.trim().parse::<f32>().ok()),
            None => (bearing.trim(), None),
        };
        let (bearing, ring) = match desc.trim().split_once('/') {
            Some((bearing, ring)) => (bearing, ring.trim().parse::<usize>().ok()),
            None => (desc.trim(), None),
        };
        let Ok(bearing) = bearing.trim().parse::<f32>() else {
            continue;
        };
        let window = unescape_field(window);
        if window.is_empty() {
            continue;
        }
        pins.push(Pin {
            bearing: bearing.rem_euclid(360.0),
            ring: ring.filter(|r| *r < shaiwin_core::wheel::MAX_RINGS),
            weight: weight
                .filter(|w| w.is_finite())
                .map(|w| w.clamp(SECTOR_WEIGHT_MIN, SECTOR_WEIGHT_MAX)),
            window,
        });
    }
    pins
}

/// The three characters that would end an entry, a pair, or the whole line before
/// their own text is finished: the comma that separates, the `=` that splits a tag
/// from the window it labels, and the backslash that does the escaping. Anything
/// else a fingerprint or a tag can hold goes through the line untouched.
fn escape_field(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(',', "\\,")
        .replace('=', "\\=")
}

/// The favourites list, one escaped fingerprint per entry, in the order the user
/// added them. The order is the whole content of the line - the dial seats the
/// favourites by it - so it is written as a list and never as a set.
pub fn format_favorites(list: &[String]) -> String {
    list.iter()
        .map(|fingerprint| escape_field(fingerprint))
        .collect::<Vec<_>>()
        .join(",")
}

/// The windows taken off the dial, one decimal handle per entry. A handle is a
/// number, so it needs none of the escaping a fingerprint does.
fn format_handles(list: &[isize]) -> String {
    list.iter()
        .map(isize::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// The other half of [`format_handles`]. A line written before windows were
/// keyed by handle held fingerprints instead; those entries name no single
/// window, so they are dropped rather than applied to whichever window still
/// matched - a strike that could land on the wrong document is worse than one
/// the user has to make again.
fn parse_handles(value: &str) -> Vec<isize> {
    value
        .split(',')
        .filter_map(|entry| entry.trim().parse::<isize>().ok())
        .filter(|hwnd| *hwnd != 0)
        .collect()
}

/// The other half of [`format_favorites`], in the same order.
pub fn parse_favorites(value: &str) -> Vec<String> {
    split_fields(value, ',')
        .into_iter()
        .map(|entry| unescape_field(&entry))
        .filter(|entry| !entry.is_empty())
        .collect()
}

/// The labels, on one line: `fingerprint=tag` pairs separated by commas. Both
/// halves are escaped, which is what lets a tag carry an `=` and a fingerprint
/// carry the comma a Windows path can hold.
pub fn format_labels(tags: &[Label]) -> String {
    tags.iter()
        .map(|label| {
            format!(
                "{}={}",
                escape_field(&label.window),
                escape_field(&label.tag)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The other half of [`format_labels`]. A pair with no separator, or an empty tag,
/// is dropped rather than guessed at - the same rule the pin line follows, for the
/// same reason: a person edited this file, and a label invented for a window is
/// worse than one that is missing.
pub fn parse_labels(value: &str) -> Vec<Label> {
    let mut tags = Vec::new();
    for pair in split_fields(value, ',') {
        let Some((window, tag)) = split_once_field(&pair, '=') else {
            continue;
        };
        let (window, tag) = (unescape_field(window), clean_tag(&unescape_field(tag)));
        if window.is_empty() || tag.is_empty() {
            continue;
        }
        tags.push(Label { window, tag });
    }
    tags
}

/// The hand-picked sector colours, on one line: `fingerprint=RRGGBB` pairs
/// separated by commas. The colour is six hex digits so the file can be read and
/// edited without a colour picker.
pub fn format_colors(colors: &[Colour]) -> String {
    colors
        .iter()
        .map(|colour| {
            format!(
                "{}={:02X}{:02X}{:02X}",
                escape_field(&colour.window),
                colour.rgb.0,
                colour.rgb.1,
                colour.rgb.2
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// The other half of [`format_colors`]. A pair without `=`, a window with no
/// name, or a colour that is not six hex digits is dropped: a colour invented for
/// a window is worse than one that is missing.
pub fn parse_colors(value: &str) -> Vec<Colour> {
    let mut colors = Vec::new();
    for pair in split_fields(value, ',') {
        let Some((window, rgb)) = split_once_field(&pair, '=') else {
            continue;
        };
        let window = unescape_field(window);
        let rgb = rgb.trim();
        if window.is_empty() || rgb.len() != 6 {
            continue;
        }
        let Ok(packed) = u32::from_str_radix(rgb, 16) else {
            continue;
        };
        colors.push(Colour {
            window,
            rgb: Rgb((packed >> 16) as u8, (packed >> 8) as u8, packed as u8),
        });
    }
    colors
}

/// The hand-picked sector sizes, on one line: `fingerprint=weight` pairs
/// separated by commas. The weight is a plain float so a hand edit can widen a
/// slice without opening the panel.
pub fn format_weights(weights: &[WindowWeight]) -> String {
    weights
        .iter()
        .map(|w| format!("{}={}", escape_field(&w.window), w.weight))
        .collect::<Vec<_>>()
        .join(",")
}

/// The other half of [`format_weights`]. A pair without `=`, a window with no
/// name, or a weight that is not a finite number is dropped: a guessed size is
/// worse than the default the dial already gives the slice.
pub fn parse_weights(value: &str) -> Vec<WindowWeight> {
    let mut weights = Vec::new();
    for pair in split_fields(value, ',') {
        let Some((window, raw)) = split_once_field(&pair, '=') else {
            continue;
        };
        let window = unescape_field(window);
        let Ok(weight) = raw.trim().parse::<f32>() else {
            continue;
        };
        if window.is_empty() || !weight.is_finite() {
            continue;
        }
        weights.push(WindowWeight { window, weight });
    }
    weights
}

/// Split on the commas that are not inside an escaped pair. Written as a walk
/// rather than `split(',')` because `D:\\a.exe|C|X,D:\\b.exe|C|Y` is two pairs
/// and the comma after `.exe` would otherwise start a third.
fn split_fields(value: &str, sep: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for c in value.chars() {
        if escaped {
            current.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => {
                current.push(c);
                escaped = true;
            }
            other if other == sep => {
                if !current.is_empty() {
                    out.push(std::mem::take(&mut current));
                }
            }
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

/// The first `sep` that is not escaped, and the two halves around it. A tag is
/// written escaped for exactly this reason: `=` is a character a person types in a
/// title, and the separator has to be the one the line itself put there.
fn split_once_field(value: &str, sep: char) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (i, c) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            other if other == sep => {
                return Some((&value[..i], &value[i + c.len_utf8()..]));
            }
            _ => {}
        }
    }
    None
}

/// Back to what the field really said. A trailing backslash with nothing behind it
/// is a hand-edit, and the literal character is the reading that loses nothing.
fn unescape_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut escaped = false;
    for c in text.chars() {
        if escaped {
            out.push(c);
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            out.push(c);
        }
    }
    if escaped {
        out.push('\\');
    }
    out
}

/// Which keys and which button open the dial, in the shape the gesture machine
/// is built from.
///
/// One thing is refused on the way: a trigger with no modifier, no key and an
/// ordinary mouse button asks for every drag, so that empty gesture falls back to
/// the shipped `Ctrl`. A key on its own is allowed - a mouse side button remapped
/// to a key is exactly the single-press trigger somebody may want - and so is a
/// side button (`Back`/`Forward`), which has no everyday click to steal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerProblem {
    /// Not a usable gesture at all.
    Invalid,
    /// The same chord as one of the two shortcuts.
    Conflict,
    /// Usable, but likely to collide with something the desktop already does.
    Risky,
}

pub fn trigger_for(mods: u32, vk: u32, button: u8, threshold: i32) -> Trigger {
    let mut want = Mods {
        alt: mods & MOD_ALT != 0,
        ctrl: mods & MOD_CONTROL != 0,
        shift: mods & MOD_SHIFT != 0,
        win: mods & MOD_WIN != 0,
    };
    // A bare left/right/middle drag would swallow every click on the desktop; a
    // bare key or a side button is the user's own choice and is left alone.
    if !(want.alt || want.ctrl || want.shift || want.win) && vk == 0 && button < 3 {
        want = Mods::ctrl();
    }
    Trigger {
        mods: want,
        input: if vk == 0 {
            TriggerInput::Mouse(button_for(button))
        } else {
            TriggerInput::Key(vk as u16)
        },
        threshold: parse_trigger_threshold(&threshold.to_string()),
    }
}

fn button_for(button: u8) -> Button {
    match button {
        1 => Button::Right,
        2 => Button::Middle,
        3 => Button::X1,
        4 => Button::X2,
        _ => Button::Left,
    }
}

fn button_name(button: u8) -> &'static str {
    match button {
        1 => "Right",
        2 => "Middle",
        3 => "Back",
        4 => "Forward",
        _ => "Left",
    }
}

impl Settings {
    pub fn trigger(&self) -> Trigger {
        trigger_for(
            self.trigger_mods,
            self.trigger_vk,
            self.trigger_button,
            self.drag_threshold,
        )
    }

    pub fn trigger_button_name(&self) -> &'static str {
        button_name(self.trigger_button)
    }

}

pub fn trigger_label(mods: u32, vk: u32, button: u8) -> String {
    let mut parts = Vec::new();
    if mods & MOD_CONTROL != 0 {
        parts.push("Ctrl".to_string());
    }
    if mods & MOD_ALT != 0 {
        parts.push("Alt".to_string());
    }
    if mods & MOD_SHIFT != 0 {
        parts.push("Shift".to_string());
    }
    if mods & MOD_WIN != 0 {
        parts.push("Win".to_string());
    }
    if vk == 0 {
        parts.push(button_name(button).to_string());
    } else {
        parts.push(key_name(vk));
    }
    parts.join("+")
}

/// The rules the recorder applies before it writes a binding down.
pub fn trigger_problem(
    mods: u32,
    vk: u32,
    button: u8,
    settings: &Settings,
) -> Option<TriggerProblem> {
    if vk != 0 && is_modifier_vk(vk) {
        return Some(TriggerProblem::Invalid);
    }
    if vk == 0 && mods == 0 && button < 3 {
        // A bare left/right/middle button would take every click on the desktop.
        return Some(TriggerProblem::Invalid);
    }
    if vk != 0
        && ((mods & MOD_ALL == settings.fav_mods && vk == settings.fav_vk)
            || (mods & MOD_ALL == settings.panel_mods && vk == settings.panel_vk))
    {
        return Some(TriggerProblem::Conflict);
    }
    if mods & MOD_WIN != 0 || (vk != 0 && mods == 0) {
        return Some(TriggerProblem::Risky);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_readable() {
        assert_eq!(hotkey_label(MOD_CONTROL | MOD_ALT, 0x48), "Ctrl+Alt+H");
        assert_eq!(hotkey_label(0, 0x70), "F1");
        assert_eq!(hotkey_label(MOD_SHIFT, 0x41), "Shift+A");
    }

    #[test]
    fn a_tag_is_one_short_line() {
        assert_eq!(clean_tag("  work  "), "work");
        assert_eq!(clean_tag("a\tb\nc"), "a b c");
        assert_eq!(clean_tag("read, later"), "read later");
        assert_eq!(clean_tag(&"x".repeat(40)).chars().count(), 24);
    }

    #[test]
    fn a_new_label_starts_with_the_window_name() {
        assert_eq!(default_tag("Inbox - Outlook", "Outlook"), "Inbox");
        assert_eq!(
            default_tag("新建标签页 - 个人 - Microsoft Edge", "Msedge"),
            "新建标签页"
        );
        assert_eq!(default_tag("", "Notepad"), "Notepad");
        assert_eq!(default_tag("  a, b\n c  ", "App"), "a b c");
    }

    #[test]
    fn a_file_that_binds_one_chord_twice_keeps_the_wheel_shortcut() {
        let mut s = Settings::default();
        s.fav_mods = s.panel_mods;
        s.fav_vk = s.panel_vk;
        s.normalize();
        // The window shortcut is never quietly relabelled Ctrl+3; the panel is
        // the one that moves, to a chord of its own.
        assert_eq!((s.fav_mods, s.fav_vk), (MOD_CONTROL, VK_T));
        assert_eq!((s.panel_mods, s.panel_vk), (MOD_CONTROL | MOD_SHIFT, VK_T));
    }

    #[test]
    fn a_parked_window_keeps_its_place_in_the_order() {
        let mut s = Settings::default();
        s.seed_seat_order(&["a|one".into(), "b|two".into(), "c|three".into()]);
        assert_eq!(s.seat_order, vec!["a|one", "b|two", "c|three"]);
        s.seat_park("b|two");
        assert_eq!(s.seat_index("b|two"), Some(1));
        assert!(s.is_parked("b|two"));
        s.seat_add("b|two");
        assert_eq!(s.seat_index("b|two"), Some(1));
        assert!(!s.is_parked("b|two"));
    }

    #[test]
    fn an_explicit_add_claims_the_place_a_down_dial_emptied() {
        let mut s = Settings::default();
        s.wheel_limit = 3;
        s.seed_seat_order(&["a|one".into(), "b|two".into(), "c|three".into()]);
        // Take the middle window off. Its place stays in the order, empty.
        s.seat_park("b|two");
        // A window that was never on the dial is added explicitly. It must land
        // in the freed place, not be appended past the limit where no sector
        // would ever reach it.
        s.seat_add("d|four");
        assert_eq!(s.seat_order[..3], vec!["a|one", "d|four", "c|three"]);
        assert_eq!(s.seat_index("d|four"), Some(1));
        assert!(!s.is_parked("d|four"));
        // The window that left stays parked and cannot come back on its own.
        assert!(s.is_parked("b|two"));
        assert_eq!(s.seat_index("b|two"), Some(3));
        // Putting it back is an explicit request too, and the dial is full.
        s.seat_add("b|two");
        assert!(s.seat_index("b|two").is_some_and(|at| at >= 3));
    }

    #[test]
    fn an_add_to_a_dial_with_room_takes_the_next_free_place() {
        let mut s = Settings::default();
        s.wheel_limit = 3;
        s.seed_seat_order(&["a|one".into(), "b|two".into()]);
        s.seat_add("c|three");
        assert_eq!(s.seat_order, vec!["a|one", "b|two", "c|three"]);
        assert!(!s.is_parked("c|three"));
    }

    #[test]
    fn the_old_cap_s_overflow_is_not_a_manual_park() {
        let mut s = Settings::default();
        s.wheel_limit = 3;
        s.seat_order = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];
        // The old writer parked everything past the limit as well as the window
        // the user actually took off.
        s.parked = vec!["b".into(), "d".into(), "e".into()];
        s.drop_legacy_overflow(true);
        assert_eq!(s.parked, vec!["b"], "only the hand-taken seat stays parked");
        // A file written now already distinguishes the two, so nothing moves.
        s.parked = vec!["b".into(), "d".into(), "e".into()];
        s.drop_legacy_overflow(false);
        assert_eq!(s.parked, vec!["b", "d", "e"]);
    }

    #[test]
    fn the_defaults_are_bound() {
        let s = Settings::default();
        assert_eq!(s.fav_label(), "Ctrl+2");
        assert_eq!(s.panel_label(), "Ctrl+T");
        // The dial is on, with the key that opens it and the button that drags
        // it out - the shipped gesture, not a disabled one waiting to be found.
        assert!(s.wheel);
        assert_eq!(s.trigger_mods, MOD_ALT);
        assert_eq!(s.trigger_vk, 0);
        assert_eq!(s.trigger_button, 0);
        assert_eq!(
            trigger_label(s.trigger_mods, s.trigger_vk, s.trigger_button),
            "Alt+Left"
        );
        assert_eq!(s.drag_threshold, TRIGGER_THRESHOLD_DEFAULT);
        assert_eq!(s.wheel_radius, WHEEL_RADIUS_DEFAULT);
        assert_eq!(s.wheel_dead, WHEEL_DEAD_DEFAULT);
        // And the language is the system's until somebody says otherwise.
        assert_eq!(s.lang, crate::i18n::AUTO);
    }

    #[test]
    fn a_pin_survives_the_file_it_is_written_to() {
        let pins = vec![
            Pin {
                bearing: 0.0,
                ring: None,
                weight: None,
                window: r"C:\Windows\notepad.exe|Notepad|Untitled".to_string(),
            },
            Pin {
                bearing: 225.0,
                ring: Some(1),
                weight: Some(1.60),
                window: r"C:\Program Files\a, b\app.exe|Class, X|First".to_string(),
            },
        ];
        let line = format_pins(&pins);
        assert_eq!(parse_pins(&line), pins);
        // The comma inside a path is the reason a pair is escaped rather than
        // forbidden. Taken literally that same line would be four pins, two of
        // them a window cut in half.
        assert_eq!(line.matches(',').count(), 3);
        // A direction that is not a number is dropped, and so is a pair with no
        // window in it. The file is hand-editable, and a guessed pin is worse
        // than a missing one.
        assert!(parse_pins("nope:C|x,90:").is_empty());
        // Out of the circle and back round: a direction is still a direction.
        assert_eq!(parse_pins("-90:C|x")[0].bearing, 270.0);
        // Writing what was read changes nothing.
        assert_eq!(format_pins(&parse_pins(&line)), line);
    }

    #[test]
    fn a_favourite_survives_the_line_it_is_written_to() {
        let list = vec![
            r"C:\Windows\notepad.exe|Notepad|Untitled".to_string(),
            r"C:\Program Files\a, b\app.exe|Class, X|=First".to_string(),
        ];
        let line = format_favorites(&list);
        assert_eq!(parse_favorites(&line), list);
        // The order is the seating, so an entry that means "second on the inner
        // ring" has to come back second.
        assert_eq!(parse_favorites(&line)[1], list[1]);
        // A hand-edited line of nothing is dropped rather than becoming a window.
        assert!(parse_favorites(",").is_empty());
        assert_eq!(format_favorites(&parse_favorites(&line)), line);
    }

    #[test]
    fn a_tag_survives_the_line_and_the_characters_that_would_break_it() {
        let tags = vec![
            Label {
                window: r"C:\Windows\notepad.exe|Notepad|Untitled".to_string(),
                tag: "work".to_string(),
            },
            Label {
                window: r"D:\a, b\app.exe|Class|First".to_string(),
                tag: "a=b".to_string(),
            },
        ];
        let line = format_labels(&tags);
        assert_eq!(parse_labels(&line), tags);
        // A tag holding an `=` is the reason the separator is the one the line
        // wrote and not the first one the pair contains.
        assert_eq!(line.matches('=').count(), 3);
        assert_eq!(format_labels(&parse_labels(&line)), line);
        // A pair with no tag, and a tag that cleaning leaves empty, are dropped;
        // a sector cannot draw nothing and call it a label.
        assert!(parse_labels("C|x=,").is_empty());
        // What arrives is cleaned, whoever wrote the file.
        assert_eq!(parse_labels(r"C|x=a\,b")[0].tag, "a b");
    }

    #[test]
    fn the_favourite_list_and_the_labels_answer_to_one_window() {
        let mut s = Settings::default();
        let fp = r"C:\Windows\notepad.exe|Notepad|Untitled";
        let other = r"D:\code.exe|Code|main.rs";
        assert!(s.add_favorite(fp));
        assert!(!s.add_favorite(fp), "the same window twice is one seat");
        assert!(s.add_favorite(other));
        assert_eq!(s.favorite_rank(fp), Some(0));
        assert_eq!(s.favorite_rank(other), Some(1));
        s.set_tag(fp, "  notes  ");
        assert_eq!(s.tag_for(fp), Some("notes"));
        // Taking a window off the dial keeps the text the user typed: the label is
        // what cost something to write, not the seat.
        assert!(s.mark_off_dial(0x1204));
        assert!(!s.mark_off_dial(0x1204));
        assert!(!s.mark_off_dial(0), "a zero handle is nobody's window");
        assert_eq!(s.tag_for(fp), Some("notes"));
        s.set_tag(other, "");
        assert_eq!(s.tag_for(other), None);
    }

    #[test]
    fn an_off_dial_line_from_before_handles_names_no_window_and_is_dropped() {
        // The old line held `exe|class|first word` entries, which stand for a
        // whole group of windows. A handle is the only thing that names one.
        let (s, _) = parse(
            r"C:\Windows\notepad.exe|Notepad|Untitled,4660
off_dial=C:\Windows\notepad.exe|Notepad|Untitled,4660
",
        );
        assert_eq!(s.off_dial, vec![4660]);
    }

    #[test]
    fn the_dial_reads_the_trigger_the_way_the_machine_wants_it() {
        use shaiwin_core::gesture::Button;
        let mut s = Settings::default();
        let t = s.trigger();
        assert!(t.mods.alt && t.input == TriggerInput::Mouse(Button::Left));
        // A chord stays a chord: the recorder can ask for any combination.
        s.trigger_mods = MOD_ALT | MOD_CONTROL | MOD_SHIFT;
        s.trigger_button = 2;
        let t = s.trigger();
        assert!(t.mods.alt && t.mods.ctrl && t.mods.shift);
        assert_eq!(t.input, TriggerInput::Mouse(Button::Middle));
        // A key trigger is carried through as a key, not as a button.
        s.trigger_mods = MOD_CONTROL;
        s.trigger_vk = 0x76;
        let t = s.trigger();
        assert_eq!(t.input, TriggerInput::Key(0x76));
        // A file that leaves every modifier out asks for Ctrl when the trigger is
        // a mouse button - a bare drag is not a gesture the desktop can afford.
        s.trigger_mods = 0;
        s.trigger_vk = 0;
        let t = s.trigger();
        assert!(t.mods.ctrl);
        assert_eq!(t.mods, shaiwin_core::gesture::Mods::ctrl());
    }

    #[test]
    fn a_recording_that_would_steal_the_desktop_is_refused() {
        let s = Settings::default();
        // A bare click is not a gesture.
        assert_eq!(trigger_problem(0, 0, 0, &s), Some(TriggerProblem::Invalid));
        // A side button alone has no everyday click to steal and is allowed.
        assert_eq!(trigger_problem(0, 0, 3, &s), None);
        assert_eq!(trigger_problem(0, 0, 4, &s), None);
        // A single key is allowed for mouse-side-button remaps; a letter warns
        // because it will also fire while typing.
        assert_eq!(trigger_problem(0, 0x41, 0, &s), Some(TriggerProblem::Risky));
        // A function key is just as usable, with the same warning.
        assert_eq!(trigger_problem(0, 0x76, 0, &s), Some(TriggerProblem::Risky));
        // A modifier is a chord part, never the key.
        assert_eq!(trigger_problem(MOD_ALT, 0x11, 0, &s), Some(TriggerProblem::Invalid));
        // Exactly the favourite or panel chord is a conflict. The favourite
        // answers to Ctrl+2, the panel to Ctrl+T.
        let fav = trigger_problem(MOD_CONTROL, VK_2, 0, &s);
        assert_eq!(fav, Some(TriggerProblem::Conflict));
        assert_eq!(trigger_problem(MOD_ALT, 0, 0, &s), None);
    }

    #[test]
    fn a_side_button_alone_is_a_trigger() {
        let s = Settings {
            trigger_mods: 0,
            trigger_vk: 0,
            trigger_button: 3,
            ..Settings::default()
        };
        let (back, _) = parse(&s.render());
        assert_eq!(back.trigger_button, 3);
        assert_eq!(back.trigger().input, TriggerInput::Mouse(Button::X1));
        assert!(!back.trigger().mods.alt && !back.trigger().mods.ctrl);
        assert_eq!(trigger_label(0, 0, 3), "Back");
        assert_eq!(trigger_label(0, 0, 4), "Forward");
    }

    #[test]
    fn the_file_round_trips_as_the_whole() {
        let s = Settings {
            wheel_radius: 180.0,
            wheel_dead: 0.5,
            drag_threshold: 14,
            trigger_mods: MOD_CONTROL | MOD_SHIFT,
            trigger_vk: 0x76,
            trigger_button: 1,
            favorites: vec![
                r"C:\Windows\notepad.exe|Notepad|Untitled".to_string(),
                r"D:\a, b\app.exe|Class|First".to_string(),
            ],
            seat_order: vec![
                r"D:\a, b\app.exe|Class|First".to_string(),
                r"C:\Windows\notepad.exe|Notepad|Untitled".to_string(),
            ],
            bound: vec![r"C:\Windows\notepad.exe|Notepad|Untitled".to_string()],
            parked: vec![r"C:\Windows\notepad.exe|Notepad|Untitled".to_string()],
            pinned_slots: vec![Pin {
                bearing: 225.0,
                ring: Some(0),
                weight: None,
                window: r"D:\a, b\app.exe|Class|First".to_string(),
            }],
            tags: vec![Label {
                window: r"C:\Windows\notepad.exe|Notepad|Untitled".to_string(),
                tag: "a=b".to_string(),
            }],
            excluded: vec!["searchui.exe".to_string()],
            off_dial: vec![0x1204, 0x3b0160],
            seat_hwnds: vec![0x1204, 0x55a20],
            activate_pos: 1,
            wheel_rotate: 45.0,
            lang: "en-US".to_string(),
            ..Settings::default()
        };
        let text = s.render();
        let (back, recognised) = parse(&text);
        assert_eq!(recognised, 37, "every line this app writes is one it reads");
        assert_eq!(back.activate_pos, 1, "and the position survives the file");
        assert_eq!(back.wheel_rotate, 45.0, "and the dial's turn survives");
        assert_eq!(
            back.favorites, s.favorites,
            "and in the order it was written"
        );
        assert_eq!(back.seat_order, s.seat_order, "the dial's order survives");
        assert_eq!(back.bound, s.bound, "and so do the explicit seats");
        assert_eq!(back.parked, s.parked, "and so do the parked seats");
        assert_eq!(back.pinned_slots, s.pinned_slots);
        assert_eq!(back.wheel_limit, s.wheel_limit);
        assert_eq!(back.dial_order, s.dial_order);
        assert_eq!(back.tags, s.tags);
        assert_eq!(back.weights, s.weights);
        assert_eq!(back.excluded, s.excluded);
        assert_eq!(back.off_dial, s.off_dial, "handles, one per window");
        assert_eq!(
            back.seat_hwnds, s.seat_hwnds,
            "and the hand-seated handles with them"
        );
        assert_eq!(back.lang, s.lang);
        assert!((back.wheel_radius - 180.0).abs() < 1e-6);
        assert!((back.wheel_dead - 0.5).abs() < 1e-6);
        assert_eq!(back.drag_threshold, 14);
        assert_eq!(back.trigger_mods, MOD_CONTROL | MOD_SHIFT);
        assert_eq!(back.trigger_vk, 0x76);
        assert_eq!(back.trigger_button, 1);
        assert_eq!(
            trigger_label(back.trigger_mods, back.trigger_vk, back.trigger_button),
            "Ctrl+Shift+F7"
        );
        // The file a person finds is the file the next start reads back, byte
        // for byte.
        assert_eq!(back.render(), text);
    }

    #[test]
    fn hand_picked_sector_weights_round_trip_and_default_is_dropped() {
        let a = r"C:\Windows\notepad.exe|Notepad|Untitled";
        let b = r"D:\code.exe|Code|main.rs";
        let mut s = Settings::default();
        assert!(s.set_weight(a, Some(1.5)));
        assert!(s.set_weight(b, Some(0.8)));
        assert_eq!(s.weight_for(a), 1.5);
        assert_eq!(s.weight_for(b), 0.8);
        assert_eq!(s.weight_for("missing"), SECTOR_WEIGHT_DEFAULT);
        let text = s.render();
        let (back, _) = parse(&text);
        assert_eq!(back.weight_for(a), 1.5);
        assert_eq!(back.weight_for(b), 0.8);
        // A size set back to the default leaves no entry: the file keeps only
        // sizes that actually change a sector, so a reset is a clean removal.
        assert!(s.set_weight(a, Some(SECTOR_WEIGHT_DEFAULT)));
        assert_eq!(s.weight_for(a), SECTOR_WEIGHT_DEFAULT);
        assert!(s.weights.iter().all(|w| w.window != a));
    }

    #[test]
    fn one_normalizer_answers_for_every_way_a_number_arrives() {
        let mut s = Settings {
            wheel_radius: 900.0,
            wheel_dead: 0.0,
            drag_threshold: 500,
            trigger_mods: 99,
            fav_vk: 0x316,
            ..Settings::default()
        };
        s.normalize();
        assert_eq!(s.wheel_radius, WHEEL_RADIUS_MAX);
        assert_eq!(s.wheel_dead, WHEEL_DEAD_DEFAULT);
        assert_eq!(s.drag_threshold, shaiwin_core::gesture::THRESHOLD_MAX);
        assert_eq!(
            s.trigger_mods,
            MOD_CONTROL | MOD_ALT,
            "unknown bits go, known ones stay"
        );
        assert_eq!(s.fav_vk, 0x16);
        // A list with a window named twice, a tag cleaning leaves empty, and an
        // exclusion in two spellings: the dial reads the first of each, so the
        // first is the one that stays.
        let a = r"C:\Windows\notepad.exe|Notepad|Untitled";
        let b = r"D:\code.exe|Code|main.rs";
        let mut s = Settings {
            favorites: vec![a.to_string(), b.to_string(), a.to_string()],
            pinned_slots: vec![
                Pin {
                    bearing: 90.0,
                    ring: None,
                    weight: None,
                    window: a.to_string(),
                },
                Pin {
                    bearing: 200.0,
                    ring: None,
                    weight: None,
                    window: a.to_string(),
                },
            ],
            tags: vec![
                Label {
                    window: a.to_string(),
                    tag: "notes".to_string(),
                },
                Label {
                    window: a.to_string(),
                    tag: "wrong".to_string(),
                },
                Label {
                    window: b.to_string(),
                    tag: "   ".to_string(),
                },
            ],
            excluded: vec!["searchui.exe".to_string(), "SEARCHUI.exe".to_string()],
            ..Settings::default()
        };
        s.normalize();
        assert_eq!(s.favorites, vec![a.to_string(), b.to_string()]);
        assert_eq!(s.pinned_slots.len(), 1);
        assert_eq!(s.pinned_slots[0].bearing, 90.0);
        assert_eq!(
            s.tags.len(),
            1,
            "one label per window, and none of them empty"
        );
        assert_eq!(s.tag_for(a), Some("notes"));
        assert_eq!(s.tag_for(b), None);
        assert_eq!(s.excluded, vec!["searchui.exe".to_string()]);
        assert!(s.is_excluded("searchui.exe"));
        // Already normalised, the same pass changes nothing - which is what makes
        // it safe to call on every path into and out of the file.
        let before = s.render();
        s.normalize();
        assert_eq!(s.render(), before);
    }

    #[test]
    fn a_line_this_app_never_wrote_is_counted_not_applied() {
        let (_, recognised) = parse("this is not a settings file\nat all");
        assert_eq!(
            recognised, 0,
            "nothing to understand, and the caller keeps the file"
        );
        let (_, recognised) = parse("wheel=1\nwheel_radius=180\nnonsense=1\n");
        assert_eq!(recognised, 2, "one line the app has never heard of");
        let (s, _) = parse("wheel_radius=180\nlang=no-such-pack");
        assert!((s.wheel_radius - 180.0).abs() < 1e-6);
        // A pack name the folder does not hold is still kept as written: the
        // folder decides what exists, and dropping the request would lose the
        // pack the user drops in tomorrow. What cannot be a name at all is.
        assert_eq!(s.lang, "no-such-pack");
        let (s, _) = parse("lang=../../windows");
        assert_eq!(s.lang, crate::i18n::AUTO);
    }

    /// A directory of this test run's own, under the system's temp folder. Never
    /// the app's data directory: a test that writes there would be writing over
    /// the settings of whoever runs it.
    fn scratch_dir(name: &str) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("shaiwin-{name}-{}-{stamp}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn a_write_lands_whole_or_not_at_all() {
        let dir = scratch_dir("write-atomic");
        let file = dir.join("settings.txt");
        write_atomic(&file, "wheel=1\n").expect("a fresh file in a fresh directory");
        assert_eq!(fs::read_to_string(&file).unwrap(), "wheel=1\n");
        write_atomic(&file, "wheel=0\n").expect("and so does the one that replaces it");
        assert_eq!(fs::read_to_string(&file).unwrap(), "wheel=0\n");
        assert!(
            !temp_beside(&file).exists(),
            "the neighbour is moved over, not left behind"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_refusal_that_clears_is_worth_waiting_for() {
        // 5 access denied, 32 sharing violation, 33 lock violation: the three a
        // virus scan or an indexer answers with, and all three over in a moment.
        for code in [5, 32, 33] {
            assert!(is_lock(&io::Error::from_raw_os_error(code)), "{code}");
        }
        assert!(is_lock(&io::Error::from(io::ErrorKind::PermissionDenied)));
        assert!(
            !is_lock(&io::Error::from(io::ErrorKind::NotFound)),
            "a missing directory does not come back if you wait for it"
        );
    }

    #[test]
    fn a_file_that_cannot_be_understood_is_kept_first() {
        let dir = scratch_dir("backup");
        let file = dir.join("settings.txt");
        let backup = dir.join("settings.bak.txt");
        fs::write(&file, "not a settings file at all").unwrap();
        assert_eq!(
            stash_damaged(&file, &backup).as_deref(),
            Some(backup.as_path()),
            "and the answer says where it went"
        );
        assert_eq!(
            fs::read_to_string(&backup).unwrap(),
            "not a settings file at all"
        );
        // A file that is not there cannot be kept. The caller has to say so
        // rather than pretend the old one is safe somewhere.
        assert!(stash_damaged(&dir.join("gone.txt"), &backup).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hand_edited_dial_is_clamped_not_trusted() {
        assert_eq!(parse_trigger_threshold(""), TRIGGER_THRESHOLD_DEFAULT);
        assert_eq!(parse_trigger_threshold("nope"), TRIGGER_THRESHOLD_DEFAULT);
        assert_eq!(
            parse_trigger_threshold("1"),
            shaiwin_core::gesture::THRESHOLD_MIN
        );
        assert_eq!(
            parse_trigger_threshold("500"),
            shaiwin_core::gesture::THRESHOLD_MAX
        );
        assert_eq!(parse_trigger_threshold("14"), 14);
        // A fraction that is not a fraction, and a radius that is not a radius,
        // both come back as the shipped numbers rather than as a broken dial.
        assert_eq!(parse_wheel_dead("0"), WHEEL_DEAD_DEFAULT);
        assert_eq!(parse_wheel_dead("1.4"), WHEEL_DEAD_DEFAULT);
        assert!((parse_wheel_dead("0.5") - 0.5).abs() < 1e-6);
        assert_eq!(parse_wheel_radius("-40"), WHEEL_RADIUS_DEFAULT);
        assert_eq!(parse_wheel_radius("nope"), WHEEL_RADIUS_DEFAULT);
        assert_eq!(parse_wheel_radius("900"), WHEEL_RADIUS_MAX);
        assert!((parse_wheel_radius("180") - 180.0).abs() < 1e-6);
        // The raw parser keeps every bit a drag could carry; it is the settings
        // reader that narrows a hand-edited value down to one key.
        assert_eq!(
            parse_trigger_mods("15"),
            MOD_ALT | MOD_CONTROL | MOD_SHIFT | MOD_WIN
        );
        // 99 is Alt and Ctrl with five bits of nonsense above them: the mask keeps
        // the two keys a drag can mean and drops the rest.
        assert_eq!(parse_trigger_mods("99"), MOD_ALT | MOD_CONTROL);
        assert_eq!(parse_trigger_mods(""), MOD_CONTROL);
    }
}
