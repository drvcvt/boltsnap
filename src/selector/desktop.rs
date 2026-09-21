//! Logical desktop geometry, independent of the window system.

pub type MonitorRect = (i32, i32, u32, u32);
/// Origin and extent of one monitor within the normalized desktop.
pub type Viewport = (u32, u32, u32, u32);

/// Bound allocations and reject incomplete or overflowing output descriptions.
pub fn bounds(monitors: &[MonitorRect]) -> Option<MonitorRect> {
    if monitors.iter().any(|r| r.2 == 0 || r.3 == 0) {
        return None;
    }
    let left = monitors.iter().map(|r| r.0).min()?;
    let top = monitors.iter().map(|r| r.1).min()?;
    let right = monitors
        .iter()
        .map(|r| i64::from(r.0) + i64::from(r.2))
        .max()?;
    let bottom = monitors
        .iter()
        .map(|r| i64::from(r.1) + i64::from(r.3))
        .max()?;
    let w = u32::try_from(right - i64::from(left)).ok()?;
    let h = u32::try_from(bottom - i64::from(top)).ok()?;
    (w <= i32::MAX as u32 / 4 && h <= i32::MAX as u32 && u64::from(w) * u64::from(h) <= 64_000_000)
        .then_some((left, top, w, h))
}

/// Pointer coordinates stay relative to the grab's starting surface, even when
/// negative or beyond that monitor. Clamp only at the entire desktop boundary.
pub fn pointer_position(viewport: Viewport, local: (f64, f64), desktop: (u32, u32)) -> (f64, f64) {
    (
        (local.0 + f64::from(viewport.0)).clamp(0., f64::from(desktop.0)),
        (local.1 + f64::from(viewport.1)).clamp(0., f64::from(desktop.1)),
    )
}

/// Intersect global damage with a monitor and return monitor-local coordinates.
pub fn local_damage((x, y, w, h): Viewport, (x0, y0, x1, y1): Viewport) -> Option<Viewport> {
    let left = x0.max(x);
    let top = y0.max(y);
    let right = x1.min(x + w);
    let bottom = y1.min(y + h);
    (right > left && bottom > top).then(|| (left - x, top - y, right - x, bottom - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implicit_grab_crosses_both_directions_and_vertical_offsets() {
        assert_eq!(
            pointer_position((0, 0, 1920, 1080), (2200., 400.), (3840, 1280)),
            (2200., 400.)
        );
        assert_eq!(
            pointer_position((1920, 200, 1920, 1080), (-200., -100.), (3840, 1280)),
            (1720., 100.)
        );
        assert_eq!(
            pointer_position((1920, 200, 1920, 1080), (-2500., 1400.), (3840, 1280)),
            (0., 1280.)
        );
    }

    #[test]
    fn negative_offset_rotated_and_mirrored_layouts() {
        assert_eq!(
            bounds(&[(-1920, -240, 1920, 1080), (0, 0, 1080, 1920)]),
            Some((-1920, -240, 3000, 2160))
        );
        assert_eq!(bounds(&[(0, 0, 1920, 1080); 2]), Some((0, 0, 1920, 1080)));
        assert_eq!(bounds(&[]), None);
        assert_eq!(bounds(&[(0, 0, 0, 1080)]), None);
        assert_eq!(
            bounds(&[(i32::MIN, 0, 1920, 1080), (i32::MAX, 0, 1920, 1080)]),
            None
        );
    }

    #[test]
    fn damage_crosses_boundary_without_touching_other_pixels() {
        assert_eq!(
            local_damage((1920, 200, 1920, 1080), (1900, 150, 1940, 240)),
            Some((0, 0, 20, 40))
        );
        assert_eq!(
            local_damage((1920, 200, 1920, 1080), (0, 0, 1920, 1080)),
            None
        );
    }

    #[test]
    fn spanning_crop_uses_desktop_scale() {
        let (x, y, w, h) =
            super::super::render::rect_to_image((90., 20.), (110., 40.), 200, 100, 400, 200)
                .unwrap();
        let image = image::RgbaImage::from_fn(400, 200, |x, _| {
            image::Rgba(if x < 200 {
                [255, 0, 0, 255]
            } else {
                [0, 0, 255, 255]
            })
        });
        let crop = image::imageops::crop_imm(&image, x, y, w, h).to_image();
        assert_eq!(crop.dimensions(), (40, 40));
        assert_eq!(crop.get_pixel(19, 0).0, [255, 0, 0, 255]);
        assert_eq!(crop.get_pixel(20, 0).0, [0, 0, 255, 255]);
    }
}
