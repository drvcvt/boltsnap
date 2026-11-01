//! Host-visible readback buffers and their small reuse cache.
use crate::capture::ReadbackFallback;
use ash::vk;

/// Host buffers kept for reuse (one per distinct byte size).
const MAX_HOST_BUFFERS: usize = 4;

/// A host-visible buffer with its own memory. Not destroyed on drop: the
/// owner calls `destroy` with the device.
pub(crate) struct HostBuffer {
    pub(super) buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    size: u64,
    coherent: bool,
}

impl HostBuffer {
    /// Create a buffer of `size` bytes in host-visible memory, preferring
    /// cached and coherent memory.
    pub(super) fn new(
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        size: u64,
        usage: vk::BufferUsageFlags,
    ) -> Result<Self, ReadbackFallback> {
        let fail = |what: &str, error: vk::Result| {
            ReadbackFallback::Allocation(format!("{what}: {error}"))
        };
        let info = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        // SAFETY: valid create info with a non-zero size (callers pass at
        // least 4 bytes).
        let buffer =
            unsafe { device.create_buffer(&info, None) }.map_err(|e| fail("vkCreateBuffer", e))?;
        let mut this = Self {
            buffer,
            memory: vk::DeviceMemory::null(),
            size,
            coherent: false,
        };
        let result = (|| {
            // SAFETY: `buffer` is a live buffer of this device.
            let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
            let (index, coherent) = host_memory_type(memory, requirements.memory_type_bits)
                .ok_or_else(|| ReadbackFallback::Allocation("no host-visible memory".into()))?;
            this.coherent = coherent;
            let allocate = vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(index);
            // SAFETY: the index is a memory type allowed for `buffer`.
            this.memory = unsafe { device.allocate_memory(&allocate, None) }
                .map_err(|e| fail("vkAllocateMemory", e))?;
            // SAFETY: fresh memory of the required size, bound once at 0.
            unsafe { device.bind_buffer_memory(buffer, this.memory, 0) }
                .map_err(|e| fail("vkBindBufferMemory", e))
        })();
        match result {
            Ok(()) => Ok(this),
            Err(error) => {
                this.destroy(device);
                Err(error)
            }
        }
    }

    /// Destroy the buffer and free its memory. No pending work may use it.
    pub(super) fn destroy(self, device: &ash::Device) {
        // SAFETY: both handles belong to `device` and are unused (contract);
        // null memory is a no-op.
        unsafe {
            device.destroy_buffer(self.buffer, None);
            device.free_memory(self.memory, None);
        }
    }

    /// Copy the first `len` bytes out, after the GPU writes completed.
    pub(super) fn read(
        &self,
        device: &ash::Device,
        len: usize,
    ) -> Result<Vec<u8>, ReadbackFallback> {
        let fail =
            |what: &str, error: vk::Result| ReadbackFallback::Copy(format!("{what}: {error}"));
        if len as u64 > self.size {
            return Err(ReadbackFallback::Copy("read past the buffer".into()));
        }
        // SAFETY: host-visible memory of this device, not mapped elsewhere.
        let data = unsafe {
            device.map_memory(self.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| fail("vkMapMemory", e))?;
        let range = [vk::MappedMemoryRange::default()
            .memory(self.memory)
            .size(vk::WHOLE_SIZE)];
        // SAFETY: the memory is mapped as a whole, so the range is valid.
        let fresh =
            self.coherent || unsafe { device.invalidate_mapped_memory_ranges(&range) }.is_ok();
        let bytes = fresh.then(|| {
            // SAFETY: the mapping covers at least `size >= len` bytes and the
            // device finished writing them (fence waited, host barrier).
            unsafe { std::slice::from_raw_parts(data.cast::<u8>(), len) }.to_vec()
        });
        // SAFETY: mapped above; the slice was copied, nothing points into it.
        unsafe { device.unmap_memory(self.memory) };
        bytes.ok_or_else(|| ReadbackFallback::Copy("vkInvalidateMappedMemoryRanges failed".into()))
    }

    /// Copy `bytes` in for the GPU to read (test uploads).
    #[cfg(test)]
    pub(super) fn write(&self, device: &ash::Device, bytes: &[u8]) -> Result<(), ReadbackFallback> {
        assert!(bytes.len() as u64 <= self.size);
        // SAFETY: host-visible memory of this device, not mapped elsewhere.
        let data = unsafe {
            device.map_memory(self.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
        }
        .map_err(|e| ReadbackFallback::Copy(e.to_string()))?;
        // SAFETY: the mapping holds at least `bytes.len()` bytes and does not
        // overlap `bytes`.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), data.cast::<u8>(), bytes.len()) };
        let range = [vk::MappedMemoryRange::default()
            .memory(self.memory)
            .size(vk::WHOLE_SIZE)];
        // SAFETY: the whole memory is mapped; unmapped right after.
        unsafe {
            if !self.coherent {
                let _ = device.flush_mapped_memory_ranges(&range);
            }
            device.unmap_memory(self.memory);
        }
        Ok(())
    }
}

/// Index of the cached host buffer of exactly `size` bytes, creating it (and
/// evicting the oldest) when missing. Cached buffers are never in use between
/// calls, because every call waits for its submission.
pub(super) fn host_slot(
    device: &ash::Device,
    memory: &vk::PhysicalDeviceMemoryProperties,
    cache: &mut Vec<HostBuffer>,
    size: u64,
) -> Result<usize, ReadbackFallback> {
    if let Some(slot) = cache.iter().position(|host| host.size == size) {
        return Ok(slot);
    }
    let host = HostBuffer::new(device, memory, size, vk::BufferUsageFlags::TRANSFER_DST)?;
    if cache.len() >= MAX_HOST_BUFFERS {
        cache.remove(0).destroy(device);
    }
    cache.push(host);
    Ok(cache.len() - 1)
}

/// A host-visible memory type in `allowed`, best first, and whether it is coherent.
fn host_memory_type(
    memory: &vk::PhysicalDeviceMemoryProperties,
    allowed: u32,
) -> Option<(u32, bool)> {
    use vk::MemoryPropertyFlags as F;
    let preferred = [
        F::HOST_VISIBLE | F::HOST_CACHED | F::HOST_COHERENT,
        F::HOST_VISIBLE | F::HOST_CACHED,
        F::HOST_VISIBLE | F::HOST_COHERENT,
        F::HOST_VISIBLE,
    ];
    preferred.iter().find_map(|wanted| {
        let types = memory.memory_types_as_slice().iter().enumerate();
        types.take(32).find_map(|(i, memory_type)| {
            let flags = memory_type.property_flags;
            (allowed & (1 << i) != 0 && flags.contains(*wanted))
                .then_some((i as u32, flags.contains(F::HOST_COHERENT)))
        })
    })
}
