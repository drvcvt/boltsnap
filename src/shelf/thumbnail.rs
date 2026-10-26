use image::imageops::FilterType;
use image::{RgbaImage, imageops};

/// Default shelf card size in pixels. Every card is exactly the current size
/// so the shelf reads as a uniform column.
pub const CARD_W: u32 = CardSize::DEFAULT.w;
pub const CARD_H: u32 = CardSize::DEFAULT.h;

/// `[shelf] card_width`; the height follows at the default aspect ratio.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CardSize {
    pub w: u32,
    pub h: u32,
}

impl CardSize {
    pub const DEFAULT: CardSize = CardSize { w: 190, h: 132 };
    pub const MIN_WIDTH: u32 = 140;
    pub const MAX_WIDTH: u32 = 380;

    pub fn from_width(width: i64) -> Option<CardSize> {
        let w = u32::try_from(width)
            .ok()
            .filter(|w| (Self::MIN_WIDTH..=Self::MAX_WIDTH).contains(w))?;
        let h = (w * Self::DEFAULT.h + Self::DEFAULT.w / 2) / Self::DEFAULT.w;
        Some(CardSize { w, h })
    }

    pub fn dims(self) -> (u32, u32) {
        (self.w, self.h)
    }
}

/// A card thumbnail at another card size, from the small image alone. Softer
/// than rendering from the source when it grows; the shelf re-renders after.
pub fn rescale(card: &RgbaImage, size: CardSize) -> RgbaImage {
    if card.dimensions() == size.dims() {
        return card.clone();
    }
    imageops::resize(card, size.w, size.h, FilterType::Triangle)
}

/// Center-crop `src` to the card aspect ratio, then resize only that crop to
/// exactly (card_w, card_h). Cropping before resizing avoids scaling huge full
/// screenshots down to an oversized intermediate image on the hot shelf path.
// Linux cards use the variants below; Windows, tests and the transfer bench use this.
#[allow(dead_code)]
pub fn make_card_thumbnail(src: &RgbaImage, card_w: u32, card_h: u32) -> RgbaImage {
    thumbnail(src, card_w, card_h)
}

/// Card for a decoded image without converting the whole image to RGBA first.
pub fn make_image_card_thumbnail(src: &image::DynamicImage, card_w: u32, card_h: u32) -> RgbaImage {
    match src {
        image::DynamicImage::ImageRgb8(rgb) => {
            image::DynamicImage::ImageRgb8(thumbnail(rgb, card_w, card_h)).into_rgba8()
        }
        image::DynamicImage::ImageRgba8(rgba) => thumbnail(rgba, card_w, card_h),
        other => thumbnail(&other.to_rgba8(), card_w, card_h),
    }
}

pub fn make_rgb_card_thumbnail(
    src: &image::ImageBuffer<image::Rgb<u8>, &[u8]>,
    card_w: u32,
    card_h: u32,
) -> RgbaImage {
    image::DynamicImage::ImageRgb8(thumbnail(src, card_w, card_h)).into_rgba8()
}

fn thumbnail<I>(src: &I, card_w: u32, card_h: u32) -> image::ImageBuffer<I::Pixel, Vec<u8>>
where
    I: image::GenericImageView,
    I::Pixel: image::Pixel<Subpixel = u8> + 'static,
{
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 || card_w == 0 || card_h == 0 {
        return image::ImageBuffer::new(card_w.max(1), card_h.max(1));
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
    let cropped = imageops::crop_imm(src, x, y, cw, ch);
    if cw >= card_w && ch >= card_h {
        // Box averaging: ~3x faster than a Triangle resize over a 4K crop and
        // equally clean when shrinking.
        imageops::thumbnail(&*cropped, card_w, card_h)
    } else {
        imageops::resize(&*cropped, card_w, card_h, FilterType::Triangle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_size_keeps_the_aspect_ratio_within_its_range() {
        let size = |width| CardSize::from_width(width).map(CardSize::dims);
        assert_eq!(size(140), Some((140, 97)));
        assert_eq!(size(190), Some((190, 132)));
        assert_eq!(size(300), Some((300, 208)));
        assert_eq!(size(380), Some((380, 264)));
        assert_eq!(size(139), None);
        assert_eq!(size(381), None);
        assert_eq!(size(-190), None);
        assert_eq!(size(i64::MAX), None);
        assert_eq!(CardSize::from_width(190), Some(CardSize::DEFAULT));
    }

    #[test]
    fn rescale_gives_exactly_the_new_size() {
        let big = CardSize::from_width(380).unwrap();
        for card in [RgbaImage::new(1, 1), RgbaImage::new(190, 132)] {
            assert_eq!(rescale(&card, big).dimensions(), big.dims());
            assert_eq!(
                rescale(&card, CardSize::from_width(140).unwrap()).dimensions(),
                (140, 97)
            );
        }
    }

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
