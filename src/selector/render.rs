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

/// Toolbar icons drawn locally, with thick strokes and rounded contours.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Record,
    Cut,
    Crop,
    Volume,
    VolumeOff,
}

const ICON_STROKE: f32 = 2.4;

fn fill_icon(pm: &mut Pixmap, path: &tiny_skia::Path, rgb: (u8, u8, u8)) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb.0, rgb.1, rgb.2, 255);
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke_icon(pm: &mut Pixmap, path: &tiny_skia::Path, rgb: (u8, u8, u8)) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb.0, rgb.1, rgb.2, 255);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: ICON_STROKE,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Default::default()
    };
    pm.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}

/// Draw `icon` in `rgb` with the top-left of its 18 px box at (`x`, `y`).
fn draw_icon(pm: &mut Pixmap, icon: Icon, x: f64, y: f64, rgb: (u8, u8, u8)) {
    let (ox, oy) = (x as f32, y as f32);
    match icon {
        Icon::Record => {
            if let Some(path) = circle_path(ox + 9.0, oy + 9.0, 4.5) {
                fill_icon(pm, &path, rgb);
            }
        }
        // Two blades crossing above two finger rings.
        Icon::Cut => {
            let mut pb = PathBuilder::new();
            pb.move_to(ox + 5.6, oy + 12.0);
            pb.line_to(ox + 13.5, oy + 2.5);
            pb.move_to(ox + 12.4, oy + 12.0);
            pb.line_to(ox + 4.5, oy + 2.5);
            if let Some(path) = pb.finish() {
                stroke_icon(pm, &path, rgb);
            }
            for cx in [4.5f32, 13.5] {
                if let Some(path) = circle_path(ox + cx, oy + 13.5, 2.4) {
                    stroke_icon(pm, &path, rgb);
                }
            }
        }
        // The four corners of a recording frame.
        Icon::Crop => {
            let mut pb = PathBuilder::new();
            for (cx, cy, sx, sy) in [
                (2.5f32, 2.5f32, 1.0f32, 1.0f32),
                (15.5, 2.5, -1.0, 1.0),
                (15.5, 15.5, -1.0, -1.0),
                (2.5, 15.5, 1.0, -1.0),
            ] {
                pb.move_to(ox + cx, oy + cy + 4.0 * sy);
                pb.line_to(ox + cx, oy + cy + 2.5 * sy);
                pb.quad_to(ox + cx, oy + cy, ox + cx + 2.5 * sx, oy + cy);
                pb.line_to(ox + cx + 4.0 * sx, oy + cy);
            }
            if let Some(path) = pb.finish() {
                stroke_icon(pm, &path, rgb);
            }
        }
        // A speaker, with sound waves or a cross.
        Icon::Volume | Icon::VolumeOff => {
            let mut body = PathBuilder::new();
            body.move_to(ox + 2.5, oy + 6.0);
            body.line_to(ox + 4.0, oy + 6.0);
            body.quad_to(ox + 4.5, oy + 6.0, ox + 5.0, oy + 5.5);
            body.line_to(ox + 8.0, oy + 3.0);
            body.quad_to(ox + 9.5, oy + 1.8, ox + 9.5, oy + 3.8);
            body.line_to(ox + 9.5, oy + 14.2);
            body.quad_to(ox + 9.5, oy + 16.2, ox + 8.0, oy + 15.0);
            body.line_to(ox + 5.0, oy + 12.5);
            body.quad_to(ox + 4.5, oy + 12.0, ox + 4.0, oy + 12.0);
            body.line_to(ox + 2.5, oy + 12.0);
            body.quad_to(ox + 1.0, oy + 12.0, ox + 1.0, oy + 10.5);
            body.line_to(ox + 1.0, oy + 7.5);
            body.quad_to(ox + 1.0, oy + 6.0, ox + 2.5, oy + 6.0);
            body.close();
            if let Some(path) = body.finish() {
                fill_icon(pm, &path, rgb);
            }
            let mut pb = PathBuilder::new();
            if icon == Icon::Volume {
                pb.move_to(ox + 12.0, oy + 6.8);
                pb.quad_to(ox + 14.0, oy + 9.0, ox + 12.0, oy + 11.2);
                pb.move_to(ox + 14.5, oy + 4.5);
                pb.quad_to(ox + 18.5, oy + 9.0, ox + 14.5, oy + 13.5);
            } else {
                pb.move_to(ox + 12.5, oy + 7.0);
                pb.line_to(ox + 16.0, oy + 10.5);
                pb.move_to(ox + 16.0, oy + 7.0);
                pb.line_to(ox + 12.5, oy + 10.5);
            }
            if let Some(path) = pb.finish() {
                stroke_icon(pm, &path, rgb);
            }
        }
    }
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

// REC pill metrics — shared by `rec_pill_rect` (placement/hit-zone) and
// `draw_rec_pill` (interior), so the clickable area exactly matches the drawn pill.
const REC_PX: f32 = 17.0;
const REC_PAD: f64 = 7.0;
const REC_DOT_R: f64 = 5.0; // red dot radius
const REC_DOT_GAP: f64 = 6.0; // gap between dot and text
const FRAME_CHECK_GAP: f64 = 6.0;
const FRAME_LABEL: &str = "REC Border Indicator";
const FRAME_PX: f32 = 13.0;
const FRAME_PAD: f64 = 7.0;
const FRAME_BOX_SIZE: f64 = 16.0;
const FRAME_BOX_GAP: f64 = 7.0;
const AUDIO_GAP: f64 = 6.0;
const AUDIO_PX: f32 = 13.0;
const AUDIO_PAD: f64 = 7.0;
const AUDIO_DOT_R: f64 = 4.0;
const AUDIO_DOT_GAP: f64 = 6.0;

/// On-screen rect `(bx, by, bw, bh)` of the REC pill for selection `sel`, placed
/// exactly like `draw_rec_pill`. Returned so the record-mode selector can hit-test
/// the pill as a clickable Start button. `None` for a sub-pixel selection.
pub fn rec_pill_rect(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let (x, y, w, h) = sel;
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(REC_PX));
    let text_w: f32 = "REC"
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let text_h = scaled.ascent() - scaled.descent();
    // The pill carries a leading red dot, so widen the content box by the dot
    // diameter plus its gap; badge_rect handles placement + on-screen clamping.
    let content_w = REC_DOT_R * 2.0 + REC_DOT_GAP + text_w as f64;
    let rect = crate::selector::edit::Rect {
        x: x as f64,
        y: y as f64,
        w: w as f64,
        h: h as f64,
    };
    Some(crate::selector::edit::badge_rect(
        rect,
        content_w,
        text_h as f64,
        REC_PAD,
        6.0,
        surf_w as f64,
        surf_h as f64,
    ))
}

