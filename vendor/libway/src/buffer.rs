use crate::{Backend, Error, Limits, Output, Result, Transform};
use memmap2::{MmapMut, MmapOptions};
use std::{
    fs::File,
    os::fd::{AsFd, FromRawFd},
};

/// Packed DRM formats. SHM's special ARGB/XRGB values are translated internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    Argb8888 = 0x34325241,
    Xrgb8888 = 0x34325258,
    Abgr8888 = 0x34324241,
    Xbgr8888 = 0x34324258,
    Rgb888 = 0x34324752,
    Bgr888 = 0x34324742,
    Argb2101010 = 0x30335241,
    Xrgb2101010 = 0x30335258,
    Abgr2101010 = 0x30334241,
    Xbgr2101010 = 0x30334258,
}
impl PixelFormat {
    pub fn bytes_per_pixel(self) -> u32 {
        if matches!(self, Self::Rgb888 | Self::Bgr888) {
            3
        } else {
            4
        }
    }
    pub(crate) fn from_shm(value: u32) -> Option<Self> {
        Self::from_drm(match value {
            0 => Self::Argb8888 as u32,
            1 => Self::Xrgb8888 as u32,
            v => v,
        })
    }
    pub(crate) fn from_drm(v: u32) -> Option<Self> {
        Some(match v {
            0x34325241 => Self::Argb8888,
            0x34325258 => Self::Xrgb8888,
            0x34324241 => Self::Abgr8888,
            0x34324258 => Self::Xbgr8888,
            0x34324752 => Self::Rgb888,
            0x34324742 => Self::Bgr888,
            0x30335241 => Self::Argb2101010,
            0x30335258 => Self::Xrgb2101010,
            0x30334241 => Self::Abgr2101010,
            0x30334258 => Self::Xbgr2101010,
            _ => return None,
        })
    }
}

