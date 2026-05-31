//! Pure rendering + geometry for the tiny-skia region selector. No Wayland here.

#![allow(dead_code)] // filled in by later tasks

use image::{RgbaImage, imageops};
use tiny_skia::{Paint, PathBuilder, Pixmap, Rect, Stroke, Transform};

use super::font;

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

/// The four surface-space rectangles (top, bottom, left, right) that together
/// cover the whole `(surf_w, surf_h)` surface minus the selection `(sx,sy,sw,sh)`.
/// No overlaps, no gaps. Negative extents are clamped to 0. Matches the egui
/// selector's four `outside_*` rects.
pub fn outside_rects(
    sx: f32,
    sy: f32,
    sw: f32,
    sh: f32,
    surf_w: f32,
    surf_h: f32,
) -> [(f32, f32, f32, f32); 4] {
    let nn = |v: f32| v.max(0.0);
    let top = (0.0, 0.0, surf_w, nn(sy));
    let bottom = (0.0, sy + sh, surf_w, nn(surf_h - (sy + sh)));
    let left = (0.0, sy, nn(sx), sh);
    let right = (sx + sw, sy, nn(surf_w - (sx + sw)), sh);
    [top, bottom, left, right]
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

/// Convert a premultiplied-RGBA `Pixmap` to a premultiplied-BGRA `wl_shm`
/// Argb8888 (little-endian) buffer: swap R and B, keep A. `canvas` must be the
/// same pixel count as the pixmap (4 bytes per pixel).
pub fn pixmap_to_argb8888(pm: &Pixmap, canvas: &mut [u8]) {
    for (src, dst) in pm.data().chunks_exact(4).zip(canvas.chunks_exact_mut(4)) {
        dst[0] = src[2]; // B
        dst[1] = src[1]; // G
        dst[2] = src[0]; // R
        dst[3] = src[3]; // A
    }
}

/// Dim alpha over the screenshot outside the selection (0-255). Matches the
/// egui selector's `from_rgba_unmultiplied(0, 0, 0, 110)`.
const DIM_ALPHA: u8 = 110;
/// Selection border width, px. Matches the egui selector.
const BORDER_W: f32 = 1.5;
/// Label render scale for the 5x7 font (≈14px tall, close to egui's 12px).
const LABEL_SCALE: u32 = 2;

/// Draw the selection overlay onto `pm` (which already contains the opaque
/// screenshot). `sel` is the surface-space selection `(x, y, w, h)`; `None`
/// means "no selection yet" — dim the whole surface.
pub fn render_overlay(pm: &mut Pixmap, sel: Option<(f32, f32, f32, f32)>) {
    let (w, h) = (pm.width() as f32, pm.height() as f32);
    let mut dim = Paint::default();
    dim.set_color_rgba8(0, 0, 0, DIM_ALPHA);
    dim.anti_alias = false;

    match sel {
        None => {
            if let Some(r) = Rect::from_xywh(0.0, 0.0, w, h) {
                pm.fill_rect(r, &dim, Transform::identity(), None);
            }
        }
        Some((sx, sy, sw, sh)) => {
            for (rx, ry, rw, rh) in outside_rects(sx, sy, sw, sh, w, h) {
                if rw > 0.0 && rh > 0.0 {
                    if let Some(r) = Rect::from_xywh(rx, ry, rw, rh) {
                        pm.fill_rect(r, &dim, Transform::identity(), None);
                    }
                }
            }
            let mut white = Paint::default();
            white.set_color_rgba8(255, 255, 255, 255);
            white.anti_alias = true;
            if let Some(r) = Rect::from_xywh(sx, sy, sw, sh) {
                let mut pb = PathBuilder::new();
                pb.push_rect(r);
                if let Some(path) = pb.finish() {
                    let stroke = Stroke {
                        width: BORDER_W,
                        ..Default::default()
                    };
                    pm.stroke_path(&path, &white, &stroke, Transform::identity(), None);
                }
            }
            // Label just above the selection's top-left, like the egui one.
            let label = format!("{}×{}", sw.round() as i32, sh.round() as i32);
            let ly = (sy as i32) - 4 - font::glyph_h_px(LABEL_SCALE) as i32;
            draw_label(pm, &label, sx as i32 + 6, ly.max(0), LABEL_SCALE);
        }
    }
}

/// Blit `text` in opaque white at (`x`, `y`) (top-left, surface pixels) using
/// the bitmap font, scaled by `scale`. Writes directly into the pixmap's
/// premultiplied RGBA bytes (white is unaffected by premultiplication).
pub fn draw_label(pm: &mut Pixmap, text: &str, x: i32, y: i32, scale: u32) {
    let w = pm.width() as i32;
    let h = pm.height() as i32;
    let data = pm.data_mut();
    let mut cx = x;
    let s = scale as i32;
    for ch in text.chars() {
        if let Some(rows) = font::glyph(ch) {
            for (ry, bits) in rows.iter().enumerate() {
                for bx in 0..font::GLYPH_W {
                    let on = bits & (1 << (font::GLYPH_W - 1 - bx)) != 0;
                    if !on {
                        continue;
                    }
                    for dy in 0..s {
                        for dx in 0..s {
                            let px = cx + bx as i32 * s + dx;
                            let py = y + ry as i32 * s + dy;
                            if px >= 0 && px < w && py >= 0 && py < h {
                                let i = ((py * w + px) * 4) as usize;
                                data[i] = 255;
                                data[i + 1] = 255;
                                data[i + 2] = 255;
                                data[i + 3] = 255;
                            }
                        }
                    }
                }
            }
        }
        cx += font::advance_px(scale) as i32;
    }
}

#[cfg(test)]
mod tests {
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
        assert_eq!(rect_to_image((10.0, 10.0), (10.5, 10.5), 200, 400, 200, 400), None);
        assert_eq!(rect_to_image((10.0, 10.0), (10.0, 10.0), 200, 400, 200, 400), None);
    }

    #[test]
    fn outside_rects_cover_complement_without_overlap() {
        let (sw, sh) = (200.0_f32, 100.0_f32);
        let sel = (40.0_f32, 30.0_f32, 60.0_f32, 20.0_f32); // x,y,w,h
        let rects = outside_rects(sel.0, sel.1, sel.2, sel.3, sw, sh);
        let outside_area: f32 = rects.iter().map(|(_, _, w, h)| w * h).sum();
        let total = sw * sh;
        let selection = sel.2 * sel.3;
        assert!((outside_area - (total - selection)).abs() < 0.01, "got {outside_area}");
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
        render_overlay(&mut pm, Some((40.0, 30.0, 60.0, 20.0)));
        let at = |x: u32, y: u32| {
            let i = ((y * pm.width() + x) * 4) as usize;
            pm.data()[i] // red channel
        };
        // Inside the selection: still bright red.
        assert!(at(60, 38) > 250, "inside should stay bright, got {}", at(60, 38));
        // Outside: dimmed by the ~43% black overlay.
        assert!(at(0, 0) < 200, "outside should be dimmed, got {}", at(0, 0));
    }

    #[test]
    fn render_overlay_none_dims_everything() {
        use tiny_skia::Pixmap;
        let mut pm = Pixmap::new(20, 20).unwrap();
        for px in pm.data_mut().chunks_exact_mut(4) {
            px.copy_from_slice(&[255, 0, 0, 255]);
        }
        render_overlay(&mut pm, None);
        let i = ((10 * pm.width() + 10) * 4) as usize;
        assert!(pm.data()[i] < 200, "whole surface should be dimmed");
    }
}
