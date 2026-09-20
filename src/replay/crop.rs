#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Crop {
    pub fn from_selection(
        [x, y, w, h]: [f64; 4],
        (sw, sh): (u32, u32),
        (vw, vh): (u32, u32),
    ) -> Result<Self, String> {
        if sw == 0
            || sh == 0
            || ![x, y, w, h].iter().all(|v| v.is_finite())
            || x < 0.0
            || y < 0.0
            || w <= 0.0
            || h <= 0.0
        {
            return Err("invalid selection dimensions".into());
        }
        let sx = vw as f64 / sw as f64;
        let sy = vh as f64 / sh as f64;
        let left = (x * sx).ceil();
        let top = (y * sy).ceil();
        let right = ((x + w) * sx).floor();
        let bottom = ((y + h) * sy).floor();
        if right > vw as f64 || bottom > vh as f64 || right - left < 2.0 || bottom - top < 2.0 {
            return Err("selection is outside the replay screen or too small".into());
        }
        Self::from_value(
            &serde_json::json!({"x":left as u32,"y":top as u32,"width":(right-left) as u32,"height":(bottom-top) as u32}),
            vw,
            vh,
        )
    }

    pub fn from_value(value: &serde_json::Value, width: u32, height: u32) -> Result<Self, String> {
        let number = |key| {
            value[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or_else(|| format!("invalid crop {key}"))
        };
        let mut crop = Self {
            x: number("x")?,
            y: number("y")?,
            width: number("width")?,
            height: number("height")?,
        };
        if crop.width < 2
            || crop.height < 2
            || crop
                .x
                .checked_add(crop.width)
                .is_none_or(|right| right > width)
            || crop
                .y
                .checked_add(crop.height)
                .is_none_or(|bottom| bottom > height)
        {
            return Err("crop is outside the recorded screen".into());
        }
        // Keep 4:2:0 boundaries inside the requested rectangle.
        let right = (crop.x + crop.width) & !1;
        let bottom = (crop.y + crop.height) & !1;
        crop.x = (crop.x + 1) & !1;
        crop.y = (crop.y + 1) & !1;
        crop.width = right - crop.x;
        crop.height = bottom - crop.y;
        if crop.width < 2 || crop.height < 2 {
            return Err("crop is too small after chroma alignment".into());
        }
        Ok(crop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn alignment_never_includes_pixels_outside_the_selection() {
        assert_eq!(
            Crop::from_value(&json!({"x": 1, "y": 3, "width": 9, "height": 9}), 20, 20).unwrap(),
            Crop {
                x: 2,
                y: 4,
                width: 8,
                height: 8
            }
        );
        for invalid in [
            json!({"x": 19,"y": 0,"width": 2,"height": 4}),
            json!({"x": 4294967295_u32,"y": 0,"width": 2,"height": 4}),
            json!({"x": 1,"y": 1,"width": 2,"height": 2}),
        ] {
            assert!(Crop::from_value(&invalid, 20, 20).is_err());
        }
    }

    #[test]
    fn selection_uses_video_pixels_at_fractional_scale() {
        let crop =
            Crop::from_selection([42.0, 31.0, 215.0, 122.0], (1280, 720), (1920, 1080)).unwrap();
        assert_eq!(
            crop,
            Crop {
                x: 64,
                y: 48,
                width: 320,
                height: 180
            }
        );
        assert!(
            Crop::from_selection([f64::NAN, 0.0, 100.0, 100.0], (1280, 720), (1920, 1080)).is_err()
        );
        assert!(
            Crop::from_selection([0.0, 0.0, 1281.0, 720.0], (1280, 720), (1920, 1080)).is_err()
        );
    }
}