/// On-screen hit box for the frame checkbox beside the REC pill.
pub fn record_frame_checkbox_rect(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let (rx, ry, rw, rh) = rec_pill_rect(sel, surf_w, surf_h)?;
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(FRAME_PX));
    let text_w: f32 = FRAME_LABEL
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let width = FRAME_PAD * 2.0 + FRAME_BOX_SIZE + FRAME_BOX_GAP + text_w as f64;
    let right = rx + rw + FRAME_CHECK_GAP;
    if right + width <= surf_w as f64 {
        return Some((right, ry, width, rh));
    }
    let left = rx - FRAME_CHECK_GAP - width;
    (left >= 0.0 && ry >= 0.0 && ry + rh <= surf_h as f64).then_some((left, ry, width, rh))
}

pub fn record_audio_button_rect(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let rec = rec_pill_rect(sel, surf_w, surf_h)?;
    let frame = record_frame_checkbox_rect(sel, surf_w, surf_h)?;
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(AUDIO_PX));
    let text_w: f32 = "AUDIO OFF"
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let width = AUDIO_PAD * 2.0 + AUDIO_DOT_R * 2.0 + AUDIO_DOT_GAP + text_w as f64;
    let height = rec.3;
    let overlaps = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
    };
    for (x, y) in [
        (frame.0 + frame.2 + AUDIO_GAP, frame.1),
        (frame.0 - AUDIO_GAP - width, frame.1),
    ] {
        let candidate = (x, y, width, height);
        if x >= 0.0
            && x + width <= surf_w as f64
            && !overlaps(candidate, rec)
            && !overlaps(candidate, frame)
        {
            return Some(candidate);
        }
    }
    let x = rec.0.min((surf_w as f64 - width).max(0.0));
    for y in [rec.1 + rec.3 + AUDIO_GAP, rec.1 - AUDIO_GAP - height] {
        if width <= surf_w as f64 && y >= 0.0 && y + height <= surf_h as f64 {
            return Some((x, y, width, height));
        }
    }
    None
}

pub struct RecordToolbar {
    pub bounds: (f64, f64, f64, f64),
    pub controls: [(f64, f64, f64, f64); 4],
}

// Keep 24 px click targets, but let the 21 px labels fill them. Only a
// single pixel separates the controls from the surrounding toolbar surface.
const TB_PAD: f64 = 1.0;
const TB_GAP: f64 = 4.0;
const TB_CARD_RADIUS: f32 = 9.0;
// Concentric outer/inner corners keep active fills inside the toolbar contour.
const TB_CONTROL_RADIUS: f32 = TB_CARD_RADIUS - TB_PAD as f32;
const TB_TEXT_PAD: f64 = 6.0;
const TB_LABELS: [&str; 4] = ["REC", "Clip", "Frame", "Audio"];

/// Left inset of a control's label, from the control's own left edge. Every
/// control carries an icon, as every module on the bar does.
fn tb_label_x() -> f64 {
    UI_ICON_PAD + UI_ICON + UI_CONTENT_GAP
}

/// The mark a control shows; the audio toggle swaps glyph the way the bar's
/// own microphone and volume modules do.
fn tb_icon(control: usize, on: bool) -> Icon {
    match control {
        0 => Icon::Record,
        1 => Icon::Cut,
        2 => Icon::Crop,
        _ if on => Icon::Volume,
        _ => Icon::VolumeOff,
    }
}

