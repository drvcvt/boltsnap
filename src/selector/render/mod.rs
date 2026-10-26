//! Pure rendering + geometry for the tiny-skia region selector. No Wayland here.

use std::sync::OnceLock;

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use image::{RgbaImage, imageops};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

/// Map a drag (two surface-space points) to an image-space crop rectangle.
/// Per-axis normalize against the surface, scale to image pixels, clamp to the
/// image, and reject anything <= 1px in either dimension (returns `None`),
/// matching the egui selector's confirm rule.
pub fn rect_to_image(
    start: (f64, f64),
    now: (f64, f64),
    surf_w: u32,
    surf_h: u32,
    img_w: u32,
    img_h: u32,
) -> Option<(u32, u32, u32, u32)> {
    if surf_w == 0 || surf_h == 0 || img_w == 0 || img_h == 0 {
        return None;
    }
    let map = |p: (f64, f64)| -> (u32, u32) {
        let nx = (p.0 / surf_w as f64).clamp(0.0, 1.0);
        let ny = (p.1 / surf_h as f64).clamp(0.0, 1.0);
        (
            (nx * img_w as f64).round() as u32,
            (ny * img_h as f64).round() as u32,
        )
    };
    let (ax, ay) = map(start);
    let (bx, by) = map(now);
    let x = ax.min(bx);
    let y = ay.min(by);
    let w = ax.max(bx).saturating_sub(x);
    let h = ay.max(by).saturating_sub(y);
    if w > 1 && h > 1 {
        Some((x, y, w, h))
    } else {
        None
    }
}

/// Build the opaque screenshot base layer as a `Pixmap` sized to the surface.
/// The capture is opaque, so straight RGBA == premultiplied RGBA and we can
/// copy bytes directly; if the surface size differs from the image (shouldn't
/// at scale 1.0) we resize first as a safety net.
pub fn base_pixmap_from_image(img: &RgbaImage, w: u32, h: u32) -> Pixmap {
    let mut pm = Pixmap::new(w.max(1), h.max(1)).expect("pixmap alloc");
    if img.width() == w && img.height() == h {
        pm.data_mut().copy_from_slice(img.as_raw());
    } else {
        let resized = imageops::resize(img, w.max(1), h.max(1), imageops::FilterType::Triangle);
        pm.data_mut().copy_from_slice(resized.as_raw());
    }
    pm
}

/// Build a fully transparent base layer sized to the surface, for record mode
/// (no screenshot to freeze). `dim_and_restore` then dims the whole surface to a
/// translucent backdrop and restores the transparent selection interior, so the
/// live screen shows through the selection while the rest is dimmed.
pub fn transparent_base(w: u32, h: u32) -> Pixmap {
    // `Pixmap::new` zero-fills, i.e. premultiplied transparent black.
    Pixmap::new(w.max(1), h.max(1)).expect("pixmap alloc")
}

/// Reuses the frame allocation and the static dimmed background while dragging.
/// The caller may paint controls into the returned frame; reset clears them.
pub struct CachedOverlay {
    dimmed: Pixmap,
    frame: Pixmap,
}

impl CachedOverlay {
    pub fn new(base: &Pixmap) -> Self {
        let mut dimmed = base.clone();
        dim_and_restore_with_style(&mut dimmed, None, SCREENSHOT_OVERLAY_STYLE);
        Self {
            frame: dimmed.clone(),
            dimmed,
        }
    }

