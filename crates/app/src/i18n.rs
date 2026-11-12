//! The strings the app shows, and the language pack files that may replace them.
//!
//! Two languages ship with the program: English and Simplified Chinese. A third
//! is a file. `%LOCALAPPDATA%\ShaiZero\languages\<tag>.txt` holds `key=value`
//! lines, `#` starts a comment, and a key beginning with `_` describes the pack
//! instead of translating it (`_name` is what the language calls itself, `_font`
//! the face it needs). A pack only has to carry the lines it wants to change:
//! anything missing falls back to the language the program ships with, and then
//! to English, so a twenty-line file is a working translation.
//!
//! The shipped languages are written into that folder on every start - never
//! over a file the user has touched - so there is always a complete, editable
//! copy of every string lying around to begin a new language from.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

use windows::core::PCWSTR;
use windows::Win32::Globalization::GetUserDefaultUILanguage;

/// Not a language: show whatever the system speaks. The only value of
/// [`Settings::lang`](crate::settings::Settings::lang) that is not a tag.
pub const AUTO: &str = "auto";
/// The language every fallback ends in, and the one the table is written in.
pub const EN: &str = "en";
/// Simplified Chinese, the language the program was written for.
pub const ZH: &str = "zh-CN";

const FACE_CJK: &str = "Microsoft YaHei UI";
const FACE_LATIN: &str = "Segoe UI";
/// Tags that name a language the program has strings for, in the order the
/// settings row shows them.
const SHIPPED: [&str; 2] = [EN, ZH];

pub fn languages_dir() -> PathBuf {
    crate::settings::data_dir().join("languages")
}

/// A tag becomes a file name, and it also comes out of a settings file somebody
/// edited by hand, so it is checked rather than trusted: nothing that could
/// climb out of the languages folder, and nothing too long to be a tag.
pub fn is_safe_tag(tag: &str) -> bool {
    !tag.is_empty()
        && tag.len() <= 40
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// What the stored or typed tag means once it has been looked at: `auto`, a tag
/// that could be a file name, or `auto` again.
pub fn lang_tag(tag: &str) -> String {
    let tag = tag.trim();
    if tag == AUTO || is_safe_tag(tag) {
        tag.to_string()
    } else {
        AUTO.to_string()
    }
}

/// The language the system speaks. Everything that is not Chinese is answered
/// with English, which is the language the table itself is written in.
pub fn system_tag() -> String {
    let lang = unsafe { GetUserDefaultUILanguage() };
    // The primary language id is the low ten bits; 0x04 is Chinese.
    if (lang & 0x3FF) == 0x04 {
        ZH.to_string()
    } else {
        EN.to_string()
    }
}

/// `auto` is what it says; any other tag is the answer.
pub fn resolve(tag: &str) -> String {
    if tag == AUTO {
        system_tag()
    } else {
        tag.to_string()
    }
}

struct Catalog {
    strings: HashMap<&'static str, &'static str>,
    /// The face this language is drawn with, as a null-terminated wide string
    /// that lives as long as the process. Decided from the language itself -
    /// see [`needs_cjk`] - so a pack never has to name a font to get one.
    face: &'static [u16],
}

/// A language written in Chinese, Japanese or Korean glyphs needs a face that
/// carries them. Decided from the tag and from the strings the pack actually
/// holds, because a partial Japanese pack would otherwise ask for Segoe and
/// render its own five lines as empty boxes.
fn needs_cjk(tag: &str, strings: &HashMap<&'static str, &'static str>) -> bool {
    let prefix = tag.split(['-', '_']).next().unwrap_or("");
    let spoken_cjk = matches!(prefix, "zh" | "ja" | "ko");
    spoken_cjk
        || strings
            .values()
            .any(|value| value.chars().any(|c| matches!(c, '\u{2E80}'..='\u{9FFF}')))
}

impl Catalog {
    /// The strings in force for `tag`: English first because the table is written
    /// in it and every key exists there, then the shipped strings for this
    /// language, then the user's file for this language.
    fn load(tag: &str) -> Catalog {
        let mut strings: HashMap<&'static str, &'static str> =
            BUILTIN.iter().map(|(key, en, _)| (*key, *en)).collect();
        if tag == ZH {
            for &(key, _, zh) in BUILTIN {
                strings.insert(key, zh);
            }
        }
        let mut face: Option<&'static str> = None;
        if let Some(text) = read_pack(tag) {
            for (key, value) in parse_pack(&text) {
                match key {
                    "_name" => {}
                    "_font" => face = Some(intern(value.trim())),
                    _ => {
                        strings.insert(intern(key), intern(value));
                    }
                }
            }
        }
        let name = match face {
            Some(named) if !named.is_empty() => named.to_string(),
            _ if needs_cjk(tag, &strings) => FACE_CJK.to_string(),
            _ => FACE_LATIN.to_string(),
        };
        Catalog {
            strings,
            face: wide_null(&name),
        }
    }

    fn get(&self, key: &str) -> &'static str {
        // Every key in the table is already in the map, so this only answers a
        // key nobody ever declared - which is the bug it should be loud about.
        match self.strings.get(key) {
            Some(found) => found,
            None => builtin(EN, key).unwrap_or_else(|| intern(key)),
        }
    }
}

