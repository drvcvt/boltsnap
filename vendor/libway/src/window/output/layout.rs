//! Checked tightly packed CPU presentation layout and its SHM pixel format:
//! a plain copy into `ABGR8888`, or the per-pixel swizzle into `ARGB8888`.
use super::{damage::PixelRect, slots::MAX_SHM_BYTES};
use crate::{Error, Result};

/// The `wl_shm` format a CPU buffer is created with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShmFormat {
    /// Always available. Needs a per-pixel swizzle from RGBA.
    Argb8888,
    /// `[31:0] A:B:G:R` little endian, so R, G, B, A in memory: the RGBA input
    /// as is. Used only when advertised and on a little-endian host.
    Abgr8888,
}

impl ShmFormat {
    pub(crate) fn wire(self) -> wayland_client::protocol::wl_shm::Format {
        match self {
            Self::Argb8888 => wayland_client::protocol::wl_shm::Format::Argb8888,
            Self::Abgr8888 => wayland_client::protocol::wl_shm::Format::Abgr8888,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) stride: i32,
    pub(crate) bytes: usize,
    /// Part of the identity: slots of another format are replaced, not reused.
    pub(crate) format: ShmFormat,
}

impl Layout {
    pub(crate) fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidDimensions);
        }
        let stride = width.checked_mul(4).ok_or(Error::LimitExceeded)?;
        let width = i32::try_from(width).map_err(|_| Error::InvalidDimensions)?;
        let height = i32::try_from(height).map_err(|_| Error::InvalidDimensions)?;
        let stride = i32::try_from(stride).map_err(|_| Error::InvalidDimensions)?;
        let bytes = u64::try_from(stride)
            .ok()
            .and_then(|stride| stride.checked_mul(u64::try_from(height).ok()?))
            .ok_or(Error::LimitExceeded)?;
        let bytes = usize::try_from(bytes).map_err(|_| Error::LimitExceeded)?;
        if bytes > MAX_SHM_BYTES || i32::try_from(bytes).is_err() {
            return Err(Error::LimitExceeded);
        }
        Ok(Self {
            width,
            height,
            stride,
            bytes,
            format: ShmFormat::Argb8888,
        })
    }

    pub(crate) fn with_format(self, format: ShmFormat) -> Self {
        Self { format, ..self }
    }

    pub(crate) fn validate(self, rgba: &[u8]) -> Result<()> {
        if rgba.len() != self.bytes {
            return Err(Error::InvalidDimensions);
        }
        Ok(())
    }

    // Input channels are already premultiplied in their encoded transfer space.
    // Packing preserves them, including alpha; it performs no color conversion.
    pub(crate) fn copy_rgba(self, rgba: &[u8], target: &mut [u8]) -> Result<()> {
        self.validate(rgba)?;
        if target.len() != self.bytes {
            return Err(Error::InvalidDimensions);
        }
        self.pack(rgba, target);
        Ok(())
    }

    // Same packing as copy_rgba for the rows of `rect` only; every other byte of
    // `target` is untouched. A rect outside the layout is rejected unwritten.
    pub(crate) fn copy_rgba_rect(
        self,
        rgba: &[u8],
        target: &mut [u8],
        rect: PixelRect,
    ) -> Result<()> {
        self.validate(rgba)?;
        if target.len() != self.bytes {
            return Err(Error::InvalidDimensions);
        }
        let (width, height) = (self.width as u32, self.height as u32);
        if rect.right() > width || rect.bottom() > height {
            return Err(Error::InvalidInput("CPU damage exceeds the buffer"));
        }
        let (stride, x) = (self.stride as usize, rect.x as usize * 4);
        for row in rect.y as usize..rect.bottom() as usize {
            let span = row * stride + x..row * stride + x + rect.width as usize * 4;
            self.pack(&rgba[span.clone()], &mut target[span]);
        }
        Ok(())
    }
}

impl Layout {
    fn pack(self, rgba: &[u8], target: &mut [u8]) {
        match self.format {
            ShmFormat::Abgr8888 => target.copy_from_slice(rgba),
            ShmFormat::Argb8888 => swizzle(rgba, target),
        }
    }
}

fn swizzle(rgba: &[u8], target: &mut [u8]) {
    for (source, target) in rgba
        .as_chunks::<4>()
        .0
        .iter()
        .zip(target.as_chunks_mut::<4>().0)
    {
        let argb = u32::from_be_bytes([source[3], source[0], source[1], source[2]]);
        target.copy_from_slice(&argb.to_ne_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_layout_accepts_4k_and_exact_budget() {
        let layout = Layout::new(3840, 2160).unwrap();
        assert_eq!(layout.stride, 15_360);
        assert_eq!(layout.bytes, 33_177_600);
        assert_eq!(Layout::new(8192, 4096).unwrap().bytes, MAX_SHM_BYTES);
        assert!(matches!(Layout::new(8192, 4097), Err(Error::LimitExceeded)));
    }

    #[test]
    fn malformed_and_unrepresentable_dimensions_are_rejected() {
        for (width, height) in [
            (0, 1),
            (1, 0),
            (u32::MAX, 1),
            (1, u32::MAX),
            (i32::MAX as u32, 1),
            (1, i32::MAX as u32),
        ] {
            assert!(Layout::new(width, height).is_err(), "{width}x{height}");
        }
    }

    #[test]
    fn native_argb_preserves_literal_color_and_alpha_patches() {
        let layout = Layout::new(3, 1).unwrap();
        let input = [0x40, 0x20, 0x10, 0x80, 0xff, 0, 0, 0xff, 0, 0, 0, 0];
        let mut output = [0xcc; 12];
        layout.copy_rgba(&input, &mut output).unwrap();
        let expected = if cfg!(target_endian = "little") {
            [0x10, 0x20, 0x40, 0x80, 0, 0, 0xff, 0xff, 0, 0, 0, 0]
        } else {
            [0x80, 0x40, 0x20, 0x10, 0xff, 0xff, 0, 0, 0, 0, 0, 0]
        };
        assert_eq!(output, expected);
    }

    #[test]
    fn wrong_source_or_destination_length_does_not_write() {
        let layout = Layout::new(1, 1).unwrap();
        for input in [&[1, 2, 3][..], &[1, 2, 3, 4, 5][..]] {
            let mut target = [0xaa; 4];
            assert!(layout.copy_rgba(input, &mut target).is_err());
            assert_eq!(target, [0xaa; 4]);
        }
        let mut target = [0xaa; 3];
        assert!(layout.copy_rgba(&[1, 2, 3, 4], &mut target).is_err());
        assert_eq!(target, [0xaa; 3]);
    }

    #[test]
    fn rect_copy_writes_only_its_rows_and_columns_and_rejects_overflow() {
        let layout = Layout::new(3, 2).unwrap();
        let input: Vec<u8> = (1..=24).collect();
        let mut full = [0; 24];
        layout.copy_rgba(&input, &mut full).unwrap();
        let mut target = [0xcc; 24];
        let rect = PixelRect::new(1, 1, 2, 1);
        layout.copy_rgba_rect(&input, &mut target, rect).unwrap();
        for (index, byte) in target.iter().enumerate() {
            let inside = (16..24).contains(&index);
            assert_eq!(*byte, if inside { full[index] } else { 0xcc }, "{index}");
        }
        let mut target = [0xcc; 24];
        for rect in [PixelRect::new(2, 0, 2, 1), PixelRect::new(0, 1, 1, 2)] {
            assert!(layout.copy_rgba_rect(&input, &mut target, rect).is_err());
        }
        assert_eq!(target, [0xcc; 24]);
    }
}
