use image::{RgbaImage, imageops};

use crate::shelf::layout::{Layout, LayoutConfig, ThumbRect};
use crate::shelf::model::ShelfModel;

/// Thumbnail card corner radius, in pixels.
const CARD_RADIUS: f32 = 10.0;
/// Card opacity: slightly translucent so text/windows behind the shelf stay
/// readable. 1.0 = fully opaque.
const CARD_OPACITY: f32 = 0.8;

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

/// Composite an opaque RGBA thumbnail as a rounded card (rounded corners,
/// transparent outside the radius, no border) in premultiplied BGRA. The
/// full-size slot is `img.dimensions()` at (dx, dy); the card is scaled by
/// `scale` (0..1) and centred in that slot, and `opacity` (0..1) multiplies the
/// global card opacity. `scale == 1.0 && opacity == 1.0` is the settled look;
/// other values drive the appear/dismiss animation.
pub fn blit_thumb_card_anim(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    img: &RgbaImage,
    dx: u32,
    dy: u32,
    scale: f32,
    opacity: f32,
) {
    let (iw, ih) = img.dimensions();
    let scale = scale.clamp(0.0, 1.0);
    let sw = ((iw as f32 * scale).round() as u32).max(1);
    let sh = ((ih as f32 * scale).round() as u32).max(1);
    // Resize only when actually scaling; the card is small so this is cheap.
    let scaled;
    let src: &RgbaImage = if sw == iw && sh == ih {
        img
    } else {
        scaled = imageops::resize(img, sw, sh, imageops::FilterType::Triangle);
        &scaled
    };
    // Centre the scaled card in the original iw×ih slot.
    let ox = dx + (iw - sw) / 2;
    let oy = dy + (ih - sh) / 2;
    let w = sw as f32;
    let h = sh as f32;
    let r = CARD_RADIUS.min(w / 2.0).min(h / 2.0);
    let global = CARD_OPACITY * opacity.clamp(0.0, 1.0);
    for sy in 0..sh {
        let py = oy + sy;
        if py >= ch {
            break;
        }
        for sx in 0..sw {
            let px = ox + sx;
            if px >= cw {
                break;
            }
            let cov = rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w, h, r);
            if cov <= 0.0 {
                continue; // transparent corner -> leave the canvas untouched
            }
            // Premultiplied BGRA: scaling RGB and alpha together by the same
            // factor keeps the pixel valid while making the card translucent.
            let a = cov * global;
            let p = src.get_pixel(sx, sy).0;
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = (p[2] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 1] = (p[1] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 2] = (p[0] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Build a premultiplied-BGRA drag icon: scale `src` to (w,h) with Lanczos3,
/// round the corners, and apply a global `opacity` (0..=1). Returns w*h*4 bytes,
/// ready to copy into a wl_shm Argb8888 buffer.
pub fn build_drag_icon(src: &RgbaImage, w: u32, h: u32, radius: f32, opacity: f32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    if w == 0 || h == 0 {
        return buf;
    }
    let scaled = image::imageops::resize(src, w, h, image::imageops::FilterType::Lanczos3);
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    for sy in 0..h {
        for sx in 0..w {
            let cov =
                rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w as f32, h as f32, r) * opacity;
            if cov <= 0.0 {
                continue;
            }
            let p = scaled.get_pixel(sx, sy).0;
            let idx = ((sy * w + sx) * 4) as usize;
            buf[idx] = (p[2] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 1] = (p[1] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 2] = (p[0] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 3] = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    buf
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
fn fill_circle(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    cx: f32,
    cy: f32,
    radius: f32,
    c: (u8, u8, u8),
    a: f32,
) {
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

/// Render the whole shelf: each thumbnail, plus hover icons on the hovered thumb.
pub fn draw_shelf(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    layout: &Layout,
    model: &ShelfModel,
    hovered: Option<u64>,
    cfg: &LayoutConfig,
    anims: &[(u64, f32, f32)],
) {
    clear(canvas);
    for r in &layout.thumbs {
        if let Some(thumb) = model.get(r.id) {
            let (scale, opacity) = anims
                .iter()
                .find(|(id, _, _)| *id == r.id)
                .map(|(_, s, o)| (*s, *o))
                .unwrap_or((1.0, 1.0));
            blit_thumb_card_anim(canvas, cw, ch, &thumb.thumb, r.x, r.y, scale, opacity);
        }
        // Hide hover icons on a card that is mid-animation (scaling/fading).
        let animating = anims.iter().any(|(id, _, _)| *id == r.id);
        if hovered == Some(r.id) && !animating {
            draw_hover_icons(canvas, cw, ch, r, cfg);
        }
    }
}

fn draw_hover_icons(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, cfg: &LayoutConfig) {
    let s = cfg.icon as f32;
    // close — top-right
    let (clx, cly, _, _) = crate::shelf::layout::close_cell(r, cfg);
    fill_circle(
        canvas, cw, ch,
        clx as f32 + s / 2.0, cly as f32 + s / 2.0, s / 2.0 - 0.5,
        BTN_BG, BTN_BG_A,
    );
    draw_glyph(canvas, cw, ch, Glyph::Close, clx as f32, cly as f32, s, GLYPH_CLOSE_RGB);
    // save — top-left
    let (sx, sy, _, _) = crate::shelf::layout::save_cell(r, cfg);
    fill_circle(
        canvas, cw, ch,
        sx as f32 + s / 2.0, sy as f32 + s / 2.0, s / 2.0 - 0.5,
        BTN_BG, BTN_BG_A,
    );
    draw_glyph(canvas, cw, ch, Glyph::Save, sx as f32, sy as f32, s, GLYPH_RGB);
}

/// Which glyph to stamp in a button cell.
enum Glyph {
    Close,
    Save,
}

/// Minimal anti-aliased glyphs centred in a cell at (x,y) of size s.
fn draw_glyph(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    glyph: Glyph,
    x: f32,
    y: f32,
    s: f32,
    c: (u8, u8, u8),
) {
    let hw = GLYPH_HALF_W;
    let inset = s * 0.30;
    let lo = inset;
    let hi = s - inset;
    match glyph {
        Glyph::Close => {
            stroke_line(canvas, cw, ch, x + lo, y + lo, x + hi, y + hi, hw, c, 1.0);
            stroke_line(canvas, cw, ch, x + hi, y + lo, x + lo, y + hi, hw, c, 1.0);
        }
        Glyph::Save => {
            // down-arrow into a tray (the modern "save / download" idiom)
            let mid = x + s / 2.0;
            let head = s * 0.18;
            stroke_line(canvas, cw, ch, mid, y + inset, mid, y + s * 0.58, hw, c, 1.0);
            stroke_line(canvas, cw, ch, mid - head, y + s * 0.40, mid, y + s * 0.58, hw, c, 1.0);
            stroke_line(canvas, cw, ch, mid + head, y + s * 0.40, mid, y + s * 0.58, hw, c, 1.0);
            stroke_line(canvas, cw, ch, x + inset, y + s * 0.74, x + s - inset, y + s * 0.74, hw, c, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn card_rounds_corners_no_border() {
        let mut img = RgbaImage::new(20, 20);
        for p in img.pixels_mut() {
            *p = image::Rgba([10, 120, 240, 255]); // low R so we can tell it from white
        }
        let mut buf = vec![0u8; 20 * 20 * 4];
        blit_thumb_card_anim(&mut buf, 20, 20, &img, 0, 0, 1.0, 1.0);
        // far corner is outside the radius -> transparent
        assert_eq!(buf[3], 0, "corner should be transparent");
        // centre carries the card alpha (~0.8*255), the thumbnail colour (low R)
        let c = ((10 * 20 + 10) * 4) as usize;
        assert!(
            buf[c + 3] > 180 && buf[c + 3] < 220,
            "centre alpha ~0.8, got {}",
            buf[c + 3]
        );
        assert!(buf[c + 2] < 60, "centre R should be the thumbnail's");
        // left-edge midpoint is the IMAGE colour now, NOT a white border
        let e = ((10 * 20 + 0) * 4) as usize;
        assert!(
            buf[e + 3] > 150,
            "left edge should carry card alpha, got {}",
            buf[e + 3]
        );
        assert!(
            buf[e + 2] < 60,
            "left edge R should be the image's, not white"
        );
    }

    #[test]
    fn drag_icon_is_premultiplied_and_rounded() {
        let mut img = RgbaImage::new(40, 40);
        for p in img.pixels_mut() {
            *p = image::Rgba([200, 100, 50, 255]);
        }
        let buf = build_drag_icon(&img, 40, 40, 8.0, 0.85);
        assert_eq!(buf.len(), 40 * 40 * 4);
        // corner transparent
        assert_eq!(buf[3], 0, "corner alpha should be 0");
        // centre alpha ~ 0.85*255
        let c = ((20 * 40 + 20) * 4) as usize;
        assert!(buf[c + 3] > 200 && buf[c + 3] < 230, "got a={}", buf[c + 3]);
        // premultiplied: every colour channel <= alpha
        assert!(buf[c] <= buf[c + 3] && buf[c + 1] <= buf[c + 3] && buf[c + 2] <= buf[c + 3]);
    }

    #[test]
    fn anim_scale_and_opacity_shrink_and_fade() {
        let mut img = RgbaImage::new(40, 40);
        for p in img.pixels_mut() {
            *p = image::Rgba([10, 120, 240, 255]);
        }
        // Half scale, half opacity: card occupies the centre, corners of the slot
        // are untouched (transparent), centre alpha ~ 0.5 * CARD_OPACITY.
        let mut buf = vec![0u8; 40 * 40 * 4];
        blit_thumb_card_anim(&mut buf, 40, 40, &img, 0, 0, 0.5, 0.5);
        // slot corner (well outside the centred 20x20 card) stays transparent
        let corner = ((2 * 40 + 2) * 4) as usize;
        assert_eq!(buf[corner + 3], 0, "slot corner should be untouched");
        // centre carries ~0.5*0.75*255 ≈ 95
        let c = ((20 * 40 + 20) * 4) as usize;
        assert!(
            buf[c + 3] > 70 && buf[c + 3] < 120,
            "centre alpha ~0.375, got {}",
            buf[c + 3]
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
        assert!(
            buf[idx + 3] > 100 && buf[idx + 3] < 160,
            "got a={}",
            buf[idx + 3]
        );
        // corner stays transparent
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn hovered_thumb_draws_icon_pixels() {
        let mut model = ShelfModel::new();
        let mut t = RgbaImage::new(260, 180);
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
        draw_shelf(
            &mut canvas,
            layout.width,
            layout.height,
            &layout,
            &model,
            Some(id),
            &cfg,
            &[],
        );
        // thumb body opaque
        let r = &layout.thumbs[0];
        let px = r.x + r.w / 2;
        let py = r.y + r.h / 2;
        let idx = ((py * layout.width + px) * 4) as usize;
        assert!(canvas[idx + 3] > 0, "thumb body should be opaque");
        // a glyph pixel near the close-button centre should differ from the
        // plain thumbnail colour (button bg darkened or glyph stroke).
        let bx = r.x + r.w - cfg.pad_icon - cfg.icon / 2;
        let by = r.y + cfg.pad_icon + cfg.icon / 2;
        let bidx = ((by * layout.width + bx) * 4) as usize;
        let plain = canvas[idx + 2]; // R of plain thumb region
        assert_ne!(
            canvas[bidx + 2],
            plain,
            "button area should not be plain thumb colour"
        );
        // a glyph pixel near the save-button (top-left) centre should differ from
        // the plain thumbnail colour.
        let sx = r.x + cfg.pad_icon + cfg.icon / 2;
        let sy = r.y + cfg.pad_icon + cfg.icon / 2;
        let sidx = ((sy * layout.width + sx) * 4) as usize;
        assert_ne!(
            canvas[sidx + 2],
            plain,
            "save button area should not be plain thumb colour"
        );
    }
}
