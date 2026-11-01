//! Uncomposed desktop capture: one untouched frame per output plus where it
//! sits on the desktop canvas. Works without the `image` feature.
use super::*;

/// One output's completed frame, untouched: the server's pixel format, no
/// orientation or resize, and `stride == width * 4`.
pub struct DesktopTile {
    /// The frame as captured, including its output snapshot.
    pub frame: Frame,
    /// Left edge of this tile on the canvas, in canvas pixels.
    pub x: u32,
    /// Top edge of this tile on the canvas, in canvas pixels.
    pub y: u32,
}

/// The desktop as tiles at one shared scale. Gaps between outputs are not
/// covered by any tile; the caller fills them.
#[non_exhaustive]
pub struct DesktopTiles {
    /// One tile per output, in output order.
    pub tiles: Vec<DesktopTile>,
    /// Canvas width in pixels, covering `logical_bounds` at `scale`.
    pub width: u32,
    /// Canvas height in pixels, covering `logical_bounds` at `scale`.
    pub height: u32,
    /// Desktop bounds in compositor-global logical units.
    pub logical_bounds: Rect,
    /// Pixels per logical unit, shared by every output.
    pub scale: f64,
    /// The buffer path the tiles came through.
    pub readback: super::TileReadback,
}

impl Connection {
    /// Capture every output on this connection without conversion, orientation,
    /// resizing or composition, as one tile per output.
    ///
    /// Returns `Ok(None)` when tiles cannot represent the desktop as is; the
    /// caller then uses `capture_desktop`. Checked before capturing: every
    /// output has the same scale and a `Normal` transform. Checked after: every
    /// frame is upright (`Normal`, not y-inverted), uses 4 bytes per pixel with
    /// `stride == width * 4`, lives in CPU memory, and matches its output's
    /// scaled logical size within one pixel. `options.limits.max_bytes` is
    /// split evenly between outputs. Errors match `capture_desktop`: no
    /// outputs return [`Error::NoOutputs`], a layout change
    /// [`Error::LayoutChanged`].
    pub fn capture_desktop_tiles(
        &mut self,
        options: &CaptureOptions,
    ) -> Result<Option<DesktopTiles>> {
        check_options(options)?;
        let end = deadline(options)?;
        self.desktop_tiles(
            options,
            end,
            super::TileReadback::Shm,
            |connection, outputs, next, end| connection.capture_batch(outputs, next, end),
        )
    }

    /// The shared tile path: checks, one batch through `capture`, the layout
    /// re-check and the per-frame tile conditions.
    pub(super) fn desktop_tiles(
        &mut self,
        options: &CaptureOptions,
        end: Instant,
        readback: super::TileReadback,
        capture: impl FnOnce(&mut Self, &[Output], &CaptureOptions, Instant) -> Result<Vec<Frame>>,
    ) -> Result<Option<DesktopTiles>> {
        let mut first_options = options.clone();
        first_options.timeout = end.saturating_duration_since(Instant::now());
        let outputs = self.outputs(&first_options)?;
        let first = outputs.first().ok_or(Error::NoOutputs)?;
        let scale = first.scale();
        if !scale.is_finite() || scale <= 0.0 {
            return Err(Error::InvalidDimensions);
        }
        if outputs
            .iter()
            .any(|o| o.scale() != scale || o.transform != Transform::Normal)
        {
            return Ok(None);
        }
        let bounds = Rect::bounds(outputs.iter().map(|o| o.logical))?;
        let (width, height) = scaled_size(bounds.width, bounds.height, scale)?;
        let mut next = options.clone();
        next.timeout = end.saturating_duration_since(Instant::now());
        next.limits.max_bytes = options.limits.max_bytes / outputs.len() as u64;
        let frames = capture(self, &outputs, &next, end)?;
        let mut verify = options.clone();
        verify.timeout = end.saturating_duration_since(Instant::now());
        if self.outputs(&verify)? != outputs {
            return Err(Error::LayoutChanged);
        }
        let mut tiles = Vec::with_capacity(frames.len());
        for (output, frame) in outputs.iter().zip(frames) {
            let expected = scaled_size(output.logical.width, output.logical.height, scale)?;
            let upright = frame.transform == Transform::Normal && !frame.y_inverted;
            let packed = frame.format.bytes_per_pixel() == 4
                && frame.width.checked_mul(4) == Some(frame.stride);
            let cpu = matches!(frame.storage, FrameStorage::Cpu(_));
            if !upright || !packed || !cpu || !near((frame.width, frame.height), expected) {
                return Ok(None);
            }
            tiles.push(DesktopTile {
                x: offset(output.logical.x, bounds.x, scale)?,
                y: offset(output.logical.y, bounds.y, scale)?,
                frame,
            });
        }
        Ok(Some(DesktopTiles {
            tiles,
            width,
            height,
            logical_bounds: bounds,
            scale,
            readback,
        }))
    }
}

/// Canvas offset of a logical coordinate, rounded like the composed path.
fn offset(at: i32, origin: i32, scale: f64) -> Result<u32> {
    let pixels = ((f64::from(at) - f64::from(origin)) * scale).round();
    if !(0.0..=f64::from(i32::MAX)).contains(&pixels) {
        return Err(Error::InvalidDimensions);
    }
    Ok(pixels as u32)
}

/// Rounding can leave the captured buffer one pixel off the scaled logical size;
/// that is placed as is instead of resampled.
pub(crate) fn near(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0.abs_diff(b.0) <= 1 && a.1.abs_diff(b.1) <= 1
}

pub(crate) fn scaled_size(w: u32, h: u32, scale: f64) -> Result<(u32, u32)> {
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
mod thread_safety {
    fn send<T: Send>() {}
    /// Boltsnap keeps a connection in a `static Mutex`, which needs `Send`,
    /// also with the `gpu` and `vulkan` features.
    #[test]
    fn connections_and_frames_are_send() {
        send::<super::Connection>();
        send::<crate::Frame>();
    }
}
