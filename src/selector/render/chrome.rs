//! The Linux selector's chrome on the app's design system (`boltsnap::ui`):
//! the selection's border and grips, the size badge, and where floating
//! chrome sits against the selection.

use boltsnap::ui::text::Fonts;
use boltsnap::ui::theme::{self, CTL_H, GAP_M, GAP_S, RADIUS, Rgb, TextStyle};
use boltsnap::ui::{Canvas, Rect};
use tiny_skia::{Paint, Pixmap, Transform};

use super::toolbar;

/// What the selector draws its chrome with: the UI face and the toolbar's
/// widths measured in it once.
pub struct Chrome {
    pub fonts: Fonts,
    pub toolbar: toolbar::Metrics,
}

impl Chrome {
    pub fn new(fonts: Fonts) -> Self {
        let toolbar = toolbar::Metrics::new(&fonts);
        Self { fonts, toolbar }
    }
}

/// The 1 px ring around the selection.
const RING: Rgb = (0xFF, 0xFF, 0xFF);
/// The 1 px line outside the ring, so it reads on light screens too.
const CONTOUR: Rgb = theme::WINDOW;
/// Grip side, a whole even number so it centres on a pixel edge.
const GRIP: i32 = 8;
/// Horizontal room around a label: as much as above and below its capitals.
pub const SIDE: f32 = (CTL_H - TextStyle::Label.cap()) / 2.0;

/// Where floating chrome of `size` sits against the selection, inside
/// `area`: above it, else below it, else inside its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Above,
    Below,
    Inside,
}

/// Top-left (whole pixels) and placement of a `w`×`h` box: `GAP_M` from the
/// selection, `GAP_S` from the edge of `area`. The caller makes sure the box
/// fits `area` with that margin.
pub fn place(sel: (f32, f32, f32, f32), (w, h): (f32, f32), area: Rect) -> (f32, f32, Placement) {
    let (sx, sy, _, sh) = sel;
    let (left, right) = (
        area.x + GAP_S,
        (area.right() - w - GAP_S).max(area.x + GAP_S),
    );
    let (top, bottom) = (
        area.y + GAP_S,
        (area.bottom() - h - GAP_S).max(area.y + GAP_S),
    );
    let above = sy - GAP_M - h;
    let below = sy + sh + GAP_M;
    let (x, y, placement) = if above >= top {
        (sx, above, Placement::Above)
    } else if below <= bottom {
        (sx, below, Placement::Below)
    } else {
        (sx + GAP_M, sy + GAP_M, Placement::Inside)
    };
    (
        x.clamp(left, right).round(),
        y.clamp(top, bottom).round(),
        placement,
    )
}

/// The selection rounded to the pixels a crop takes: left, top, right,
/// bottom (exclusive). `None` when nothing is left.
fn pixels(sel: (f32, f32, f32, f32)) -> Option<(i32, i32, i32, i32)> {
    let (x, y, w, h) = sel;
    if ![x, y, w, h].iter().all(|v| v.is_finite()) || w < 1.0 || h < 1.0 {
        return None;
    }
    let r = (
        x.round() as i32,
        y.round() as i32,
        (x + w).round() as i32,
        (y + h).round() as i32,
    );
    (r.2 > r.0 && r.3 > r.1).then_some(r)
}

/// Opaque, unblended, on whole pixels: drawing it twice leaves the same
/// pixels, so a frame may redraw it over itself.
fn solid(pm: &mut Pixmap, (x0, y0, x1, y1): (i32, i32, i32, i32), color: Rgb) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.0, color.1, color.2, 255);
    paint.anti_alias = false;
    if let Some(rect) = tiny_skia::Rect::from_ltrb(x0 as f32, y0 as f32, x1 as f32, y1 as f32) {
        pm.fill_rect(rect, &paint, Transform::identity(), None);
    }
}

