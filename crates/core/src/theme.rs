//! Shared design tokens: colours, and the stable accent colour that tells the
//! user which windows belong to the same application.
//!
//! Two Chrome windows must look like siblings and two different apps must not.
//! The accent is derived from the executable name, not from the window handle,
//! so it survives restarts and stays consistent for the same app.

/// An 8-bit sRGB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Rgb(r, g, b)
    }

    /// Blends towards white; `t` of 0 leaves the colour alone.
    pub fn lighten(self, t: f32) -> Rgb {
        let t = t.clamp(0.0, 1.0);
        Rgb(
            (self.0 as f32 + (255.0 - self.0 as f32) * t) as u8,
            (self.1 as f32 + (255.0 - self.1 as f32) * t) as u8,
            (self.2 as f32 + (255.0 - self.2 as f32) * t) as u8,
        )
    }

    /// Blends towards black.
    pub fn darken(self, t: f32) -> Rgb {
        let t = 1.0 - t.clamp(0.0, 1.0);
        Rgb(
            (self.0 as f32 * t) as u8,
            (self.1 as f32 * t) as u8,
            (self.2 as f32 * t) as u8,
        )
    }

    pub fn as_tuple(self) -> (u8, u8, u8) {
        (self.0, self.1, self.2)
    }
}

/// The default sector colour: one quiet grey for every window the user has not
/// personally named a colour. A dial full of palette colours reads as a toy;
/// grey reads as furniture, and the one window the user does colour stands out
/// precisely because nothing else on the dial is trying to.
pub const DEFAULT_SECTOR_COLOUR: Rgb = Rgb(0x61, 0x61, 0x61);

/// A palette picked for legibility on both light and dark wallpapers: mid
/// saturation, mid lightness, no two neighbours close in hue.
pub const PALETTE: [Rgb; 12] = [
    Rgb(0x3B, 0x82, 0xF6), // blue
    Rgb(0xEF, 0x6C, 0x3B), // orange
    Rgb(0x22, 0xA5, 0x7A), // green
    Rgb(0xA8, 0x55, 0xF7), // violet
    Rgb(0xE0, 0x4F, 0x6E), // rose
    Rgb(0x0E, 0xA5, 0xA8), // teal
    Rgb(0xD9, 0x77, 0x06), // amber
    Rgb(0x63, 0x66, 0xF1), // indigo
    Rgb(0x84, 0xCC, 0x16), // lime
    Rgb(0xC0, 0x26, 0xD3), // magenta
    Rgb(0x08, 0x91, 0xB2), // cyan
    Rgb(0xB4, 0x53, 0x09), // brown-orange
];

/// Maximum number of rarity-colour schemes the UI can present. The app saves
/// the chosen scheme as a number starting at 1 (0 = off, use the auto palette).
pub const SCHEME_MAX: usize = 2;

/// The Hawkeye (Delta Force) rarity ladder: red → gold → violet → blue → green.
/// Red is loudest, gold a step down but still catches the eye, violet and blue
/// are the common fare, green the quietest of the five. A preset palette rather
/// than auto: the user picks one and gets a consistent visual language across
/// every dial they open.
pub const SCHEME_HAWKEYE_COLOURS: [Rgb; 5] = [
    Rgb(0xE5, 0x39, 0x35), // red
    Rgb(0xFF, 0xCC, 0x33), // gold
    Rgb(0xA8, 0x55, 0xF7), // violet
    Rgb(0x3B, 0x82, 0xF6), // blue
    Rgb(0x22, 0xA5, 0x7A), // green
];
pub const SCHEME_HAWKEYE_WEIGHTS: [f32; 5] = [1.5, 1.25, 1.0, 0.9, 0.8];

