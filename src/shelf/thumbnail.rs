use image::RgbaImage;
use image::imageops::FilterType;

/// Fixed shelf card size in pixels. Every card is exactly this size so the shelf
/// reads as a uniform column. Tweak these to resize the cards.
pub const CARD_W: u32 = 260;
pub const CARD_H: u32 = 180;

/// Scale `src` to *cover* (card_w, card_h) preserving aspect ratio, then
/// center-crop to exactly (card_w, card_h). May upscale small inputs — that is
/// the cost of a uniform grid, and it only affects the preview; the original PNG
/// is never modified. Always returns an image of exactly card_w × card_h.
pub fn make_card_thumbnail(src: &RgbaImage, card_w: u32, card_h: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 || card_w == 0 || card_h == 0 {
        return RgbaImage::new(card_w.max(1), card_h.max(1));
    }
    let scale = (card_w as f32 / w as f32).max(card_h as f32 / h as f32);
    let nw = ((w as f32 * scale).round() as u32).max(card_w);
    let nh = ((h as f32 * scale).round() as u32).max(card_h);
    let scaled = image::imageops::resize(src, nw, nh, FilterType::Lanczos3);
    let x0 = (nw - card_w) / 2;
    let y0 = (nh - card_h) / 2;
    image::imageops::crop_imm(&scaled, x0, y0, card_w, card_h).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_becomes_exact_card_size() {
        let src = RgbaImage::new(800, 400); // 2:1
        let t = make_card_thumbnail(&src, 260, 180);
        assert_eq!(t.dimensions(), (260, 180));
    }

    #[test]
    fn portrait_becomes_exact_card_size() {
        let src = RgbaImage::new(400, 800); // 1:2
        let t = make_card_thumbnail(&src, 260, 180);
        assert_eq!(t.dimensions(), (260, 180));
    }

    #[test]
    fn tiny_image_is_upscaled_to_card_size() {
        let src = RgbaImage::new(50, 30);
        let t = make_card_thumbnail(&src, 260, 180);
        assert_eq!(t.dimensions(), (260, 180));
    }

    #[test]
    fn center_crop_keeps_the_middle() {
        // Left half red, right half blue; cover-crop into the card keeps the
        // central seam roughly centered.
        let mut src = RgbaImage::new(400, 100);
        for (x, _y, p) in src.enumerate_pixels_mut() {
            *p = if x < 200 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 255, 255])
            };
        }
        let t = make_card_thumbnail(&src, 260, 180);
        let left = t.get_pixel(120, 90).0;
        let right = t.get_pixel(140, 90).0;
        assert!(left[0] > left[2], "left of center should be reddish");
        assert!(right[2] > right[0], "right of center should be bluish");
    }
}