static CATALOG: OnceLock<RwLock<Catalog>> = OnceLock::new();

fn catalog() -> &'static RwLock<Catalog> {
    CATALOG.get_or_init(|| RwLock::new(Catalog::load(&system_tag())))
}

/// Switch to `tag` (`auto` follows the system) and reload the packs. Called at
/// startup with the stored setting and again when the settings window saves.
pub fn activate(tag: &str) {
    let loaded = Catalog::load(&lang_tag(&resolve(&lang_tag(tag))));
    let mut guard = match catalog().write() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    *guard = loaded;
}

fn with_catalog<R>(f: impl FnOnce(&Catalog) -> R) -> R {
    // A poisoned lock means a panic happened while the catalog was being
    // replaced, which a release build cannot reach at all (`panic = "abort"`).
    // In a test build the language being one frame stale beats crashing.
    let guard = match catalog().read() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&guard)
}

/// Looks up a key. An unknown key comes back as itself, which makes a missing
/// translation obvious without crashing.
pub fn t(key: &str) -> &'static str {
    with_catalog(|c| c.get(key))
}

/// A string with `{placeholder}` holes filled in.
pub fn t_fmt(key: &str, args: &[(&str, &str)]) -> String {
    fill(t(key), args)
}

fn fill(text: &str, args: &[(impl AsRef<str>, impl AsRef<str>)]) -> String {
    let mut out = text.to_string();
    for (hole, value) in args {
        out = out.replace(hole.as_ref(), value.as_ref());
    }
    out
}

/// The face the interface is drawn with for the language in force.
pub fn ui_face() -> PCWSTR {
    with_catalog(|c| PCWSTR::from_raw(c.face.as_ptr()))
}

/// A language and what it calls itself: the name is never translated, because
/// "日本語" on a button is how you find the Japanese line when you cannot read
/// the interface.
pub struct Language {
    pub tag: String,
    pub name: String,
}

/// Every language the settings row can offer: `auto`, then the shipped ones,
/// then each file found in the languages folder. A file named after a shipped
/// language is that language with edits, not a second choice.
pub fn languages() -> Vec<Language> {
    let mut found = SHIPPED
        .iter()
        .map(|tag| Language {
            tag: tag.to_string(),
            name: native_name(tag),
        })
        .collect::<Vec<_>>();
    if let Ok(entries) = fs::read_dir(languages_dir()) {
        let mut extra = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("txt") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let tag = stem.to_string();
            // `auto` is the setting that follows the system, never a file of its
            // own: a folder that happens to hold `auto.txt` would otherwise offer
            // the row two buttons that mean the same thing.
            if !is_safe_tag(&tag) || tag == AUTO || SHIPPED.contains(&tag.as_str()) {
                continue;
            }
            let name = match read_pack(&tag).and_then(|text| pack_name(&text)) {
                Some(named) => named,
                None => tag.clone(),
            };
            extra.push(Language { tag, name });
        }
        extra.sort_by(|a, b| a.tag.cmp(&b.tag));
        found.extend(extra);
    }
    found
}

/// What a shipped language calls itself. Never translated: "日本語" on a button
/// is how you find the Japanese line in an interface you cannot read.
fn native_name(tag: &str) -> String {
    match tag {
        EN => "English".to_string(),
        ZH => "简体中文".to_string(),
        other => other.to_string(),
    }
}

fn read_pack(tag: &str) -> Option<String> {
    if !is_safe_tag(tag) {
        return None;
    }
    let path = languages_dir().join(format!("{tag}.txt"));
    match fs::read_to_string(&path) {
        // An editor that does not write UTF-8 - Notepad's "Unicode, little-endian"
        // or an ANSI save - is the one way a pack can be in the folder and still
        // not appear, so the refusal says which file and why.
        Err(why) if path.exists() => {
            crate::log!("language pack {} cannot be read as UTF-8: {why}", tag);
            None
        }
        Err(_) => None,
        Ok(text) if text.trim().is_empty() => None,
        Ok(text) => Some(text),
    }
}

