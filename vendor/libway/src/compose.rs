use crate::{
    capture::{near, scaled_size},
    *,
};
use image::{RgbaImage, imageops};

/// CPU composition of outputs at their largest scale (at least 1.0). Requires `image`.
pub struct DesktopCapture {
    /// Upright RGBA8 canvas; desktop gaps are opaque black and alpha stays premultiplied.
    pub image: RgbaImage,
    /// Output snapshots contributing to the canvas, in capture order.
    pub outputs: Vec<Output>,
    /// Requested region or desktop bounds in compositor-global logical units.
    pub logical_bounds: Rect,
    /// Pixels per logical unit in the canvas.
    pub scale: f64,
}
impl Frame {
    /// Upright RGBA8 image. Includes Y inversion and all eight buffer transforms.
    /// Requires `image`. Preserves premultiplied alpha; GPU frames return
    /// [`Error::Unsupported`] because libway does not perform readback.
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
    /// Capture and compose the entire desktop within one timeout and resource budget.
    /// Every output is captured on this connection with all capture requests in
    /// flight together; the compositor may still copy them one after another, and
    /// the result is never atomic. Layout changes return
    /// [`Error::LayoutChanged`]; no outputs returns [`Error::NoOutputs`]. Requires `image`.
    ///
    /// ```no_run
    /// use libway::{CaptureOptions, Connection};
    /// # fn main() -> libway::Result<()> {
    /// let options = CaptureOptions::default();
    /// let mut connection = Connection::connect(&options)?;
    /// let desktop = connection.capture_desktop(&options)?;
    /// println!("{}x{}", desktop.image.width(), desktop.image.height());
    /// # Ok(())
    /// # }
    /// ```
    pub fn capture_desktop(&mut self, options: &CaptureOptions) -> Result<DesktopCapture> {
        self.compose(None, options)
    }
    /// Capture a compositor-global logical region, filling gaps with opaque black.
    /// Invalid extents return [`Error::InvalidDimensions`]; a region intersecting no
    /// output returns [`Error::NoOutputs`]. Other errors match [`Self::capture_desktop`].
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
        // All frames exist at once, so each output gets an equal share of the
        // budget left after the canvas.
        let share = options
            .limits
            .max_bytes
            .checked_sub(bytes as u64)
            .ok_or(Error::LimitExceeded)?
            / outputs.len() as u64;
        // All outputs are captured on this connection with every request in
        // flight together; each gets an equal share of the budget left after
        // the canvas for its frame, conversion and resize scratch.
        let mut next = options.clone();
        next.timeout = end.saturating_duration_since(std::time::Instant::now());
        next.limits.max_bytes = share / 4;
        // Canvas rectangle of each output at canvas scale.
        let rects = outputs
            .iter()
            .map(|o| {
                let (w, h) = scaled_size(o.logical.width, o.logical.height, scale)?;
                Ok((
                    offset(o.logical.x, bounds.x, scale),
                    offset(o.logical.y, bounds.y, scale),
                    w,
                    h,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut raw = crate::buffer::zeroed(bytes)?;
        // Desktop gaps are opaque black, independent of export codec.
        if !covers(&rects, width, height) {
            for p in raw.as_chunks_mut::<4>().0 {
                p[3] = 255;
            }
        }
        let mut canvas = RgbaImage::from_raw(width, height, raw).ok_or(Error::InvalidDimensions)?;
        // Each frame is composed as soon as it is ready, while the compositor
        // copies the next output. `placed` keeps the size actually written,
        // which rounding can leave one pixel short of the rectangle.
        let mut placed = rects.clone();
        self.capture_batch_each(&outputs, &next, end, &mut |index, frame| {
            let (x, y, w, h) = rects[index];
            if upright(&frame, (w, h)) {
                placed[index] = (x, y, frame.width, frame.height);
                return convert_into(&mut canvas, &frame, x, y);
            }
            let pixels = place(&outputs[index], frame, share, scale, &next.limits)?;
            placed[index] = (x, y, pixels.width(), pixels.height());
            blit(&mut canvas, &pixels, x, y);
            Ok(())
        })?;
        if covers(&rects, width, height) && !covers(&placed, width, height) {
            fill_gaps(&mut canvas, &placed);
        }
        let mut verify = options.clone();
        verify.timeout = end.saturating_duration_since(std::time::Instant::now());
        if self.outputs(&verify)? != original {
            return Err(Error::LayoutChanged);
        }
        let image = Some(canvas);
        Ok(DesktopCapture {
            image: image.ok_or(Error::NoOutputs)?,
            outputs,
            logical_bounds: bounds,
            scale,
        })
    }
}
/// Bring one captured output to canvas scale: orient, then resize when needed.
fn place(
    output: &Output,
    frame: Frame,
    share: u64,
    scale: f64,
    limits: &Limits,
) -> Result<RgbaImage> {
    let (ow, oh) = scaled_size(output.logical.width, output.logical.height, scale)?;
    let output_bytes = limits.buffer(
        ow,
        oh,
        ow.checked_mul(4).ok_or(Error::InvalidDimensions)?,
        4,
    )?;
    let upright = frame.transform.size(frame.width, frame.height);
    let resize = !near(upright, (ow, oh));
    // image's separable Gaussian resize uses an RGBA32F intermediate.
    let scratch = if resize {
        resize_scratch(upright.0, oh)?
    } else {
        0
    };
    let required = (u64::from(frame.stride) * u64::from(frame.height))
        .checked_add(u64::from(frame.width) * u64::from(frame.height) * 8)
        .and_then(|n| n.checked_add(output_bytes as u64))
        .and_then(|n| n.checked_add(scratch))
        .ok_or(Error::LimitExceeded)?;
    if required > share {
        return Err(Error::LimitExceeded);
    }
    let pixels = frame.to_image()?;
    drop(frame);
    let pixels = if resize {
        imageops::resize(&pixels, ow, oh, imageops::FilterType::Gaussian)
    } else {
        pixels
    };
    Ok(pixels)
}
/// Whether the output rectangles cover every canvas pixel. Outputs do not
/// overlap, so clipped areas summing to the canvas area means full coverage.
fn covers(rects: &[(i64, i64, u32, u32)], width: u32, height: u32) -> bool {
    let area: u64 = rects
        .iter()
        .map(|&(x, y, w, h)| {
            let w = (x + i64::from(w)).min(i64::from(width)) - x.max(0);
            let h = (y + i64::from(h)).min(i64::from(height)) - y.max(0);
            (w.max(0) * h.max(0)) as u64
        })
        .sum();
    area == u64::from(width) * u64::from(height)
}
/// Make every canvas pixel outside `placed` opaque black. Only needed when a
/// frame came out a pixel short after the canvas was judged fully covered.
fn fill_gaps(canvas: &mut RgbaImage, placed: &[(i64, i64, u32, u32)]) {
    for (x, y, pixel) in canvas.enumerate_pixels_mut() {
        let (x, y) = (i64::from(x), i64::from(y));
        let inside = placed.iter().any(|&(px, py, w, h)| {
            (px..px + i64::from(w)).contains(&x) && (py..py + i64::from(h)).contains(&y)
        });
        if !inside {
            pixel.0[3] = 255;
        }
    }
}
/// Canvas offset of a logical coordinate at canvas scale.
fn offset(at: i32, origin: i32, scale: f64) -> i64 {
    ((f64::from(at) - f64::from(origin)) * scale).round() as i64
}
/// Whether `frame` can be converted straight into its canvas rectangle: CPU
/// pixels, upright, and within one pixel of the rectangle's size.
fn upright(frame: &Frame, size: (u32, u32)) -> bool {
    matches!(frame.storage, FrameStorage::Cpu(_))
        && frame.transform == Transform::Normal
        && !frame.y_inverted
        && near((frame.width, frame.height), size)
}
/// Convert `frame`'s pixels straight into `target` at (`x`, `y`), clipped to
/// the target, row by row, with the same conversion as [`Frame::rgba8`].
fn convert_into(target: &mut RgbaImage, frame: &Frame, x: i64, y: i64) -> Result<()> {
    let FrameStorage::Cpu(buffer) = &frame.storage else {
        return Err(Error::Unsupported("composition of GPU frames"));
    };
    let bytes = buffer.bytes();
    let bpp = frame.format.bytes_per_pixel() as usize;
    let stride = frame.stride as usize;
    if bytes.len() < stride * frame.height as usize || stride < frame.width as usize * bpp {
        return Err(Error::InvalidDimensions);
    }
    let (tw, th) = (i64::from(target.width()), i64::from(target.height()));
    let (left, top) = (x.max(0), y.max(0));
    let right = (x + i64::from(frame.width)).min(tw);
    let bottom = (y + i64::from(frame.height)).min(th);
    if left >= right || top >= bottom {
        return Ok(());
    }
    let pixels = (right - left) as usize;
    let tstride = tw as usize * 4;
    let target = &mut **target;
    for row in top..bottom {
        let s = (row - y) as usize * stride + (left - x) as usize * bpp;
        let t = row as usize * tstride + left as usize * 4;
        crate::buffer::convert_row(
            &bytes[s..s + pixels * bpp],
            &mut target[t..t + pixels * 4],
            frame.format,
        );
    }
    Ok(())
}
/// Copy `source` into `target` at (`x`, `y`), clipped to the target, row by row.
fn blit(target: &mut RgbaImage, source: &RgbaImage, x: i64, y: i64) {
    let (tw, th) = (i64::from(target.width()), i64::from(target.height()));
    let (sw, sh) = (i64::from(source.width()), i64::from(source.height()));
    let (left, top) = (x.max(0), y.max(0));
    let (right, bottom) = ((x + sw).min(tw), (y + sh).min(th));
    if left >= right || top >= bottom {
        return;
    }
    let len = ((right - left) * 4) as usize;
    let (tstride, sstride) = (tw as usize * 4, sw as usize * 4);
    let (target, source) = (&mut **target, &**source);
    for row in top..bottom {
        let t = row as usize * tstride + left as usize * 4;
        let s = (row - y) as usize * sstride + (left - x) as usize * 4;
        target[t..t + len].copy_from_slice(&source[s..s + len]);
    }
}
// image resizes vertically first: source width × destination height, RGBA32F.
fn resize_scratch(source_width: u32, destination_height: u32) -> Result<u64> {
    (u64::from(source_width) * u64::from(destination_height))
        .checked_mul(16)
        .ok_or(Error::LimitExceeded)
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
    fn fractional_scales_round_and_skip_one_pixel_resamples() {
        // 2560x1440 at 1.2 is 2133x1200 logical; 2133 * 1.2 = 2559.6.
        assert_eq!(scaled_size(2133, 1200, 1.2).unwrap(), (2560, 1440));
        assert_eq!(scaled_size(1097, 686, 1.75).unwrap(), (1920, 1201));
        assert!(near((1920, 1200), (1920, 1201)));
        assert!(!near((1920, 1200), (1920, 1202)));
    }
    #[test]
    fn blit_copies_rows_with_clipping_and_coverage_counts_clipped_area() {
        let mut canvas = RgbaImage::new(4, 3);
        let source = RgbaImage::from_fn(3, 2, |x, y| image::Rgba([(1 + x + 3 * y) as u8, 0, 0, 9]));
        blit(&mut canvas, &source, -1, 2);
        let red = |c: &RgbaImage| c.pixels().map(|p| p[0]).collect::<Vec<_>>();
        assert_eq!(red(&canvas), [0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 0, 0]);
        blit(&mut canvas, &source, 9, 9);
        assert_eq!(canvas.get_pixel(0, 2)[3], 9);
        let halves = [(0, 0, 2, 3), (2, 0, 3, 3)];
        assert!(covers(&halves, 4, 3));
        assert!(!covers(&halves[..1], 4, 3));
        // A frame one pixel short leaves its last column opaque black.
        let mut canvas = RgbaImage::new(4, 3);
        fill_gaps(&mut canvas, &[(0, 0, 2, 3), (2, 0, 1, 3)]);
        let alpha: Vec<_> = canvas.pixels().map(|p| p[3]).collect();
        assert_eq!(alpha, [0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0, 255]);
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