/// A classic blue-first ladder for people who never got loot games. The red is
/// replaced with a deep cobalt so a favourite window does not suddenly read as
/// "alert", and the weight differences are gentler.
pub const SCHEME_CLASSIC_COLOURS: [Rgb; 5] = [
    Rgb(0x1D, 0x4E, 0xA6), // cobalt
    Rgb(0x0E, 0xA5, 0xA8), // teal
    Rgb(0xA8, 0x55, 0xF7), // violet
    Rgb(0xEF, 0x6C, 0x3B), // orange
    Rgb(0xE0, 0x4F, 0x6E), // rose
];
pub const SCHEME_CLASSIC_WEIGHTS: [f32; 5] = [1.18, 1.08, 1.0, 1.0, 1.0];

/// Tag labels carry words the user typed, so their colour has to come from the
/// words themselves: the same tag is the same colour wherever it appears, and it
/// stays that colour after a restart. These are deeper than [`PALETTE`] because
/// white label text has to stay legible on them.
pub const TAG_PALETTE: [Rgb; 10] = [
    Rgb(0x3F, 0x5F, 0xDB), // cobalt
    Rgb(0xB6, 0x3A, 0x33), // brick
    Rgb(0x1D, 0x7F, 0x5F), // pine
    Rgb(0x6F, 0x46, 0xB8), // iris
    Rgb(0xA8, 0x37, 0x63), // plum
    Rgb(0x0C, 0x73, 0x7F), // lagoon
    Rgb(0x8A, 0x55, 0x06), // ochre
    Rgb(0x3B, 0x49, 0xB5), // indigo
    Rgb(0x55, 0x70, 0x1A), // moss
    Rgb(0x8E, 0x37, 0xA3), // orchid
];

/// The colour of a tag. Empty text still gets a stable colour so the editor's
/// preview never has to special-case a half-typed tag.
pub fn tag_color(tag: &str) -> Rgb {
    let mut hash: u32 = 2_166_136_261;
    for b in tag.as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    TAG_PALETTE[(hash as usize) % TAG_PALETTE.len()]
}

/// Black or white, whichever is legible on `base`. The crossover is the WCAG
/// point where both choices contrast equally.
pub fn tag_ink(base: Rgb) -> Rgb {
    let linear = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(base.0) + 0.7152 * linear(base.1) + 0.0722 * linear(base.2);
    if luminance > 0.179 {
        Rgb(32, 32, 40)
    } else {
        Rgb(255, 255, 255)
    }
}

/// Picks the accent for an application key (executable name, lower-cased).
/// Deterministic across runs and cheap enough to call per frame.
pub fn accent_for(key: &str) -> Rgb {
    if key.is_empty() {
        return PALETTE[0];
    }
    PALETTE[accent_hash(key) as usize % PALETTE.len()]
}