/// The `_name` a pack gives itself, if it has one.
fn pack_name(text: &str) -> Option<String> {
    parse_pack(text)
        .into_iter()
        .find(|(key, _)| *key == "_name")
        .map(|(_, value)| value.to_string())
        .filter(|value| !value.is_empty())
}

/// `key=value` lines, `#` comments, blank lines, and an empty value that means
/// "show nothing here" rather than "use the fallback".
fn parse_pack(text: &str) -> Vec<(&str, &str)> {
    // Notepad puts a byte-order mark in front of every UTF-8 file it saves, and
    // a pack is meant to be edited in it.
    let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.starts_with('#') {
            continue;
        }
        out.push((key, value.trim()));
    }
    out
}

/// Write out the shipped languages so the folder holds something to copy and
/// translate. An existing file is never rewritten: it is the user's.
pub fn export_templates() {
    let dir = languages_dir();
    let mut fresh = false;
    for tag in SHIPPED {
        let path = dir.join(format!("{tag}.txt"));
        if path.exists() {
            continue;
        }
        if !fresh {
            let _ = fs::create_dir_all(&dir);
            fresh = true;
        }
        let _ = fs::write(&path, template(tag));
    }
}

/// A complete pack for a shipped language, with the holes explained in the
/// language being exported.
fn template(tag: &str) -> String {
    let mut text = format!("# ShaiZero language pack: {tag}\n# {usage}\n\n_name = {name}\n\n",
        usage = match tag {
            ZH => "一行一个 key = 文本。# 开头是注释。留空的行不算修改这一句，写 key = 才是把这句变成没有。\n# 只改你想改的行；没写的自动用程序自带的文本，最后兜底是英文。\n# _name 是这个语言对自己的称呼，_font 可以指定别的字体。",
            _ => "One key = text per line. # starts a comment. A line you leave out keeps\n# the string the program ships with, and English answers last.\n# _name is what this language calls itself; _font can name another face.",
        },
        name = native_name(tag),
    );
    let mut group = "";
    for (key, en, zh) in BUILTIN {
        let here = group_of(key);
        if here != group {
            group = here;
            if !here.is_empty() {
                text.push_str(&format!("\n# {here}\n"));
            }
        }
        let value = if tag == ZH { zh } else { en };
        text.push_str(&format!("{key} = {value}\n"));
    }
    text
}

/// The exporter reads the table's own naming to break the file into the sections
/// a person translating expects, so the grouping cannot drift from the strings.
fn group_of(key: &str) -> &'static str {
    if key.starts_with("settings_") {
        "settings window"
    } else if key.starts_with("tag_") {
        "window tags"
    } else if key.starts_with("lang") {
        "language"
    } else if key.starts_with("win_") || key.starts_with("fav_") {
        "the windows on the wheel"
    } else {
        ""
    }
}

/// A string that outlives the catalog it came from, so `t()` can still hand back
/// `&'static str` and the call sites stay as they were. Packs are read once per
/// language change; leaking their contents is the whole trade.
fn intern(text: &str) -> &'static str {
    Box::leak(text.to_string().into_boxed_str())
}

fn wide_null(text: &str) -> &'static [u16] {
    let mut wide: Vec<u16> = text.encode_utf16().collect();
    wide.push(0);
    Box::leak(wide.into_boxed_slice())
}

macro_rules! table {
    ($($key:literal => $en:literal | $zh:literal),* $(,)?) => {
        /// Every string the program ships, as `(key, English, Simplified Chinese)`
        /// in the order the exporter writes them out.
        const BUILTIN: &[(&str, &str, &str)] = &[$(($key, $en, $zh),)*];
    };
}

