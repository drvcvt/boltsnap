//! Pure rendering + geometry for the tiny-skia region selector. No Wayland here.

#![allow(dead_code)] // filled in by later tasks

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
}
