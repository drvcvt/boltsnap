//! One dma-buf read back to the CPU: implicit-fence wait, import, copy, map.
use super::{device::VulkanReadback, host::HostBuffer, vk_format};
use crate::gpu::linear::wait_readable;
use crate::{CpuBuffer, capture::ReadbackFallback, gpu::GpuBuffer};
use ash::vk;
use std::{
    os::fd::{AsRawFd, IntoRawFd},
    time::Instant,
};

/// Old layout of the acquire of a freshly imported image. External images
/// must be created UNDEFINED (VUID-VkImageCreateInfo-pNext-01443), and the
/// old layout must be UNDEFINED or the current one (VUID-01197). UNDEFINED
/// formally allows discarding contents; single-plane modifiers carry no
/// metadata to reset, and the hardware test checks the bytes survive.
pub(super) const FRESH_LAYOUT: vk::ImageLayout = vk::ImageLayout::UNDEFINED;

/// `DRM_FORMAT_MOD_INVALID`: an implicit layout Vulkan cannot import.
const MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// The single mip level and layer of a color image.
pub(super) const COLOR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

impl VulkanReadback {
    /// Read one completed dma-buf back as tightly packed `width * height * 4`
    /// bytes in the buffer's own byte order (B,G,R,A/X; X is kept as is):
    /// leased host memory when it is cached and a lease is free, else a heap
    /// copy.
    pub(crate) fn read(
        &mut self,
        buffer: &GpuBuffer,
        width: u32,
        height: u32,
        end: Instant,
    ) -> Result<CpuBuffer, ReadbackFallback> {
        if self.lost.get() {
            return Err(ReadbackFallback::Copy("the Vulkan device is lost".into()));
        }
        self.importable(buffer, width, height, vk::ImageUsageFlags::TRANSFER_SRC)?;
        let len = usize::try_from(u64::from(width) * u64::from(height) * 4)
            .map_err(|_| ReadbackFallback::Allocation("size overflow".into()))?;
        let fd = buffer
            .planes()
            .first()
            .ok_or_else(|| ReadbackFallback::Import("no plane".into()))?
            .fd();
        wait_readable(fd, end)?;
        let host = self.host.take(&self.device, &self.memory, len as u64)?;
        let image = match self.import(buffer, width, height, vk::ImageUsageFlags::TRANSFER_SRC) {
            Ok(image) => image,
            Err(error) => {
                self.host.give(&self.device, host);
                return Err(error);
            }
        };
        let family = self.family;
        let submitted = self.submit(
            |device, command| {
                let acquire = vk::ImageMemoryBarrier::default()
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(FRESH_LAYOUT)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                    .dst_queue_family_index(family)
                    .image(image.image)
                    .subresource_range(COLOR);
                let region = vk::BufferImageCopy::default()
                    .buffer_row_length(width)
                    .buffer_image_height(height)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    });
                let visible = vk::BufferMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::HOST_READ)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .buffer(host.buffer)
                    .size(vk::WHOLE_SIZE);
                // SAFETY: `command` is recording; the image is bound to its
                // imported memory and created with TRANSFER_SRC, the buffer
                // with TRANSFER_DST and at least `len` bytes, which is exactly
                // what the region covers (row length = width, 4 bytes/texel).
                unsafe {
                    device.cmd_pipeline_barrier(
                        command,
                        vk::PipelineStageFlags::TOP_OF_PIPE,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[],
                        &[acquire],
                    );
                    device.cmd_copy_image_to_buffer(
                        command,
                        image.image,
                        vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                        host.buffer,
                        &[region],
                    );
                    device.cmd_pipeline_barrier(
                        command,
                        vk::PipelineStageFlags::TRANSFER,
                        vk::PipelineStageFlags::HOST,
                        vk::DependencyFlags::empty(),
                        &[],
                        &[visible],
                        &[],
                    );
                }
            },
            end,
        );
        if self.lost.get() {
            // Submitted work may still use the image and the host buffer:
            // leak both (fail closed).
            std::mem::forget(image);
            let _leaked = host; // no `Drop`: buffer and memory stay allocated
            // Fail closed for good: whatever a later wait returns, the device
            // is never destroyed with these children alive, and leases out
            // now leak too.
            self.owner.leak();
            return Err(submitted
                .err()
                .unwrap_or_else(|| ReadbackFallback::Copy("the Vulkan device is lost".into())));
        }
        // Not lost: the submission completed or was drained.
        drop(image);
        if let Err(error) = submitted {
            self.host.give(&self.device, host);
            return Err(error);
        }
        self.finish(host, len)
    }

    /// Hand the written host buffer out: as a lease when its memory is
    /// cached and the lease cap allows, else as a heap copy.
    fn finish(&self, host: HostBuffer, len: usize) -> Result<CpuBuffer, ReadbackFallback> {
        if let Err(error) = host.invalidate(&self.device) {
            self.host.give(&self.device, host);
            return Err(error);
        }
        if host.cached && self.host.reserve_lease() {
            return Ok(CpuBuffer::leased(self.host.lease(host, len, &self.owner)));
        }
        let bytes = host.bytes(len).to_vec();
        self.host.give(&self.device, host);
        Ok(CpuBuffer::heap(bytes))
    }

    /// Check that `buffer` can be imported as a `width` x `height` image
    /// with `usage`; returns its Vulkan format and modifier.
    pub(super) fn importable(
        &self,
        buffer: &GpuBuffer,
        width: u32,
        height: u32,
        usage: vk::ImageUsageFlags,
    ) -> Result<(vk::Format, u64), ReadbackFallback> {
        let format = vk_format(buffer.format())
            .ok_or(ReadbackFallback::UnsupportedFormat(buffer.format()))?;
        if buffer.planes().len() != 1 {
            return Err(ReadbackFallback::Import(format!(
                "{} planes, only one is supported",
                buffer.planes().len()
            )));
        }
        let modifier = buffer.modifier();
        if modifier == MOD_INVALID {
            return Err(ReadbackFallback::Import("implicit modifier".into()));
        }
        let (full_width, full_height) = buffer.dimensions();
        if width == 0 || height == 0 || width > full_width || height > full_height {
            return Err(ReadbackFallback::Import(format!(
                "{width}x{height} does not fit the {full_width}x{full_height} buffer"
            )));
        }
        let max = self.max_extent(format, modifier, usage).ok_or_else(|| {
            ReadbackFallback::Import(format!("modifier {modifier:#x} is not importable"))
        })?;
        if width > max.width || height > max.height {
            return Err(ReadbackFallback::Import(format!(
                "{width}x{height} exceeds {}x{}",
                max.width, max.height
            )));
        }
        Ok((format, modifier))
    }

    /// Import the single plane of `buffer` as a bound image. The plane fd is
    /// duplicated; the duplicate belongs to Vulkan only once the memory exists.
    pub(super) fn import(
        &self,
        buffer: &GpuBuffer,
        width: u32,
        height: u32,
        usage: vk::ImageUsageFlags,
    ) -> Result<Imported<'_>, ReadbackFallback> {
        let import =
            |what: &str, error: vk::Result| ReadbackFallback::Import(format!("{what}: {error}"));
        let (format, modifier) = self.importable(buffer, width, height, usage)?;
        let plane = buffer
            .planes()
            .first()
            .ok_or_else(|| ReadbackFallback::Import("no plane".into()))?;
        let layout = [vk::SubresourceLayout::default()
            .offset(u64::from(plane.offset()))
            .row_pitch(u64::from(plane.stride()))];
        let mut explicit = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
            .drm_format_modifier(modifier)
            .plane_layouts(&layout);
        let mut external = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(format)
            .extent(vk::Extent3D {
                width,
                height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(FRESH_LAYOUT)
            .push_next(&mut external)
            .push_next(&mut explicit);
        // F_DUPFD_CLOEXEC; closed on every return below unless Vulkan took it.
        let fd = plane
            .fd()
            .try_clone_to_owned()
            .map_err(|error| ReadbackFallback::Import(format!("dup: {error}")))?;
        // SAFETY: `importable` confirmed the device can import this format,
        // modifier, usage and extent from a dma-buf; the chained structs and
        // the one-entry plane layout outlive the call.
        let image = unsafe { self.device.create_image(&info, None) }
            .map_err(|error| import("vkCreateImage", error))?;
        let mut imported = Imported {
            device: &self.device,
            image,
            memory: vk::DeviceMemory::null(),
        };
        // SAFETY: `image` is a live image of this device.
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let mut fd_props = vk::MemoryFdPropertiesKHR::default();
        // SAFETY: `fd` is an open descriptor owned by this function; the call
        // only inspects it.
        unsafe {
            self.memory_fd.get_memory_fd_properties(
                vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT,
                fd.as_raw_fd(),
                &mut fd_props,
            )
        }
        .map_err(|error| import("vkGetMemoryFdPropertiesKHR", error))?;
        let types = requirements.memory_type_bits & fd_props.memory_type_bits;
        if types == 0 {
            return Err(ReadbackFallback::Import("no shared memory type".into()));
        }
        let mut fd_info = vk::ImportMemoryFdInfoKHR::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
            .fd(fd.as_raw_fd());
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(types.trailing_zeros())
            .push_next(&mut fd_info)
            .push_next(&mut dedicated);
        // SAFETY: the memory type is allowed for both the image and the fd,
        // the dedicated image is unbound, and `fd` stays open for the call. On
        // success Vulkan owns `fd`; on failure it does not, and `fd` is closed
        // when it drops.
        imported.memory = unsafe { self.device.allocate_memory(&allocate, None) }
            .map_err(|error| import("vkAllocateMemory", error))?;
        let _owned_by_vulkan = fd.into_raw_fd();
        // SAFETY: dedicated memory of `image`, bound once at offset 0.
        unsafe { self.device.bind_image_memory(image, imported.memory, 0) }
            .map_err(|error| import("vkBindImageMemory", error))?;
        Ok(imported)
    }

    /// Record with `record`, submit, and wait for the fence until `end`. On
    /// any failure after submitting, the queue is drained first, so the caller
    /// may destroy what the commands used. When it cannot be drained, or the
    /// device is lost, `lost` is set and the caller must not destroy them.
    pub(super) fn submit(
        &self,
        record: impl FnOnce(&ash::Device, vk::CommandBuffer),
        end: Instant,
    ) -> Result<(), ReadbackFallback> {
        let copy = |what: &str, error: vk::Result| {
            if error == vk::Result::ERROR_DEVICE_LOST {
                self.lost.set(true);
            }
            ReadbackFallback::Copy(format!("{what}: {error}"))
        };
        if self.lost.get() {
            return Err(ReadbackFallback::Copy("the Vulkan device is lost".into()));
        }
        let device = &self.device;
        let begin = vk::CommandBufferBeginInfo::default()
            .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);
        // SAFETY: the command buffer is never pending here: every earlier
        // submission was waited for, by fence or by draining the queue, and a
        // failed drain set `lost`, checked above. Its pool allows resetting
        // single buffers.
        unsafe {
            device
                .reset_command_buffer(self.command, vk::CommandBufferResetFlags::empty())
                .map_err(|error| copy("vkResetCommandBuffer", error))?;
            device
                .begin_command_buffer(self.command, &begin)
                .map_err(|error| copy("vkBeginCommandBuffer", error))?;
        }
        record(device, self.command);
        // SAFETY: the buffer is recording; the fence is not in use by a
        // pending submission (see above).
        unsafe {
            device
                .end_command_buffer(self.command)
                .map_err(|error| copy("vkEndCommandBuffer", error))?;
            device
                .reset_fences(&[self.fence])
                .map_err(|error| copy("vkResetFences", error))?;
        }
        let commands = [self.command];
        let submit = [vk::SubmitInfo::default().command_buffers(&commands)];
        // SAFETY: the buffer is executable, the queue is accessed only through
        // `&mut VulkanReadback` callers, and the fence is unsignaled.
        let result = unsafe { device.queue_submit(self.queue, &submit, self.fence) }
            .map_err(|error| copy("vkQueueSubmit", error))
            .and_then(|()| {
                let left = end.saturating_duration_since(Instant::now());
                let timeout = u64::try_from(left.as_nanos()).unwrap_or(u64::MAX);
                // SAFETY: the fence belongs to this device.
                unsafe { device.wait_for_fences(&[self.fence], true, timeout) }.map_err(|error| {
                    match error {
                        vk::Result::TIMEOUT => ReadbackFallback::Timeout,
                        error => copy("vkWaitForFences", error),
                    }
                })
            });
        if result.is_err() {
            // SAFETY: draining has no preconditions beyond external queue
            // synchronization (`&mut` callers); on success nothing submitted
            // still uses the caller's objects.
            let drained = unsafe { device.queue_wait_idle(self.queue) }
                .or_else(|_| unsafe { device.device_wait_idle() });
            if drained.is_err() {
                self.lost.set(true);
            }
        }
        result
    }
}

/// An imported dma-buf image and its memory, destroyed on drop. The caller
/// keeps it alive until every submission that used it completed.
pub(super) struct Imported<'a> {
    device: &'a ash::Device,
    pub(super) image: vk::Image,
    memory: vk::DeviceMemory,
}

impl Drop for Imported<'_> {
    fn drop(&mut self) {
        // SAFETY: both handles belong to `device` and no pending work uses
        // them (see the type's contract). Freeing the memory also closes the
        // fd Vulkan took over; a null memory handle is a no-op.
        unsafe {
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}
