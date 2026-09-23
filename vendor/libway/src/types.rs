use crate::{Error, Result};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputId(pub(crate) u64, pub(crate) u64);

/// Coordinates in compositor-global logical units, never buffer pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}
impl Rect {
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

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}
impl Transform {
    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90 | Self::Rotate270 | Self::Flipped90 | Self::Flipped270
        )
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    pub id: OutputId,
    pub name: String,
    pub description: String,
    pub logical: Rect,
    /// Current mode dimensions in pixels, before output transform.
    pub mode_size: (u32, u32),
    pub transform: Transform,
}
impl Output {
    pub fn scale(&self) -> f64 {
        let (_, h) = self.transform.size(self.mode_size.0, self.mode_size.1);
        f64::from(h) / f64::from(self.logical.height)
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    #[default]
    Auto,
    Ext,
    Wlr,
}
#[derive(Debug, Default, Clone, Copy)]
pub struct Capabilities {
    pub ext_output_capture: bool,
    pub wlr_screencopy_version: u32,
    pub linux_dmabuf_version: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_pixels: u64,
    pub max_bytes: u64,
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

#[derive(Debug, Default, Clone)]
pub struct Cancellation(Arc<AtomicBool>);
impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub backend: Backend,
    pub cursor: bool,
    /// Applies to protocol setup and capture, not only the final copy.
    pub timeout: Duration,
    pub limits: Limits,
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