pub(crate) struct ShmBuffer {
    pub file: File,
    pub map: MmapMut,
}
impl ShmBuffer {
    pub fn new(bytes: usize) -> Result<Self> {
        // SAFETY: constant NUL-terminated name and valid flags. The new FD is owned once.
        let fd = unsafe {
            libc::memfd_create(
                c"libway-frame".as_ptr(),
                libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        let file = unsafe { File::from_raw_fd(fd) };
        file.set_len(bytes as u64)?;
        // Prevent accidental shrinking (SIGBUS), but leave writes available to the compositor.
        use std::os::fd::AsRawFd;
        if unsafe {
            libc::fcntl(
                file.as_raw_fd(),
                libc::F_ADD_SEALS,
                libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL,
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        // SAFETY: this library owns a sized, sealed memfd. No Rust references to its bytes
        // are exposed until the compositor has sent the capture completion event.
        let map = unsafe { MmapOptions::new().len(bytes).map_mut(&file)? };
        Ok(Self { file, map })
    }
}

/// CPU storage is immutable and only accessible after successful capture completion.
/// GPU storage retains its device, allocation and exported plane descriptors.
pub enum FrameStorage {
    Cpu(CpuBuffer),
    #[cfg(feature = "gpu")]
    Gpu(crate::gpu::GpuBuffer),
}
pub struct CpuBuffer {
    pub(crate) shm: ShmBuffer,
}
impl CpuBuffer {
    pub fn bytes(&self) -> &[u8] {
        &self.shm.map
    }
}

/// A completed frame. Times are compositor presentation timestamps, not wall clock.
/// No color space is implied by the pixel format. Capture protocols used here do not
/// provide enough color metadata to promise HDR conversion or sRGB tagging.
pub struct Frame {
    pub output: Output,
    pub backend: Backend,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: PixelFormat,
    pub transform: Transform,
    pub y_inverted: bool,
    pub presentation_time: Option<std::time::Duration>,
    pub damage: Vec<(u32, u32, u32, u32)>,
    pub storage: FrameStorage,
}
impl Frame {
    /// Copy CPU pixels into tightly packed RGBA8 in the buffer's original orientation.
    /// Alpha is kept premultiplied as in wl_shm; X channels are normalized to opaque.
    pub fn rgba8(&self) -> Result<Vec<u8>> {
        match &self.storage {
            FrameStorage::Cpu(b) => {
                convert_rgba(b.bytes(), self.width, self.height, self.stride, self.format)
            }
            #[cfg(feature = "gpu")]
            FrameStorage::Gpu(_) => Err(Error::Unsupported(
                "GPU readback; import DMA-BUF planes into your graphics/encoder API",
            )),
        }
    }
}

pub(crate) fn convert_rgba(
    bytes: &[u8],
    w: u32,
    h: u32,
    stride: u32,
    format: PixelFormat,
) -> Result<Vec<u8>> {
    let bpp = format.bytes_per_pixel() as usize;
    let size = Limits {
        max_pixels: u64::MAX,
        max_bytes: i32::MAX as u64,
        max_outputs: usize::MAX,
    }
    .buffer(w, h, stride, bpp as u32)?;
    if bytes.len() < size {
        return Err(Error::InvalidDimensions);
    }
    let len = (w as usize)
        .checked_mul(h as usize)
        .and_then(|n| n.checked_mul(4))
        .ok_or(Error::LimitExceeded)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len)
        .map_err(|_| Error::LimitExceeded)?;
    if matches!(
        format,
        PixelFormat::Argb8888
            | PixelFormat::Xrgb8888
            | PixelFormat::Abgr8888
            | PixelFormat::Xbgr8888
    ) {
        out.resize(len, 0);
        let swapped = matches!(format, PixelFormat::Argb8888 | PixelFormat::Xrgb8888);
        let alpha = matches!(format, PixelFormat::Argb8888 | PixelFormat::Abgr8888);
        let (r, b) = if swapped { (2, 0) } else { (0, 2) };
        for (source, target) in bytes[..size]
            .chunks_exact(stride as usize)
            .zip(out.chunks_exact_mut(w as usize * 4))
        {
            for (p, rgba) in source[..w as usize * 4]
                .chunks_exact(4)
                .zip(target.chunks_exact_mut(4))
            {
                rgba.copy_from_slice(&[p[r], p[1], p[b], if alpha { p[3] } else { 255 }]);
            }
        }
        return Ok(out);
    }
    for row in bytes[..size].chunks_exact(stride as usize) {
        for p in row[..w as usize * bpp].chunks_exact(bpp) {
            let rgba = match format {
                PixelFormat::Bgr888 => [p[0], p[1], p[2], 255],
                PixelFormat::Rgb888 => [p[2], p[1], p[0], 255],
                f => {
                    let v = u32::from_le_bytes(p.try_into().map_err(|_| Error::InvalidDimensions)?);
                    match f {
                        PixelFormat::Argb8888 | PixelFormat::Xrgb8888 => [
                            (v >> 16) as u8,
                            (v >> 8) as u8,
                            v as u8,
                            if f == PixelFormat::Argb8888 {
                                (v >> 24) as u8
                            } else {
                                255
                            },
                        ],
                        PixelFormat::Abgr8888 | PixelFormat::Xbgr8888 => [
                            v as u8,
                            (v >> 8) as u8,
                            (v >> 16) as u8,
                            if f == PixelFormat::Abgr8888 {
                                (v >> 24) as u8
                            } else {
                                255
                            },
                        ],
                        _ => {
                            let low = ((v & 1023) >> 2) as u8;
                            let green = (((v >> 10) & 1023) >> 2) as u8;
                            let high = (((v >> 20) & 1023) >> 2) as u8;
                            let a =
                                if matches!(f, PixelFormat::Argb2101010 | PixelFormat::Abgr2101010)
                                {
                                    ((v >> 30) * 85) as u8
                                } else {
                                    255
                                };
                            if matches!(f, PixelFormat::Argb2101010 | PixelFormat::Xrgb2101010) {
                                [high, green, low, a]
                            } else {
                                [low, green, high, a]
                            }
                        }
                    }
                }
            };
            out.extend_from_slice(&rgba);
        }
    }
    Ok(out)
}

pub(crate) fn create_wl_shm(
    shm: &wayland_client::protocol::wl_shm::WlShm,
    qh: &wayland_client::QueueHandle<crate::capture::State>,
    buffer: &ShmBuffer,
    width: u32,
    height: u32,
    stride: u32,
    wire_format: wayland_client::protocol::wl_shm::Format,
) -> wayland_client::protocol::wl_buffer::WlBuffer {
    let pool = shm.create_pool(buffer.file.as_fd(), buffer.map.len() as i32, qh, ());
    let wl = pool.create_buffer(
        0,
        width as i32,
        height as i32,
        stride as i32,
        wire_format,
        qh,
        (),
    );
    pool.destroy();
    wl
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_formats_padding_alpha_and_ten_bit_channels() {
        for (format, word, expected) in [
            (PixelFormat::Argb8888, 0x80102030u32, [16, 32, 48, 128]),
            (PixelFormat::Xrgb8888, 0x00102030, [16, 32, 48, 255]),
            (PixelFormat::Abgr8888, 0x80302010, [16, 32, 48, 128]),
            (PixelFormat::Xbgr8888, 0x00302010, [16, 32, 48, 255]),
            (
                PixelFormat::Argb2101010,
                (2 << 30) | (1023 << 20) | (512 << 10) | 4,
                [255, 128, 1, 170],
            ),
            (
                PixelFormat::Xrgb2101010,
                (1023 << 20) | (512 << 10) | 4,
                [255, 128, 1, 255],
            ),
            (
                PixelFormat::Abgr2101010,
                (2 << 30) | (4 << 20) | (512 << 10) | 1023,
                [255, 128, 1, 170],
            ),
            (
                PixelFormat::Xbgr2101010,
                (4 << 20) | (512 << 10) | 1023,
                [255, 128, 1, 255],
            ),
        ] {
            let mut bytes = word.to_le_bytes().to_vec();
            bytes.extend_from_slice(&[222; 4]);
            assert_eq!(
                convert_rgba(&bytes, 1, 1, 8, format).unwrap(),
                expected,
                "{format:?}"
            );
        }
        assert_eq!(
            convert_rgba(&[48, 32, 16, 0], 1, 1, 4, PixelFormat::Rgb888).unwrap(),
            [16, 32, 48, 255]
        );
        assert_eq!(
            convert_rgba(&[16, 32, 48, 0], 1, 1, 4, PixelFormat::Bgr888).unwrap(),
            [16, 32, 48, 255]
        );
    }
    #[test]
    fn malformed_buffers_are_rejected_before_reading() {
        for (w, h, stride, len) in [
            (0, 1, 4, 4),
            (1, 0, 4, 4),
            (2, 1, 4, 8),
            (2, 2, 8, 15),
            (u32::MAX, 1, 4, 4),
        ] {
            assert!(convert_rgba(&vec![0; len], w, h, stride, PixelFormat::Xrgb8888).is_err());
        }
    }
    #[test]
    fn shared_storage_cannot_be_resized() {
        let shm = ShmBuffer::new(32).unwrap();
        assert!(shm.file.set_len(16).is_err());
        assert!(shm.file.set_len(64).is_err());
        assert_eq!(shm.map.len(), 32);
    }
}