table! {
    "app_name" => "ShaiZero" | "ShaiZero",
    "open_at_login" => "Start with Windows" | "开机时启动",
    "quit" => "Quit ShaiZero" | "退出 ShaiZero",
    "settings" => "Settings…" | "设置…",
    "hotkey_failed" => "Some shortcuts are already in use" | "有些快捷键被占用了",
    "hotkey_failed_body" => "Taken: {list}. Change them under Settings in the tray menu." | "已被占用：{list}。可以在托盘菜单的「设置」里换一个组合。",
    "elevated_tip" => "ShaiZero is running as administrator. Only do that when the windows you switch are themselves administrator windows; running normally is safer." | "ShaiZero 正以管理员权限运行。只有当你要切换的窗口本身以管理员权限运行时才需要这样，平时以普通权限运行更安全。",
    "settings_title" => "ShaiZero settings" | "ShaiZero 设置",
    // The four sections the left column names. One word each: it is a column of
    // tabs, not a table of contents, and the sentence a section belongs to is
    // written on the card the click brings up.
    "settings_group_keys" => "Shortcuts" | "快捷键",
    "settings_nav_lang" => "Language" | "语言",
    "settings_nav_about" => "About" | "关于",
    "settings_fav" => "Add to dial" | "上盘快捷键",
    "settings_panel_key" => "Wheel settings" | "轮盘设置",
    "settings_wheel_key" => "Summon the wheel" | "呼出轮盘",
    "settings_key_unset" => "Not set" | "未设置",
    "settings_key_capture" => "Press a shortcut" | "按下组合键",
    "settings_no_mods" => "Give the shortcut at least one modifier (Ctrl, Alt, Shift or Win)." | "快捷键至少要带一个 Ctrl、Alt、Shift 或 Win。",
    "settings_hotkey_failed" => "That shortcut could not be registered. Another program may be using it." | "这个快捷键注册失败，可能被其他程序占用了。",
    "settings_hotkey_dup" => "Two shortcuts are the same. Give each one its own combination." | "两个快捷键用了同一个组合，请各配一个。",
    "settings_lang" => "Interface language" | "界面语言",
    "settings_lang_note" => "One file, one language" | "一个文件，一门语言",
    "lang_auto" => "System" | "跟随系统",
    "lang_folder" => "Language files…" | "语言文件…",
    "tag_clear" => "Remove tag" | "清除标签",
    "tag_title" => "Window tag" | "窗口标签",
    "tag_placeholder" => "A name you will recognise" | "起个一眼能认出的名字",
    "tag_no_window" => "There is no window in front to name." | "前台没有可以命名的窗口。",
    "wheel_minimized" => "off on the taskbar" | "在任务栏上",
    "tag_ok" => "Save" | "保存",
    "tag_cancel" => "Cancel" | "取消",
    "untitled_window" => "Window" | "窗口",
    // The wheel. Every one of these is a line the dial says while the user is
    // still holding the button down, so they are written to be read in a second
    // and thrown away. Each is one whole clause with nothing joined onto it: a
    // separator the program adds itself loses its spaces on the way through a
    // language pack, and a hub two centimetres wide has no room for a sentence.
    "wheel_cancel_hint" => "Let go in the middle, or press Esc" | "回到中间松开，或按 Esc",
    "wheel_current" => "in use" | "正在用",
    "wheel_pin" => "hold Ctrl to pin" | "按住 Ctrl 钉住",
    "wheel_unpin" => "hold Ctrl to free it" | "按住 Ctrl 取消",
    "wheel_needs_admin" => "That window runs as administrator. Start ShaiZero as administrator to switch to it." | "该窗口以管理员权限运行。要以管理员身份运行 ShaiZero 才能切换过去。",
    "wheel_raise_failed" => "That window did not come to the front. It may be blocked by the system's foreground lock; try again, or run ShaiZero as administrator." | "该窗口未能切到前台，可能被系统前台锁阻止；请重试，或以管理员身份运行 ShaiZero。",
    "wheel_raise_cloaked" => "That window is hidden by the system, possibly on another virtual desktop, so it cannot be brought to the front." | "该窗口被系统隐藏，可能在另一个虚拟桌面，无法切到前台。",
    // The shortcut that keeps a window, and the section of the settings page that
    // manages them. The two words the shortcut writes are the whole answer to
    // "what did that key do", so each is one clause; the row's are cell labels and
    // stay as short as the cells they are measured into - a hundred and five
    // logical pixels, four to a card.
    "fav_added" => "On the dial" | "已上盘",
    "fav_already" => "Already on the dial" | "已在盘上",
    "fav_prompt" => "Name this window" | "给这个窗口起个标签",
    // The four words the window page used to put on a card are gone with it. The
    // only window words left are the ones the dial's own list says.
    "win_empty" => "Nothing is on the wheel yet" | "盘上还没有窗口",
    // The settings cards. A card is a title, one line of what the row is for, and
    // the control beside them. Both lines are measured into the same 206 logical
    // pixels: the sentence at 11.5 - about seventeen Chinese characters - and the
    // title at 13 semibold, which is fifteen. So a title is a name, never the
    // thing it does: the panel is the only place a setting is explained, and a
    // word that had to be cut to fit is worse than a short one. One clause in the
    // note, what it is for, and no numbers.
    "settings_wheel" => "Wheel" | "轮盘",
    "settings_wheel_note_brief" => "Hold the key, drag out, let go" | "按住触发键拖开，松手就切过去",
    "settings_wheel_trigger" => "Wheel shortcut" | "轮盘显示快捷键",
    "settings_wheel_drag" => "Drag with" | "鼠标键",
    "settings_note_mouse" => "Which button you pull with" | "用哪个键拖",
    "settings_dial_search" => "Search windows" | "搜索窗口",
    "settings_dial_no_match" => "No matching windows" | "没有匹配的窗口",
    "settings_trigger_conflict" => "That chord is already used by another shortcut here." | "该组合已被本程序的其他快捷键占用。",
    "settings_trigger_risky" => "This chord is often taken by the desktop; it still saves." | "该组合常被系统占用；仍会保存。",
    "settings_wheel_left" => "Left button" | "鼠标左键",
    "settings_wheel_right" => "Right button" | "鼠标右键",
    "settings_wheel_middle" => "Middle button" | "鼠标中键",
    "settings_wheel_back" => "Back side button" | "鼠标侧键（后退）",
    "settings_wheel_forward" => "Forward side button" | "鼠标侧键（前进）",
    "settings_wheel_needs_mod" => "The trigger needs a key, a side mouse button, or one modifier: Ctrl, Alt, Shift or Win." | "触发需要设置一个按键、鼠标侧键，或一个修饰键：Ctrl、Alt、Shift 或 Win。",
    "settings_trigger_invalid" => "Bind a single key, or a modifier with one key or mouse button. A bare mouse button would take every click." | "请录制单个按键，或「修饰键 + 主键/鼠标键」；单独鼠标键会占用所有点击。",
    "settings_wheel_threshold" => "Starting distance" | "起手距离",
    "settings_note_threshold" => "How far before it starts" | "拖出多远算起手势",
    "settings_note_wheel_key" => "Hold it and the dial opens at the pointer" | "按住就在指针处开盘",
    "settings_lang_folder" => "Language files" | "语言文件",
    "settings_note_folder" => "Drop in a file, gain a language" | "放进去一个，就多一门语言",
    "settings_about_version" => "Version" | "版本",
    "settings_note_version" => "Which build is running" | "这一版是哪个号",
    "settings_about_log" => "Log" | "日志",
    "settings_open_log" => "Open the log" | "打开日志",
    "settings_about_motion" => "Reduce motion" | "减弱动态效果",
    "settings_note_motion" => "The dial and knobs land in place" | "盘子和开关直接到位，不滑过去",
    "settings_about_license" => "License" | "许可",
    // 轮盘: the picture and the four choices that change it. The preview note is
    // the only place the mouse's part in editing is written down.
    "settings_nav_dial" => "Wheel" | "轮盘",
    "settings_dial_preview" => "Preview" | "轮盘预览",
    "settings_dial_preview_note" => "Click to name a slice, right-click to take it off, drag to place it, drag the blank ring outside to set the angle (only here)" | "单击扇区改名，右键移出，拖动换位；拖外侧留白处调整角度（仅此处可调）",
    "settings_dial_theme" => "Theme" | "主题",
    // The compact strip beside the window list has room for a word, not a
    // sentence: these are the short names the page paints in its left column.
    "settings_dial_theme_short" => "Theme" | "主题",
    "settings_dial_accent_short" => "Accent" | "强调",
    "settings_dial_radius_short" => "Size" | "大小",
    "settings_dial_seam_short" => "Gap" | "间距",
    "settings_dial_anim_short" => "Motion" | "动画",
    "settings_dial_limit_short" => "Count" | "上限",
    "settings_dial_order_short" => "Order" | "顺序",
    "settings_dial_theme_note" => "Seven whole looks: aurora, outline, glass, glow, card, neon, paper" | "七套完整的形态：极光、描边、玻璃、微光、卡片、霓虹、纸卡",
    "settings_dial_accent_note" => "Structure stays with the theme; this only tints the sectors" | "结构随主题锁定，这里只调扇区颜色",
    "settings_dial_radius" => "Size" | "大小",
    "settings_dial_radius_note" => "How far the dial reaches" | "盘子张开多远",
    "settings_dial_seam" => "Spacing" | "间距",
    "settings_dial_seam_note" => "The air between two slices" | "两个扇区之间留多少空",
    "settings_dial_anim" => "Animation" | "动画",
    "settings_dial_anim_note" => "How long the dial takes to land" | "盘子展开要多久",
    "settings_dial_limit" => "How many windows" | "上盘窗口数",
    "settings_dial_limit_note" => "How many windows start on the dial by themselves; Ctrl+2 adds one regardless" | "启动时默认上盘的窗口数；Ctrl+2 绑定的窗口不受此限",
    "settings_dial_remove" => "Remove" | "移出",
    "settings_dial_restore" => "Put back" | "上盘",
    "settings_dial_selected" => "Selected" | "选中扇区",
    "settings_dial_select_hint" => "Click a sector to tune its angle and size" | "点选扇区后可调角度与大小",
    "settings_dial_drop_remove" => "Release here to take it off the dial" | "松手即移出轮盘",
    "settings_dial_removed_msg" => "{name} left the dial - double-click it in the list to bring it back" | "{name} 已移出轮盘，在列表中双击可加回",
    "settings_dial_back_msg" => "{name} is back on the dial" | "{name} 已重新上盘",
    // The list's two headings and the five walls a seat can run into. Each
    // refusal names the window and says why, because "nothing happened" is the
    // one answer the panel must never give to a double-click.
    "settings_dial_group_on" => "On the dial" | "在轮盘上",
    "settings_dial_group_off" => "Not on the dial" | "未上轮盘",
    // One line beside the two headings: the rows carry no button any more, so
    // the gesture is the only way in or out and has to be said somewhere.
    "settings_dial_double_hint" => "Double-click: put on / take off" | "双击上盘 / 下盘",
    "settings_dial_seat_not_running" => "{name} is not running - open it to put it on the dial" | "{name} 未上盘：窗口未运行，打开后可上盘",
    "settings_dial_seat_excluded" => "{name} was not seated - this application is struck off" | "{name} 未上盘：该应用已被排除，先在设置里放行",
    "settings_dial_seat_failed" => "{name} was not seated - try again" | "{name} 未上盘：置座失败，请重试",
    "settings_dial_angle_short" => "Angle" | "角度",
    "settings_dial_size_short" => "Size" | "大小",
    "menu_sector_select" => "Select this sector" | "选中此扇区",
    "menu_sector_remove" => "Take it off the dial" | "移出轮盘",
    "menu_sector_colour" => "Choose colour…" | "自定义颜色…",
    "menu_sector_colour_reset" => "Reset colour" | "恢复自动配色",
    "menu_sector_bigger" => "Bigger sector" | "扇区变大",
    "menu_sector_smaller" => "Smaller sector" | "扇区变小",
    "menu_sector_size_reset" => "Reset size" | "恢复默认大小",
    "menu_sector_rename" => "Rename…" | "重命名…",
    "colour_panel_title" => "Sector colour" | "扇区颜色",
    "colour_panel_presets" => "Presets" | "预设",
    "colour_panel_done" => "Done" | "完成",
    "menu_scheme_none" => "Auto (no preset)" | "自动配色",
    "menu_scheme_hawkeye" => "Hawkeye red/gold/violet/blue/green" | "烽火摸金 红/金/紫/蓝/绿",
    "menu_scheme_classic" => "Classic cobalt/teal/violet/orange/rose" | "经典 蓝/青/紫/橙/粉",
    "settings_dial_limit_ceiling" => "At the ceiling of {n}" | "已达上限 {n}",
    "settings_dial_order" => "Which ones" | "选取顺序",
    "settings_dial_order_note" => "Newest first, or the favourites list first" | "按最近使用，或常用优先",
    // Where a picked window lands: the one row whose two cells are whole
    // sentences, because "in place" and "centre on this screen" are the
    // decision itself and a shortened name would only be guessed at.
    "settings_dial_position_short" => "Position" | "位置",
    "settings_dial_position_note" => "In place, or centred on the screen under the pointer" | "原位显示，或居中到鼠标所在屏幕",
    "settings_dial_rotate_short" => "Rotation" | "整体旋转",
    "settings_dial_rotate_note" => "Turn the whole dial clockwise; 0° puts the first sector at 12 o'clock" | "顺时针转动整个轮盘；0° 时第一个扇区位于正上方",
    "settings_dial_position_original" => "In place" | "原位置",
    "settings_dial_position_center" => "Centre on this screen" | "鼠标屏居中",
    "settings_order_recent" => "Newest" | "最近使用",
    "settings_order_favourite" => "Favourites" | "常用优先",
    "dial_theme_outline" => "Outline" | "描边",
    "dial_theme_glass" => "Glass" | "玻璃",
    "dial_theme_neon" => "Glow" | "微光",
    "dial_theme_card" => "Card" | "卡片",
    "dial_theme_neon2" => "Neon" | "霓虹",
    "dial_theme_paper" => "Paper" | "纸卡",
    "dial_theme_site" => "Aurora" | "极光",
    "dial_accent_low" => "Soft" | "淡",
    "dial_accent_medium" => "Medium" | "中",
    "dial_accent_high" => "Vivid" | "浓",
    "dial_anim_instant" => "None" | "即时",
    "dial_anim_quick" => "Quick" | "快速",
    "dial_anim_normal" => "Normal" | "标准",
    "dial_anim_slow" => "Slow" | "慢速",
    "dial_preview_empty" => "No windows to show yet" | "盘上还没有窗口",
    "win_move_up" => "Move up" | "上移",
    "win_move_down" => "Move down" | "下移",
}