    pub fn reset_regions(
        &mut self,
        base: &Pixmap,
        sel: Option<(f32, f32, f32, f32)>,
        regions: &[(u32, u32, u32, u32)],
    ) -> &mut Pixmap {
        let selected = selection_bounds(sel, base.width(), base.height());
        let stride = base.width() as usize * 4;
        for &(x0, y0, x1, y1) in regions {
            for y in y0..y1 {
                let start = y as usize * stride + x0 as usize * 4;
                let end = y as usize * stride + x1 as usize * 4;
                self.frame.data_mut()[start..end].copy_from_slice(&self.dimmed.data()[start..end]);
                if let Some((sx0, sy0, sx1, sy1)) = selected {
                    let (left, right) = (x0.max(sx0), x1.min(sx1));
                    if y >= sy0 && y < sy1 && left < right {
                        let start = y as usize * stride + left as usize * 4;
                        let end = y as usize * stride + right as usize * 4;
                        self.frame.data_mut()[start..end].copy_from_slice(&base.data()[start..end]);
                    }
                }
            }
        }
        &mut self.frame
    }

    pub fn frame(&self) -> &Pixmap {
        &self.frame
    }

    #[allow(dead_code)] // Full-frame reference and conservative fallback.
    pub fn reset(&mut self, base: &Pixmap, sel: Option<(f32, f32, f32, f32)>) -> &mut Pixmap {
        assert_eq!(
            (base.width(), base.height()),
            (self.dimmed.width(), self.dimmed.height())
        );
        self.frame.data_mut().copy_from_slice(self.dimmed.data());
        if let Some((x0, y0, x1, y1)) = selection_bounds(sel, base.width(), base.height()) {
            let stride = base.width() as usize * 4;
            let span = (x1 - x0) as usize * 4;
            for y in y0..y1 {
                let start = y as usize * stride + x0 as usize * 4;
                self.frame.data_mut()[start..start + span]
                    .copy_from_slice(&base.data()[start..start + span]);
            }
        }
        &mut self.frame
    }
}

fn selection_bounds(
    sel: Option<(f32, f32, f32, f32)>,
    w: u32,
    h: u32,
) -> Option<(u32, u32, u32, u32)> {
    let (sx, sy, sw, sh) = sel?;
    if sw < 1.0 || sh < 1.0 {
        return None;
    }
    let x0 = (sx.max(0.0).floor() as u32).min(w);
    let y0 = (sy.max(0.0).floor() as u32).min(h);
    let x1 = ((sx + sw).max(0.0).ceil() as u32).min(w);
    let y1 = ((sy + sh).max(0.0).ceil() as u32).min(h);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1, y1))
}

/// Convert a premultiplied-RGBA `Pixmap` to a premultiplied-BGRA `wl_shm`
/// Argb8888 (little-endian) buffer: swap R and B, keep A. `canvas` must be the
/// same pixel count as the pixmap (4 bytes per pixel).
#[allow(dead_code)] // Windows and full-frame reference.
pub fn pixmap_to_argb8888(pm: &Pixmap, canvas: &mut [u8]) {
    for (src, dst) in pm.data().chunks_exact(4).zip(canvas.chunks_exact_mut(4)) {
        dst[0] = src[2]; // B
        dst[1] = src[1]; // G
        dst[2] = src[0]; // R
        dst[3] = src[3]; // A
    }
}

#[derive(Clone, Copy)]
pub struct OverlayStyle {
    dim_alpha: u8,
    selection_dim_alpha: u8,
    border_rgb: (u8, u8, u8),
    border_width: f32,
    border_outline_alpha: u8,
    border_outline_inset: f32,
    border_outline_extra: f32,
    handle_rgb: (u8, u8, u8),
    handle_outline_alpha: u8,
    handle_size: f32,
    handle_outline: f32,
}

/// Linux screenshot selector appearance. Windows screenshot mode also uses this
/// exact style so both platforms render the same pixels.
pub const SCREENSHOT_OVERLAY_STYLE: OverlayStyle = OverlayStyle {
    dim_alpha: 110,
    selection_dim_alpha: 0,
    border_rgb: (255, 255, 255),
    border_width: 1.5,
    border_outline_alpha: 150,
    border_outline_inset: -1.0,
    border_outline_extra: 2.0,
    handle_rgb: (255, 255, 255),
    handle_outline_alpha: 180,
    handle_size: 10.0,
    handle_outline: 1.0,
};