/// FNV-1a: stable across Rust versions, unlike std's DefaultHasher.
fn accent_hash(key: &str) -> u32 {
    let mut hash: u32 = 2_166_136_261;
    for b in key.as_bytes() {
        hash ^= *b as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

/// How many shades each palette entry offers once a dial holds more windows than
/// the palette has colours. Thirty-six distinct colours before anything repeats.
const ACCENT_SHADES: usize = 3;

/// The colour in `slot`, where the first pass is the palette itself and later
/// passes are lighter and darker versions of the same hues.
fn accent_slot(_hash: u32, slot: usize) -> Rgb {
    let base = PALETTE[slot % PALETTE.len()];
    match slot / PALETTE.len() {
        0 => base,
        1 => base.lighten(0.20),
        // A darker third pass; the seed only decides which shade a colliding
        // window takes, never whether it is legible.
        _ => base.darken(0.18),
    }
}

/// Colours for the seats of one dial, one per entry in `marks`.
///
/// [`accent_for`] hashes a window's executable, so several windows can land on
/// the same palette entry and a dial full of look-alikes becomes a ring of
/// identical slices. This resolves those collisions: the answer depends only on
/// the *set* of marks, not their order, so dragging a sector to a new place
/// repaints nothing and a colour stays with its window. Up to
/// `PALETTE.len() * ACCENT_SHADES` seats get distinct colours; past that the
/// palette has to repeat.
pub fn seat_accents(marks: &[&str]) -> Vec<Rgb> {
    let slots = PALETTE.len() * ACCENT_SHADES;
    let mut unique: Vec<&str> = marks.to_vec();
    unique.sort_unstable();
    unique.dedup();
    // The lowest hash claims its slot first, so which of two colliding windows
    // moves is deterministic and does not depend on the order they arrived in.
    let mut keyed: Vec<(u32, &str)> = unique
        .iter()
        .map(|mark| {
            (
                if mark.is_empty() {
                    0
                } else {
                    accent_hash(mark)
                },
                *mark,
            )
        })
        .collect();
    keyed.sort_unstable();
    let mut taken = vec![false; slots];
    let mut chosen: std::collections::HashMap<&str, Rgb> =
        std::collections::HashMap::with_capacity(unique.len());
    for (hash, mark) in keyed {
        let start = hash as usize % slots;
        let slot = (0..slots)
            .map(|step| (start + step) % slots)
            .find(|slot| !taken[*slot])
            .unwrap_or(start);
        taken[slot] = true;
        chosen.insert(mark, accent_slot(hash, slot));
    }
    marks
        .iter()
        .map(|mark| chosen.get(mark).copied().unwrap_or(PALETTE[0]))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_is_stable() {
        assert_eq!(accent_for("chrome.exe"), accent_for("chrome.exe"));
    }

    #[test]
    fn different_apps_usually_differ() {
        let names = [
            "chrome.exe",
            "code.exe",
            "wechat.exe",
            "explorer.exe",
            "notepad.exe",
            "slack.exe",
        ];
        let mut seen = std::collections::HashSet::new();
        for n in names {
            seen.insert(accent_for(n));
        }
        assert!(seen.len() >= 4, "palette collided too often: {seen:?}");
    }

    #[test]
    fn every_seat_on_a_small_dial_gets_its_own_colour() {
        let marks: Vec<String> = (0..12).map(|n| format!("C:\\app.exe|C|w{n}")).collect();
        let refs: Vec<&str> = marks.iter().map(String::as_str).collect();
        let colours = seat_accents(&refs);
        let seen: std::collections::HashSet<Rgb> = colours.iter().copied().collect();
        assert_eq!(seen.len(), marks.len(), "colours repeated: {colours:?}");
    }

    #[test]
    fn a_seats_colour_does_not_depend_on_where_it_is_drawn() {
        let marks = ["a|C|one", "b|C|two", "c|C|three", "d|C|four"];
        let before = seat_accents(&marks);
        let mut moved = marks;
        moved.reverse();
        let after = seat_accents(&moved);
        for (mark, colour) in marks.iter().zip(before) {
            let at = moved.iter().position(|m| m == mark).unwrap();
            assert_eq!(after[at], colour, "{mark} was repainted by a reorder");
        }
    }

    #[test]
    fn thirty_six_seats_still_do_not_repeat() {
        let marks: Vec<String> = (0..36).map(|n| format!("C:\\app.exe|C|w{n}")).collect();
        let refs: Vec<&str> = marks.iter().map(String::as_str).collect();
        let seen: std::collections::HashSet<Rgb> =
            seat_accents(&refs).into_iter().collect();
        assert_eq!(seen.len(), 36);
    }

    #[test]
    fn a_tag_keeps_its_colour() {
        assert_eq!(tag_color("客户报价"), tag_color("客户报价"));
        assert_eq!(tag_color("Deploy"), tag_color("Deploy"));
    }

    #[test]
    fn tags_that_mean_different_things_look_different() {
        let tags = ["工作", "客户", "参考", "临时", "待办", "重要"];
        let mut seen = std::collections::HashSet::new();
        for tag in tags {
            seen.insert(tag_color(tag));
        }
        assert!(seen.len() >= 4, "tag palette collided too often: {seen:?}");
    }

    #[test]
    fn label_ink_is_readable_on_every_tag_colour() {
        for base in TAG_PALETTE {
            assert_eq!(tag_ink(base), Rgb(255, 255, 255));
        }
        assert_eq!(tag_ink(Rgb(250, 240, 200)), Rgb(32, 32, 40));
    }
}