/// The line the hub writes under the window it is showing, once that window's
/// own title is above it. Two of these are states and one is an offer: the window
/// already in front needs nothing said about it, and any other window can be
/// given a direction or freed from one.
///
/// A window off on the taskbar comes first, because it is the one fact the user
/// cannot check by looking - there is nothing to look at - and reaching it is a
/// restore rather than a switch.
///
/// Both offers are whole sentences in the table rather than a prefix the program
/// joins to a verb, because a joiner the program adds is a joiner whose spaces get
/// trimmed away by a language pack.
pub fn wheel_sub(current: bool, pinned: bool, minimized: bool) -> &'static str {
    t(if minimized {
        "wheel_minimized"
    } else if current {
        "wheel_current"
    } else if pinned {
        "wheel_unpin"
    } else {
        "wheel_pin"
    })
}

/// What the hub says while it is showing no window at all - the pointer is on the
/// hub or between two sectors, and letting go there is the cancel.
pub fn wheel_cancel_hint() -> &'static str {
    t("wheel_cancel_hint")
}

/// The two names a seat needs: the window's own title, and the application it
/// belongs to. A window with no title is not a sector with no text: it gets the
/// one word the interface already uses for it. The application comes
/// back here rather than being read from the seat because what the dial shows is
/// its first character, and the thing that has a first character is a name.
pub fn wheel_parts(title: &str, app: &str) -> (String, String) {
    let head = if title.trim().is_empty() {
        t("untitled_window").to_string()
    } else {
        title.to_string()
    };
    (head, app.to_string())
}