/// Windows recording selector appearance. Screenshot mode uses
/// `SCREENSHOT_OVERLAY_STYLE` instead.
#[cfg(any(target_os = "windows", test))]
pub const WINDOWS_RECORD_OVERLAY_STYLE: OverlayStyle = OverlayStyle {
    dim_alpha: 110,
    selection_dim_alpha: 24,
    border_rgb: (255, 255, 255),
    border_width: 1.5,
    border_outline_alpha: 0,
    border_outline_inset: 0.0,
    border_outline_extra: 0.0,
    handle_rgb: (255, 255, 255),
    handle_outline_alpha: 240,
    handle_size: 12.0,
    handle_outline: 1.5,
};

/// Draw the selection overlay onto `pm` (which already contains the opaque
/// screenshot). `sel` is the surface-space selection `(x, y, w, h)`; `None`
/// means "no selection yet" — dim the whole surface.
#[cfg(test)]
pub fn dim_and_restore(pm: &mut Pixmap, sel: Option<(f32, f32, f32, f32)>) {
    dim_and_restore_with_style(pm, sel, SCREENSHOT_OVERLAY_STYLE);
}

pub fn dim_and_restore_with_style(
    pm: &mut Pixmap,
    sel: Option<(f32, f32, f32, f32)>,
    style: OverlayStyle,
) {
    let w = pm.width();
    let h = pm.height();

    // Integer pixel bounds of the selection interior, clamped to the surface.
    // floor/ceil so the bright region fully covers the selection. `None` when
    // there is no selection or it is sub-pixel.
    let bounds = selection_bounds(sel, w, h);

    // Save the selection's bright pixels, then dim with a SINGLE full-surface
    // fill, then write the bright pixels back. One uniform fill (instead of four
    // rects around the selection) leaves no non-antialiased fractional-edge
    // seams — those left 1px undimmed rows at the selection/screen boundary.
    let rowbytes = (w * 4) as usize;
    let saved = bounds.map(|(x0, y0, x1, y1)| {
        let span = ((x1 - x0) * 4) as usize;
        let mut buf = Vec::with_capacity(((y1 - y0) as usize) * span);
        let data = pm.data();
        for y in y0..y1 {
            let start = y as usize * rowbytes + x0 as usize * 4;
            buf.extend_from_slice(&data[start..start + span]);
        }
        buf
    });

    let mut dim = Paint::default();
    dim.set_color_rgba8(0, 0, 0, style.dim_alpha);
    dim.anti_alias = false;
    if let Some(r) = Rect::from_xywh(0.0, 0.0, w as f32, h as f32) {
        pm.fill_rect(r, &dim, Transform::identity(), None);
    }

    let (Some((x0, y0, x1, y1)), Some(saved)) = (bounds, saved) else {
        return;
    };

    // Restore the bright selection interior (opaque copy on integer bounds).
    let span = ((x1 - x0) * 4) as usize;
    let data = pm.data_mut();
    for (i, y) in (y0..y1).enumerate() {
        let start = y as usize * rowbytes + x0 as usize * 4;
        let src = i * span;
        data[start..start + span].copy_from_slice(&saved[src..src + span]);
    }

    if style.selection_dim_alpha > 0 {
        let mut selection_dim = Paint::default();
        selection_dim.set_color_rgba8(0, 0, 0, style.selection_dim_alpha);
        selection_dim.anti_alias = false;
        if let Some(rect) =
            Rect::from_xywh(x0 as f32, y0 as f32, (x1 - x0) as f32, (y1 - y0) as f32)
        {
            pm.fill_rect(rect, &selection_dim, Transform::identity(), None);
        }
    }
}

/// Stroke the selection border: white, with a 1px darker outline just outside so
/// it reads on light and dark screenshots.
#[cfg(any(not(target_os = "windows"), test))]
pub fn draw_border(pm: &mut Pixmap, sel: (f32, f32, f32, f32)) {
    draw_border_with_style(pm, sel, SCREENSHOT_OVERLAY_STYLE);
}

