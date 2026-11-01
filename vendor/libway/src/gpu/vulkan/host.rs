//! Host-visible readback buffers, persistently mapped.
use crate::capture::ReadbackFallback;
use ash::vk;

/// A host-visible buffer with its own memory, mapped for its whole life. Not
/// destroyed on drop: the owner calls `destroy` with the device.
pub(crate) struct HostBuffer {
    pub(super) buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    pub(super) size: u64,
    coherent: bool,
    /// HOST_CACHED memory: CPU reads straight from the mapping are as fast as
    /// from the heap. Uncached or write-combined memory is only copied out.
    pub(super) cached: bool,
    /// The whole memory, mapped once after allocation.
    mapped: *mut u8,
}

// SAFETY: the mapping belongs to this buffer alone. The GPU writes it only
// inside a readback submission, which the readback (holding the buffer by
// `&mut`) waits for before any CPU access; afterwards it is only read,
// through `&self`, until `destroy` takes the buffer by value.
unsafe impl Send for HostBuffer {}
// SAFETY: as above; shared access is read-only.
unsafe impl Sync for HostBuffer {}

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
            cached: false,
            mapped: std::ptr::null_mut(),
        };
        let result = (|| {
            // SAFETY: `buffer` is a live buffer of this device.
            let requirements = unsafe { device.get_buffer_memory_requirements(buffer) };
            let (index, flags) = host_memory_type(memory, requirements.memory_type_bits)
                .ok_or_else(|| ReadbackFallback::Allocation("no host-visible memory".into()))?;
            this.coherent = flags.contains(vk::MemoryPropertyFlags::HOST_COHERENT);
            this.cached = flags.contains(vk::MemoryPropertyFlags::HOST_CACHED);
            let allocate = vk::MemoryAllocateInfo::default()
                .allocation_size(requirements.size)
                .memory_type_index(index);
            // SAFETY: the index is a memory type allowed for `buffer`.
            this.memory = unsafe { device.allocate_memory(&allocate, None) }
                .map_err(|e| fail("vkAllocateMemory", e))?;
            // SAFETY: fresh memory of the required size, bound once at 0.
            unsafe { device.bind_buffer_memory(buffer, this.memory, 0) }
                .map_err(|e| fail("vkBindBufferMemory", e))?;
            // SAFETY: host-visible memory of this device, mapped once as a
            // whole; freeing the memory in `destroy` unmaps it.
            this.mapped = unsafe {
                device.map_memory(this.memory, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            }
            .map_err(|e| fail("vkMapMemory", e))?
            .cast();
            Ok(())
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

    /// Make the GPU's writes visible to the CPU, after the submission that
    /// wrote them completed.
    pub(super) fn invalidate(&self, device: &ash::Device) -> Result<(), ReadbackFallback> {
        if self.coherent {
            return Ok(());
        }
        let range = [vk::MappedMemoryRange::default()
            .memory(self.memory)
            .size(vk::WHOLE_SIZE)];
        // SAFETY: the memory is mapped as a whole, so the range is valid.
        unsafe { device.invalidate_mapped_memory_ranges(&range) }
            .map_err(|e| ReadbackFallback::Copy(format!("vkInvalidateMappedMemoryRanges: {e}")))
    }

    /// The first `len` bytes of the mapping. The caller made the GPU's
    /// writes visible (`invalidate`) and no submission writes it any more.
    pub(super) fn bytes(&self, len: usize) -> &[u8] {
        assert!(len as u64 <= self.size, "read past the host buffer");
        // SAFETY: the mapping covers `size >= len` bytes, lives as long as
        // `self` (until `destroy` takes it by value) and is not written while
        // shared (see the `Send` impl).
        unsafe { std::slice::from_raw_parts(self.mapped, len) }
    }

    /// Copy `bytes` in for the GPU to read (test uploads).
    #[cfg(test)]
    pub(super) fn write(
        &mut self,
        device: &ash::Device,
        bytes: &[u8],
    ) -> Result<(), ReadbackFallback> {
        assert!(bytes.len() as u64 <= self.size);
        // SAFETY: the mapping holds at least `bytes.len()` bytes, `&mut self`
        // means no slice of it is shared and no submission uses it, and it
        // does not overlap `bytes`.
        unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), self.mapped, bytes.len()) };
        if !self.coherent {
            let range = [vk::MappedMemoryRange::default()
                .memory(self.memory)
                .size(vk::WHOLE_SIZE)];
            // SAFETY: the whole memory is mapped.
            let _ = unsafe { device.flush_mapped_memory_ranges(&range) };
        }
        Ok(())
    }
}

/// A host-visible memory type in `allowed`, best first, with its flags.
fn host_memory_type(
    memory: &vk::PhysicalDeviceMemoryProperties,
    allowed: u32,
) -> Option<(u32, vk::MemoryPropertyFlags)> {
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
            (allowed & (1 << i) != 0 && flags.contains(*wanted)).then_some((i as u32, flags))
        })
    })
}
