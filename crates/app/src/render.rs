//! Drawing the dial.
//!
//! The wheel is rebuilt every frame: the sectors come and go with
//! the window list, and the opening animation scales the whole disc. It is a
//! small picture (at most three rings) and the only per-frame text is cached by
//! `label`, so the cost stays where it can be seen - one second of animation -
//! rather than on the desktop the rest of the time.

use std::collections::HashMap;

use shaiwin_core::wheel::Ring;
use shaiwin_core::Rgb;
use tiny_skia::{
    BlendMode, Color, FillRule, FilterQuality, GradientStop, LinearGradient, Mask, Paint, Path,
    PathBuilder, Pixmap, PixmapPaint, Point, RadialGradient, Shader, SpreadMode, Stroke,
    StrokeDash, Transform,
};

pub struct Painter {
    text: Option<crate::text::Text>,
    labels: HashMap<String, Pixmap>,
    /// Window icons, keyed by handle. An icon is a property of the window
    /// rather than of the frame, and a dial can be rebuilt sixty times a second,
    /// so the RGBA is read once per window per dial and thrown away with the
    /// sprites when the language changes.
    icons: HashMap<isize, Option<Pixmap>>,
}

impl Painter {
    pub fn new() -> Painter {
        Painter {
            text: crate::text::Text::new(),
            labels: HashMap::new(),
            icons: HashMap::new(),
        }
    }

    /// The caches hold pictures of the old language. A label is keyed by its
    /// words, size and colour, none of which is the face they were drawn with, so
    /// switching from a Chinese line to a Russian one changes nothing that the key
    /// can see - and every cache has to go.
    pub fn forget_sprites(&mut self) {
        self.labels.clear();
        self.icons.clear();
        if let Some(text) = self.text.as_mut() {
            text.forget_fonts();
        }
    }

    /// The icon of `hwnd`, read from the window itself the first time it is
    /// asked for. `None` is cached too: an application without an icon should
    /// not be asked again every frame.
    fn icon(&mut self, hwnd: isize) -> Option<&Pixmap> {
        let pix = self.icons.entry(hwnd).or_insert_with(|| {
            shaiwin_platform_win::window::icon_rgba(windows::Win32::Foundation::HWND(
                hwnd as *mut core::ffi::c_void,
            ))
            .and_then(|(rgba, w, h)| {
                let mut pix = Pixmap::new(w, h)?;
                pix.data_mut().copy_from_slice(&rgba);
                Some(pix)
            })
        });
        pix.as_ref()
    }

    fn elide(&mut self, text: &str, px: f32, max_w: f32) -> String {
        if self.text_width(text, px) <= max_w {
            return text.to_string();
        }
        self.elide_with(text, px, max_w, false)
    }

    fn elide_bold(&mut self, text: &str, px: f32, max_w: f32) -> String {
        if self.text_width_bold(text, px) <= max_w {
            return text.to_string();
        }
        self.elide_with(text, px, max_w, true)
    }

    fn elide_with(&mut self, text: &str, px: f32, max_w: f32, bold: bool) -> String {
        let ellipsis = "…";
        let measure = |this: &mut Self, s: &str| {
            if bold {
                this.text_width_bold(s, px)
            } else {
                this.text_width(s, px)
            }
        };
        let ellipsis_w = measure(self, ellipsis);
        let mut out = String::new();
        let mut width = 0.0;
        for ch in text.chars() {
            let advance = measure(self, ch.encode_utf8(&mut [0u8; 4]));
            if width + advance > max_w - ellipsis_w {
                break;
            }
            width += advance;
            out.push(ch);
        }
        out.push('…');
        out
    }

    fn label(&mut self, text: &str, px: f32, rgb: (u8, u8, u8)) -> Option<&Pixmap> {
        let key = format!("{text}|{px:.2}|{rgb:?}");
        if !self.labels.contains_key(&key) {
            let (rgba, width, height) = self.text.as_mut()?.render(text, px, rgb)?;
            let mut pix = Pixmap::new(width, height)?;
            pix.data_mut().copy_from_slice(&rgba);
            self.labels.insert(key.clone(), pix);
        }
        self.labels.get(&key)
    }

    fn label_bold(&mut self, text: &str, px: f32, rgb: (u8, u8, u8)) -> Option<&Pixmap> {
        let key = format!("b|{text}|{px:.2}|{rgb:?}");
        if !self.labels.contains_key(&key) {
            let (rgba, width, height) = self.text.as_mut()?.render_bold(text, px, rgb)?;
            let mut pix = Pixmap::new(width, height)?;
            pix.data_mut().copy_from_slice(&rgba);
            self.labels.insert(key.clone(), pix);
        }
        self.labels.get(&key)
    }

    fn text_width(&mut self, text: &str, px: f32) -> f32 {
        self.text
            .as_mut()
            .and_then(|t| t.width(text, px))
            .unwrap_or(text.chars().count() as f32 * px * 0.5)
    }

    fn text_width_bold(&mut self, text: &str, px: f32) -> f32 {
        self.text
            .as_mut()
            .and_then(|t| t.width_bold(text, px))
            .unwrap_or(text.chars().count() as f32 * px * 0.58)
    }
}