pub fn draw_border_with_style(pm: &mut Pixmap, sel: (f32, f32, f32, f32), style: OverlayStyle) {
    let (x, y, w, h) = sel;
    if w < 1.0 || h < 1.0 {
        return;
    }
    let mut dark = Paint::default();
    dark.set_color_rgba8(0, 0, 0, style.border_outline_alpha);
    dark.anti_alias = true;
    let mut accent = Paint::default();
    accent.set_color_rgba8(
        style.border_rgb.0,
        style.border_rgb.1,
        style.border_rgb.2,
        255,
    );
    accent.anti_alias = true;
    // Dark outline slightly outside, then the high-contrast border on top.
    for (inset, paint, width) in [
        (
            style.border_outline_inset,
            &dark,
            style.border_width + style.border_outline_extra,
        ),
        (0.0, &accent, style.border_width),
    ] {
        if let Some(r) = Rect::from_xywh(x + inset, y + inset, w - 2.0 * inset, h - 2.0 * inset) {
            let mut pb = PathBuilder::new();
            pb.push_rect(r);
            if let Some(path) = pb.finish() {
                let stroke = Stroke {
                    width,
                    ..Default::default()
                };
                pm.stroke_path(&path, paint, &stroke, Transform::identity(), None);
            }
        }
    }
}

/// Draw the 8 resize handles (corners + edge midpoints) as white squares with a
/// dark outline, centered on the selection's corners/edge-midpoints.
#[cfg(any(not(target_os = "windows"), test))]
pub fn draw_handles(pm: &mut Pixmap, sel: (f32, f32, f32, f32)) {
    draw_handles_with_style(pm, sel, SCREENSHOT_OVERLAY_STYLE);
}

pub fn draw_handles_with_style(pm: &mut Pixmap, sel: (f32, f32, f32, f32), style: OverlayStyle) {
    let (x, y, w, h) = sel;
    if w < 1.0 || h < 1.0 {
        return;
    }
    let handle_size = style.handle_size;
    let radius = handle_size * 0.35;
    let (l, t, r, b) = (x, y, x + w, y + h);
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let centers = [
        (l, t),
        (cx, t),
        (r, t),
        (r, cy),
        (r, b),
        (cx, b),
        (l, b),
        (l, cy),
    ];
    let mut dark = Paint::default();
    dark.set_color_rgba8(0, 0, 0, style.handle_outline_alpha);
    dark.anti_alias = true;
    let mut accent = Paint::default();
    accent.set_color_rgba8(
        style.handle_rgb.0,
        style.handle_rgb.1,
        style.handle_rgb.2,
        255,
    );
    accent.anti_alias = true;
    for (hx, hy) in centers {
        if let Some(path) = rounded_rect(
            hx - handle_size / 2.0 - style.handle_outline,
            hy - handle_size / 2.0 - style.handle_outline,
            handle_size + 2.0 * style.handle_outline,
            handle_size + 2.0 * style.handle_outline,
            radius + style.handle_outline,
        ) {
            pm.fill_path(&path, &dark, FillRule::Winding, Transform::identity(), None);
        }
        if let Some(path) = rounded_rect(
            hx - handle_size / 2.0,
            hy - handle_size / 2.0,
            handle_size,
            handle_size,
            radius,
        ) {
            pm.fill_path(
                &path,
                &accent,
                FillRule::Winding,
                Transform::identity(),
                None,
            );
        }
    }
}

/// The font every overlay label is drawn with, at the two weights the desktop
/// bar sets its labels in: Medium, and DemiBold for an active module. The
/// platform layer installs the desktop UI font (`set_ui_font`) so the selector
/// reads like the rest of the desktop; without one this is the embedded DejaVu
/// Sans subset the shelf falls back to (printable ASCII + ×), which has one
/// weight and so renders both the same.
static UI_FONT: OnceLock<[FontVec; 2]> = OnceLock::new();

