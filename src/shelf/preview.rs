//! The centered full-image enlarge ("lightbox") view: pure geometry + render.

use image::RgbaImage;

/// Fit an (img_w, img_h) image inside a (screen_w, screen_h) area with `margin`
/// px of breathing room on each side, preserving aspect ratio, centered.
/// Returns (draw_w, draw_h, off_x, off_y) in screen pixels.
pub fn fit_centered(
    img_w: u32,
    img_h: u32,
    screen_w: u32,
    screen_h: u32,
    margin: u32,
) -> (u32, u32, u32, u32) {
    if img_w == 0 || img_h == 0 || screen_w == 0 || screen_h == 0 {
        return (0, 0, screen_w / 2, screen_h / 2);
    }
    let avail_w = screen_w.saturating_sub(margin * 2).max(1);
    let avail_h = screen_h.saturating_sub(margin * 2).max(1);
    let scale = (avail_w as f32 / img_w as f32).min(avail_h as f32 / img_h as f32);
    let dw = ((img_w as f32 * scale).round() as u32).clamp(1, screen_w);
    let dh = ((img_h as f32 * scale).round() as u32).clamp(1, screen_h);
    let off_x = (screen_w - dw) / 2;
    let off_y = (screen_h - dh) / 2;
    (dw, dh, off_x, off_y)
}

/// Backdrop colour + opacity for the lightbox (premultiplied BGRA fill).
const BACKDROP: (u8, u8, u8) = (8, 8, 12);
const BACKDROP_A: f32 = 0.78;

/// Render the enlarge view into a premultiplied-BGRA `canvas` of (sw, sh): a
/// dimmed backdrop plus the full `img` fitted and centered with `margin`.
pub fn render_lightbox(canvas: &mut [u8], sw: u32, sh: u32, img: &RgbaImage, margin: u32) {
    let (br, bg, bb) = BACKDROP;
    let a = BACKDROP_A;
    for px in canvas.chunks_exact_mut(4) {
        px[0] = (bb as f32 * a) as u8;
        px[1] = (bg as f32 * a) as u8;
        px[2] = (br as f32 * a) as u8;
        px[3] = (a * 255.0) as u8;
    }
    let (iw, ih) = img.dimensions();
    let (dw, dh, ox, oy) = fit_centered(iw, ih, sw, sh, margin);
    if dw == 0 || dh == 0 {
        return;
    }
    // Triangle, not Lanczos3: the lightbox shows the screenshot at ~1:1, where
    // Triangle is visually identical but ~6x cheaper. Lanczos3 here cost ~2s on a
    // 2MP image in a debug build, stalling the click-to-open.
    let scaled = image::imageops::resize(img, dw, dh, image::imageops::FilterType::Triangle);
    for sy in 0..dh {
        let py = oy + sy;
        if py >= sh {
            break;
        }
        for sx in 0..dw {
            let pxn = ox + sx;
            if pxn >= sw {
                break;
            }
            let p = scaled.get_pixel(sx, sy).0; // opaque screenshot
            let idx = ((py * sw + pxn) * 4) as usize;
            canvas[idx] = p[2];
            canvas[idx + 1] = p[1];
            canvas[idx + 2] = p[0];
            canvas[idx + 3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_image_is_width_limited_and_centered() {
        let (dw, dh, ox, oy) = fit_centered(2000, 500, 1000, 1000, 0);
        assert_eq!(dw, 1000);
        assert_eq!(dh, 250);
        assert_eq!(ox, 0);
        assert_eq!(oy, 375);
    }

    #[test]
    fn tall_image_is_height_limited_and_centered() {
        let (dw, dh, ox, oy) = fit_centered(500, 2000, 1000, 1000, 0);
        assert_eq!(dw, 250);
        assert_eq!(dh, 1000);
        assert_eq!(ox, 375);
        assert_eq!(oy, 0);
    }

    #[test]
    fn margin_shrinks_the_drawable_area() {
        let (dw, dh, _, _) = fit_centered(1000, 1000, 1000, 1000, 100);
        assert!(dw <= 800 && dh <= 800, "got {dw}x{dh}");
    }

    #[test]
    fn render_lightbox_dims_backdrop_and_draws_image() {
        let mut img = RgbaImage::new(100, 50);
        for p in img.pixels_mut() {
            *p = image::Rgba([0, 200, 0, 255]);
        }
        let (sw, sh) = (400u32, 300u32);
        let mut canvas = vec![0u8; (sw * sh * 4) as usize];
        render_lightbox(&mut canvas, sw, sh, &img, 20);
        // backdrop is visible at the corner
        assert!(canvas[3] > 0, "backdrop should be visible");
        // centre: the green image, opaque
        let c = ((sh / 2 * sw + sw / 2) * 4) as usize;
        assert!(canvas[c + 3] > 250, "image centre opaque");
        assert!(canvas[c + 1] > 150, "image centre green");
    }
}
