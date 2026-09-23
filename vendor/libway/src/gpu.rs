//! Optional GBM allocator. Open a render node explicitly; no modesetting, device
//! guessing, EGL context, or CPU readback is performed. Objects are thread-local.
use crate::{Error, Limits, PixelFormat, Result, capture::Constraints};
use std::{
    fs::{File, OpenOptions},
    os::{
        fd::{AsFd, BorrowedFd, OwnedFd},
        unix::fs::{FileTypeExt, MetadataExt},
    },
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

/// Explicit GBM device shared by its allocations. Thread-local (`!Send`/`!Sync`);
/// create and use it on the capture/import thread. Requires `gpu` and system GBM.
#[derive(Clone)]
pub struct GpuAllocator {
    device: Rc<gbm::Device<Arc<File>>>,
    device_id: u64,
}
impl GpuAllocator {
    /// Open a caller-selected DRM render node for read/write access and initialize GBM.
    /// File/device failures yield [`Error::Io`]; non-device paths yield [`Error::Unsupported`].
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let meta = file.metadata()?;
        if !meta.file_type().is_char_device() {
            return Err(Error::Unsupported("GPU allocator requires a DRM device"));
        }
        let device_id = meta.rdev();
        Ok(Self {
            device: Rc::new(gbm::Device::new(Arc::new(file))?),
            device_id,
        })
    }
    /// Device number (`st_rdev`) used to match compositor DMA-BUF device constraints.
    pub fn device_id(&self) -> u64 {
        self.device_id
    }
    pub(crate) fn allocate(
        &self,
        c: &Constraints,
        global: &[(u32, u64)],
        limits: Limits,
    ) -> Result<GpuBuffer> {
        let (width, height) = c.size;
        limits.buffer(
            width,
            height,
            width.checked_mul(4).ok_or(Error::InvalidDimensions)?,
            4,
        )?;
        if let Some(device) = c.device
            && !same_device(device, self.device_id)
        {
            return Err(Error::Unsupported(
                "capture source requires a different DRM device",
            ));
        }
        for (fourcc, mods) in &c.dma {
            let Some(format) = PixelFormat::from_drm(*fourcc) else {
                continue;
            };
            let Ok(gbm_format) = gbm::Format::try_from(*fourcc) else {
                continue;
            };
            let candidates = modifiers(*fourcc, mods, c.dma_any_modifier, global);
            if candidates.is_empty() {
                continue;
            }
            let explicit: Vec<_> = candidates
                .iter()
                .copied()
                .filter(|m| *m != u64::from(gbm::Modifier::Invalid))
                .map(gbm::Modifier::from)
                .collect();
            let mut bo = if explicit.is_empty() {
                None
            } else {
                self.device
                    .create_buffer_object_with_modifiers2::<()>(
                        width,
                        height,
                        gbm_format,
                        explicit.into_iter(),
                        gbm::BufferObjectFlags::RENDERING,
                    )
                    .ok()
            };
            if bo.is_none() && candidates.contains(&u64::from(gbm::Modifier::Invalid)) {
                bo = self
                    .device
                    .create_buffer_object::<()>(
                        width,
                        height,
                        gbm_format,
                        gbm::BufferObjectFlags::RENDERING,
                    )
                    .ok();
            }
            let Some(bo) = bo else { continue };
            let actual = u64::from(bo.modifier());
            // Implicit layout is only sent when explicitly advertised, never disguised
            // as linear. Otherwise the allocated modifier must match the intersection.
            let modifier = if candidates.contains(&actual) {
                actual
            } else if candidates.contains(&u64::from(gbm::Modifier::Invalid)) {
                u64::from(gbm::Modifier::Invalid)
            } else {
                continue;
            };
            let count = bo.plane_count();
            if count == 0 || count > 4 {
                continue;
            }
            let mut planes = Vec::new();
            let mut estimated = 0_u64;
            for i in 0..count {
                let stride = bo.stride_for_plane(i as i32);
                let offset = bo.offset(i as i32);
                if stride == 0 {
                    return Err(Error::InvalidDimensions);
                }
                estimated = estimated
                    .checked_add(u64::from(offset) + u64::from(stride) * u64::from(height))
                    .ok_or(Error::LimitExceeded)?;
                let fd = bo
                    .fd_for_plane(i as i32)
                    .map_err(|e| Error::CaptureFailed(format!("export DMA-BUF plane: {e}")))?;
                planes.push(Plane { fd, stride, offset });
            }
            if estimated > limits.max_bytes {
                return Err(Error::LimitExceeded);
            }
            return Ok(GpuBuffer {
                bo,
                _device: self.device.clone(),
                planes,
                format,
                modifier,
            });
        }
        Err(Error::UnsupportedFormat)
    }
}
fn modifiers(format: u32, source: &[u64], unrestricted: bool, global: &[(u32, u64)]) -> Vec<u64> {
    let mut result = Vec::new();
    for &(f, m) in global {
        if f == format && (unrestricted || source.contains(&m)) && !result.contains(&m) {
            result.push(m);
        }
    }
    result
}
fn device_path(device: u64) -> PathBuf {
    PathBuf::from(format!(
        "/sys/dev/char/{}:{}/device",
        libc::major(device),
        libc::minor(device)
    ))
}
fn same_device(a: u64, b: u64) -> bool {
    if a == b {
        return true;
    }
    match (device_path(a).canonicalize(), device_path(b).canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}
/// A borrowed plane descriptor is valid while its owning GPU frame remains alive.
/// Consumers importing a frame asynchronously must retain the frame until their
/// GPU/encoder completes. libway never reuses a published GPU allocation.
pub struct Plane {
    fd: OwnedFd,
    stride: u32,
    offset: u32,
}
impl Plane {
    /// Borrow the owned DMA-BUF descriptor. Duplicate it if ownership must be transferred;
    /// retaining only the borrowed descriptor does not retain the allocation.
    pub fn fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
    /// Row stride in bytes for this plane.
    pub fn stride(&self) -> u32 {
        self.stride
    }
    /// Byte offset of this plane within its descriptor.
    pub fn offset(&self) -> u32 {
        self.offset
    }
}
/// Owned thread-local GBM allocation, device reference and exported plane descriptors.
/// Keep the whole buffer alive until asynchronous GPU/encoder use has completed.
pub struct GpuBuffer {
    // BO must be destroyed before the device's last File handle closes.
    bo: gbm::BufferObject<()>,
    _device: Rc<gbm::Device<Arc<File>>>,
    planes: Vec<Plane>,
    format: PixelFormat,
    modifier: u64,
}
impl GpuBuffer {
    /// Exported planes in import order; descriptors remain owned by this buffer.
    pub fn planes(&self) -> &[Plane] {
        &self.planes
    }
    /// DRM packed pixel format of the allocation.
    pub fn format(&self) -> PixelFormat {
        self.format
    }
    /// Modifier used for Wayland import (may be INVALID for implicit layouts).
    pub fn modifier(&self) -> u64 {
        self.modifier
    }
    /// Actual modifier reported by GBM, which may differ from the implicit import modifier.
    pub fn allocation_modifier(&self) -> u64 {
        self.bo.modifier().into()
    }
    /// Allocation width and height in buffer pixels before transforms.
    pub fn dimensions(&self) -> (u32, u32) {
        (self.bo.width(), self.bo.height())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modifier_negotiation_never_invents_linear_or_implicit_support() {
        let f = PixelFormat::Xrgb8888 as u32;
        let implicit = u64::from(gbm::Modifier::Invalid);
        let global = [(f, 0), (f, 7), (f, 7), (f, implicit), (f + 1, 9)];
        assert_eq!(modifiers(f, &[7, 9], false, &global), [7]);
        assert!(modifiers(f, &[], false, &global).is_empty());
        assert_eq!(modifiers(f, &[], true, &global), [0, 7, implicit]);
        assert_eq!(modifiers(f, &[implicit], false, &global), [implicit]);
        assert!(modifiers(f, &[12], false, &global).is_empty());
    }
}

#[cfg(test)]
mod hardware_tests {
    use super::*;
    #[test]
    #[ignore = "requires LIBWAY_TEST_RENDER_NODE; allocates only, never captures the desktop"]
    fn gbm_allocations_export_owned_planes() {
        let path =
            std::env::var_os("LIBWAY_TEST_RENDER_NODE").expect("set an explicit render node");
        let allocator = GpuAllocator::open(path).unwrap();
        let format = PixelFormat::Xrgb8888 as u32;
        let implicit = u64::from(gbm::Modifier::Invalid);
        let constraints = Constraints {
            size: (64, 32),
            dma: vec![(format, vec![0, implicit])],
            device: Some(allocator.device_id()),
            ..Default::default()
        };
        let buffer = allocator
            .allocate(
                &constraints,
                &[(format, 0), (format, implicit)],
                Limits::default(),
            )
            .unwrap();
        // Import the exported allocation through a second GBM device. This is a
        // real driver round trip, independent of Wayland and desktop contents.
        let import_file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(std::env::var_os("LIBWAY_TEST_RENDER_NODE").unwrap())
            .unwrap();
        let importer = gbm::Device::new(import_file).unwrap();
        let mut fds = [None; 4];
        let mut strides = [0; 4];
        let mut offsets = [0; 4];
        for (i, p) in buffer.planes().iter().enumerate() {
            fds[i] = Some(p.fd());
            strides[i] = p.stride() as i32;
            offsets[i] = p.offset() as i32;
        }
        let imported = importer
            .import_buffer_object_from_dma_buf_with_modifiers::<()>(
                buffer.planes().len() as u32,
                fds,
                64,
                32,
                gbm::Format::Xrgb8888,
                gbm::BufferObjectFlags::RENDERING,
                strides,
                offsets,
                gbm::Modifier::from(buffer.allocation_modifier()),
            )
            .unwrap();
        assert_eq!((imported.width(), imported.height()), (64, 32));
        drop(imported);
        drop(allocator);
        assert_eq!(buffer.dimensions(), (64, 32));
        assert!(!buffer.planes().is_empty());
        for plane in buffer.planes() {
            assert!(plane.stride() > 0);
            let fd = plane.fd().try_clone_to_owned().unwrap();
            let file = File::from(fd);
            assert!(file.metadata().is_ok());
        }
        eprintln!(
            "GBM export: {} planes, modifier {:#x}, actual {:#x}",
            buffer.planes().len(),
            buffer.modifier(),
            buffer.allocation_modifier()
        );
    }
}
