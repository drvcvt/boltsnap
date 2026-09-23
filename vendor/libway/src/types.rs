use crate::{Error, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

/// Opaque output identity, valid only on the capture connection that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputId(pub(crate) u64, pub(crate) u64);

/// Coordinates in compositor-global logical units, never buffer pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge in logical units; negative desktop coordinates are valid.
    pub x: i32,
    /// Top edge in logical units.
    pub y: i32,
    /// Horizontal extent in logical units; capture requires 1..=i32::MAX.
    pub width: u32,
    /// Vertical extent in logical units; capture requires 1..=i32::MAX.
    pub height: u32,
}
impl Rect {
    /// Reject zero or protocol-inexpressible extents with [`Error::InvalidDimensions`].
    pub fn validate(self) -> Result<Self> {
        if self.width == 0
            || self.height == 0
            || self.width > i32::MAX as u32
            || self.height > i32::MAX as u32
        {
            return Err(Error::InvalidDimensions);
        }
        Ok(self)
    }
    /// Intersection with exclusive right/bottom edges; disjoint or touching rectangles yield None.
    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = i64::from(self.x).max(i64::from(other.x));
        let y = i64::from(self.y).max(i64::from(other.y));
        let right = (i64::from(self.x) + i64::from(self.width))
            .min(i64::from(other.x) + i64::from(other.width));
        let bottom = (i64::from(self.y) + i64::from(self.height))
            .min(i64::from(other.y) + i64::from(other.height));
        (right > x && bottom > y).then(|| Self {
            x: x as i32,
            y: y as i32,
            width: (right - x) as u32,
            height: (bottom - y) as u32,
        })
    }
    /// Smallest enclosing rectangle. Empty input yields [`Error::NoOutputs`]; invalid
    /// input or an unrepresentable extent yields [`Error::InvalidDimensions`].
    pub fn bounds(rects: impl IntoIterator<Item = Self>) -> Result<Self> {
        let mut it = rects.into_iter();
        let first = it.next().ok_or(Error::NoOutputs)?.validate()?;
        let (mut x, mut y) = (i64::from(first.x), i64::from(first.y));
        let (mut right, mut bottom) = (x + i64::from(first.width), y + i64::from(first.height));
        for r in it {
            r.validate()?;
            x = x.min(i64::from(r.x));
            y = y.min(i64::from(r.y));
            right = right.max(i64::from(r.x) + i64::from(r.width));
            bottom = bottom.max(i64::from(r.y) + i64::from(r.height));
        }
        Self {
            x: x as i32,
            y: y as i32,
            width: u32::try_from(right - x).map_err(|_| Error::InvalidDimensions)?,
            height: u32::try_from(bottom - y).map_err(|_| Error::InvalidDimensions)?,
        }
        .validate()
    }
}

/// Wayland buffer transform; rotations follow the protocol's counterclockwise convention.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// Identity.
    #[default]
    Normal,
    /// Quarter-turn counterclockwise.
    Rotate90,
    /// Half-turn.
    Rotate180,
    /// Three quarter-turns counterclockwise.
    Rotate270,
    /// Reflection about the vertical axis.
    Flipped,
    /// Vertical-axis reflection followed by a quarter-turn.
    Flipped90,
    /// Vertical-axis reflection followed by a half-turn.
    Flipped180,
    /// Vertical-axis reflection followed by three quarter-turns.
    Flipped270,
}
impl Transform {
    /// Whether the transform exchanges horizontal and vertical dimensions.
    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }
    /// Dimensions after applying this transform, in the input's units.
    pub fn size(self, width: u32, height: u32) -> (u32, u32) {
        if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        }
    }
    pub(crate) fn from_wire(v: u32) -> Result<Self> {
        Ok(match v {
            0 => Self::Normal,
            1 => Self::Rotate90,
            2 => Self::Rotate180,
            3 => Self::Rotate270,
            4 => Self::Flipped,
            5 => Self::Flipped90,
            6 => Self::Flipped180,
            7 => Self::Flipped270,
            _ => return Err(Error::CaptureFailed("unknown buffer transform".into())),
        })
    }
}

/// Snapshot of an output's identity and layout. Refresh through the capture connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Connection-local identity, invalid after removal.
    pub id: OutputId,
    /// Compositor-provided name, or a generated fallback when unavailable.
    pub name: String,
    /// Human-readable compositor description; may be empty.
    pub description: String,
    /// Position and size in compositor-global logical units.
    pub logical: Rect,
    /// Current mode dimensions in pixels, before output transform.
    pub mode_size: (u32, u32),
    /// Compositor output transform.
    pub transform: Transform,
}
impl Output {
    /// Transformed mode height divided by logical height, including fractional scaling.
    /// Caller-created outputs need a nonzero logical height for a finite result.
    pub fn scale(&self) -> f64 {
        let (_, h) = self.transform.size(self.mode_size.0, self.mode_size.1);
        f64::from(h) / f64::from(self.logical.height)
    }
}

