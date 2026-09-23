use crate::*;
use image::{RgbaImage, imageops};

pub struct DesktopCapture {
    pub image: RgbaImage,
    pub outputs: Vec<Output>,
    pub logical_bounds: Rect,
    pub scale: f64,
}
impl Frame {
    /// Upright RGBA8 image. Includes Y inversion and all eight buffer transforms.
    pub fn to_image(&self) -> Result<RgbaImage> {
        let mut image = RgbaImage::from_raw(self.width, self.height, self.rgba8()?)
            .ok_or(Error::InvalidDimensions)?;
        if self.y_inverted {
            imageops::flip_vertical_in_place(&mut image);
        }
        Ok(orient(image, self.transform))
    }
}
fn orient(mut image: RgbaImage, t: Transform) -> RgbaImage {
    if matches!(
        t,
        Transform::Flipped | Transform::Flipped90 | Transform::Flipped180 | Transform::Flipped270
    ) {
        imageops::flip_horizontal_in_place(&mut image);
    }
    match t {
        // Undo the compositor's transform. Reflections reverse the rotation
        // order, so flipped quarter-turns are their own inverse.
        Transform::Rotate90 | Transform::Flipped270 => imageops::rotate90(&image),
        Transform::Rotate180 | Transform::Flipped180 => {
            imageops::rotate180_in_place(&mut image);
            image
        }
        Transform::Rotate270 | Transform::Flipped90 => imageops::rotate270(&image),
        _ => image,
    }
}
impl Connection {
    pub fn capture_desktop(&mut self, options: &CaptureOptions) -> Result<DesktopCapture> {
        self.compose(None, options)
    }
    pub fn capture_region(
        &mut self,
        region: Rect,
        options: &CaptureOptions,
    ) -> Result<DesktopCapture> {
        self.compose(Some(region.validate()?), options)
    }
    fn compose(
        &mut self,
        region: Option<Rect>,
        options: &CaptureOptions,
    ) -> Result<DesktopCapture> {
        let end = std::time::Instant::now()
            .checked_add(options.timeout)
            .ok_or(Error::InvalidDimensions)?;
        let original = self.outputs(options)?;
        let outputs: Vec<_> = original
            .iter()
            .filter(|o| region.is_none_or(|r| r.intersection(o.logical).is_some()))
            .cloned()
            .collect();
        if outputs.is_empty() {
            return Err(Error::NoOutputs);
        }
        let bounds = region.unwrap_or(Rect::bounds(outputs.iter().map(|o| o.logical))?);
        let scale = outputs.iter().map(Output::scale).fold(1.0, f64::max);
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Error::InvalidDimensions);
        }
        let (width, height) = scaled_size(bounds.width, bounds.height, scale)?;
        let bytes = options.limits.buffer(
            width,
            height,
            width.checked_mul(4).ok_or(Error::InvalidDimensions)?,
            4,
        )?;
        // A single output covering the requested bounds can become the result
        // directly, avoiding a second full-size allocation and pixel copy.
        let direct = outputs.len() == 1 && outputs[0].logical == bounds;
        let mut image = if direct {
            None
        } else {
            let mut raw = Vec::new();
            raw.try_reserve_exact(bytes)
                .map_err(|_| Error::LimitExceeded)?;
            raw.resize(bytes, 0);
            // Desktop gaps are opaque black, independent of export codec.
            for p in raw.chunks_exact_mut(4) {
                p[3] = 255;
            }
            Some(RgbaImage::from_raw(width, height, raw).ok_or(Error::InvalidDimensions)?)
        };
        for output in &outputs {
            let mut next = options.clone();
            next.timeout = end.saturating_duration_since(std::time::Instant::now());
            // Account for the target plus capture, conversion, and transform/resize
            // scratch. Limits are deliberately conservative instead of overcommitting.
            next.limits.max_bytes = options
                .limits
                .max_bytes
                .checked_sub(bytes as u64)
                .ok_or(Error::LimitExceeded)?
                / 4;
            let frame = self.capture(output.id, &next)?;
            let (ow, oh) = scaled_size(output.logical.width, output.logical.height, scale)?;
            let output_bytes = next.limits.buffer(
                ow,
                oh,
                ow.checked_mul(4).ok_or(Error::InvalidDimensions)?,
                4,
            )?;
            let upright_size = frame.transform.size(frame.width, frame.height);
            // image's separable Gaussian resize uses an RGBA32F intermediate.
            let scratch = if upright_size != (ow, oh) {
                resize_scratch(upright_size.0, oh)?
            } else {
                0
            };
            let required = (bytes as u64)
                .checked_add(u64::from(frame.stride) * u64::from(frame.height))
                .and_then(|n| n.checked_add(u64::from(frame.width) * u64::from(frame.height) * 8))
                .and_then(|n| n.checked_add(output_bytes as u64))
                .and_then(|n| n.checked_add(scratch))
                .ok_or(Error::LimitExceeded)?;
            if required > options.limits.max_bytes {
                return Err(Error::LimitExceeded);
            }
            let pixels = frame.to_image()?;
            let pixels = if pixels.dimensions() == (ow, oh) {
                pixels
            } else {
                imageops::resize(&pixels, ow, oh, imageops::FilterType::Gaussian)
            };
            let x = ((f64::from(output.logical.x) - f64::from(bounds.x)) * scale) as i64;
            let y = ((f64::from(output.logical.y) - f64::from(bounds.y)) * scale) as i64;
            if let Some(target) = &mut image {
                imageops::replace(target, &pixels, x, y);
            } else {
                image = Some(pixels);
            }
        }
        let mut verify = options.clone();
        verify.timeout = end.saturating_duration_since(std::time::Instant::now());
        if self.outputs(&verify)? != original {
            return Err(Error::LayoutChanged);
        }
        Ok(DesktopCapture {
            image: image.ok_or(Error::NoOutputs)?,
            outputs,
            logical_bounds: bounds,
            scale,
        })
    }
}
// image resizes vertically first: source width × destination height, RGBA32F.
fn resize_scratch(source_width: u32, destination_height: u32) -> Result<u64> {
    (u64::from(source_width) * u64::from(destination_height))
        .checked_mul(16)
        .ok_or(Error::LimitExceeded)
}
fn scaled_size(w: u32, h: u32, scale: f64) -> Result<(u32, u32)> {
    let (w, h) = (f64::from(w) * scale, f64::from(h) * scale);
    if !w.is_finite()
        || !h.is_finite()
        || w < 1.0
        || h < 1.0
        || w > f64::from(i32::MAX)
        || h > f64::from(i32::MAX)
    {
        return Err(Error::InvalidDimensions);
    }
    Ok((w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resize_scratch_accounts_for_vertical_first_intermediate() {
        assert_eq!(resize_scratch(4000, 3000).unwrap(), 192_000_000);
        assert_eq!(resize_scratch(100, 50).unwrap(), 80_000);
        assert!(matches!(
            resize_scratch(i32::MAX as u32, i32::MAX as u32),
            Err(Error::LimitExceeded)
        ));
    }
    #[test]
    fn inverse_transforms_match_asymmetric_reference_pixels() {
        // Buffer pixels: 1 2 3 / 4 5 6. Expected upright raster in row order,
        // derived from wl_output's flip-then-counterclockwise convention.
        let input =
            RgbaImage::from_fn(3, 2, |x, y| image::Rgba([(1 + x + 3 * y) as u8, 0, 0, 255]));
        for (t, size, expected) in [
            (Transform::Normal, (3, 2), [1, 2, 3, 4, 5, 6]),
            (Transform::Rotate90, (2, 3), [4, 1, 5, 2, 6, 3]),
            (Transform::Rotate180, (3, 2), [6, 5, 4, 3, 2, 1]),
            (Transform::Rotate270, (2, 3), [3, 6, 2, 5, 1, 4]),
            (Transform::Flipped, (3, 2), [3, 2, 1, 6, 5, 4]),
            (Transform::Flipped90, (2, 3), [1, 4, 2, 5, 3, 6]),
            (Transform::Flipped180, (3, 2), [4, 5, 6, 1, 2, 3]),
            (Transform::Flipped270, (2, 3), [6, 3, 5, 2, 4, 1]),
        ] {
            let result = orient(input.clone(), t);
            assert_eq!(result.dimensions(), size, "{t:?}");
            assert_eq!(
                result.pixels().map(|p| p[0]).collect::<Vec<_>>(),
                expected,
                "{t:?}"
            );
        }
    }
}