fn fill_rrect(pix: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, paint: Paint<'static>) {
    if let Some(path) = rrect_path(x, y, w, h, r) {
        pix.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

fn rrect_path(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    if w <= 0.0 || h <= 0.0 {
        return None;
    }
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let k = 0.552_284_8 * r;
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.cubic_to(x + w - r + k, y, x + w, y + r - k, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.cubic_to(x + w, y + h - r + k, x + w - r + k, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.cubic_to(x + r - k, y + h, x, y + h - r + k, x, y + h - r);
    pb.line_to(x, y + r);
    pb.cubic_to(x, y + r - k, x + r - k, y, x + r, y);
    pb.close();
    pb.finish()
}

fn solid(r: u8, g: u8, b: u8, a: f32) -> Paint<'static> {
    let mut paint = Paint {
        anti_alias: true,
        ..Default::default()
    };
    paint.set_color(Color::from_rgba8(
        r,
        g,
        b,
        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
    ));
    paint
}

fn stroke_path(
    canvas: &mut Pixmap,
    path: &Path,
    paint: Paint<'static>,
    width: f32,
    transform: Transform,
) {
    let stroke = Stroke {
        width,
        ..Default::default()
    };
    canvas.stroke_path(path, &paint, &stroke, transform, None);
}

// MARK: - The wheel at the cursor
//
// The dial is rebuilt every frame: the sectors come and go with
// the window list, and the opening animation scales the whole disc. It is a
// small picture (at most three rings) and the only per-frame text is cached by
// `label`, so the cost stays where it can be seen - one second of animation -
// rather than on the desktop the rest of the time.

/// How far the highlighted sector leaves the dial, and how much bigger it gets.
/// Both are the reference implementation's numbers (design §1).
pub const WHEEL_POP: f32 = 8.0;
const WHEEL_POP_SCALE: f32 = 1.025;
// The circumferential gap between two neighbouring sectors, in logical pixels
// measured at the sector's own radius. `core::wheel::RING_GAP` is the radial
// seam; this is what keeps a ring of six windows from reading as one band.

/// The glass the sectors sit on: the margin past the outermost ring.
const DISC_MARGIN: f32 = 10.0;
/// The empty lane the drop shadow needs past `core::wheel::Geometry::extent`.
/// The wash reaches 26px beyond the glass and is drawn from 8px above the
/// dial's centre, so the window has to be 22px wider on every side than the
/// dial itself; without it the shadow runs off the canvas and is cut square.
pub const DISC_SHADOW_PAD: f32 = 22.0;
/// How big the hub has to be before its thumbnail is worth drawing.
const HUB_BIG: f32 = 88.0;

/// The dial's ink. Idle labels are a cold grey-white, the one under the pointer
/// is pure white, and the sub-line sits a step behind its heading.
const WHEEL_INK: (u8, u8, u8) = (196, 208, 224);
const WHEEL_INK_HOT: (u8, u8, u8) = (255, 255, 255);
const WHEEL_INK_SUB: (u8, u8, u8) = (170, 184, 204);

/// Neon's ink is cold light, never a printed grey; paper's is ink on a sheet.
/// Both are structure - the accent row cannot tint them.
const NEON_INK: (u8, u8, u8) = (232, 246, 255);
const NEON_SUB: (u8, u8, u8) = (150, 215, 255);
const PAPER_INK: (u8, u8, u8) = (43, 38, 32);
const PAPER_SUB: (u8, u8, u8) = (110, 102, 90);

/// Linear blend of two colours, `t` running 0 (a) to 1 (b).
fn mix_rgb(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let Rgb(ar, ag, ab) = a;
    let Rgb(br, bg, bb) = b;
    let t = t.clamp(0.0, 1.0);
    Rgb(
        (ar as f32 * (1.0 - t) + br as f32 * t).round() as u8,
        (ag as f32 * (1.0 - t) + bg as f32 * t).round() as u8,
        (ab as f32 * (1.0 - t) + bb as f32 * t).round() as u8,
    )
}

/// One seated sector, as the shell has worked it out. The painter is told which
/// ring band it occupies and how far it has come; it is not told what a window is.
pub struct WheelSector {
    pub ring: usize,
    /// Degrees clockwise from 12 o'clock.
    pub bearing: f32,
    /// The sector's angular width, degrees.
    pub span: f32,
    pub accent: Rgb,
    /// The one character that names the application.
    pub glyph: String,
    /// The window's handle, so the painter can ask for its icon. Zero for a
    /// seat whose window is not running.
    pub hwnd: isize,
    /// The window's own title, for a sector wide enough to read it. The
    /// application is already named by `glyph` and `accent`, so the second line has
    /// to carry the thing those cannot: which of the app's windows this is.
    pub name: String,
    pub hot: bool,
    pub pinned: bool,
    /// The window this sector offers is the one already in front: dashed rim,
    /// and releasing on it does nothing.
    pub current: bool,
    /// 0..1 how far this sector has slid out under the pointer.
    pub pop: f32,
    /// 0..1 this sector's own share of the opening.
    pub appear: f32,
}

/// The hub's preview of the window under the pointer.
pub struct WheelPreview<'a> {
    pub head: &'a str,
    pub sub: &'a str,
    pub shot: Option<&'a Pixmap>,
    pub accent: Rgb,
}

/// One frame of the dial.
pub struct WheelView<'a> {
    /// Device pixels per logical pixel, from the monitor the press was on.
    pub scale: f32,
    /// The ruler, in logical pixels, as `core::wheel::Geometry` has it.
    pub radius: f32,
    pub hub: f32,
    pub bounds: &'a [Ring],
    pub sectors: &'a [WheelSector],
    pub preview: Option<WheelPreview<'a>>,
    /// What the hub says when it is not showing a window: the line that tells the
    /// user this is the place a release cancels. Only read when `preview` is `None`,
    /// so a hub with a window on it has no room taken by advice.
    pub note: Option<&'a str>,
    /// The pointer is on the hub, which is the release that cancels.
    pub cancelling: bool,
    /// 0..1 for the disc as a whole, so the glass arrives with the sectors.
    pub open: f32,
    /// The air between neighbouring sectors, in logical pixels.
    pub seam: f32,
    /// The named look: 0 minimal outline, 1 dark glass, 2 glow, 3 tactile card,
    /// 4 neon, 5 paper card. It decides the disc, the fill, the rim and the light
    /// around it.
    pub theme: u32,
}

impl Painter {
    /// Paints the dial into `canvas`, which is the square the wheel window is:
    /// the centre of the canvas is the centre of the wheel.
    pub fn compose_wheel_into(&mut self, canvas: &mut Pixmap, view: &WheelView) {
        self.compose_base_into(canvas, view);
        self.compose_seats_into(canvas, view);
    }

    /// The cross-screen locator: a dim over the *other* monitor, a bright rim
    /// around the window the pointer is naming, and that window's title on a
    /// dark plate. Unlike [`compose_veil_into`] this cuts no hole: there is no
    /// DWM thumbnail on that screen, and a hole would only show the desktop
    /// behind the window rather than telling the user where to look.
    pub fn compose_locator_into(
        &mut self,
        canvas: &mut Pixmap,
        outline: Option<(f32, f32, f32, f32)>,
        accent: Rgb,
        name: &str,
        px: f32,
    ) {
        canvas.fill(Color::from_rgba8(4, 7, 12, (0.42_f32 * 255.0).round() as u8));
        let Some((x, y, w, h)) = outline else {
            return;
        };
        let Rgb(r, g, b) = accent;
        if let Some(path) = rrect_path(x, y, w, h, 8.0) {
            stroke_path(
                canvas,
                &path,
                solid(r, g, b, 0.55),
                6.0,
                Transform::identity(),
            );
            stroke_path(
                canvas,
                &path,
                solid(255, 255, 255, 0.92),
                2.0,
                Transform::identity(),
            );
        }
        // The name goes just above the rim, on a plate that keeps it readable
        // over a bright window. Fall back inside the rectangle if the window
        // reaches the top edge of the monitor.
        let text = self.elide_bold(name, px, canvas.width() as f32 - 24.0);
        let label = self.label_bold(&text, px, (255, 255, 255));
        if let Some(label) = label {
            let (lw, lh) = (label.width() as f32, label.height() as f32);
            let lx = (x + 2.0).clamp(6.0, (canvas.width() as f32 - lw - 6.0).max(6.0));
            let ly = if y - lh - 12.0 >= 4.0 {
                y - lh - 12.0
            } else {
                y + 8.0
            };
            fill_rrect(
                canvas,
                lx - 5.0,
                ly - 4.0,
                lw + 10.0,
                lh + 8.0,
                5.0,
                solid(6, 10, 18, 0.88),
            );
            blit_alpha(canvas, label, lx, ly, 1.0);
        }
    }

    /// The part of the dial that does not move: the shadow and the glass plate.
    /// The settings page keeps these pixels and copies them back once a frame,
    /// so a drag only pays for the sectors that are actually moving.
    pub fn compose_base_into(&mut self, canvas: &mut Pixmap, view: &WheelView) {
        self.compose_still_into(canvas, view);
        // The themes older than paper and neon have always painted the moving
        // half here as well, on top of the still one; this keeps that order so
        // the pixels do not change.
        if view.theme < 4 {
            self.compose_seats_into(canvas, view);
        }
    }

    /// Shadow, glass and rim only: what a sector leaning out never touches. The
    /// live wheel keeps a copy of this and starts each frame from it, which is
    /// the saving that matters - the still half is the larger part of the frame.
    pub fn compose_still_into(&mut self, canvas: &mut Pixmap, view: &WheelView) {
        if view.theme == 4 {
            self.compose_neon_base_into(canvas, view);
            return;
        }
        if view.theme == 5 {
            self.compose_paper_base_into(canvas, view);
            return;
        }
        canvas.fill(Color::TRANSPARENT);
        let s = view.scale.max(0.1);
        let cx = canvas.width() as f32 / 2.0;
        let cy = canvas.height() as f32 / 2.0;
        let outer = view
            .bounds
            .last()
            .map(|b| b.hi)
            .unwrap_or(view.radius)
            .max(view.hub + 10.0);
        let rim = (outer + DISC_MARGIN) * s;
        let appear = view.open.clamp(0.0, 1.0);

        // The dial's own shadow. A layered window cannot blur what is behind it,
        // so the fall-off is drawn rather than filtered: a radial wash that
        // begins under the glass and ends in nothing, lifted off the centre
        // because that is where a light from above would put it.
        let shadow = match view.theme {
            // Outline wants almost no lift; the card theme casts a soft shadow
            // under paper; neon keeps a small dark halo so the glow reads.
            0 => 0.14,
            2 => 0.30,
            3 => 0.40,
            _ => 0.55,
        } * appear;
        if shadow > 0.01 {
            let (sx, sy) = (cx, cy - 8.0 * s);
            // The wash must die out before the square canvas ends. Drawn to its
            // full reach it runs past the edge of the window and is cut there,
            // and a cut shadow is a visible square around a round dial. Keep a
            // pixel of margin so the last row and column stay empty.
            let reach = (rim + 26.0 * s)
                .min(cx - 1.0)
                .min(sy - 1.0)
                .min(canvas.height() as f32 - sy - 1.0)
                .min(canvas.width() as f32 - cx - 1.0);
            if let Some(path) = circle_path(sx, sy, reach) {
                if let Some(shader) = RadialGradient::new(
                    Point::from_xy(sx, sy),
                    (rim * 0.85).min(reach),
                    Point::from_xy(sx, sy),
                    reach,
                    vec![
                        GradientStop::new(0.0, Color::from_rgba8(0, 0, 0, (shadow * 255.0) as u8)),
                        GradientStop::new(1.0, Color::from_rgba8(0, 0, 0, 0)),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    canvas.fill_path(
                        &path,
                        &Paint {
                            shader,
                            ..Default::default()
                        },
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
            }
        }

        // The glass. `backdrop-filter: blur(26px)` is what the mock does and what
        // Windows will not do to a layered window, so the plate is one step more
        // opaque than the mock needs and keeps the two details that make glass
        // read as glass: a light rim along the top and a shade under the bottom.
        // Each theme is this same plate at a different temperature: outline keeps
        // almost none of it, neon goes near-black, and the card theme lays warm
        // paper under the sectors.
        if appear > 0.01 {
            // (top rgb, top alpha, bottom rgb, bottom alpha, rim rgb, rim alpha,
            // light-arc alpha)
            let (top_rgb, top_a, bottom_rgb, bottom_a, rim_rgb, rim_a, light_a) = match view.theme {
                0 => (
                    (26, 30, 38),
                    0.16,
                    (12, 14, 18),
                    0.20,
                    (255, 255, 255),
                    0.14,
                    0.20,
                ),
                2 => (
                    (10, 12, 22),
                    0.82,
                    (3, 4, 9),
                    0.94,
                    (190, 225, 255),
                    0.20,
                    0.32,
                ),
                3 => (
                    (246, 244, 238),
                    0.93,
                    (222, 218, 208),
                    0.96,
                    (255, 255, 255),
                    0.60,
                    0.55,
                ),
                _ => (
                    (41, 50, 64),
                    0.42,
                    (11, 15, 21),
                    0.55,
                    (255, 255, 255),
                    0.10,
                    0.26,
                ),
            };
            let top = ((top_a + 0.20 * appear) * 255.0) as u8;
            let bottom = ((bottom_a + 0.20 * appear) * 255.0) as u8;
            if let Some(path) = circle_path(cx, cy, rim) {
                if let Some(shader) = LinearGradient::new(
                    Point::from_xy(cx, cy - rim),
                    Point::from_xy(cx, cy + rim),
                    vec![
                        GradientStop::new(
                            0.0,
                            Color::from_rgba8(top_rgb.0, top_rgb.1, top_rgb.2, top),
                        ),
                        GradientStop::new(
                            1.0,
                            Color::from_rgba8(bottom_rgb.0, bottom_rgb.1, bottom_rgb.2, bottom),
                        ),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    canvas.fill_path(
                        &path,
                        &Paint {
                            shader,
                            anti_alias: true,
                            ..Default::default()
                        },
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                stroke_path(
                    canvas,
                    &path,
                    solid(rim_rgb.0, rim_rgb.1, rim_rgb.2, rim_a * appear),
                    1.2 * s,
                    Transform::identity(),
                );
                let pi = std::f32::consts::PI;
                if let Some(light) = arc_path(cx, cy, rim - 0.8 * s, pi, 2.0 * pi) {
                    stroke_path(
                        canvas,
                        &light,
                        solid(255, 255, 255, light_a * appear),
                        1.5 * s,
                        Transform::identity(),
                    );
                }
            }
        }
    }

    /// The moving part: every sector at its current angle, and the hub on top.
    /// Drawn onto a canvas that already holds [`compose_still_into`]'s pixels.
    pub fn compose_seats_into(&mut self, canvas: &mut Pixmap, view: &WheelView) {
        let s = view.scale.max(0.1);
        let cx = canvas.width() as f32 / 2.0;
        let cy = canvas.height() as f32 / 2.0;
        if view.theme == 4 {
            for sec in view.sectors {
                self.draw_neon_sector(canvas, view, sec, cx, cy, s);
            }
            self.draw_neon_hub(canvas, view, cx, cy, s);
            return;
        }
        if view.theme == 5 {
            for sec in view.sectors {
                self.draw_paper_sector(canvas, view, sec, cx, cy, s);
            }
            self.draw_paper_hub(canvas, view, cx, cy, s);
            return;
        }
        for sec in view.sectors {
            self.draw_sector(canvas, view, sec, cx, cy, s);
        }
        self.draw_hub(canvas, view, cx, cy, s);
    }

    /// Neon's still half: a near-black plate with its own cyan halo, a soft inner
    /// wash, and nothing else. The sectors are drawn once, by `compose_seats_into`,
    /// so the live dial and the preview land on the same pixels - the old themes
    /// draw them in both halves and stack the alpha twice.
    fn compose_neon_base_into(&mut self, canvas: &mut Pixmap, view: &WheelView) {
        canvas.fill(Color::TRANSPARENT);
        let s = view.scale.max(0.1);
        let cx = canvas.width() as f32 / 2.0;
        let cy = canvas.height() as f32 / 2.0;
        let outer = view
            .bounds
            .last()
            .map(|b| b.hi)
            .unwrap_or(view.radius)
            .max(view.hub + 10.0);
        let rim = (outer + DISC_MARGIN) * s;
        let appear = view.open.clamp(0.0, 1.0);

        // The dark halo under the tubes: the one thing a layered window can do
        // that a blurred shadow cannot - it is a wash, drawn in one gradient.
        let (sx, sy) = (cx, cy - 8.0 * s);
        let reach = (rim + 26.0 * s)
            .min(cx - 1.0)
            .min(sy - 1.0)
            .min(canvas.height() as f32 - sy - 1.0)
            .min(canvas.width() as f32 - cx - 1.0);
        if let Some(path) = circle_path(sx, sy, reach) {
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(sx, sy),
                (rim * 0.85).min(reach),
                Point::from_xy(sx, sy),
                reach,
                vec![
                    GradientStop::new(
                        0.0,
                        Color::from_rgba8(0, 0, 0, (0.34 * appear * 255.0) as u8),
                    ),
                    GradientStop::new(1.0, Color::from_rgba8(0, 0, 0, 0)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                canvas.fill_path(
                    &path,
                    &Paint {
                        shader,
                        ..Default::default()
                    },
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }

        // The plate itself, then the light that seems to come from inside it.
        if appear > 0.01 {
            if let Some(path) = circle_path(cx, cy, rim) {
                if let Some(shader) = LinearGradient::new(
                    Point::from_xy(cx, cy - rim),
                    Point::from_xy(cx, cy + rim),
                    vec![
                        GradientStop::new(
                            0.0,
                            Color::from_rgba8(10, 12, 22, (0.94 * appear * 255.0) as u8),
                        ),
                        GradientStop::new(
                            1.0,
                            Color::from_rgba8(2, 3, 8, (0.98 * appear * 255.0) as u8),
                        ),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    canvas.fill_path(
                        &path,
                        &Paint {
                            shader,
                            anti_alias: true,
                            ..Default::default()
                        },
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                // Inner glow: a cyan wash that starts at the rim and dies towards
                // the hub, so the disc reads as lit from behind rather than flat.
                if let Some(inner) = circle_path(cx, cy, rim - 1.5 * s) {
                    if let Some(shader) = RadialGradient::new(
                        Point::from_xy(cx, cy),
                        (rim * 0.30).max(1.0),
                        Point::from_xy(cx, cy),
                        rim,
                        vec![
                            GradientStop::new(0.0, Color::from_rgba8(120, 220, 255, 0)),
                            GradientStop::new(
                                0.62,
                                Color::from_rgba8(120, 220, 255, (0.03 * appear * 255.0) as u8),
                            ),
                            GradientStop::new(
                                1.0,
                                Color::from_rgba8(120, 220, 255, (0.11 * appear * 255.0) as u8),
                            ),
                        ],
                        SpreadMode::Pad,
                        Transform::identity(),
                    ) {
                        canvas.fill_path(
                            &inner,
                            &Paint {
                                shader,
                                anti_alias: true,
                                ..Default::default()
                            },
                            FillRule::Winding,
                            Transform::identity(),
                            None,
                        );
                    }
                }
                // Self-illuminating rim: a bright line with its own halo.
                stroke_path(
                    canvas,
                    &path,
                    solid(120, 220, 255, 0.10 * appear),
                    4.5 * s,
                    Transform::identity(),
                );
                stroke_path(
                    canvas,
                    &path,
                    solid(120, 220, 255, 0.34 * appear),
                    1.2 * s,
                    Transform::identity(),
                );
            }
        }
    }

    /// Paper's still half: a warm sheet with a soft grey shadow, no specular arc -
    /// the layering comes from the cards and their shadows, not from light.
    fn compose_paper_base_into(&mut self, canvas: &mut Pixmap, view: &WheelView) {
        canvas.fill(Color::TRANSPARENT);
        let s = view.scale.max(0.1);
        let cx = canvas.width() as f32 / 2.0;
        let cy = canvas.height() as f32 / 2.0;
        let outer = view
            .bounds
            .last()
            .map(|b| b.hi)
            .unwrap_or(view.radius)
            .max(view.hub + 10.0);
        let rim = (outer + DISC_MARGIN) * s;
        let appear = view.open.clamp(0.0, 1.0);

        let (sx, sy) = (cx, cy + 6.0 * s);
        let reach = (rim + 24.0 * s)
            .min(cx - 1.0)
            .min(sy - 1.0)
            .min(canvas.height() as f32 - sy - 1.0)
            .min(canvas.width() as f32 - cx - 1.0);
        if let Some(path) = circle_path(sx, sy, reach) {
            if let Some(shader) = RadialGradient::new(
                Point::from_xy(sx, sy),
                (rim * 0.86).min(reach),
                Point::from_xy(sx, sy),
                reach,
                vec![
                    GradientStop::new(
                        0.0,
                        Color::from_rgba8(60, 50, 35, (0.22 * appear * 255.0) as u8),
                    ),
                    GradientStop::new(1.0, Color::from_rgba8(60, 50, 35, 0)),
                ],
                SpreadMode::Pad,
                Transform::identity(),
            ) {
                canvas.fill_path(
                    &path,
                    &Paint {
                        shader,
                        ..Default::default()
                    },
                    FillRule::Winding,
                    Transform::identity(),
                    None,
                );
            }
        }

        if appear > 0.01 {
            if let Some(path) = circle_path(cx, cy, rim) {
                if let Some(shader) = LinearGradient::new(
                    Point::from_xy(cx, cy - rim),
                    Point::from_xy(cx, cy + rim),
                    vec![
                        GradientStop::new(
                            0.0,
                            Color::from_rgba8(248, 245, 238, (0.97 * appear * 255.0) as u8),
                        ),
                        GradientStop::new(
                            1.0,
                            Color::from_rgba8(238, 233, 221, (0.97 * appear * 255.0) as u8),
                        ),
                    ],
                    SpreadMode::Pad,
                    Transform::identity(),
                ) {
                    canvas.fill_path(
                        &path,
                        &Paint {
                            shader,
                            anti_alias: true,
                            ..Default::default()
                        },
                        FillRule::Winding,
                        Transform::identity(),
                        None,
                    );
                }
                stroke_path(
                    canvas,
                    &path,
                    solid(206, 199, 184, 0.90 * appear),
                    1.0 * s,
                    Transform::identity(),
                );
            }
        }
    }

    /// One sector: the tinted annulus, its rim, the name it carries, and the pin
    /// mark if the window owns a direction.
    fn draw_sector(
        &mut self,
        canvas: &mut Pixmap,
        view: &WheelView,
        sec: &WheelSector,
        cx: f32,
        cy: f32,
        s: f32,
    ) {
        let Some(band) = view.bounds.get(sec.ring) else {
            return;
        };
        let a = sec.appear.clamp(0.0, 1.0);
        if a <= 0.001 {
            return;
        }
        let grow = 0.78 + 0.22 * back_out(a);
        let mid = (sec.bearing - 90.0).to_radians();
        let half = sec.span.to_radians() / 2.0;
        let lo = band.lo * s;
        let hi = band.hi * s;
        let rr = (lo + hi) / 2.0;
        // The seam is a fixed distance, so on a crowded ring it costs a wider
        // angle than on an empty one - which is what keeps them all the same
        // thickness on screen.
        let seam = (view.seam * s / rr.max(1.0)).min(half * 0.45);
        let (a0, a1) = (mid - half + seam, mid + half - seam);
        let pop = sec.pop * WHEEL_POP * s;
        let pop_scale = 1.0 + (WHEEL_POP_SCALE - 1.0) * sec.pop;
        let k = grow * pop_scale;
        let ts = Transform::from_translate(-cx, -cy)
            .post_scale(k, k)
            .post_translate(cx + mid.cos() * pop, cy + mid.sin() * pop);
        let Some(path) = sector_path(cx, cy, lo + 1.5 * s, hi - 1.0 * s, a0, a1) else {
            return;
        };
        let Rgb(r, g, b) = sec.accent;
        // The card theme lays each slice on the paper with a small drop shadow
        // under it: a second fill of the same path, offset down, drawn first.
        if view.theme == 3 {
            let ts_shadow = ts.post_translate(0.0, 2.2 * s);
            canvas.fill_path(
                &path,
                &solid(24, 20, 16, 0.30),
                FillRule::Winding,
                ts_shadow,
                None,
            );
        }
        let (inner_a, outer_a) = match view.theme {
            // Outline: almost hollow, so the rim is the sector.
            0 => {
                if sec.hot {
                    (0.12, 0.04)
                } else {
                    (0.035, 0.015)
                }
            }
            // Neon: saturated and bright, the glow is added after the fill.
            2 => {
                if sec.hot {
                    (0.92, 0.72)
                } else {
                    (0.48, 0.24)
                }
            }
            // Card: opaque paper-like slices, the most solid of the four.
            3 => {
                if sec.hot {
                    (0.94, 0.80)
                } else {
                    (0.74, 0.54)
                }
            }
            _ => {
                if sec.hot {
                    (0.50, 0.30)
                } else {
                    (0.16, 0.08)
                }
            }
        };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(cx, cy),
            lo,
            Point::from_xy(cx, cy),
            hi,
            vec![
                GradientStop::new(0.0, Color::from_rgba8(r, g, b, (inner_a * 255.0) as u8)),
                GradientStop::new(1.0, Color::from_rgba8(r, g, b, (outer_a * 255.0) as u8)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            canvas.fill_path(
                &path,
                &Paint {
                    shader,
                    anti_alias: true,
                    ..Default::default()
                },
                FillRule::Winding,
                ts,
                None,
            );
        }
        // Neon: two wide, faint strokes under the rim read as light escaping the
        // slice - a layered window cannot blur, so the halo is drawn in rings.
        if view.theme == 2 && sec.hot {
            for (width, alpha) in [(10.0, 0.10f32), (5.5, 0.18)] {
                stroke_path(canvas, &path, solid(r, g, b, alpha), width * s, ts);
            }
        }
        let (rim_a, rim_w) = match view.theme {
            0 => (
                if sec.hot { 1.0 } else { 0.80 },
                if sec.hot { 1.6 } else { 1.3 },
            ),
            2 => (
                if sec.hot { 1.0 } else { 0.85 },
                if sec.hot { 2.4 } else { 1.2 },
            ),
            3 => (
                if sec.hot { 0.98 } else { 0.62 },
                if sec.hot { 1.8 } else { 1.1 },
            ),
            _ => {
                if sec.hot {
                    (0.95, 1.8)
                } else {
                    (0.30, 1.0)
                }
            }
        };
        stroke_path(canvas, &path, solid(r, g, b, rim_a), rim_w * s, ts);
        if sec.current {
            // The window already in front: hollow and dashed, so the direction
            // the pin remembers is still drawn but nothing is claimed by it.
            let stroke = Stroke {
                width: 1.2 * s,
                dash: StrokeDash::new(vec![3.0 * s, 3.0 * s], 0.0),
                ..Default::default()
            };
            canvas.stroke_path(&path, &solid(205, 218, 235, 0.5), &stroke, ts, None);
        }

        // The name. A sector's own height sets the type size, which is what makes
        // a six-window dial readable and a forty-window one simply a ring of
        // initials.
        let px = ((hi - lo) * 0.30).clamp(12.0 * s, 22.0 * s);
        let arc = sec.span.to_radians() * rr;
        let room = (arc - 10.0 * s).max(10.0);
        let show_name = arc > 50.0 * s;
        let (ink, sub_ink) = match view.theme {
            0 => {
                let base = if sec.hot {
                    (255, 255, 255)
                } else {
                    (228, 234, 244)
                };
                (base, (206, 214, 228))
            }
            2 => {
                let base = if sec.hot {
                    (255, 255, 255)
                } else {
                    (238, 248, 255)
                };
                (base, (198, 228, 255))
            }
            3 => {
                let base = if sec.hot {
                    (255, 255, 255)
                } else {
                    (250, 246, 238)
                };
                (base, (232, 222, 206))
            }
            _ => {
                let base = if sec.hot { WHEEL_INK_HOT } else { WHEEL_INK };
                (base, WHEEL_INK_SUB)
            }
        };
        let glyph = sec.glyph.clone();
        let name = if show_name {
            Some(self.elide(&sec.name, 10.5 * s, room))
        } else {
            None
        };
        let gx = cx + mid.cos() * rr;
        let gy = cy + mid.sin() * rr;
        let dy = if name.is_some() { -px * 0.48 } else { 0.0 };
        // The window's own icon when one can be read: the application is named
        // by the picture it puts in the taskbar, and the letter is the fallback
        // for a window that has none. The icon is centred on the same spot the
        // letter was, at a size that fits the band it sits in.
        let icon_px = if name.is_some() {
            (px * 1.55).clamp(16.0 * s, 34.0 * s)
        } else {
            ((hi - lo) * 0.56).clamp(16.0 * s, 38.0 * s)
        };
        let mut drew_icon = false;
        if sec.hwnd != 0 {
            if let Some(icon) = self.icon(sec.hwnd) {
                let paint = PixmapPaint {
                    opacity: if sec.hot { 1.0 } else { 0.94 } * a,
                    quality: FilterQuality::Bilinear,
                    ..Default::default()
                };
                // The icon comes back from Windows at whatever size the window
                // answers with - 16, 32, or the large icon - so it is scaled to
                // the band rather than drawn at its own size. The translation is
                // part of the same transform because `draw_pixmap`'s own `x,y`
                // is where the unscaled image would start.
                let sx = icon_px / icon.width() as f32;
                let sy = icon_px / icon.height() as f32;
                let at = Transform::from_scale(sx, sy).post_translate(
                    (gx - icon_px / 2.0).round(),
                    (gy + dy - icon_px / 2.0).round(),
                );
                canvas.draw_pixmap(0, 0, icon.as_ref(), &paint, at, None);
                drew_icon = true;
            }
        }
        if !drew_icon {
            if let Some(p) = self.label_bold(&glyph, px, ink) {
                blit_alpha(
                    canvas,
                    p,
                    gx - p.width() as f32 / 2.0,
                    gy + dy - p.height() as f32 / 2.0,
                    a,
                );
            }
        }
        if let Some(text) = &name {
            if let Some(p) = self.label(text, 10.5 * s, sub_ink) {
                blit_alpha(
                    canvas,
                    p,
                    gx - p.width() as f32 / 2.0,
                    gy + px * 0.58 - p.height() as f32 / 2.0,
                    a * 0.92,
                );
            }
        }

        if sec.pinned {
            // The pin: a dot on the rim with a short stem - the same mark the
            // reference puts on a seat that was chosen rather than filled.
            let bx = cx + mid.cos() * (hi - 9.0 * s);
            let by = cy + mid.sin() * (hi - 9.0 * s);
            fill_circle(canvas, bx, by, 2.6 * s, solid(236, 244, 252, 0.95));
            let mut pb = PathBuilder::new();
            pb.move_to(bx, by);
            pb.line_to(bx, by + 6.0 * s);
            if let Some(stem) = pb.finish() {
                stroke_path(
                    canvas,
                    &stem,
                    solid(236, 244, 252, 0.8),
                    1.2 * s,
                    Transform::identity(),
                );
            }
        }
    }

    /// One neon tube: the light that leaks into the seam first, then the fill,
    /// then the bright inner edge, then the rim. The seams are the same angular
    /// gap the other themes use - the glow strokes fall into it.
    fn draw_neon_sector(
        &mut self,
        canvas: &mut Pixmap,
        view: &WheelView,
        sec: &WheelSector,
        cx: f32,
        cy: f32,
        s: f32,
    ) {
        let Some(band) = view.bounds.get(sec.ring) else {
            return;
        };
        let a = sec.appear.clamp(0.0, 1.0);
        if a <= 0.001 {
            return;
        }
        let grow = 0.78 + 0.22 * back_out(a);
        let mid = (sec.bearing - 90.0).to_radians();
        let half = sec.span.to_radians() / 2.0;
        let lo = band.lo * s;
        let hi = band.hi * s;
        let rr = (lo + hi) / 2.0;
        let seam = (view.seam * s / rr.max(1.0)).min(half * 0.45);
        let (a0, a1) = (mid - half + seam, mid + half - seam);
        let pop = sec.pop * WHEEL_POP * s;
        let pop_scale = 1.0 + (WHEEL_POP_SCALE - 1.0) * sec.pop;
        let k = grow * pop_scale;
        let ts = Transform::from_translate(-cx, -cy)
            .post_scale(k, k)
            .post_translate(cx + mid.cos() * pop, cy + mid.sin() * pop);
        let Some(path) = sector_path(cx, cy, lo + 1.5 * s, hi - 1.0 * s, a0, a1) else {
            return;
        };
        let Rgb(r, g, b) = sec.accent;
        // A brighter version of the sector's own colour: the glow around the
        // tube, never the tube itself.
        let bright = mix_rgb(sec.accent, Rgb::new(255, 255, 255), 0.42);
        let Rgb(br, bg, bb) = bright;

        // Light in the seam: wide, faint strokes that spill out of the slice.
        for (width, alpha) in [(12.0, 0.05f32), (7.0, 0.12), (2.2, 0.55)] {
            stroke_path(canvas, &path, solid(br, bg, bb, alpha * a), width * s, ts);
        }
        if sec.hot {
            for (width, alpha) in [(30.0, 0.06f32), (18.0, 0.12)] {
                stroke_path(canvas, &path, solid(br, bg, bb, alpha * a), width * s, ts);
            }
        }

        let (inner_a, outer_a) = if sec.hot { (0.78, 1.0) } else { (0.58, 0.96) };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(cx, cy),
            lo,
            Point::from_xy(cx, cy),
            hi,
            vec![
                GradientStop::new(0.0, Color::from_rgba8(r, g, b, (inner_a * a * 255.0) as u8)),
                GradientStop::new(1.0, Color::from_rgba8(r, g, b, (outer_a * a * 255.0) as u8)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            canvas.fill_path(
                &path,
                &Paint {
                    shader,
                    anti_alias: true,
                    ..Default::default()
                },
                FillRule::Winding,
                ts,
                None,
            );
        }
        // Inner glow: a lit band inside the outer edge and a dimmer one inside
        // the hub edge, so the tube is bright on both walls.
        if let Some(band_path) = sector_path(cx, cy, hi - 3.5 * s, hi - 0.6 * s, a0, a1) {
            canvas.fill_path(
                &band_path,
                &solid(br, bg, bb, 0.62 * a),
                FillRule::Winding,
                ts,
                None,
            );
        }
        if let Some(band_path) = sector_path(cx, cy, lo + 1.4 * s, lo + 2.9 * s, a0, a1) {
            canvas.fill_path(
                &band_path,
                &solid(br, bg, bb, 0.35 * a),
                FillRule::Winding,
                ts,
                None,
            );
        }
        stroke_path(
            canvas,
            &path,
            solid(br, bg, bb, if sec.hot { 1.0 } else { 0.95 } * a),
            if sec.hot { 2.6 } else { 2.0 } * s,
            ts,
        );
        if sec.current {
            let stroke = Stroke {
                width: 1.2 * s,
                dash: StrokeDash::new(vec![3.0 * s, 3.0 * s], 0.0),
                ..Default::default()
            };
            canvas.stroke_path(&path, &solid(150, 225, 255, 0.55), &stroke, ts, None);
        }

        let px = ((hi - lo) * 0.30).clamp(12.0 * s, 22.0 * s);
        let arc = sec.span.to_radians() * rr;
        let room = (arc - 10.0 * s).max(10.0);
        let show_name = arc > 50.0 * s;
        let glyph = sec.glyph.clone();
        let name = if show_name {
            Some(self.elide(&sec.name, 10.5 * s, room))
        } else {
            None
        };
        let gx = cx + mid.cos() * rr;
        let gy = cy + mid.sin() * rr;
        let dy = if name.is_some() { -px * 0.48 } else { 0.0 };
        let icon_px = if name.is_some() {
            (px * 1.55).clamp(16.0 * s, 34.0 * s)
        } else {
            ((hi - lo) * 0.56).clamp(16.0 * s, 38.0 * s)
        };
        let ink = if sec.hot {
            (255, 255, 255)
        } else {
            NEON_INK
        };
        let sub = NEON_SUB;
        let mut drew_icon = false;
        if sec.hwnd != 0 {
            if let Some(icon) = self.icon(sec.hwnd) {
                let paint = PixmapPaint {
                    opacity: if sec.hot { 1.0 } else { 0.94 } * a,
                    quality: FilterQuality::Bilinear,
                    ..Default::default()
                };
                let sx = icon_px / icon.width() as f32;
                let sy = icon_px / icon.height() as f32;
                let at = Transform::from_scale(sx, sy).post_translate(
                    (gx - icon_px / 2.0).round(),
                    (gy + dy - icon_px / 2.0).round(),
                );
                canvas.draw_pixmap(0, 0, icon.as_ref(), &paint, at, None);
                drew_icon = true;
            }
        }
        // The text's own light: the same glyph in the accent colour, laid down
        // first at a pixel's offset and then covered by the white letter. Only a
        // lit sector gets the full four-sided halo; the rest get a hint.
        if !drew_icon {
            let offsets: &[(f32, f32)] = if sec.hot {
                &[(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)]
            } else {
                &[(0.0, -1.0)]
            };
            for (ox, oy) in offsets {
                if let Some(p) = self.label_bold(&glyph, px, (br, bg, bb)) {
                    blit_alpha(
                        canvas,
                        p,
                        gx - p.width() as f32 / 2.0 + ox * s,
                        gy + dy - p.height() as f32 / 2.0 + oy * s,
                        if sec.hot { 0.35 } else { 0.18 } * a,
                    );
                }
            }
            if let Some(p) = self.label_bold(&glyph, px, ink) {
                blit_alpha(
                    canvas,
                    p,
                    gx - p.width() as f32 / 2.0,
                    gy + dy - p.height() as f32 / 2.0,
                    a,
                );
            }
        }
        if let Some(text) = &name {
            let name_offsets: &[(f32, f32)] = if sec.hot {
                &[(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)]
            } else {
                &[(0.0, -1.0)]
            };
            for (ox, oy) in name_offsets {
                if let Some(p) = self.label(text, 10.5 * s, (br, bg, bb)) {
                    blit_alpha(
                        canvas,
                        p,
                        gx - p.width() as f32 / 2.0 + ox * s,
                        gy + px * 0.58 - p.height() as f32 / 2.0 + oy * s,
                        (if sec.hot { 0.22 } else { 0.10 }) * a,
                    );
                }
            }
            if let Some(p) = self.label(text, 10.5 * s, sub) {
                blit_alpha(
                    canvas,
                    p,
                    gx - p.width() as f32 / 2.0,
                    gy + px * 0.58 - p.height() as f32 / 2.0,
                    a * 0.92,
                );
            }
        }

        if sec.pinned {
            let bx = cx + mid.cos() * (hi - 9.0 * s);
            let by = cy + mid.sin() * (hi - 9.0 * s);
            fill_circle(canvas, bx, by, 2.6 * s, solid(236, 244, 252, 0.95));
            let mut pb = PathBuilder::new();
            pb.move_to(bx, by);
            pb.line_to(bx, by + 6.0 * s);
            if let Some(stem) = pb.finish() {
                stroke_path(
                    canvas,
                    &stem,
                    solid(236, 244, 252, 0.8),
                    1.2 * s,
                    Transform::identity(),
                );
            }
        }
    }

    /// One paper card: a slice of tinted paper, lifted off the sheet by its own
    /// shadow, edged with one pixel of ink and capped with a highlight along the
    /// inner edge. The seam is paper, not a stroke.
    fn draw_paper_sector(
        &mut self,
        canvas: &mut Pixmap,
        view: &WheelView,
        sec: &WheelSector,
        cx: f32,
        cy: f32,
        s: f32,
    ) {
        let Some(band) = view.bounds.get(sec.ring) else {
            return;
        };
        let a = sec.appear.clamp(0.0, 1.0);
        if a <= 0.001 {
            return;
        }
        let grow = 0.78 + 0.22 * back_out(a);
        let mid = (sec.bearing - 90.0).to_radians();
        let half = sec.span.to_radians() / 2.0;
        let lo = band.lo * s;
        let hi = band.hi * s;
        let rr = (lo + hi) / 2.0;
        let seam = (view.seam * s / rr.max(1.0)).min(half * 0.45);
        let (a0, a1) = (mid - half + seam, mid + half - seam);
        let pop = sec.pop * WHEEL_POP * s;
        let pop_scale = 1.0 + (WHEEL_POP_SCALE - 1.0) * sec.pop;
        let k = grow * pop_scale;
        let ts = Transform::from_translate(-cx, -cy)
            .post_scale(k, k)
            .post_translate(cx + mid.cos() * pop, cy + mid.sin() * pop);
        let Some(path) = sector_path(cx, cy, lo + 1.5 * s, hi - 1.0 * s, a0, a1) else {
            return;
        };
        let paper = Rgb::new(248, 245, 238);
        let tint = mix_rgb(sec.accent, paper, 0.55);
        let Rgb(r, g, b) = tint;
        let edge = mix_rgb(sec.accent, Rgb::new(43, 38, 32), 0.55);
        let Rgb(er, eg, eb) = edge;

        // The card's shadow under it, then the paper itself.
        let lift = if sec.hot { 3.2 } else { 2.5 };
        let ts_shadow = ts.post_translate(0.0, lift * s);
        canvas.fill_path(
            &path,
            &solid(60, 50, 35, if sec.hot { 0.24 } else { 0.18 } * a),
            FillRule::Winding,
            ts_shadow,
            None,
        );
        let (inner_a, outer_a) = if sec.hot { (1.0, 0.95) } else { (0.94, 0.86) };
        if let Some(shader) = RadialGradient::new(
            Point::from_xy(cx, cy),
            lo,
            Point::from_xy(cx, cy),
            hi,
            vec![
                GradientStop::new(0.0, Color::from_rgba8(r, g, b, (inner_a * a * 255.0) as u8)),
                GradientStop::new(1.0, Color::from_rgba8(r, g, b, (outer_a * a * 255.0) as u8)),
            ],
            SpreadMode::Pad,
            Transform::identity(),
        ) {
            canvas.fill_path(
                &path,
                &Paint {
                    shader,
                    anti_alias: true,
                    ..Default::default()
                },
                FillRule::Winding,
                ts,
                None,
            );
        }
        if let Some(band_path) = sector_path(cx, cy, lo + 1.4 * s, lo + 2.4 * s, a0, a1) {
            canvas.fill_path(
                &band_path,
                &solid(255, 255, 255, 0.42 * a),
                FillRule::Winding,
                ts,
                None,
            );
        }
        stroke_path(
            canvas,
            &path,
            solid(er, eg, eb, 0.95 * a),
            if sec.hot { 1.2 } else { 1.0 } * s,
            ts,
        );
        if sec.current {
            let stroke = Stroke {
                width: 1.2 * s,
                dash: StrokeDash::new(vec![3.0 * s, 3.0 * s], 0.0),
                ..Default::default()
            };
            canvas.stroke_path(&path, &solid(43, 38, 32, 0.45), &stroke, ts, None);
        }

        let px = ((hi - lo) * 0.30).clamp(12.0 * s, 22.0 * s);
        let arc = sec.span.to_radians() * rr;
        let room = (arc - 10.0 * s).max(10.0);
        let show_name = arc > 50.0 * s;
        let glyph = sec.glyph.clone();
        let name = if show_name {
            Some(self.elide(&sec.name, 10.5 * s, room))
        } else {
            None
        };
        let gx = cx + mid.cos() * rr;
        let gy = cy + mid.sin() * rr;
        let dy = if name.is_some() { -px * 0.48 } else { 0.0 };
        let icon_px = if name.is_some() {
            (px * 1.55).clamp(16.0 * s, 34.0 * s)
        } else {
            ((hi - lo) * 0.56).clamp(16.0 * s, 38.0 * s)
        };
        let ink = if sec.hot { (30, 26, 21) } else { PAPER_INK };
        let sub = PAPER_SUB;
        let mut drew_icon = false;
        if sec.hwnd != 0 {
            if let Some(icon) = self.icon(sec.hwnd) {
                let paint = PixmapPaint {
                    opacity: if sec.hot { 1.0 } else { 0.94 } * a,
                    quality: FilterQuality::Bilinear,
                    ..Default::default()
                };
                let sx = icon_px / icon.width() as f32;
                let sy = icon_px / icon.height() as f32;
                let at = Transform::from_scale(sx, sy).post_translate(
                    (gx - icon_px / 2.0).round(),
                    (gy + dy - icon_px / 2.0).round(),
                );
                canvas.draw_pixmap(0, 0, icon.as_ref(), &paint, at, None);
                drew_icon = true;
            }
        }
        if !drew_icon {
            if let Some(p) = self.label_bold(&glyph, px, ink) {
                blit_alpha(
                    canvas,
                    p,
                    gx - p.width() as f32 / 2.0,
                    gy + dy - p.height() as f32 / 2.0,
                    a,
                );
            }
        }
        if let Some(text) = &name {
            if let Some(p) = self.label(text, 10.5 * s, sub) {
                blit_alpha(
                    canvas,
                    p,
                    gx - p.width() as f32 / 2.0,
                    gy + px * 0.58 - p.height() as f32 / 2.0,
                    a * 0.92,
                );
            }
        }

        if sec.pinned {
            let bx = cx + mid.cos() * (hi - 9.0 * s);
            let by = cy + mid.sin() * (hi - 9.0 * s);
            fill_circle(canvas, bx, by, 2.6 * s, solid(43, 38, 32, 0.9));
            let mut pb = PathBuilder::new();
            pb.move_to(bx, by);
            pb.line_to(bx, by + 6.0 * s);
            if let Some(stem) = pb.finish() {
                stroke_path(
                    canvas,
                    &stem,
                    solid(43, 38, 32, 0.8),
                    1.2 * s,
                    Transform::identity(),
                );
            }
        }
    }

    /// Neon's hub gets one extra ring of light outside it; the rest of the hub -
    /// the thumbnail, the heading, the sub-line - is the shared body with a dark
    /// skin.
    fn draw_neon_hub(&mut self, canvas: &mut Pixmap, view: &WheelView, cx: f32, cy: f32, s: f32) {
        let hub = view.hub * s;
        if let Some(path) = circle_path(cx, cy, hub - 1.0 * s) {
            stroke_path(
                canvas,
                &path,
                solid(120, 220, 255, 0.14),
                3.0 * s,
                Transform::identity(),
            );
        }
        self.draw_hub(canvas, view, cx, cy, s);
    }

    /// Paper's hub sits on the sheet: a short shadow under it, then the shared
    /// body with a paper skin.
    fn draw_paper_hub(&mut self, canvas: &mut Pixmap, view: &WheelView, cx: f32, cy: f32, s: f32) {
        let hub = view.hub * s;
        if let Some(path) = circle_path(cx, cy, hub - 1.0 * s) {
            canvas.fill_path(
                &path,
                &solid(60, 50, 35, 0.16),
                FillRule::Winding,
                Transform::from_translate(0.0, 2.0 * s),
                None,
            );
        }
        self.draw_hub(canvas, view, cx, cy, s);
    }

    /// The middle of the dial: the preview of the window under the pointer, or
    /// the one line the wheel has to say instead.
    fn draw_hub(&mut self, canvas: &mut Pixmap, view: &WheelView, cx: f32, cy: f32, s: f32) {
        let hub = view.hub * s;
        let appear = view.open.clamp(0.0, 1.0);
        let Some(path) = circle_path(cx, cy, hub - 1.0 * s) else {
            return;
        };
        // The hub is part of the theme's structure, so it gets its own fixed
        // skin: near-black with a cyan rim on neon, a lit sheet on paper.
        let (fill, rim_cancel, rim_preview, rim_idle) = match view.theme {
            4 => (
                (5, 7, 14, 0.94),
                (150, 225, 255, 0.95),
                (120, 220, 255, 0.75),
                (90, 140, 180, 0.85),
            ),
            5 => (
                (253, 251, 246, 0.98),
                (43, 38, 32, 0.90),
                (120, 110, 95, 0.75),
                (180, 170, 155, 0.85),
            ),
            _ => (
                (10, 14, 20, 0.72),
                (160, 190, 225, 0.90),
                (160, 190, 225, 0.55),
                (70, 86, 110, 0.80),
            ),
        };
        let (fr, fg, fb, fa) = fill;
        canvas.fill_path(
            &path,
            &solid(fr, fg, fb, fa),
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        let rim = if view.cancelling {
            let (r, g, b, a) = rim_cancel;
            solid(r, g, b, a)
        } else if view.preview.is_some() {
            let (r, g, b, a) = rim_preview;
            solid(r, g, b, a)
        } else {
            let (r, g, b, a) = rim_idle;
            solid(r, g, b, a)
        };
        stroke_path(canvas, &path, rim, 1.4 * s, Transform::identity());

        if let Some(pre) = &view.preview {
            let big = hub >= HUB_BIG * s;
            let px1 = (hub * 0.19).clamp(10.0 * s, 13.5 * s);
            let px2 = (hub * 0.15).clamp(9.0 * s, 11.0 * s);
            let mut ny = cy - hub * 0.06;
            let mut y2 = ny + px1 * 0.75 + px2 * 0.45;
            let Rgb(r, g, b) = pre.accent;
            if big {
                let ch = hub * 0.42;
                let mut aspect = 16.0 / 10.0;
                if let Some(shot) = pre.shot {
                    if shot.height() > 0 {
                        aspect = shot.width() as f32 / shot.height() as f32;
                    }
                }
                let cw = (hub * 1.5).min(ch * aspect);
                let x0 = cx - cw / 2.0;
                let box_h = ch + 6.0 * s + px1 * 1.25 + px2 * 1.1;
                let y0 = cy - box_h / 2.0;
                ny = y0 + ch + 6.0 * s + px1 * 0.6;
                y2 = ny + px1 * 0.55 + px2 * 0.85;
                fill_rrect(
                    canvas,
                    x0,
                    y0,
                    cw,
                    ch,
                    6.0 * s,
                    solid(
                        if view.theme == 5 { 238 } else { 21 },
                        if view.theme == 5 { 233 } else { 28 },
                        if view.theme == 5 { 221 } else { 38 },
                        if view.theme == 5 { 0.90 } else { 1.0 },
                    ),
                );
                let frame = rrect_path(x0, y0, cw, ch, 6.0 * s);
                if let (Some(shot), Some(frame)) = (pre.shot, frame.as_ref()) {
                    if let Some(mut mask) = Mask::new(canvas.width(), canvas.height()) {
                        mask.fill_path(frame, FillRule::Winding, true, Transform::identity());
                        let k = (cw / shot.width() as f32).min(ch / shot.height() as f32);
                        let dw = shot.width() as f32 * k;
                        let dh = shot.height() as f32 * k;
                        let at =
                            Transform::from_translate(x0 + (cw - dw) / 2.0, y0 + (ch - dh) / 2.0)
                                .pre_scale(k, k);
                        let paint = PixmapPaint {
                            quality: FilterQuality::Bilinear,
                            ..Default::default()
                        };
                        canvas.draw_pixmap(0, 0, shot.as_ref(), &paint, at, Some(&mask));
                    }
                }
                if let Some(frame) = frame {
                    stroke_path(
                        canvas,
                        &frame,
                        solid(r, g, b, 0.85),
                        1.3 * s,
                        Transform::identity(),
                    );
                }
            }
            let head_room = hub * 1.8;
            let head = self.elide_bold(pre.head, px1, head_room);
            let sub = self.elide(pre.sub, px2, head_room);
            let head_ink = match view.theme {
                4 => NEON_INK,
                5 => PAPER_INK,
                _ => (242, 246, 251),
            };
            let sub_ink = match view.theme {
                4 => NEON_SUB,
                5 => PAPER_SUB,
                _ => (127, 154, 194),
            };
            if let Some(p) = self.label_bold(&head, px1, head_ink) {
                blit_alpha(
                    canvas,
                    p,
                    cx - p.width() as f32 / 2.0,
                    ny - p.height() as f32 / 2.0,
                    appear,
                );
            }
            if let Some(p) = self.label(&sub, px2, sub_ink) {
                blit_alpha(
                    canvas,
                    p,
                    cx - p.width() as f32 / 2.0,
                    y2 - p.height() as f32 / 2.0,
                    appear * 0.9,
                );
            }
        } else if let Some(note) = view.note {
            let px = (hub * 0.16).clamp(11.0 * s, 15.0 * s);
            let text = self.elide(note, px, hub * 1.7);
            let note_ink = match view.theme {
                4 => NEON_INK,
                5 => PAPER_INK,
                _ => WHEEL_INK,
            };
            if let Some(p) = self.label(&text, px, note_ink) {
                blit_alpha(
                    canvas,
                    p,
                    cx - p.width() as f32 / 2.0,
                    cy - p.height() as f32 / 2.0,
                    appear,
                );
            }
        }
    }
}

/// The whole-screen dim. Composed only when the highlighted window changes - a
/// full-screen layer is expensive precisely because it is not the dial.
///
/// `cut` decides whether the outline is also opened up: it is only true when the
/// window's own pixels are really under it (the compositor accepted a thumbnail,
/// or the window is already in front), so an empty bright rectangle can never
/// appear. The rim is stroked either way, because a hover that answers with a
/// bare dim looks like the wheel has stopped listening.
pub fn compose_veil_into(
    canvas: &mut Pixmap,
    outline: Option<(f32, f32, f32, f32)>,
    cut: bool,
    accent: Rgb,
) {
    // The dim is one colour over the whole screen. Filling it as a path made
    // tiny-skia rasterise two million antialiased pixels on every hover of a new
    // window, which measured at tens of milliseconds - the stutter while
    // choosing. `Pixmap::fill` is a straight store, so the same picture costs a
    // fraction.
    canvas.fill(Color::from_rgba8(4, 7, 12, (0.42_f32 * 255.0).round() as u8));
    let Some((x, y, ow, oh)) = outline else {
        return;
    };
    let Some(hole) = rrect_path(x, y, ow, oh, 8.0) else {
        return;
    };
    if cut {
        // The hole: whatever is under the target window keeps its own pixels, so
        // it reads as the only lit thing on the screen. A `Clear` fill touches
        // only the hole's bounding box, not the screen.
        let clear = Paint {
            shader: Shader::SolidColor(Color::TRANSPARENT),
            blend_mode: BlendMode::Clear,
            anti_alias: false,
            ..Default::default()
        };
        canvas.fill_path(&hole, &clear, FillRule::Winding, Transform::identity(), None);
    }
    let Rgb(r, g, b) = accent;
    if let Some(glow) = rrect_path(x - 1.0, y - 1.0, ow + 2.0, oh + 2.0, 9.0) {
        stroke_path(
            canvas,
            &glow,
            solid(r, g, b, 0.55),
            5.0,
            Transform::identity(),
        );
    }
    stroke_path(
        canvas,
        &hole,
        solid(255, 255, 255, 0.85),
        1.8,
        Transform::identity(),
    );
}

/// The overshoot the opening is asked for: past 1 on the way to 1.
fn back_out(t: f32) -> f32 {
    let c1 = 1.701_58;
    let c3 = c1 + 1.0;
    let x = t - 1.0;
    1.0 + c3 * x * x * x + c1 * x * x
}

/// A whole circle, as a path. tiny-skia 0.12 has no arc builder, so a quarter
/// turn is one cubic - the same constant the rounded rectangles in this file
/// already use, which is why the circles are round at any radius.
fn circle_path(cx: f32, cy: f32, r: f32) -> Option<Path> {
    arc_path_closed(cx, cy, r, 0.0, std::f32::consts::TAU)
}

/// One arc of a circle, from `a0` to `a1` radians, as an open path.
fn arc_path(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    if r <= 0.0 || a1 - a0 < 1e-5 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(cx + r * a0.cos(), cy + r * a0.sin());
    push_arc(&mut pb, cx, cy, r, a0, a1);
    pb.finish()
}

/// A whole circle as one closed contour, which a fill needs.
fn arc_path_closed(cx: f32, cy: f32, r: f32, a0: f32, a1: f32) -> Option<Path> {
    if r <= 0.0 || a1 - a0 < 1e-5 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(cx + r * a0.cos(), cy + r * a0.sin());
    push_arc(&mut pb, cx, cy, r, a0, a1);
    pb.close();
    pb.finish()
}

/// Annular sector: the ring band between `lo` and `hi`, over the angles between
/// `a0` and `a1`. Bearings run clockwise from 12 o'clock, so the shell hands the
/// canvas angle over already converted.
fn sector_path(cx: f32, cy: f32, lo: f32, hi: f32, a0: f32, a1: f32) -> Option<Path> {
    if hi <= lo || a1 <= a0 {
        return None;
    }
    let mut pb = PathBuilder::new();
    pb.move_to(cx + lo * a0.cos(), cy + lo * a0.sin());
    push_arc(&mut pb, cx, cy, lo, a0, a1);
    pb.line_to(cx + hi * a1.cos(), cy + hi * a1.sin());
    push_arc(&mut pb, cx, cy, hi, a1, a0);
    pb.close();
    pb.finish()
}

/// Adds a circular arc as a run of cubics. A quarter turn is as much as one
/// cubic can carry without a visible corner, so a full circle is four.
fn push_arc(pb: &mut PathBuilder, cx: f32, cy: f32, r: f32, from: f32, to: f32) {
    let delta = to - from;
    if delta.abs() < 1e-6 {
        return;
    }
    let steps = (delta.abs() / (std::f32::consts::FRAC_PI_2))
        .ceil()
        .max(1.0) as i32;
    let step = delta / steps as f32;
    let lambda = (4.0 / 3.0) * (step / 4.0).tan() * r;
    let mut a = from;
    for _ in 0..steps {
        let b = a + step;
        let (x0, y0) = (cx + r * a.cos(), cy + r * a.sin());
        let (x1, y1) = (cx + r * b.cos(), cy + r * b.sin());
        pb.cubic_to(
            x0 - lambda * a.sin(),
            y0 + lambda * a.cos(),
            x1 + lambda * b.sin(),
            y1 - lambda * b.cos(),
            x1,
            y1,
        );
        a = b;
    }
}

fn fill_circle(canvas: &mut Pixmap, cx: f32, cy: f32, r: f32, paint: Paint<'static>) {
    if let Some(path) = circle_path(cx, cy, r) {
        canvas.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// A cached label, faded. The label's own pixels carry the colour; this scales
/// only its coverage, which is what a sector needs as it arrives.
fn blit_alpha(canvas: &mut Pixmap, label: &Pixmap, x: f32, y: f32, alpha: f32) {
    canvas.draw_pixmap(
        x.round() as i32,
        y.round() as i32,
        label.as_ref(),
        &PixmapPaint {
            opacity: alpha.clamp(0.0, 1.0),
            ..Default::default()
        },
        Transform::identity(),
        None,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use shaiwin_core::wheel::{Geometry, Ring};

    /// One frame of the shipped dial: six seats in one ring, glass theme, fully
    /// open. That is the picture whose square edge the desktop showed.
    fn glass_frame<'a>(
        sectors: &'a [WheelSector],
        bounds: &'a [Ring],
    ) -> WheelView<'a> {
        WheelView {
            scale: 1.0,
            radius: 220.0,
            hub: 92.4,
            bounds,
            sectors,
            preview: None,
            note: None,
            cancelling: false,
            open: 1.0,
            seam: 8.0,
            theme: 1,
        }
    }

    fn six_seats() -> Vec<WheelSector> {
        (0..6)
            .map(|i| WheelSector {
                ring: 0,
                bearing: i as f32 * 60.0,
                span: 55.0,
                accent: Rgb::new(90, 150, 210),
                glyph: "S".to_string(),
                hwnd: 0,
                name: String::new(),
                hot: i == 0,
                pinned: false,
                current: false,
                pop: 0.0,
                appear: 1.0,
            })
            .collect()
    }

    fn alpha(pix: &Pixmap, x: u32, y: u32) -> u8 {
        pix.data()[(y * pix.width() + x) as usize * 4 + 3]
    }

    /// Premultiplied RGB at a pixel, un-premultiplied by its own alpha so a
    /// nearly transparent pixel still reports the colour it was given.
    fn colour(pix: &Pixmap, x: u32, y: u32) -> (u8, u8, u8) {
        let at = (y * pix.width() + x) as usize * 4;
        let d = pix.data();
        let a = d[at + 3] as f32 / 255.0;
        if a <= 0.001 {
            return (0, 0, 0);
        }
        (
            (d[at] as f32 / a).round() as u8,
            (d[at + 1] as f32 / a).round() as u8,
            (d[at + 2] as f32 / a).round() as u8,
        )
    }

    /// Every pixel the square canvas holds must be either the disc (with its
    /// soft shadow) or nothing. A shadow that runs off the edge of its canvas
    /// is cut into a square: that is the border the user saw.
    #[test]
    fn the_disc_canvas_is_alpha_zero_outside_the_disc() {
        let geometry = Geometry {
            radius: 220.0,
            dead_zone: 0.42,
        };
        let bounds = geometry.ring_bounds(1);
        let side = ((geometry.extent(1) + DISC_SHADOW_PAD) * 2.0) as u32;
        let seats = six_seats();
        let mut painter = Painter::new();
        for theme in 0..6 {
            let mut view = glass_frame(&seats, &bounds);
            view.theme = theme;
            if theme >= 4 {
                view.open = 1.0;
            }
            let Some(mut canvas) = Pixmap::new(side, side) else {
                return;
            };
            let started = std::time::Instant::now();
            for _ in 0..2 {
                painter.compose_wheel_into(&mut canvas, &view);
            }
            let warm = started.elapsed().as_secs_f64() * 1000.0;
            let started = std::time::Instant::now();
            for _ in 0..20 {
                painter.compose_wheel_into(&mut canvas, &view);
            }
            let per_frame = started.elapsed().as_secs_f64() / 20.0 * 1000.0;

            let mut border_max = 0u8;
            let mut border_at = (0u32, 0u32);
            for i in 0..side {
                for (x, y) in [(i, 0), (i, side - 1), (0, i), (side - 1, i)] {
                    let a = alpha(&canvas, x, y);
                    if a > border_max {
                        border_max = a;
                        border_at = (x, y);
                    }
                }
            }
            let corners = [
                alpha(&canvas, 0, 0),
                alpha(&canvas, side - 1, 0),
                alpha(&canvas, 0, side - 1),
                alpha(&canvas, side - 1, side - 1),
            ];
            // Points clear of the dial, halfway between its rim and the frame.
            let cx = side as f32 / 2.0;
            let sy = cx;
            let mut faint = (0.0f32, 0u32, 0u32, 0u8);
            let mut solid = (0.0f32, 0u32, 0u32, 0u8);
            for y in 0..side {
                for x in 0..side {
                    let a = alpha(&canvas, x, y);
                    if a == 0 {
                        continue;
                    }
                    let r = (x as f32 - cx).hypot(y as f32 - sy);
                    if a >= 8 {
                        if r > solid.0 {
                            solid = (r, x, y, a);
                        }
                    } else if r > faint.0 {
                        faint = (r, x, y, a);
                    }
                }
            }

            if theme == 1 {
                if let Some(path) = std::env::var_os("SPINWIN_DISC_PNG") {
                    let _ = canvas.save_png(path);
                }
            }

            println!(
                "theme {theme} disc {side}x{side}: first {warm:.2}ms, then {per_frame:.2}ms/frame, border max alpha={border_max} at {border_at:?}, corners={corners:?}, furthest solid={solid:?} faint={faint:?}"
            );
            assert_eq!(
                corners, [0, 0, 0, 0],
                "theme {theme}: the corners of the square must be empty"
            );
            assert!(
                solid.0 <= side as f32 / 2.0 - 2.0,
                "theme {theme}: the wash must die out inside the frame (solid {solid:?})"
            );
            assert_eq!(
                border_max, 0,
                "theme {theme}: no pixel on the canvas edge may carry the shadow: a cut shadow is a square edge"
            );
            // The two new themes must differ in structure, not only in colour:
            // neon is a lit black plate, paper is a lit sheet.
            let (r, g, b) = colour(&canvas, side / 2, side / 2);
            let luma = (r as u32 + g as u32 + b as u32) / 3;
            if theme == 4 {
                assert!(
                    luma < 40,
                    "theme 4 must be a near-black plate at its hub, got {luma}"
                );
            }
            if theme == 5 {
                assert!(
                    luma > 200,
                    "theme 5 must be a light sheet at its hub, got {luma}"
                );
            }
        }
    }

    /// The scrim is meant to be one flat colour over the whole monitor: no edge
    /// anywhere on the screen. Only the window under the pointer is cut out.
    #[test]
    fn veil_is_one_scrim_without_an_edge() {
        let Some(mut pix) = Pixmap::new(640, 400) else {
            return;
        };
        compose_veil_into(&mut pix, None, false, Rgb::new(90, 150, 210));
        let corners = [
            alpha(&pix, 0, 0),
            alpha(&pix, 639, 0),
            alpha(&pix, 0, 399),
            alpha(&pix, 639, 399),
            alpha(&pix, 320, 200),
        ];
        println!("veil corners+centre alpha={corners:?}");
        assert!(
            corners.iter().all(|&a| a == 107),
            "the scrim must be the same everywhere: {corners:?}"
        );

        compose_veil_into(
            &mut pix,
            Some((120.0, 60.0, 300.0, 220.0)),
            true,
            Rgb::new(90, 150, 210),
        );
        let inside = alpha(&pix, 270, 170);
        let outside = alpha(&pix, 10, 10);
        println!("veil hole inside={inside} outside={outside}");
        assert_eq!(inside, 0, "the lit window must be cut clean out of the scrim");
        assert_eq!(outside, 107, "the rest of the screen keeps the one scrim");
    }

    /// A hover whose thumbnail never arrived still answers: the outline is drawn
    /// but not opened, so the screen keeps the scrim and gains a frame.
    #[test]
    fn an_outline_without_a_cut_is_a_frame_on_the_scrim() {
        let Some(mut pix) = Pixmap::new(640, 400) else {
            return;
        };
        compose_veil_into(
            &mut pix,
            Some((120.0, 60.0, 300.0, 220.0)),
            false,
            Rgb::new(90, 150, 210),
        );
        let inside = alpha(&pix, 270, 170);
        let edge = alpha(&pix, 270, 60);
        let outside = alpha(&pix, 10, 10);
        println!("veil frame inside={inside} edge={edge} outside={outside}");
        assert_eq!(inside, 107, "without a cut the window stays under the scrim");
        assert_eq!(outside, 107, "the rest of the screen is unchanged");
        assert!(edge > 150, "the rim must still mark the window, got {edge}");
    }

    /// The live wheel no longer composes the whole frame: it copies the still
    /// half and draws the moving half on top. That has to be the same picture,
    /// down to the last pixel - including the second sector pass the themes
    /// older than paper and neon have always made.
    #[test]
    fn cached_frame_matches_compose_wheel() {
        let geometry = Geometry {
            radius: 220.0,
            dead_zone: 0.42,
        };
        let bounds = geometry.ring_bounds(1);
        let side = ((geometry.extent(1) + DISC_SHADOW_PAD) * 2.0) as u32;
        let seats = six_seats();
        let mut painter = Painter::new();
        for theme in [0, 1, 4, 5] {
            let mut view = glass_frame(&seats, &bounds);
            view.theme = theme;
            let (Some(mut whole), Some(mut cached)) = (Pixmap::new(side, side), Pixmap::new(side, side))
            else {
                return;
            };
            painter.compose_wheel_into(&mut whole, &view);
            painter.compose_still_into(&mut cached, &view);
            if theme < 4 {
                painter.compose_seats_into(&mut cached, &view);
            }
            painter.compose_seats_into(&mut cached, &view);
            assert!(
                whole.data() == cached.data(),
                "theme {theme}: cached still + seats must reproduce compose_wheel_into"
            );
        }
    }

    /// Where the frame time goes, so the shadow change can be compared with the
    /// number the wheel's own meter reported before it. The scrim is the full
    /// screen and stays a straight store; the dial is the part that repaints on
    /// every hover.
    #[test]
    fn wheel_frame_costs() {
        let geometry = Geometry {
            radius: 220.0,
            dead_zone: 0.42,
        };
        let bounds = geometry.ring_bounds(1);
        let side = ((geometry.extent(1) + DISC_SHADOW_PAD) * 2.0) as u32;
        let seats = six_seats();
        let mut painter = Painter::new();
        let mut view = glass_frame(&seats, &bounds);
        view.theme = 1;

        // The scrim: one monitor-sized canvas, once per opened wheel and once per
        // newly highlighted window, not per hover frame.
        if let Some(mut veil) = Pixmap::new(1920, 1080) {
            let started = std::time::Instant::now();
            for _ in 0..5 {
                compose_veil_into(&mut veil, None, false, Rgb::new(90, 150, 210));
            }
            let plain = started.elapsed().as_secs_f64() / 5.0 * 1000.0;
            let started = std::time::Instant::now();
            for _ in 0..5 {
                compose_veil_into(
                    &mut veil,
                    Some((300.0, 200.0, 900.0, 600.0)),
                    true,
                    Rgb::new(90, 150, 210),
                );
            }
            let holed = started.elapsed().as_secs_f64() / 5.0 * 1000.0;
            println!("veil 1920x1080: {plain:.2}ms full, {holed:.2}ms with a lit window");
        }

        if let Some(mut canvas) = Pixmap::new(side, side) {
            let started = std::time::Instant::now();
            for _ in 0..2 {
                painter.compose_wheel_into(&mut canvas, &view);
            }
            let first = started.elapsed().as_secs_f64() / 2.0 * 1000.0;
            let started = std::time::Instant::now();
            for _ in 0..20 {
                painter.compose_wheel_into(&mut canvas, &view);
            }
            let per = started.elapsed().as_secs_f64() / 20.0 * 1000.0;
            let started = std::time::Instant::now();
            for _ in 0..20 {
                painter.compose_base_into(&mut canvas, &view);
            }
            let base = started.elapsed().as_secs_f64() / 20.0 * 1000.0;
            let started = std::time::Instant::now();
            for _ in 0..20 {
                painter.compose_seats_into(&mut canvas, &view);
            }
            let seats = started.elapsed().as_secs_f64() / 20.0 * 1000.0;
            // What the live wheel now pays per hover frame: the still half is a
            // copy of the frame before it, and only the moving half is drawn.
            let mut still = canvas.clone();
            painter.compose_still_into(&mut still, &view);
            let started = std::time::Instant::now();
            for _ in 0..20 {
                canvas.data_mut().copy_from_slice(still.data());
                painter.compose_seats_into(&mut canvas, &view);
                painter.compose_seats_into(&mut canvas, &view);
            }
            let cached = started.elapsed().as_secs_f64() / 20.0 * 1000.0;
            println!(
                "dial {side}x{side}: {first:.2}ms first, {per:.2}ms per hover frame (base {base:.2}ms, seats {seats:.2}ms, cached still {cached:.2}ms)"
            );
        }
    }
}
