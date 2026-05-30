use image::RgbaImage;

use crate::shelf::layout::{Layout, LayoutConfig, ThumbRect};
use crate::shelf::model::ShelfModel;

/// Thumbnail card corner radius and white border width, in pixels.
const CARD_RADIUS: f32 = 8.0;
const CARD_BORDER: f32 = 1.0;

/// Hover-button styling: a small translucent dark circle with an anti-aliased
/// glyph. Minimal and unobtrusive over the screenshot.
const BTN_BG: (u8, u8, u8) = (18, 18, 24);
const BTN_BG_A: f32 = 0.52;
const GLYPH_RGB: (u8, u8, u8) = (244, 244, 248);
const GLYPH_CLOSE_RGB: (u8, u8, u8) = (255, 124, 124);
const GLYPH_HALF_W: f32 = 0.8; // half stroke width -> ~1.6px strokes

/// Set the whole canvas to transparent (0,0,0,0).
pub fn clear(canvas: &mut [u8]) {
    for b in canvas.iter_mut() {
        *b = 0;
    }
}

/// Composite an opaque RGBA thumbnail as a rounded "card": rounded corners
/// (transparent outside the radius) plus a 1px white border, both anti-aliased.
/// The tile occupies exactly `img.dimensions()` at (dx, dy). Screenshots are
/// opaque, so only the rounded-rect coverage of each pixel varies.
pub fn blit_thumb_card(canvas: &mut [u8], cw: u32, ch: u32, img: &RgbaImage, dx: u32, dy: u32) {
    let (iw, ih) = img.dimensions();
    let w = iw as f32;
    let h = ih as f32;
    let r = CARD_RADIUS.min(w / 2.0).min(h / 2.0);
    let b = CARD_BORDER;
    for sy in 0..ih {
        let py = dy + sy;
        if py >= ch {
            break;
        }
        for sx in 0..iw {
            let px = dx + sx;
            if px >= cw {
                break;
            }
            let fx = sx as f32 + 0.5;
            let fy = sy as f32 + 0.5;
            let outer = rr_coverage(fx, fy, w, h, r);
            if outer <= 0.0 {
                continue; // transparent corner -> leave the canvas untouched
            }
            let inner = rr_coverage(fx - b, fy - b, w - 2.0 * b, h - 2.0 * b, (r - b).max(0.0));
            let fill = inner.clamp(0.0, 1.0);
            let border = (outer - inner).clamp(0.0, 1.0);
            let p = img.get_pixel(sx, sy).0;
            // Coverage-weighted (already premultiplied) BGRA: the thumbnail in
            // the fill region, white in the 1px border ring, alpha = outer.
            let rr = p[0] as f32 * fill + 255.0 * border;
            let gg = p[1] as f32 * fill + 255.0 * border;
            let bb = p[2] as f32 * fill + 255.0 * border;
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = bb.round().clamp(0.0, 255.0) as u8;
            canvas[idx + 1] = gg.round().clamp(0.0, 255.0) as u8;
            canvas[idx + 2] = rr.round().clamp(0.0, 255.0) as u8;
            canvas[idx + 3] = (outer * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Coverage of a rounded rectangle (size w×h, corner radius r) at point (px,py):
/// ~1.0 well inside, ~0.0 well outside, anti-aliased across ~1px at the edge.
/// Uses the standard rounded-box signed distance field.
fn rr_coverage(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = w / 2.0;
    let cy = h / 2.0;
    let dx = (px - cx).abs() - (cx - r);
    let dy = (py - cy).abs() - (cy - r);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let inside = dx.max(dy).min(0.0);
    let sdf = outside + inside - r;
    (0.5 - sdf).clamp(0.0, 1.0)
}

/// Source-over composite a straight-alpha colour onto the premultiplied BGRA
/// canvas at integer (x,y). `a` is fractional coverage in 0..=1.
fn blend_px(canvas: &mut [u8], cw: u32, ch: u32, x: i32, y: i32, r: u8, g: u8, b: u8, a: f32) {
    if x < 0 || y < 0 || x as u32 >= cw || y as u32 >= ch {
        return;
    }
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let idx = ((y as u32 * cw + x as u32) * 4) as usize;
    let inv = 1.0 - a;
    let nb = b as f32 * a + canvas[idx] as f32 * inv;
    let ng = g as f32 * a + canvas[idx + 1] as f32 * inv;
    let nr = r as f32 * a + canvas[idx + 2] as f32 * inv;
    let na = a * 255.0 + canvas[idx + 3] as f32 * inv;
    canvas[idx] = nb.round().clamp(0.0, 255.0) as u8;
    canvas[idx + 1] = ng.round().clamp(0.0, 255.0) as u8;
    canvas[idx + 2] = nr.round().clamp(0.0, 255.0) as u8;
    canvas[idx + 3] = na.round().clamp(0.0, 255.0) as u8;
}

/// Anti-aliased filled circle.
fn fill_circle(canvas: &mut [u8], cw: u32, ch: u32, cx: f32, cy: f32, radius: f32, c: (u8, u8, u8), a: f32) {
    let x0 = (cx - radius - 1.0).floor() as i32;
    let x1 = (cx + radius + 1.0).ceil() as i32;
    let y0 = (cy - radius - 1.0).floor() as i32;
    let y1 = (cy + radius + 1.0).ceil() as i32;
    for py in y0..=y1 {
        for px in x0..=x1 {
            let d = (((px as f32 + 0.5) - cx).powi(2) + ((py as f32 + 0.5) - cy).powi(2)).sqrt();
            let cov = (radius + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, c.0, c.1, c.2, a * cov);
            }
        }
    }
}

/// Anti-aliased line segment with round caps, half-width `hw`.
fn stroke_line(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    hw: f32,
    c: (u8, u8, u8),
    a: f32,
) {
    let minx = (x0.min(x1) - hw - 1.0).floor() as i32;
    let maxx = (x0.max(x1) + hw + 1.0).ceil() as i32;
    let miny = (y0.min(y1) - hw - 1.0).floor() as i32;
    let maxy = (y0.max(y1) + hw + 1.0).ceil() as i32;
    let bax = x1 - x0;
    let bay = y1 - y0;
    let blen2 = (bax * bax + bay * bay).max(1e-6);
    for py in miny..=maxy {
        for px in minx..=maxx {
            let pax = (px as f32 + 0.5) - x0;
            let pay = (py as f32 + 0.5) - y0;
            let t = ((pax * bax + pay * bay) / blen2).clamp(0.0, 1.0);
            let dx = pax - bax * t;
            let dy = pay - bay * t;
            let dist = (dx * dx + dy * dy).sqrt();
            let cov = (hw + 0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, c.0, c.1, c.2, a * cov);
            }
        }
    }
}

/// Anti-aliased square outline (four strokes) for the copy glyph.
fn stroke_rect(canvas: &mut [u8], cw: u32, ch: u32, x: f32, y: f32, w: f32, h: f32, hw: f32, c: (u8, u8, u8), a: f32) {
    stroke_line(canvas, cw, ch, x, y, x + w, y, hw, c, a);
    stroke_line(canvas, cw, ch, x, y + h, x + w, y + h, hw, c, a);
    stroke_line(canvas, cw, ch, x, y, x, y + h, hw, c, a);
    stroke_line(canvas, cw, ch, x + w, y, x + w, y + h, hw, c, a);
}

/// Render the whole shelf: each thumbnail, plus hover icons on the hovered thumb.
pub fn draw_shelf(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    layout: &Layout,
    model: &ShelfModel,
    hovered: Option<u64>,
    cfg: &LayoutConfig,
) {
    clear(canvas);
    for r in &layout.thumbs {
        if let Some(thumb) = model.get(r.id) {
            blit_thumb_card(canvas, cw, ch, &thumb.thumb, r.x, r.y);
        }
        if hovered == Some(r.id) {
            draw_hover_icons(canvas, cw, ch, r, cfg);
        }
    }
}

fn draw_hover_icons(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, cfg: &LayoutConfig) {
    // slot 0 close (rightmost), 1 copy, 2 edit — cell math mirrors Layout::icon_rect
    // so the visible buttons line up with the hit zones.
    for slot in 0..3u32 {
        let right = (r.x + r.w).saturating_sub(cfg.pad_icon);
        let cellx = right
            .saturating_sub((slot + 1) * cfg.icon)
            .saturating_sub(slot * cfg.icon_gap);
        let celly = r.y + cfg.pad_icon;
        let s = cfg.icon as f32;
        let cx = cellx as f32 + s / 2.0;
        let cy = celly as f32 + s / 2.0;
        // translucent circular button
        fill_circle(canvas, cw, ch, cx, cy, s / 2.0 - 0.5, BTN_BG, BTN_BG_A);
        let glyph_c = if slot == 0 { GLYPH_CLOSE_RGB } else { GLYPH_RGB };
        draw_glyph(canvas, cw, ch, slot, cellx as f32, celly as f32, s, glyph_c);
    }
}

/// Minimal anti-aliased glyphs centred in a cell at (x,y) of size s.
/// 0 = close (X), 1 = copy (two offset squares), 2 = edit (pencil).
fn draw_glyph(canvas: &mut [u8], cw: u32, ch: u32, slot: u32, x: f32, y: f32, s: f32, c: (u8, u8, u8)) {
    let hw = GLYPH_HALF_W;
    let inset = s * 0.30;
    let lo = inset;
    let hi = s - inset;
    match slot {
        0 => {
            // X
            stroke_line(canvas, cw, ch, x + lo, y + lo, x + hi, y + hi, hw, c, 1.0);
            stroke_line(canvas, cw, ch, x + hi, y + lo, x + lo, y + hi, hw, c, 1.0);
        }
        1 => {
            // copy: two offset rounded-ish squares (outlines)
            let side = (hi - lo) * 0.74;
            let off = (hi - lo) * 0.26;
            // back square (upper-right)
            stroke_rect(canvas, cw, ch, x + lo + off, y + lo, side, side, hw, c, 1.0);
            // front square (lower-left), drawn after so it reads on top
            stroke_rect(canvas, cw, ch, x + lo, y + lo + off, side, side, hw, c, 1.0);
        }
        _ => {
            // edit: a pencil — diagonal body with a small nib corner at the lower-left
            let tipx = x + lo;
            let tipy = y + hi;
            stroke_line(canvas, cw, ch, tipx, tipy, x + hi, y + lo, hw, c, 1.0);
            let nib = (hi - lo) * 0.26;
            stroke_line(canvas, cw, ch, tipx, tipy, tipx + nib, tipy, hw, c, 1.0);
            stroke_line(canvas, cw, ch, tipx, tipy, tipx, tipy - nib, hw, c, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn card_rounds_corners_and_draws_white_border() {
        let mut img = RgbaImage::new(20, 20);
        for p in img.pixels_mut() {
            *p = image::Rgba([10, 120, 240, 255]); // R=10 so we can tell it from white
        }
        let mut buf = vec![0u8; 20 * 20 * 4];
        blit_thumb_card(&mut buf, 20, 20, &img, 0, 0);
        // far corner is outside the radius -> transparent
        assert_eq!(buf[3], 0, "corner should be transparent");
        // centre is opaque fill = the thumbnail colour (low R), not white
        let c = ((10 * 20 + 10) * 4) as usize;
        assert!(buf[c + 3] > 250, "centre should be opaque");
        assert!(buf[c + 2] < 60, "centre R should be the thumbnail's, not white");
        // left-edge midpoint is the white border (all channels high)
        let e = ((10 * 20 + 0) * 4) as usize;
        assert!(
            buf[e + 3] > 250 && buf[e] > 240 && buf[e + 1] > 240 && buf[e + 2] > 240,
            "left edge should be a white border pixel"
        );
    }

    #[test]
    fn clear_zeros_buffer() {
        let mut buf = vec![9u8; 16];
        clear(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn fill_circle_blends_center_opaqueish() {
        let mut buf = vec![0u8; 24 * 24 * 4];
        fill_circle(&mut buf, 24, 24, 12.0, 12.0, 8.0, (10, 20, 30), 0.5);
        let idx = ((12 * 24 + 12) * 4) as usize;
        // center alpha ~ 0.5*255
        assert!(buf[idx + 3] > 100 && buf[idx + 3] < 160, "got a={}", buf[idx + 3]);
        // corner stays transparent
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn hovered_thumb_draws_icon_pixels() {
        let mut model = ShelfModel::new();
        let mut t = RgbaImage::new(200, 140);
        for p in t.pixels_mut() {
            *p = image::Rgba([10, 20, 200, 255]);
        }
        let id = model.add(PathBuf::from("/tmp/x.png"), t, "area".into());
        let cfg = LayoutConfig::default();
        let sizes: Vec<(u64, u32, u32)> = model
            .newest_first()
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        let layout = Layout::compute(&sizes, &cfg);
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        draw_shelf(&mut canvas, layout.width, layout.height, &layout, &model, Some(id), &cfg);
        // thumb body opaque
        let r = &layout.thumbs[0];
        let px = r.x + r.w / 2;
        let py = r.y + r.h / 2;
        let idx = ((py * layout.width + px) * 4) as usize;
        assert!(canvas[idx + 3] > 0, "thumb body should be opaque");
        // a glyph pixel near the close-button centre should differ from the
        // plain thumbnail colour (button bg darkened or white glyph stroke).
        let bx = r.x + r.w - cfg.pad_icon - cfg.icon / 2;
        let by = r.y + cfg.pad_icon + cfg.icon / 2;
        let bidx = ((by * layout.width + bx) * 4) as usize;
        let plain = canvas[idx + 2]; // R of plain thumb region
        assert_ne!(canvas[bidx + 2], plain, "button area should not be plain thumb colour");
    }
}
