use crate::*;
use image::{RgbaImage, imageops};
use std::os::unix::net::UnixStream;

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
    /// Outputs are captured concurrently on separate connections to the same server
    /// socket when its path is known, otherwise sequentially; never atomically. Layout changes return
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
        // Outputs are captured concurrently, so each gets an equal share of the
        // budget left after the canvas.
        let share = options
            .limits
            .max_bytes
            .checked_sub(bytes as u64)
            .ok_or(Error::LimitExceeded)?
            / outputs.len() as u64;
        let placed = match self.peer.clone() {
            Some(peer) if outputs.len() > 1 => {
                // One pending capture per connection: give each output its own
                // connection to the same server.
                let peer = &peer;
                std::thread::scope(|scope| {
                    let workers: Vec<_> = outputs
                        .iter()
                        .map(|output| {
                            scope.spawn(move || {
                                let socket = UnixStream::connect(peer).map_err(Error::Io)?;
                                let mut connection = Connection::from_socket(socket, options)?;
                                let own = connection
                                    .outputs(options)?
                                    .into_iter()
                                    .find(|o| {
                                        Output {
                                            id: output.id,
                                            ..o.clone()
                                        } == *output
                                    })
                                    .ok_or(Error::LayoutChanged)?;
                                connection.place(&own, share, bounds, scale, options, end)
                            })
                        })
                        .collect();
                    workers
                        .into_iter()
                        .map(|worker| {
                            worker.join().unwrap_or(Err(Error::CaptureFailed(
                                "output capture thread panicked".into(),
                            )))
                        })
                        .collect::<Result<Vec<_>>>()
                })?
            }
            _ => outputs
                .iter()
                .map(|output| self.place(output, share, bounds, scale, options, end))
                .collect::<Result<Vec<_>>>()?,
        };
        let mut verify = options.clone();
        verify.timeout = end.saturating_duration_since(std::time::Instant::now());
        if self.outputs(&verify)? != original {
            return Err(Error::LayoutChanged);
        }
        // A single output covering the requested bounds becomes the result
        // directly, avoiding a second full-size allocation and pixel copy.
        let image = match placed.as_slice() {
            [(pixels, 0, 0)] if pixels.dimensions() == (width, height) => {
                placed.into_iter().next().map(|(pixels, _, _)| pixels)
            }
            _ => {
                let mut raw = crate::buffer::zeroed(bytes)?;
                // Desktop gaps are opaque black, independent of export codec.
                if !covers(&placed, width, height) {
                    for p in raw.chunks_exact_mut(4) {
                        p[3] = 255;
                    }
                }
                let mut canvas =
                    RgbaImage::from_raw(width, height, raw).ok_or(Error::InvalidDimensions)?;
                for (pixels, x, y) in &placed {
                    blit(&mut canvas, pixels, *x, *y);
                }
                Some(canvas)
            }
        };
        Ok(DesktopCapture {
            image: image.ok_or(Error::NoOutputs)?,
            outputs,
            logical_bounds: bounds,
            scale,
        })
    }
    /// Capture one output and bring it to canvas scale. Returns the pixels and
    /// their canvas offset.
    fn place(
        &mut self,
        output: &Output,
        share: u64,
        bounds: Rect,
        scale: f64,
        options: &CaptureOptions,
        end: std::time::Instant,
    ) -> Result<(RgbaImage, i64, i64)> {
        let mut next = options.clone();
        next.timeout = end.saturating_duration_since(std::time::Instant::now());
        // Capture, conversion and transform/resize scratch share this output's
        // budget. Limits are deliberately conservative instead of overcommitting.
        next.limits.max_bytes = share / 4;
        let frame = self.capture(output.id, &next)?;
        let (ow, oh) = scaled_size(output.logical.width, output.logical.height, scale)?;
        let output_bytes = next.limits.buffer(
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
        let x = ((f64::from(output.logical.x) - f64::from(bounds.x)) * scale).round() as i64;
        let y = ((f64::from(output.logical.y) - f64::from(bounds.y)) * scale).round() as i64;
        Ok((pixels, x, y))
    }
}
/// Rounding can leave the captured buffer one pixel off the scaled logical size;
/// that is placed as is instead of resampled.
fn near(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1
}
/// Whether the placed images cover every canvas pixel. Outputs do not overlap,
/// so clipped areas summing to the canvas area means full coverage.
fn covers(placed: &[(RgbaImage, i64, i64)], width: u32, height: u32) -> bool {
    let area: u64 = placed
        .iter()
        .map(|(pixels, x, y)| {
            let w = (x + i64::from(pixels.width())).min(i64::from(width)) - x.max(&0);
            let h = (y + i64::from(pixels.height())).min(i64::from(height)) - y.max(&0);
            (w.max(0) * h.max(0)) as u64
        })
        .sum();
    area == u64::from(width) * u64::from(height)
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
    Ok((w.round() as u32, h.round() as u32))
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
        let halves = vec![(RgbaImage::new(2, 3), 0, 0), (RgbaImage::new(3, 3), 2, 0)];
        assert!(covers(&halves, 4, 3));
        assert!(!covers(&halves[..1], 4, 3));
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
