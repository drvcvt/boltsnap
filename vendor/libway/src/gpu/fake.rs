//! Test-only dma-bufs from `/dev/udmabuf`: real dma-buf descriptors (`poll`,
//! sync ioctls, `mmap`) over plain memory, standing in for LINEAR GBM
//! buffers where no driver can allocate them. Never read through Vulkan.
use super::*;
use std::os::fd::{AsRawFd, FromRawFd};

/// `_IOW('u', 0x42, struct udmabuf_create)` from `linux/udmabuf.h`.
const UDMABUF_CREATE: libc::c_ulong = 0x4018_7542;
const UDMABUF_FLAGS_CLOEXEC: u32 = 1;

#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

/// A dma-buf of at least `len` bytes, or `None` when `/dev/udmabuf` cannot
/// be used here.
pub(crate) fn udmabuf(len: usize) -> Option<OwnedFd> {
    let size = len.max(1).div_ceil(4096) * 4096;
    let device = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/udmabuf")
        .ok()?;
    // SAFETY: constant NUL-terminated name and valid flags; checked below.
    let raw = unsafe {
        libc::memfd_create(
            c"libway-udmabuf".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if raw < 0 {
        return None;
    }
    // SAFETY: `raw` is a fresh descriptor owned by nobody else.
    let memfd = unsafe { File::from_raw_fd(raw) };
    memfd.set_len(size as u64).ok()?;
    // SAFETY: plain fcntl on an owned memfd; udmabuf requires F_SEAL_SHRINK.
    if unsafe { libc::fcntl(memfd.as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_SHRINK) } < 0 {
        return None;
    }
    let create = UdmabufCreate {
        memfd: memfd.as_raw_fd() as u32,
        flags: UDMABUF_FLAGS_CLOEXEC,
        offset: 0,
        size: size as u64,
    };
    // SAFETY: the ioctl reads one `struct udmabuf_create` from a valid
    // pointer for the duration of the call and returns a new descriptor.
    let fd = unsafe { libc::ioctl(device.as_raw_fd(), UDMABUF_CREATE, &create) };
    // SAFETY: a non-negative result is a fresh descriptor owned by nobody else.
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

/// A LINEAR buffer with a padded stride for the first offered format and
/// modifier the allocator admits, like [`GpuAllocator::allocate`].
pub(super) fn allocate(allocator: &GpuAllocator, c: &Constraints) -> Result<GpuBuffer> {
    let (width, height) = c.size;
    let stride = width.checked_mul(4).ok_or(Error::InvalidDimensions)? + 64;
    for (fourcc, modifiers) in &c.dma {
        let Some(format) = PixelFormat::from_drm(*fourcc) else {
            continue;
        };
        let admitted = allocator.allowed.as_ref().is_none_or(|allowed| {
            allowed.formats.contains(&format) && allowed.modifiers.contains(&0)
        });
        if !admitted || !(modifiers.contains(&0) || c.dma_any_modifier) {
            continue;
        }
        let fd =
            udmabuf(stride as usize * height as usize).ok_or(Error::Unsupported("/dev/udmabuf"))?;
        return Ok(GpuBuffer {
            bo: None,
            device: allocator.device.clone(),
            planes: vec![Plane {
                fd,
                stride,
                offset: 0,
            }],
            format,
            modifier: 0,
        });
    }
    Err(Error::UnsupportedFormat)
}