/// A 1 px ring just outside `(x0, y0, x1, y1)`, `out` pixels further out.
fn ring(pm: &mut Pixmap, (x0, y0, x1, y1): (i32, i32, i32, i32), out: i32, color: Rgb) {
    let (l, t, r, b) = (x0 - out - 1, y0 - out - 1, x1 + out + 1, y1 + out + 1);
    solid(pm, (l, t, r, t + 1), color);
    solid(pm, (l, b - 1, r, b), color);
    solid(pm, (l, t + 1, l + 1, b - 1), color);
    solid(pm, (r - 1, t + 1, r, b - 1), color);
}

/// The selection's border: a white ring on the pixels just outside what the
/// crop takes, and a dark contour outside that.
pub fn draw_frame(pm: &mut Pixmap, sel: (f32, f32, f32, f32)) {
    let Some(px) = pixels(sel) else { return };
    ring(pm, px, 1, CONTOUR);
    ring(pm, px, 0, RING);
}

/// The eight grips: white squares with a dark contour, centred on the crop's
/// corners and on the middles of its edges.
pub fn draw_grips(pm: &mut Pixmap, sel: (f32, f32, f32, f32)) {
    let Some((x0, y0, x1, y1)) = pixels(sel) else {
        return;
    };
    let mx = ((x0 + x1) as f32 / 2.0).round() as i32;
    let my = ((y0 + y1) as f32 / 2.0).round() as i32;
    let half = GRIP / 2;
    for (cx, cy) in [
        (x0, y0),
        (mx, y0),
        (x1, y0),
        (x1, my),
        (x1, y1),
        (mx, y1),
        (x0, y1),
        (x0, my),
    ] {
        solid(
            pm,
            (cx - half - 1, cy - half - 1, cx + half + 1, cy + half + 1),
            CONTOUR,
        );
        solid(pm, (cx - half, cy - half, cx + half, cy + half), RING);
    }
}

/// "1450 × 942", spaces that do not break.
fn size_label(sel: (f32, f32, f32, f32)) -> String {
    format!(
        "{}\u{a0}×\u{a0}{}",
        sel.2.round() as i32,
        sel.3.round() as i32
    )
}

/// Widest digit: every digit gets that much room, so the badge keeps its
/// width while the numbers change (the face has no `tnum` for ab_glyph).
fn digit_cell(fonts: &Fonts) -> f32 {
    ('0'..='9')
        .map(|d| fonts.width(TextStyle::Label, d.encode_utf8(&mut [0; 4])))
        .fold(0.0, f32::max)
}

/// Width of `text` with tabular digits.
fn tabular_width(fonts: &Fonts, text: &str) -> f32 {
    let cell = digit_cell(fonts);
    text.chars()
        .map(|c| {
            if c.is_ascii_digit() {
                cell
            } else {
                fonts.width(TextStyle::Label, c.encode_utf8(&mut [0; 4]))
            }
        })
        .sum()
}

/// Where the size badge of `sel` sits inside `area`, in whole pixels.
pub fn size_badge_rect(fonts: &Fonts, sel: (f32, f32, f32, f32), area: Rect) -> Option<Rect> {
    pixels(sel)?;
    let w = (2.0 * SIDE + tabular_width(fonts, &size_label(sel))).ceil();
    let (x, y, _) = place(sel, (w, CTL_H), area);
    Some(Rect::new(x, y, w, CTL_H))
}

/// The selection's size on the toolbar's surface, digits in equal cells.
pub fn draw_size_badge(canvas: &mut Canvas, fonts: &Fonts, sel: (f32, f32, f32, f32), rect: Rect) {
    canvas.fill(rect, RADIUS, theme::PANEL, 1.0);
    let cell = digit_cell(fonts);
    let baseline = fonts.baseline(TextStyle::Label, rect.y, rect.h);
    let mut x = rect.x + SIDE;
    for c in size_label(sel).chars() {
        let glyph = c.encode_utf8(&mut [0; 4]).to_owned();
        let w = fonts.width(TextStyle::Label, &glyph);
        let room = if c.is_ascii_digit() { cell } else { w };
        fonts.draw(
            canvas,
            TextStyle::Label,
            &glyph,
            (x + (room - w) / 2.0, baseline),
            theme::TEXT,
            1.0,
        );
        x += room;
    }
}