/// Capture protocol selection, independent of CPU/GPU storage choice.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Prefer EXT, falling back to WLR when unavailable or buffer support is incompatible.
    #[default]
    Auto,
    /// Require ext-image-copy-capture and its output source protocol.
    Ext,
    /// Require wlr-screencopy.
    Wlr,
}
/// Advertised capture capabilities; advertisement does not guarantee buffer compatibility.
#[derive(Debug, Default, Clone, Copy)]
pub struct Capabilities {
    /// Both EXT output-source and image-copy managers are present.
    pub ext_output_capture: bool,
    /// Bound wlr-screencopy version, or zero when absent.
    pub wlr_screencopy_version: u32,
    /// Bound linux-dmabuf version, or zero when absent.
    pub linux_dmabuf_version: u32,
}

/// Capture allocation limits. Defaults: 64 million pixels, 768 MiB, 32 outputs.
/// Zero values are rejected when creating a capture connection or stream.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum pixels per buffer or composed canvas.
    pub max_pixels: u64,
    /// Maximum buffer bytes including row padding; composition also budgets scratch storage.
    pub max_bytes: u64,
    /// Maximum simultaneously advertised outputs retained by the connection.
    pub max_outputs: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_pixels: 64_000_000,
            max_bytes: 768 * 1024 * 1024,
            max_outputs: 32,
        }
    }
}
impl Limits {
    pub(crate) fn buffer(self, width: u32, height: u32, stride: u32, bpp: u32) -> Result<usize> {
        if width == 0
            || height == 0
            || stride == 0
            || width > i32::MAX as u32
            || height > i32::MAX as u32
            || stride > i32::MAX as u32
            || u64::from(stride) < u64::from(width) * u64::from(bpp)
        {
            return Err(Error::InvalidDimensions);
        }
        let bytes = u64::from(stride) * u64::from(height);
        if u64::from(width) * u64::from(height) > self.max_pixels
            || bytes > self.max_bytes
            || bytes > i32::MAX as u64
        {
            return Err(Error::LimitExceeded);
        }
        usize::try_from(bytes).map_err(|_| Error::LimitExceeded)
    }
}

/// Shared cancellation flag. Clones observe the same irreversible cancellation.
/// Capture polls the flag between protocol waits; cancelling does not itself wake a poll.
#[derive(Debug, Default, Clone)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    /// Request cancellation of operations using this token or any clone.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Capture policy. Defaults to automatic backend, no cursor, and a five-second timeout.
#[derive(Debug, Clone)]
pub struct CaptureOptions {
    /// Required protocol or automatic fallback policy.
    pub backend: Backend,
    /// Ask the compositor to paint cursors into output frames; false by default.
    pub cursor: bool,
    /// Applies to protocol setup and capture, not only the final copy.
    pub timeout: Duration,
    /// Resource bounds applied to advertised outputs and allocated storage.
    pub limits: Limits,
    /// Shared token checked during waits; a cancelled token cannot be reset.
    pub cancellation: Cancellation,
}
impl Default for CaptureOptions {
    fn default() -> Self {
        Self {
            backend: Backend::Auto,
            cursor: false,
            timeout: Duration::from_secs(5),
            limits: Limits::default(),
            cancellation: Cancellation::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounds_and_intersections_do_not_overflow_signed_coordinates() {
        let left = Rect {
            x: -1920,
            y: -200,
            width: 1920,
            height: 1080,
        };
        let right = Rect {
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
        };
        assert_eq!(
            Rect::bounds([left, right]).unwrap(),
            Rect {
                x: -1920,
                y: -200,
                width: 4480,
                height: 1640
            }
        );
        assert!(left.intersection(right).is_none());
        assert!(
            Rect::bounds([
                Rect {
                    x: i32::MIN,
                    ..left
                },
                Rect {
                    x: i32::MAX,
                    ..right
                }
            ])
            .is_err()
        );
        assert!(Rect::bounds([]).is_err());
        let edge = Rect {
            x: i32::MAX,
            y: 0,
            width: 100,
            height: 2,
        };
        assert_eq!(edge.intersection(edge), Some(edge));
    }
    #[test]
    fn limits_cover_pixels_stride_bytes_and_protocol_integer_range() {
        let l = Limits {
            max_pixels: 100,
            max_bytes: 1000,
            max_outputs: 1,
        };
        assert!(l.buffer(10, 10, 40, 4).is_ok());
        assert!(matches!(l.buffer(10, 11, 40, 4), Err(Error::LimitExceeded)));
        assert!(matches!(
            l.buffer(10, 10, 101, 4),
            Err(Error::LimitExceeded)
        ));
        assert!(matches!(
            l.buffer(10, 10, 39, 4),
            Err(Error::InvalidDimensions)
        ));
        assert!(l.buffer(1, 1, u32::MAX, 4).is_err());
    }
}