pub fn record_toolbar(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<RecordToolbar> {
    let (sx, sy, sw, sh) = sel;
    if ![sx, sy, sw, sh].iter().all(|v| v.is_finite()) || sw < 1.0 || sh < 1.0 {
        return None;
    }
    // Measured in DemiBold, the widest a label ever gets, so a module keeps its
    // width when its state changes and the strip never shifts under the cursor.
    let widths: [f64; 4] = std::array::from_fn(|i| {
        (tb_label_x() + text_width(TB_LABELS[i], UI_TEXT_PX, true)).ceil() + TB_TEXT_PAD
    });
    // One strip if it fits, else a 2×2 grid, else a single column.
    let (columns, column_widths, column_x, row_y, width, height) =
        [4usize, 2, 1].into_iter().find_map(|columns| {
            let rows = 4 / columns;
            let column_widths: [f64; 4] = std::array::from_fn(|c| {
                if c < columns {
                    widths
                        .iter()
                        .skip(c)
                        .step_by(columns)
                        .copied()
                        .fold(0.0, f64::max)
                } else {
                    0.0
                }
            });
            let mut column_x = [0.0; 4];
            let mut inner_w = 0.0;
            for c in 0..columns {
                if c > 0 {
                    inner_w += TB_GAP;
                }
                column_x[c] = inner_w;
                inner_w += column_widths[c];
            }
            let mut row_y = [0.0; 4];
            let mut inner_h = 0.0;
            for (r, y) in row_y.iter_mut().enumerate().take(rows) {
                if r > 0 {
                    inner_h += TB_GAP;
                }
                *y = inner_h;
                inner_h += UI_BLOCK_H;
            }
            let (width, height) = (inner_w + TB_PAD * 2.0, inner_h + TB_PAD * 2.0);
            (width + 16.0 <= surf_w as f64 && height + 16.0 <= surf_h as f64).then_some((
                columns,
                column_widths,
                column_x,
                row_y,
                width,
                height,
            ))
        })?;
    let mut x = (sx as f64).clamp(8.0, surf_w as f64 - width - 8.0);
    let above = sy as f64 - height - 12.0;
    let below = (sy + sh) as f64 + 12.0;
    let y = if above >= 8.0 {
        above
    } else if below + height <= surf_h as f64 - 8.0 {
        below
    } else {
        x = (sx as f64 + 12.0).clamp(8.0, surf_w as f64 - width - 8.0);
        sy as f64 + 12.0
    }
    .clamp(8.0, surf_h as f64 - height - 8.0);
    let controls = std::array::from_fn(|i| {
        (
            x + TB_PAD + column_x[i % columns],
            y + TB_PAD + row_y[i / columns],
            column_widths[i % columns],
            UI_BLOCK_H,
        )
    });
    Some(RecordToolbar {
        bounds: (x, y, width, height),
        controls,
    })
}

pub fn draw_record_toolbar(
    pm: &mut Pixmap,
    toolbar: &RecordToolbar,
    states: (bool, bool, bool),
    hovered: Option<usize>,
) {
    let (clip_enabled, frame_enabled, audio_enabled) = states;
    fill_rounded(pm, toolbar.bounds, TB_CARD_RADIUS, UI_SURFACE);
    for (i, &(x, y, w, h)) in toolbar.controls.iter().enumerate() {
        let enabled = i != 1 || clip_enabled;
        let on = (i == 2 && frame_enabled) || (i == 3 && audio_enabled);
        // Record keeps a resting fill as the primary action; every module takes
        // the bar's white hover fill.
        let fill = match (hovered == Some(i) && enabled, i == 0) {
            (true, true) => Some(BAR_ACTIVE),
            (true, false) => Some(BAR_HOVER),
            (false, true) => Some(BAR_HOVER_SOFT),
            (false, false) => None,
        };
        if let Some(rgba) = fill {
            fill_rounded(pm, (x, y, w, h), TB_CONTROL_RADIUS, rgba);
        }
        // Record is the one place with hue. Elsewhere the icon says on or off
        // the way the bar's own modules do, by going bright or dim, and an
        // active module sets its label in DemiBold.
        let (icon_rgb, strong) = match i {
            0 => (UI_RECORD, true),
            1 if enabled => (UI_FOREGROUND, false),
            1 => (UI_FAINT, false),
            _ if on => (UI_FOREGROUND, true),
            _ => (UI_DIM, false),
        };
        draw_icon(
            pm,
            tb_icon(i, on),
            x + UI_ICON_PAD,
            y + (h - UI_ICON) / 2.0,
            icon_rgb,
        );
        draw_text_aa(
            pm,
            (x + tb_label_x()) as f32,
            centered_baseline(y, h, UI_TEXT_PX),
            TB_LABELS[i],
            UI_TEXT_PX,
            // A label is always the bar's foreground; only an unavailable
            // action dims, the way a disabled menu entry does.
            if enabled { UI_FOREGROUND } else { UI_DIM },
            strong,
        );
    }
}

#[allow(dead_code)]
pub fn draw_record_audio_button(
    pm: &mut Pixmap,
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    enabled: bool,
) {
    let Some((x, y, w, h)) = record_audio_button_rect(sel, surf_w, surf_h) else {
        return;
    };
    let mut background = Paint::default();
    background.set_color_rgba8(0x12, 0x12, 0x12, 230);
    background.anti_alias = true;
    if let Some(path) = rounded_rect(x as f32, y as f32, w as f32, h as f32, 7.0) {
        pm.fill_path(
            &path,
            &background,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let mut dot = Paint::default();
    let (dot_rgb, text_rgb) = if enabled {
        ((0xff, 0x3b, 0x30), (0xf0, 0xf0, 0xf0))
    } else {
        ((0x70, 0x70, 0x70), (0xa0, 0xa0, 0xa0))
    };
    dot.set_color_rgba8(dot_rgb.0, dot_rgb.1, dot_rgb.2, 255);
    dot.anti_alias = true;
    if let Some(path) = circle_path(
        (x + AUDIO_PAD + AUDIO_DOT_R) as f32,
        (y + h / 2.0) as f32,
        AUDIO_DOT_R as f32,
    ) {
        pm.fill_path(&path, &dot, FillRule::Winding, Transform::identity(), None);
    }
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(AUDIO_PX));
    let text_h = scaled.ascent() - scaled.descent();
    let baseline = y as f32 + (h as f32 - text_h) / 2.0 + scaled.ascent();
    draw_text_aa(
        pm,
        (x + AUDIO_PAD + AUDIO_DOT_R * 2.0 + AUDIO_DOT_GAP) as f32,
        baseline,
        if enabled { "AUDIO ON" } else { "AUDIO OFF" },
        AUDIO_PX,
        text_rgb,
        false,
    );
}

#[allow(dead_code)]
pub fn draw_record_frame_checkbox(
    pm: &mut Pixmap,
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    checked: bool,
) {
    let Some((x, y, w, h)) = record_frame_checkbox_rect(sel, surf_w, surf_h) else {
        return;
    };
    let mut background = Paint::default();
    background.set_color_rgba8(0x12, 0x12, 0x12, 230);
    background.anti_alias = true;
    if let Some(path) = rounded_rect(x as f32, y as f32, w as f32, h as f32, 7.0) {
        pm.fill_path(
            &path,
            &background,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    let box_x = x + FRAME_PAD;
    let box_y = y + (h - FRAME_BOX_SIZE) / 2.0;
    let mut box_paint = Paint::default();
    box_paint.set_color_rgba8(0xd0, 0xd0, 0xd0, 255);
    box_paint.anti_alias = true;
    if let Some(path) = rounded_rect(
        box_x as f32,
        box_y as f32,
        FRAME_BOX_SIZE as f32,
        FRAME_BOX_SIZE as f32,
        3.0,
    ) {
        pm.stroke_path(
            &path,
            &box_paint,
            &Stroke {
                width: 1.5,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    if checked {
        let mut check = PathBuilder::new();
        check.move_to((box_x + 3.0) as f32, (box_y + 8.0) as f32);
        check.line_to((box_x + 6.5) as f32, (box_y + 11.5) as f32);
        check.line_to((box_x + 13.5) as f32, (box_y + 4.5) as f32);
        let Some(check) = check.finish() else { return };
        let mut check_paint = Paint::default();
        check_paint.set_color_rgba8(0xff, 0x3b, 0x30, 255);
        check_paint.anti_alias = true;
        pm.stroke_path(
            &check,
            &check_paint,
            &Stroke {
                width: 2.2,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(FRAME_PX));
    let text_h = scaled.ascent() - scaled.descent();
    let baseline = y as f32 + ((h as f32 - text_h) / 2.0) + scaled.ascent();
    draw_text_aa(
        pm,
        (box_x + FRAME_BOX_SIZE + FRAME_BOX_GAP) as f32,
        baseline,
        FRAME_LABEL,
        FRAME_PX,
        (0xf0, 0xf0, 0xf0),
        false,
    );
}

/// Draw the recording affordance: a small pill near the selection with a red
/// filled dot and the text "REC". Placed like the dimension badge (above-left,
/// flipping at edges) and reuses the badge font/text rendering. Red accent so it
/// reads as record. Used by the record-mode selector instead of the W×H badge.
/// The clickable hit-zone is `rec_pill_rect` (same box).
#[allow(dead_code)]
pub fn draw_rec_pill(pm: &mut Pixmap, sel: (f32, f32, f32, f32), surf_w: u32, surf_h: u32) {
    let Some((bx, by, bw, bh)) = rec_pill_rect(sel, surf_w, surf_h) else {
        return;
    };
    let label = "REC";
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(REC_PX));
    let mut pill = Paint::default();
    pill.set_color_rgba8(0x12, 0x12, 0x12, 230); // #121212, matching the W×H badge
    pill.anti_alias = true;
    if let Some(path) = rounded_rect(bx as f32, by as f32, bw as f32, bh as f32, 7.0) {
        pm.fill_path(&path, &pill, FillRule::Winding, Transform::identity(), None);
    }
    // Red record dot, vertically centered in the pill.
    let mut dot = Paint::default();
    dot.set_color_rgba8(0xff, 0x3b, 0x30, 255); // record red
    dot.anti_alias = true;
    let dot_cx = bx + REC_PAD + REC_DOT_R;
    let dot_cy = by + bh / 2.0;
    if let Some(path) = circle_path(dot_cx as f32, dot_cy as f32, REC_DOT_R as f32) {
        pm.fill_path(&path, &dot, FillRule::Winding, Transform::identity(), None);
    }
    let baseline = by as f32 + REC_PAD as f32 + scaled.ascent();
    draw_text_aa(
        pm,
        (bx + REC_PAD + REC_DOT_R * 2.0 + REC_DOT_GAP) as f32,
        baseline,
        label,
        REC_PX,
        (0xf0, 0xf0, 0xf0),
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

#[cfg(test)]
mod tests {
    #[test]
    fn toolbar_highlights_stay_inside_outer_contour() {
        for (width, height) in [(800, 500), (240, 200), (160, 240)] {
            for offset in [0.0, 0.25, 0.5, 0.75] {
                let toolbar =
                    record_toolbar((20.0 + offset, 150.0 + offset, 80.0, 40.0), width, height)
                        .unwrap();
                let mut outline = Pixmap::new(width, height).unwrap();
                fill_rounded(&mut outline, toolbar.bounds, TB_CARD_RADIUS, UI_SURFACE);
                for hovered in 0..4 {
                    let mut rendered = Pixmap::new(width, height).unwrap();
                    draw_record_toolbar(&mut rendered, &toolbar, (true, true, true), Some(hovered));
                    for (base, drawn) in outline.pixels().iter().zip(rendered.pixels()) {
                        assert_eq!(
                            drawn.alpha(),
                            base.alpha(),
                            "highlight changes the toolbar's antialiased contour"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn record_toolbar_keeps_controls_together_at_edges() {
        for (width, height) in [(1920, 1080), (400, 300), (240, 200), (160, 240)] {
            for sel in [
                (0.0, 0.0, width as f32, height as f32),
                (width as f32 - 30.0, height as f32 - 30.0, 20.0, 20.0),
                (20.0, 70.0, 80.0, 40.0),
            ] {
                let toolbar = super::record_toolbar(sel, width, height).unwrap();
                let (x, y, w, h) = toolbar.bounds;
                assert!(
                    x >= 8.0
                        && y >= 8.0
                        && x + w <= width as f64 - 8.0
                        && y + h <= height as f64 - 8.0
                );
                for (i, &(bx, by, bw, bh)) in toolbar.controls.iter().enumerate() {
                    assert!(bx >= x && by >= y && bx + bw <= x + w && by + bh <= y + h);
                    for &(cx, cy, cw, ch) in &toolbar.controls[i + 1..] {
                        assert!(bx + bw <= cx || cx + cw <= bx || by + bh <= cy || cy + ch <= by);
                    }
                }
            }
        }
        let toolbar = super::record_toolbar((90.0, 90.0, 400.0, 200.0), 800, 500).unwrap();
        assert_eq!(toolbar.bounds.0, 90.0);
        assert!(toolbar.bounds.1 + toolbar.bounds.3 < 90.0);
    }

    use super::*;

    #[test]
    fn rect_to_image_forward_and_reversed_match() {
        // 1:1 surface->image. Drag (10,20)->(110,220) and the reverse must
        // produce the same crop rect.
        let fwd = rect_to_image((10.0, 20.0), (110.0, 220.0), 200, 400, 200, 400);
        let rev = rect_to_image((110.0, 220.0), (10.0, 20.0), 200, 400, 200, 400);
        assert_eq!(fwd, Some((10, 20, 100, 200)));
        assert_eq!(fwd, rev);
    }

    #[test]
    fn rect_to_image_clamps_out_of_bounds() {
        let r = rect_to_image((-50.0, -50.0), (10_000.0, 10_000.0), 200, 400, 200, 400);
        assert_eq!(r, Some((0, 0, 200, 400)));
    }

    #[test]
    fn rect_to_image_rejects_subpixel() {
        assert_eq!(
            rect_to_image((10.0, 10.0), (10.5, 10.5), 200, 400, 200, 400),
            None
        );
        assert_eq!(
            rect_to_image((10.0, 10.0), (10.0, 10.0), 200, 400, 200, 400),
            None
        );
    }

    #[test]
    fn pixmap_to_argb8888_swaps_r_and_b_keeps_premult() {
        use tiny_skia::Pixmap;
        let mut pm = Pixmap::new(1, 1).unwrap();
        // premultiplied RGBA sample with alpha < 255 and channels <= alpha.
        pm.data_mut().copy_from_slice(&[10, 20, 30, 200]); // R,G,B,A
        let mut canvas = [0u8; 4];
        pixmap_to_argb8888(&pm, &mut canvas);
        assert_eq!(canvas, [30, 20, 10, 200]); // B,G,R,A
        // premultiplied invariant survives: no colour channel exceeds alpha.
        assert!(canvas[0] <= canvas[3] && canvas[1] <= canvas[3] && canvas[2] <= canvas[3]);
    }

    #[test]
    fn base_pixmap_copies_opaque_image_1to1() {
        use image::{Rgba, RgbaImage};
        let mut img = RgbaImage::new(2, 2);
        for p in img.pixels_mut() {
            *p = Rgba([10, 20, 30, 255]);
        }
        let pm = base_pixmap_from_image(&img, 2, 2);
        assert_eq!(pm.width(), 2);
        assert_eq!(&pm.data()[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn render_overlay_dims_outside_keeps_inside_bright() {
        use tiny_skia::Pixmap;
        // Opaque red base.
        let mut pm = Pixmap::new(200, 100).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        dim_and_restore(&mut pm, Some((40.0, 30.0, 60.0, 20.0)));
        let at = |x: u32, y: u32| {
            let i = ((y * pm.width() + x) * 4) as usize;
            pm.data()[i] // red channel
        };
        // Inside the selection: still bright red.
        assert!(
            at(60, 38) > 250,
            "inside should stay bright, got {}",
            at(60, 38)
        );
        // Outside: dimmed by the ~43% black overlay.
        assert!(at(0, 0) < 200, "outside should be dimmed, got {}", at(0, 0));
    }

    #[test]
    fn cached_overlay_matches_original_across_moves_and_transparency() {
        for alpha in [0, 128, 255] {
            let mut base = Pixmap::new(80, 60).unwrap();
            base.fill(tiny_skia::Color::from_rgba8(170, 90, 35, alpha));
            let mut cache = CachedOverlay::new(&base);
            for selection in [
                None,
                Some((12.5, 9.5, 35.0, 22.0)),
                Some((-5.5, -3.0, 20.0, 90.0)),
                Some((70.0, 50.0, 40.0, 20.0)),
                Some((2.0, 2.0, 0.5, 10.0)),
                None,
            ] {
                let mut expected = base.clone();
                dim_and_restore(&mut expected, selection);
                let actual = cache.reset(&base, selection);
                assert_eq!(
                    actual.data(),
                    expected.data(),
                    "alpha={alpha}, selection={selection:?}"
                );
                // Simulate controls drawn after the background pass. The next
                // reset must remove their pixels even when the selection moves.
                actual.data_mut()[0..4].fill(255);
            }
        }
    }

    #[test]
    fn windows_record_overlay_dims_selection_less_than_overview() {
        let mut default = Pixmap::new(100, 100).unwrap();
        let mut windows = Pixmap::new(100, 100).unwrap();
        for pixmap in [&mut default, &mut windows] {
            for pixel in pixmap.data_mut().chunks_exact_mut(4) {
                pixel.copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        let selection = Some((20.0, 20.0, 60.0, 60.0));
        dim_and_restore(&mut default, selection);
        dim_and_restore_with_style(&mut windows, selection, WINDOWS_RECORD_OVERLAY_STYLE);

        let pixel = |pixmap: &Pixmap, x: u32, y: u32| {
            let offset = ((y * pixmap.width() + x) * 4) as usize;
            <[u8; 4]>::try_from(&pixmap.data()[offset..offset + 4]).unwrap()
        };
        assert_eq!(pixel(&default, 0, 0), [145, 145, 145, 255]);
        assert_eq!(pixel(&windows, 0, 0), [145, 145, 145, 255]);
        assert_eq!(pixel(&windows, 50, 50), [231, 231, 231, 255]);
    }

    #[test]
    fn windows_record_border_is_single_and_handles_use_neutral_accent() {
        let mut pm = Pixmap::new(120, 120).unwrap();
        for pixel in pm.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[64, 64, 64, 255]);
        }
        let selection = (30.0, 30.0, 60.0, 60.0);
        draw_border_with_style(&mut pm, selection, WINDOWS_RECORD_OVERLAY_STYLE);

        let pixel = |pixmap: &Pixmap, x: u32, y: u32| {
            let offset = ((y * pixmap.width() + x) * 4) as usize;
            <[u8; 4]>::try_from(&pixmap.data()[offset..offset + 4]).unwrap()
        };
        assert_eq!(pixel(&pm, 28, 60), [64, 64, 64, 255]);
        assert!(pixel(&pm, 30, 60)[0] > 200);

        draw_handles_with_style(&mut pm, selection, WINDOWS_RECORD_OVERLAY_STYLE);
        assert_eq!(pixel(&pm, 30, 30), [255, 255, 255, 255]);
    }

    #[test]
    fn windows_screenshot_style_matches_linux_rendering_byte_for_byte() {
        let mut linux = Pixmap::new(120, 100).unwrap();
        for (index, pixel) in linux.data_mut().chunks_exact_mut(4).enumerate() {
            let value = (index % 251) as u8;
            pixel.copy_from_slice(&[value, 255 - value, value / 2, 255]);
        }
        let mut windows = linux.clone();
        let selection = (24.0, 18.0, 72.0, 60.0);

        dim_and_restore(&mut linux, Some(selection));
        draw_border(&mut linux, selection);
        draw_handles(&mut linux, selection);

        dim_and_restore_with_style(&mut windows, Some(selection), SCREENSHOT_OVERLAY_STYLE);
        draw_border_with_style(&mut windows, selection, SCREENSHOT_OVERLAY_STYLE);
        draw_handles_with_style(&mut windows, selection, SCREENSHOT_OVERLAY_STYLE);

        assert_eq!(windows.data(), linux.data());
    }

    #[test]
    fn default_border_remains_white() {
        let mut pm = Pixmap::new(120, 120).unwrap();
        for pixel in pm.data_mut().chunks_exact_mut(4) {
            pixel.copy_from_slice(&[0, 0, 0, 255]);
        }
        draw_border(&mut pm, (30.0, 30.0, 60.0, 60.0));

        let offset = ((30 * pm.width() + 60) * 4) as usize;
        let pixel = &pm.data()[offset..offset + 4];
        assert!(pixel[0] > 180);
        assert_eq!(pixel[0], pixel[1]);
        assert_eq!(pixel[1], pixel[2]);
        assert_eq!(pixel[3], 255);
    }

    #[test]
    fn render_overlay_none_dims_everything() {
        use tiny_skia::Pixmap;
        let mut pm = Pixmap::new(20, 20).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        dim_and_restore(&mut pm, None);
        let i = ((10 * pm.width() + 10) * 4) as usize;
        assert!(pm.data()[i] < 200, "whole surface should be dimmed");
    }

    #[test]
    fn render_overlay_dim_has_no_undimmed_row_seam() {
        use tiny_skia::Pixmap;
        let mut pm = Pixmap::new(200, 100).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        // Fractional *.5 edges are the worst case for non-AA rect tiling: the
        // boundary pixel row gets ~0.5 coverage from each neighbouring dim rect
        // and can end up undimmed. x=0 is far left of the selection (sx=40.5),
        // so EVERY row there must be fully dimmed (red ~145). A seam shows up as
        // a bright row.
        dim_and_restore(&mut pm, Some((40.5, 30.5, 60.0, 20.0)));
        for y in 0..100u32 {
            let r = pm.data()[((y * 200) * 4) as usize];
            assert!(r < 160, "left column row {y} not fully dimmed: r={r}");
        }
    }

    #[test]
    fn draw_rec_pill_paints_red_dot_and_pill() {
        use tiny_skia::Pixmap;
        let mut pm = Pixmap::new(400, 300).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 0, 255]); // opaque black backdrop
        }
        // Selection well clear of the top so the pill sits above it on-screen.
        let sel = (120.0, 150.0, 160.0, 90.0);
        draw_rec_pill(&mut pm, sel, 400, 300);
        // Somewhere in the pill band above the selection there must be a strongly
        // red pixel (the record dot). Scan the rows just above the selection top.
        let at = |x: u32, y: u32| {
            let i = ((y * pm.width() + x) * 4) as usize;
            (pm.data()[i], pm.data()[i + 1], pm.data()[i + 2])
        };
        let mut found_red = false;
        for y in 110..150u32 {
            for x in 120..280u32 {
                let (r, g, b) = at(x, y);
                if r > 180 && g < 100 && b < 100 {
                    found_red = true;
                }
            }
        }
        assert!(found_red, "REC pill should paint a red record dot");
    }

    #[test]
    fn rec_pill_rect_is_above_selection_and_hittable() {
        // Selection well clear of the top edge → pill sits above it.
        let sel = (120.0, 150.0, 160.0, 90.0);
        let (bx, by, bw, bh) =
            rec_pill_rect(sel, 400, 300).expect("pill rect for a real selection");
        assert!(bw > 0.0 && bh > 0.0, "pill has positive size");
        // Placed above the selection top (badge_rect places above-left here).
        assert!(by + bh <= sel.1 as f64, "pill sits above the selection");
        // Its own centre is inside the rect (so a click there hits the button).
        let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
        assert!(cx >= bx && cx < bx + bw && cy >= by && cy < by + bh);
        // Sub-pixel selection → no pill.
        assert!(rec_pill_rect((10.0, 10.0, 0.0, 0.0), 400, 300).is_none());
    }

    #[test]
    fn frame_checkbox_sits_beside_rec_without_overlap() {
        let sel = (100.0, 100.0, 800.0, 500.0);
        let rec = rec_pill_rect(sel, 1920, 1080).unwrap();
        let check = record_frame_checkbox_rect(sel, 1920, 1080).unwrap();
        assert!(check.0 >= rec.0 + rec.2);
        assert!(
            check.2 > check.3 * 3.0,
            "control reserves room for its label"
        );
        assert!(check.0 + check.2 <= 1920.0);
        assert!(check.1 >= 0.0 && check.1 + check.3 <= 1080.0);
    }

    #[test]
    fn frame_checkbox_stays_on_screen_at_selection_edges() {
        for sel in [
            (0.0, 0.0, 120.0, 80.0),
            (280.0, 0.0, 120.0, 80.0),
            (0.0, 220.0, 120.0, 80.0),
            (280.0, 220.0, 120.0, 80.0),
        ] {
            let rec = rec_pill_rect(sel, 400, 300).unwrap();
            let check = record_frame_checkbox_rect(sel, 400, 300).unwrap();
            assert!(check.0 >= 0.0 && check.0 + check.2 <= 400.0);
            assert!(check.1 >= 0.0 && check.1 + check.3 <= 300.0);
            assert!(
                check.0 + check.2 <= rec.0
                    || rec.0 + rec.2 <= check.0
                    || check.1 + check.3 <= rec.1
                    || rec.1 + rec.3 <= check.1
            );
        }
    }

    #[test]
    fn checked_frame_control_draws_a_checkmark_instead_of_a_solid_fill() {
        use tiny_skia::Pixmap;
        let sel = (100.0, 100.0, 200.0, 100.0);
        let rect = record_frame_checkbox_rect(sel, 400, 300).unwrap();
        let mut checked = Pixmap::new(400, 300).unwrap();
        draw_record_frame_checkbox(&mut checked, sel, 400, 300, true);
        let mut red = 0;
        for y in rect.1 as u32..(rect.1 + rect.3) as u32 {
            for x in rect.0 as u32..(rect.0 + rect.3) as u32 {
                let i = ((y * checked.width() + x) * 4) as usize;
                let pixel = &checked.data()[i..i + 4];
                red += usize::from(pixel[0] > 180 && pixel[1] < 100 && pixel[2] < 100);
            }
        }
        assert!(red > 0, "checked control paints a red checkmark");
        assert!(red < 100, "checked control must not use a solid red fill");
    }

    #[test]
    fn frame_control_draws_its_explanatory_label() {
        use tiny_skia::Pixmap;
        let sel = (100.0, 100.0, 200.0, 100.0);
        let rect = record_frame_checkbox_rect(sel, 400, 300).unwrap();
        let mut pm = Pixmap::new(400, 300).unwrap();
        draw_record_frame_checkbox(&mut pm, sel, 400, 300, false);
        let mut found_text = false;
        for y in rect.1 as u32..(rect.1 + rect.3) as u32 {
            for x in (rect.0 + rect.3) as u32..(rect.0 + rect.2) as u32 {
                let i = ((y * pm.width() + x) * 4) as usize;
                let pixel = &pm.data()[i..i + 4];
                found_text |= pixel[0] > 100 && pixel[1] > 100 && pixel[2] > 100;
            }
        }
        assert!(found_text, "control paints a readable label");
    }

    #[test]
    fn record_audio_button_stays_visible_and_clear_of_other_controls() {
        let overlaps = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
            a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
        };
        for sel in [
            (8.0, 8.0, 160.0, 100.0),
            (232.0, 8.0, 160.0, 100.0),
            (8.0, 192.0, 160.0, 100.0),
            (232.0, 192.0, 160.0, 100.0),
        ] {
            let rec = rec_pill_rect(sel, 400, 300).unwrap();
            let frame = record_frame_checkbox_rect(sel, 400, 300).unwrap();
            let audio = record_audio_button_rect(sel, 400, 300).unwrap();
            assert!(!overlaps(audio, rec));
            assert!(!overlaps(audio, frame));
            assert!(audio.0 >= 0.0 && audio.1 >= 0.0);
            assert!(audio.0 + audio.2 <= 400.0 && audio.1 + audio.3 <= 300.0);
        }
    }

    #[test]
    fn record_audio_button_has_distinct_on_and_off_rendering() {
        let (w, h) = (400, 300);
        let sel = (80.0, 80.0, 200.0, 120.0);
        let mut on = Pixmap::new(w, h).unwrap();
        let mut off = Pixmap::new(w, h).unwrap();
        draw_record_audio_button(&mut on, sel, w, h, true);
        draw_record_audio_button(&mut off, sel, w, h, false);
        assert_ne!(on.data(), off.data());
        assert!(on.data().iter().any(|byte| *byte != 0));
        assert!(off.data().iter().any(|byte| *byte != 0));
    }

    #[test]
    fn draw_handles_marks_corners_white() {
        use tiny_skia::Pixmap;
        let mut pm = Pixmap::new(200, 200).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 0, 255]);
        }
        draw_handles(&mut pm, (50.0, 50.0, 100.0, 100.0));
        // A pixel at the top-left corner handle center should be white.
        let i = ((50 * pm.width() + 50) * 4) as usize;
        assert!(
            pm.data()[i] > 200 && pm.data()[i + 1] > 200,
            "corner handle should be white"
        );
    }
}
