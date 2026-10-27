//! Pure rendering + geometry for the tiny-skia region selector. No Wayland here.

use boltsnap::ui::Canvas;
use image::{RgbaImage, imageops};
use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

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
    /// Drawn on at scale 1: the selector's buffer is in logical pixels.
    frame: Canvas,
}

impl CachedOverlay {
    pub fn new(base: &Pixmap) -> Self {
        let mut dimmed = base.clone();
        dim_and_restore_with_style(&mut dimmed, None, SCREENSHOT_OVERLAY_STYLE);
        Self {
            frame: Canvas {
                pixmap: dimmed.clone(),
                scale: 1.0,
            },
            dimmed,
        }
    }

    pub fn reset_regions(
        &mut self,
        base: &Pixmap,
        sel: Option<(f32, f32, f32, f32)>,
        regions: &[(u32, u32, u32, u32)],
    ) -> &mut Canvas {
        let selected = selection_bounds(sel, base.width(), base.height());
        let stride = base.width() as usize * 4;
        for &(x0, y0, x1, y1) in regions {
            for y in y0..y1 {
                let start = y as usize * stride + x0 as usize * 4;
                let end = y as usize * stride + x1 as usize * 4;
                self.frame.pixmap.data_mut()[start..end]
                    .copy_from_slice(&self.dimmed.data()[start..end]);
                if let Some((sx0, sy0, sx1, sy1)) = selected {
                    let (left, right) = (x0.max(sx0), x1.min(sx1));
                    if y >= sy0 && y < sy1 && left < right {
                        let start = y as usize * stride + left as usize * 4;
                        let end = y as usize * stride + right as usize * 4;
                        self.frame.pixmap.data_mut()[start..end]
                            .copy_from_slice(&base.data()[start..end]);
                    }
                }
            }
        }
        &mut self.frame
    }

    pub fn frame(&self) -> &Pixmap {
        &self.frame.pixmap
    }

    #[allow(dead_code)] // Full-frame reference and conservative fallback.
    pub fn reset(&mut self, base: &Pixmap, sel: Option<(f32, f32, f32, f32)>) -> &mut Canvas {
        assert_eq!(
            (base.width(), base.height()),
            (self.dimmed.width(), self.dimmed.height())
        );
        self.frame
            .pixmap
            .data_mut()
            .copy_from_slice(self.dimmed.data());
        if let Some((x0, y0, x1, y1)) = selection_bounds(sel, base.width(), base.height()) {
            let stride = base.width() as usize * 4;
            let span = (x1 - x0) as usize * 4;
            for y in y0..y1 {
                let start = y as usize * stride + x0 as usize * 4;
                self.frame.pixmap.data_mut()[start..start + span]
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

/// The Windows screenshot selector's appearance, and the dim both platforms
/// lay over the screen. Linux draws its border and grips in `chrome.rs`.
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

pub mod chrome;
pub mod hint;
mod legacy;
pub mod toolbar;

pub use chrome::{Chrome, draw_frame, draw_grips, draw_size_badge, size_badge_rect};

// Only the Windows backend and the tests draw these.
#[cfg_attr(not(target_os = "windows"), allow(unused_imports))]
pub use legacy::*;

#[cfg(test)]
mod chrome_tests;
#[cfg(test)]
pub mod gates;
#[cfg(test)]
mod tests;
