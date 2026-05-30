use image::RgbaImage;
use image::imageops::FilterType;

/// Downscale `src` to fit within (max_w, max_h), preserving aspect ratio.
/// Never upscales: images already smaller are returned at original size.
pub fn make_thumbnail(src: &RgbaImage, max_w: u32, max_h: u32) -> RgbaImage {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return src.clone();
    }
    let scale = (max_w as f32 / w as f32)
        .min(max_h as f32 / h as f32)
        .min(1.0);
    if scale >= 1.0 {
        return src.clone();
    }
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    image::imageops::resize(src, nw, nh, FilterType::Triangle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_in_box_and_preserves_aspect() {
        let src = RgbaImage::new(800, 400); // 2:1
        let t = make_thumbnail(&src, 170, 120);
        let (w, h) = t.dimensions();
        assert!(w <= 170 && h <= 120, "got {w}x{h}");
        // 2:1 aspect: width is the limiting dim -> 170x85
        assert_eq!((w, h), (170, 85));
    }

    #[test]
    fn does_not_upscale_small_images() {
        let src = RgbaImage::new(50, 30);
        let t = make_thumbnail(&src, 170, 120);
        assert_eq!(t.dimensions(), (50, 30));
    }

    #[test]
    fn tall_image_limited_by_height() {
        let src = RgbaImage::new(200, 800); // 1:4
        let t = make_thumbnail(&src, 170, 120);
        let (w, h) = t.dimensions();
        assert!(h <= 120 && w <= 170);
        assert_eq!((w, h), (30, 120));
    }
}