/// Install the overlay fonts. Call before the first frame; later calls are ignored.
pub fn set_ui_font(medium: FontVec, demibold: FontVec) {
    let _ = UI_FONT.set([medium, demibold]);
}

fn ui_font(strong: bool) -> &'static FontVec {
    let fonts = UI_FONT.get_or_init(|| {
        [
            crate::shelf::font::fallback_popup_font(),
            crate::shelf::font::fallback_popup_font(),
        ]
    });
    &fonts[usize::from(strong)]
}

/// Advance width of `label` at `px`, in pixels.
fn text_width(label: &str, px: f32, strong: bool) -> f64 {
    let font = ui_font(strong);
    let scaled = font.as_scaled(PxScale::from(px));
    label
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum::<f32>() as f64
}

/// Build a rounded-rectangle path (corner radius clamped to half the smaller side).
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// Draw `text` anti-aliased in `rgb` with its left baseline at (`x`, `baseline`),
/// compositing premultiplied src-over the pixmap. `strong` picks the DemiBold
/// face the bar uses for an active label.
fn draw_text_aa(
    pm: &mut Pixmap,
    x: f32,
    baseline: f32,
    text: &str,
    px: f32,
    rgb: (u8, u8, u8),
    strong: bool,
) {
    let font = ui_font(strong);
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let w = pm.width() as i32;
    let h = pm.height() as i32;
    let (tr, tg, tb) = rgb;
    let mut caret = x;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            let data = pm.data_mut();
            outlined.draw(|gx, gy, cov| {
                let px_x = bounds.min.x as i32 + gx as i32;
                let px_y = bounds.min.y as i32 + gy as i32;
                if px_x < 0 || px_x >= w || px_y < 0 || px_y >= h {
                    return;
                }
                let a = cov.clamp(0.0, 1.0);
                let i = ((px_y * w + px_x) * 4) as usize;
                let blend = |s: u8, d: u8| (s as f32 * a + d as f32 * (1.0 - a)).round() as u8;
                data[i] = blend(tr, data[i]);
                data[i + 1] = blend(tg, data[i + 1]);
                data[i + 2] = blend(tb, data[i + 2]);
                data[i + 3] = blend(255, data[i + 3]);
            });
        }
        caret += scaled.h_advance(id);
    }
}

// Floating chrome uses the bar's colours, with larger text and minimal padding.
// Keep the background opaque: the selector surface has no compositor blur.
const UI_SURFACE: (u8, u8, u8, u8) = (0x18, 0x18, 0x18, 0xff);
const BAR_HOVER: (u8, u8, u8, u8) = (0xff, 0xff, 0xff, 0x18);
const BAR_HOVER_SOFT: (u8, u8, u8, u8) = (0xff, 0xff, 0xff, 0x14);
const BAR_ACTIVE: (u8, u8, u8, u8) = (0xff, 0xff, 0xff, 0x26);
const UI_FOREGROUND: (u8, u8, u8) = (0xee, 0xee, 0xee);
const UI_DIM: (u8, u8, u8) = (0x99, 0x99, 0x99);
const UI_FAINT: (u8, u8, u8) = (0x88, 0x88, 0x88);
const UI_RECORD: (u8, u8, u8) = (0xd9, 0x53, 0x4f);
const UI_BLOCK_H: f64 = 24.0;
const UI_TEXT_PX: f32 = 21.0;
const UI_BADGE_H: f64 = 22.0;
const UI_RADIUS: f32 = 6.0;
const UI_ICON: f64 = 18.0;
const UI_ICON_PAD: f64 = 5.0;
const UI_TEXT_PAD: f64 = 3.0;
const UI_CONTENT_GAP: f64 = 5.0;

