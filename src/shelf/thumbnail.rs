use image::imageops::FilterType;
use image::{RgbaImage, imageops};

/// Fixed shelf card size in pixels. Every card is exactly this size so the shelf
/// reads as a uniform column. Tweak these to resize the cards.
pub const CARD_W: u32 = 208;
pub const CARD_H: u32 = 144;

/// Center-crop `src` to the card aspect ratio, then resize only that crop to
/// exactly (card_w, card_h). Cropping before resizing avoids scaling huge full
/// screenshots down to an oversized intermediate image on the hot shelf path.
pub fn make_card_thumbnail(src: &RgbaImage, card_w: u32, card_h: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 || card_w == 0 || card_h == 0 {
        return RgbaImage::new(card_w.max(1), card_h.max(1));
    }

    let src_aspect = w as f32 / h as f32;
    let card_aspect = card_w as f32 / card_h as f32;
    let (x, y, cw, ch) = if src_aspect > card_aspect {
        let cw = ((h as f32 * card_aspect).round() as u32).clamp(1, w);
        ((w - cw) / 2, 0, cw, h)
    } else {
        let ch = ((w as f32 / card_aspect).round() as u32).clamp(1, h);
        (0, (h - ch) / 2, w, ch)
    };
    let cropped = imageops::crop_imm(src, x, y, cw, ch).to_image();
    imageops::resize(&cropped, card_w, card_h, FilterType::Triangle)
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
