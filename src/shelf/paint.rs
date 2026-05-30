use image::RgbaImage;

use crate::shelf::layout::{Layout, LayoutConfig, ThumbRect};
use crate::shelf::model::ShelfModel;

const ICON_BG: (u8, u8, u8, u8) = (20, 20, 28, 220);
const ICON_CLOSE_BG: (u8, u8, u8, u8) = (40, 16, 16, 230);
const GLYPH: (u8, u8, u8, u8) = (240, 240, 245, 255);

/// Set the whole canvas to transparent (0,0,0,0).
pub fn clear(canvas: &mut [u8]) {
    for b in canvas.iter_mut() {
        *b = 0;
    }
}

/// Fill an axis-aligned rect with a straight-alpha color, writing premultiplied BGRA.
pub fn fill_rect_bgra(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    r: u8,
    g: u8,
    b: u8,
    a: u8,
) {
    let x1 = (x + w).min(cw);
    let y1 = (y + h).min(ch);
    let (pr, pg, pb) = premul(r, g, b, a);
    for py in y..y1 {
        for px in x..x1 {
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = pb;
            canvas[idx + 1] = pg;
            canvas[idx + 2] = pr;
            canvas[idx + 3] = a;
        }
    }
}

/// Composite a straight-alpha RGBA image onto the canvas at (dx,dy) using source-over.
pub fn blit_rgba(canvas: &mut [u8], cw: u32, ch: u32, img: &RgbaImage, dx: u32, dy: u32) {
    let (iw, ih) = img.dimensions();
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
            let p = img.get_pixel(sx, sy).0;
            let (sr, sg, sb, sa) = (p[0], p[1], p[2], p[3]);
            let idx = ((py * cw + px) * 4) as usize;
            if sa == 255 {
                canvas[idx] = sb;
                canvas[idx + 1] = sg;
                canvas[idx + 2] = sr;
                canvas[idx + 3] = 255;
            } else if sa == 0 {
                // leave dst
            } else {
                // source-over with premultiplied dst
                let (spr, spg, spb) = premul(sr, sg, sb, sa);
                let inv = 255u32 - sa as u32;
                let db = canvas[idx] as u32;
                let dg = canvas[idx + 1] as u32;
                let dr = canvas[idx + 2] as u32;
                let da = canvas[idx + 3] as u32;
                canvas[idx] = (spb as u32 + db * inv / 255) as u8;
                canvas[idx + 1] = (spg as u32 + dg * inv / 255) as u8;
                canvas[idx + 2] = (spr as u32 + dr * inv / 255) as u8;
                canvas[idx + 3] = (sa as u32 + da * inv / 255) as u8;
            }
        }
    }
}

#[inline]
fn premul(r: u8, g: u8, b: u8, a: u8) -> (u8, u8, u8) {
    let a = a as u32;
    (
        (r as u32 * a / 255) as u8,
        (g as u32 * a / 255) as u8,
        (b as u32 * a / 255) as u8,
    )
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
            blit_rgba(canvas, cw, ch, &thumb.thumb, r.x, r.y);
        }
        if hovered == Some(r.id) {
            draw_hover_icons(canvas, cw, ch, r, cfg);
        }
    }
}

fn draw_hover_icons(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, cfg: &LayoutConfig) {
    // slot 0 close (rightmost), 1 copy, 2 edit — mirror Layout::icon_rect math.
    for slot in 0..3u32 {
        let right = (r.x + r.w).saturating_sub(cfg.pad_icon);
        // Saturating: on a thumb narrower than the icon strip, clamp to the
        // left edge instead of underflowing u32.
        let x = right
            .saturating_sub((slot + 1) * cfg.icon)
            .saturating_sub(slot * cfg.icon_gap);
        let y = r.y + cfg.pad_icon;
        let bg = if slot == 0 { ICON_CLOSE_BG } else { ICON_BG };
        fill_rect_bgra(canvas, cw, ch, x, y, cfg.icon, cfg.icon, bg.0, bg.1, bg.2, bg.3);
        draw_glyph(canvas, cw, ch, slot, x, y, cfg.icon);
    }
}

/// Crude vector glyphs: 0=close (X), 1=copy (two squares), 2=edit (diagonal stroke).
fn draw_glyph(canvas: &mut [u8], cw: u32, ch: u32, slot: u32, x: u32, y: u32, s: u32) {
    let m = s / 5; // margin
    match slot {
        0 => {
            // X: two diagonals
            for i in m..(s - m) {
                put(canvas, cw, ch, x + i, y + i);
                put(canvas, cw, ch, x + i, y + (s - 1 - i));
            }
        }
        1 => {
            // copy: two overlapping square outlines
            stroke_rect(canvas, cw, ch, x + m, y + m, s - 2 * m - m, s - 2 * m - m);
            stroke_rect(canvas, cw, ch, x + 2 * m, y + 2 * m, s - 2 * m - m, s - 2 * m - m);
        }
        _ => {
            // edit: a single diagonal pencil stroke
            for i in m..(s - m) {
                put(canvas, cw, ch, x + i, y + (s - 1 - i));
            }
        }
    }
}

fn put(canvas: &mut [u8], cw: u32, ch: u32, x: u32, y: u32) {
    if x < cw && y < ch {
        let idx = ((y * cw + x) * 4) as usize;
        let (r, g, b, a) = GLYPH;
        let (pr, pg, pb) = premul(r, g, b, a);
        canvas[idx] = pb;
        canvas[idx + 1] = pg;
        canvas[idx + 2] = pr;
        canvas[idx + 3] = a;
    }
}

fn stroke_rect(canvas: &mut [u8], cw: u32, ch: u32, x: u32, y: u32, w: u32, h: u32) {
    for i in 0..w {
        put(canvas, cw, ch, x + i, y);
        put(canvas, cw, ch, x + i, y + h.saturating_sub(1));
    }
    for j in 0..h {
        put(canvas, cw, ch, x, y + j);
        put(canvas, cw, ch, x + w.saturating_sub(1), y + j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn fill_opaque_red_is_bgra() {
        let mut buf = vec![0u8; 4]; // 1x1
        fill_rect_bgra(&mut buf, 1, 1, 0, 0, 1, 1, 255, 0, 0, 255);
        // premultiplied opaque red -> B=0 G=0 R=255 A=255
        assert_eq!(buf, vec![0, 0, 255, 255]);
    }

    #[test]
    fn blit_straight_alpha_premultiplies() {
        // a 1x1 source: half-transparent white (straight alpha)
        let mut src = RgbaImage::new(1, 1);
        src.put_pixel(0, 0, image::Rgba([255, 255, 255, 128]));
        let mut buf = vec![0u8; 4]; // transparent dst
        blit_rgba(&mut buf, 1, 1, &src, 0, 0);
        // premultiplied: 255*128/255 = 128 for each color, A=128
        assert_eq!(buf, vec![128, 128, 128, 128]);
    }

    #[test]
    fn clear_zeros_buffer() {
        let mut buf = vec![9u8; 16];
        clear(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn draw_shelf_fills_thumb_region_nontransparent() {
        let mut model = ShelfModel::new();
        let mut t = RgbaImage::new(40, 30);
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
        // a pixel in the middle of the (only) thumb must be non-transparent
        let r = &layout.thumbs[0];
        let px = r.x + r.w / 2;
        let py = r.y + r.h / 2;
        let idx = ((py * layout.width + px) * 4) as usize;
        assert!(canvas[idx + 3] > 0, "thumb body should be opaque");
    }
}