/// Fill a rounded rect with a straight (non-premultiplied) RGBA colour.
fn fill_rounded(pm: &mut Pixmap, rect: (f64, f64, f64, f64), radius: f32, rgba: (u8, u8, u8, u8)) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgba.0, rgba.1, rgba.2, rgba.3);
    paint.anti_alias = true;
    if let Some(path) = rounded_rect(
        rect.0 as f32,
        rect.1 as f32,
        rect.2 as f32,
        rect.3 as f32,
        radius,
    ) {
        pm.fill_path(
            &path,
            &paint,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
}

/// Baseline that centres `px`-sized text in a module of height `h` at `y`.
fn centered_baseline(y: f64, h: f64, px: f32) -> f32 {
    let scaled = ui_font(false).as_scaled(PxScale::from(px));
    y as f32 + (h as f32 - (scaled.ascent() - scaled.descent())) / 2.0 + scaled.ascent()
}

/// Draw the `W×H` dimension badge: a rounded translucent pill with crisp,
/// anti-aliased text. Placed by `edit::badge_rect` (above-left, flipping at edges).
pub fn badge_bounds(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let (x, y, w, h) = sel;
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let label = format!("{}×{}", w.round() as i32, h.round() as i32);
    let scaled = ui_font(false).as_scaled(PxScale::from(UI_TEXT_PX));
    let text_w = text_width(&label, UI_TEXT_PX, false);
    let text_h = (scaled.ascent() - scaled.descent()) as f64;
    let pad = ((UI_BADGE_H - text_h) / 2.0).max(0.0);
    Some(crate::selector::edit::badge_rect(
        crate::selector::edit::Rect {
            x: x as f64,
            y: y as f64,
            w: w as f64,
            h: h as f64,
        },
        text_w + 2.0 * (UI_TEXT_PAD - pad).max(0.0),
        text_h,
        pad,
        6.0,
        surf_w as f64,
        surf_h as f64,
    ))
}

pub fn draw_badge(pm: &mut Pixmap, sel: (f32, f32, f32, f32), surf_w: u32, surf_h: u32) {
    let Some((bx, by, bw, bh)) = badge_bounds(sel, surf_w, surf_h) else {
        return;
    };
    draw_badge_at(pm, sel, (bx, by, bw, bh));
}

pub fn draw_badge_at(
    pm: &mut Pixmap,
    sel: (f32, f32, f32, f32),
    (bx, by, bw, bh): (f64, f64, f64, f64),
) {
    let label = format!("{}×{}", sel.2.round() as i32, sel.3.round() as i32);
    let text_w = text_width(&label, UI_TEXT_PX, false);
    fill_rounded(pm, (bx, by, bw, bh), UI_RADIUS, UI_SURFACE);
    draw_text_aa(
        pm,
        (bx + (bw - text_w) / 2.0) as f32,
        centered_baseline(by, bh, UI_TEXT_PX),
        &label,
        UI_TEXT_PX,
        UI_FOREGROUND,
        false,
    );
}

/// Build a circle path centered at (`cx`, `cy`) with radius `r`, as four quad
/// arcs (good enough visually for a tiny dot).
fn circle_path(cx: f32, cy: f32, r: f32) -> Option<tiny_skia::Path> {
    if r <= 0.0 {
        return None;
    }
    // kappa for a quad approximation of a quarter circle control offset.
    let k = r * 0.5522847;
    let mut pb = PathBuilder::new();
    pb.move_to(cx, cy - r);
    pb.cubic_to(cx + k, cy - r, cx + r, cy - k, cx + r, cy);
    pb.cubic_to(cx + r, cy + k, cx + k, cy + r, cx, cy + r);
    pb.cubic_to(cx - k, cy + r, cx - r, cy + k, cx - r, cy);
    pb.cubic_to(cx - r, cy - k, cx - k, cy - r, cx, cy - r);
    pb.close();
    pb.finish()
}

/// Draw the magnifier loupe at the cursor: a `LOUPE`px square sampling an
/// `SAMPLE`px window of `base` around `cursor`, nearest-neighbor upscaled, with a
/// pixel grid, a center-pixel marker, and a border. Placed by `magnifier_placement`.
pub fn draw_magnifier(
    pm: &mut Pixmap,
    base: &Pixmap,
    cursor: (f64, f64),
    surf_w: u32,
    surf_h: u32,
) {
    let position = crate::selector::edit::magnifier_placement(
        cursor,
        120.0,
        24.0,
        surf_w as f64,
        surf_h as f64,
    );
    draw_magnifier_at(pm, base, cursor, position);
}

pub fn draw_magnifier_at(pm: &mut Pixmap, base: &Pixmap, cursor: (f64, f64), (lx, ly): (f64, f64)) {
    use crate::selector::edit::magnifier_source;
    const LOUPE: u32 = 120;
    const SAMPLE: u32 = 30;
    let (lx, ly) = (lx.round() as u32, ly.round() as u32);
    let (sx, sy, sw, sh) = magnifier_source(cursor, SAMPLE, base.width(), base.height());
    if sw == 0 || sh == 0 {
        return;
    }
    let zoom = LOUPE / sw.max(1); // integer zoom
    let bd = base.data();
    let bw = base.width();
    let pw = pm.width();
    let ph = pm.height();
    // Blit nearest-neighbor.
    for dy in 0..(sh * zoom).min(LOUPE) {
        let oy = ly + dy;
        if oy >= ph {
            break;
        }
        let svy = sy + dy / zoom;
        for dx in 0..(sw * zoom).min(LOUPE) {
            let ox = lx + dx;
            if ox >= pw {
                break;
            }
            let svx = sx + dx / zoom;
            let si = ((svy * bw + svx) * 4) as usize;
            let di = ((oy * pw + ox) * 4) as usize;
            // pixel grid: darken the first row/col of each zoomed cell
            let grid = zoom >= 4 && (dx % zoom == 0 || dy % zoom == 0);
            for c in 0..3 {
                pm.data_mut()[di + c] = if grid { bd[si + c] / 2 } else { bd[si + c] };
            }
            pm.data_mut()[di + 3] = 255;
        }
    }
    // Cursor-pixel marker: track the cursor's actual pixel WITHIN the (possibly
    // edge-clamped) sample window, so it follows the cursor instead of sticking at
    // the loupe centre when the window clamps near a screen edge (~SAMPLE/2 px).
    let mut mark = Paint::default();
    mark.set_color_rgba8(255, 80, 80, 255);
    let off_x = (cursor.0.round() as i64 - sx as i64).clamp(0, sw as i64 - 1) as f32;
    let off_y = (cursor.1.round() as i64 - sy as i64).clamp(0, sh as i64 - 1) as f32;
    let cxp = lx as f32 + off_x * zoom as f32;
    let cyp = ly as f32 + off_y * zoom as f32;
    if let Some(r) = Rect::from_xywh(cxp, cyp, zoom as f32, zoom as f32) {
        let mut pb = PathBuilder::new();
        pb.push_rect(r);
        if let Some(path) = pb.finish() {
            pm.stroke_path(
                &path,
                &mark,
                &Stroke {
                    width: 1.5,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
    }
    // Loupe border.
    let mut border = Paint::default();
    border.set_color_rgba8(240, 240, 240, 255);
    if let Some(r) = Rect::from_xywh(lx as f32, ly as f32, LOUPE as f32, LOUPE as f32) {
        let mut pb = PathBuilder::new();
        pb.push_rect(r);
        if let Some(path) = pb.finish() {
            pm.stroke_path(
                &path,
                &border,
                &Stroke {
                    width: 2.0,
                    ..Default::default()
                },
                Transform::identity(),
                None,
            );
        }
    }
}

mod legacy;
mod toolbar;

// Only the Windows backend and the tests draw these.
#[cfg_attr(not(target_os = "windows"), allow(unused_imports))]
pub use legacy::*;
pub use toolbar::*;

#[cfg(test)]
mod tests;