/// The letter a sector draws in its own circle: the first character of the
/// application's name, in caps where the alphabet has caps. A Chinese name gives
/// its first glyph and a Japanese one does the same, which is why this never asks
/// what script it is.
pub fn wheel_glyph(name: &str) -> String {
    let trimmed = name.trim_start();
    match trimmed.chars().next() {
        Some(c) => c.to_uppercase().collect(),
        None => "·".to_string(),
    }
}

/// Built-in strings for `tag`, and `None` for a language the program has nothing
/// for. Used by the look-up and by the tests that check the table against itself.
pub fn builtin(tag: &str, key: &str) -> Option<&'static str> {
    if tag != EN && tag != ZH {
        return None;
    }
    BUILTIN
        .iter()
        .find(|row| row.0 == key)
        .map(|row| if tag == ZH { row.2 } else { row.1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two halves of the table are the same document. A key with no English
    /// is a key that shows a raw snake_case name, and a key with no Chinese is
    /// the same bug in the language the program was written for.
    #[test]
    fn every_string_exists_in_both_languages() {
        for (key, en, zh) in BUILTIN {
            assert!(!en.is_empty(), "{key} has no English");
            assert!(!zh.is_empty(), "{key} has no Chinese");
        }
        let mut keys: Vec<&str> = BUILTIN.iter().map(|(key, _, _)| *key).collect();
        let before = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(before, keys.len(), "a key is declared twice");
    }

    /// A translator who loses a hole gets a sentence with no number in it and no
    /// way to find out why, so the holes have to match line for line.
    #[test]
    fn the_holes_match_across_languages() {
        for &(key, en, zh) in BUILTIN {
            let (mut en, mut zh) = (holes(en), holes(zh));
            en.sort();
            zh.sort();
            assert_eq!(en, zh, "{key}");
        }
    }

    fn holes(text: &str) -> Vec<String> {
        text.split('{')
            .skip(1)
            .map(|part| part.split('}').next().unwrap_or(part).to_string())
            .collect()
    }

    /// `languages/ru.txt` is the proof that a language can be added without
    /// touching the program, so it is checked the way the program reads it: every
    /// line present, every hole kept, every balloon still fitting its balloon.
    /// A user's own pack is nobody's business; this one is the example.
    #[test]
    fn the_example_pack_covers_the_table() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../languages/ru.txt");
        let text = fs::read_to_string(path).expect("the Russian example pack is in the repo");
        let entries = parse_pack(&text);
        let value = |key: &str| entries.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
        assert_eq!(pack_name(&text).as_deref(), Some("Русский"));
        assert_eq!(
            entries.iter().filter(|(k, _)| !k.starts_with('_')).count(),
            BUILTIN.len(),
            "the example pack lost or gained a line"
        );
        for &(key, en, zh) in BUILTIN {
            let Some(ru) = value(key) else {
                panic!("the example pack has no {key}");
            };
            let (mut want, mut have) = (holes(en), holes(ru));
            want.extend(holes(zh));
            want.sort();
            want.dedup();
            have.sort();
            have.dedup();
            assert_eq!(want, have, "{key} changed the holes");
            assert!(!ru.trim().is_empty(), "{key} is empty");
            if BALLOON_KEYS.contains(&key) {
                assert!(
                    ru.chars().count() <= crate::tray::BALLOON_MAX,
                    "{key} is {} characters, and the balloon cuts it at {}",
                    ru.chars().count(),
                    crate::tray::BALLOON_MAX
                );
            }
        }
    }

    #[test]
    fn a_pack_is_lines_of_key_value() {
        let text = "# comment\r\nquit = Exit now\r\n\r\nsettings_fav =\r\n_name = Français\r\n_font = Segoe UI\r\n";
        let entries = parse_pack(text);
        assert_eq!(
            entries,
            vec![
                ("quit", "Exit now"),
                ("settings_fav", ""),
                ("_name", "Français"),
                ("_font", "Segoe UI"),
            ]
        );
        assert_eq!(pack_name(text).as_deref(), Some("Français"));
        // The same file saved by Notepad, which writes a byte-order mark first.
        assert_eq!(
            pack_name(&format!("\u{FEFF}{text}")).as_deref(),
            Some("Français")
        );
    }

    /// A tag is a file name and it comes from a hand-edited settings file, so the
    /// one thing it must never be is a path.
    #[test]
    fn a_tag_is_a_file_name_and_nothing_else() {
        assert!(is_safe_tag("zh-CN"));
        assert!(is_safe_tag("pt_BR"));
        assert!(!is_safe_tag("../settings"));
        assert!(!is_safe_tag("a/b"));
        assert!(!is_safe_tag("a\\b"));
        assert!(!is_safe_tag(""));
        assert!(!is_safe_tag(&"x".repeat(41)));
        assert_eq!(lang_tag("  ja "), "ja");
        assert_eq!(lang_tag(".."), AUTO);
        assert_eq!(lang_tag("anything else"), AUTO);
    }

    #[test]
    fn the_pack_survives_a_round_trip() {
        for tag in SHIPPED {
            let pack = template(tag);
            let entries = parse_pack(&pack);
            let named = entries
                .iter()
                .filter(|(key, _)| !key.starts_with('_'))
                .count();
            assert_eq!(named, BUILTIN.len(), "{tag} lost or gained a line");
            for (key, _, _) in BUILTIN {
                let value = entries
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| *v)
                    .unwrap_or_default();
                assert_eq!(value, builtin(tag, key).unwrap_or_default(), "{key}");
            }
        }
    }

    /// The strings that end up in a tray balloon, and therefore in an array
    /// Windows cuts off without asking anybody.
    const BALLOON_KEYS: &[&str] = &["elevated_tip", "hotkey_failed_body"];

    /// The tray balloon holds 255 characters and then cuts, which is how the
    /// English first-run lesson used to end in the middle of a word.
    #[test]
    fn the_longest_balloons_fit_the_balloons() {
        for key in BALLOON_KEYS {
            for (built, text) in [(EN, builtin(EN, key)), (ZH, builtin(ZH, key))] {
                let text = text.expect("the key is in the table");
                assert!(
                    text.chars().count() <= crate::tray::BALLOON_MAX,
                    "{key} in {built} is {} characters",
                    text.chars().count()
                );
            }
        }
    }

    #[test]
    fn an_unknown_language_answers_in_english() {
        assert_eq!(builtin("fr", "quit"), None);
        assert_eq!(builtin(EN, "quit"), Some("Quit ShaiZero"));
        assert_eq!(builtin(ZH, "quit"), Some("退出 ShaiZero"));
    }

    #[test]
    fn only_the_languages_we_have_are_cjk() {
        let empty = HashMap::new();
        assert!(needs_cjk(ZH, &empty));
        assert!(needs_cjk("ja", &empty));
        assert!(!needs_cjk(EN, &empty));
        // A half-translated Japanese pack still needs the face, and a French
        // pack that quotes a Japanese title gets one without asking.
        let mut quoted = HashMap::new();
        quoted.insert("quit", "閉じる");
        assert!(needs_cjk("ja-JP", &quoted));
        assert!(needs_cjk("fr", &quoted));
        assert!(!needs_cjk("fr", &empty));
    }

    #[test]
    fn holes_are_filled() {
        assert_eq!(
            fill("press {hotkey} now", &[("{hotkey}", "Ctrl+2")]),
            "press Ctrl+2 now"
        );
        assert_eq!(
            fill("nothing here", &[("{hotkey}", "Ctrl+2")]),
            "nothing here"
        );
    }
}
