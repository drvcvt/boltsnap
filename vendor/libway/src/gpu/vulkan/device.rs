//! Instance, physical device choice and logical device of the Vulkan readback.
use super::{host::HostBuffer, load, vk_format};
use crate::{PixelFormat, capture::ReadbackFallback, gpu::nodes::drm_nodes};
use ash::{ext, khr, vk};
use std::{cell::Cell, ffi::CStr, mem::ManuallyDrop, path::PathBuf};

/// Device extensions the import needs on every API version.
const REQUIRED: [&CStr; 5] = [
    khr::external_memory_fd::NAME,
    ext::external_memory_dma_buf::NAME,
    ext::image_drm_format_modifier::NAME,
    ext::queue_family_foreign::NAME,
    ext::physical_device_drm::NAME,
];

/// One Vulkan device matched to the compositor's GPU, with one queue, one
/// reusable command buffer and fence, and a few cached host buffers. `Send`
/// but not `Sync` (through `lost`): queue, command buffer, fence and host
/// memory need external synchronization, which `read(&mut self)` gives.
pub(crate) struct VulkanReadback {
    /// Keeps `libvulkan` mapped until `Drop` destroyed every object below;
    /// leaked with them when the device cannot be drained.
    entry: ManuallyDrop<ash::Entry>,
    pub(super) instance: ash::Instance,
    pub(super) physical: vk::PhysicalDevice,
    pub(super) device: ash::Device,
    pub(super) queue: vk::Queue,
    pub(super) family: u32,
    pub(super) pool: vk::CommandPool,
    pub(super) command: vk::CommandBuffer,
    pub(super) fence: vk::Fence,
    pub(super) memory_fd: khr::external_memory_fd::Device,
    pub(super) memory: vk::PhysicalDeviceMemoryProperties,
    /// Readback targets keyed by byte size; never in use between calls.
    pub(super) host: Vec<HostBuffer>,
    render_minor: u32,
    /// The device was lost, or submitted work could not be drained: nothing
    /// is submitted any more, and the owner drops and rebuilds it.
    pub(super) lost: Cell<bool>,
}

/// The physical device picked for a compositor device, before a logical
/// device exists.
struct Chosen {
    physical: vk::PhysicalDevice,
    family: u32,
    render_minor: u32,
    extensions: Vec<&'static CStr>,
}

impl VulkanReadback {
    /// Load Vulkan and open the device that matches `compositor_device` (a
    /// `dev_t` from `dmabuf_device`), or the only DRM-capable device when the
    /// compositor named none.
    pub(crate) fn new(compositor_device: Option<u64>) -> Result<Self, ReadbackFallback> {
        let entry = load()?;
        // SAFETY: `entry` holds a loaded `vkGetInstanceProcAddr`; querying the
        // instance version has no preconditions.
        let version = match unsafe { entry.try_enumerate_instance_version() } {
            Ok(Some(version)) => version,
            _ => vk::API_VERSION_1_0,
        };
        let api = if version >= vk::API_VERSION_1_3 {
            vk::API_VERSION_1_3
        } else if version >= vk::API_VERSION_1_2 {
            vk::API_VERSION_1_2
        } else if version >= vk::API_VERSION_1_1 {
            vk::API_VERSION_1_1
        } else {
            return Err(ReadbackFallback::NoVulkanDevice);
        };
        let app = vk::ApplicationInfo::default()
            .application_name(c"libway")
            .engine_name(c"libway")
            .api_version(api);
        let info = vk::InstanceCreateInfo::default().application_info(&app);
        // SAFETY: `info` and `app` are valid for the call and request no
        // layers or extensions. The instance is destroyed before `entry`.
        let instance = unsafe { entry.create_instance(&info, None) }
            .map_err(|_| ReadbackFallback::NoVulkanDevice)?;
        let chosen = match choose(&instance, compositor_device, api) {
            Ok(chosen) => chosen,
            Err(error) => {
                // SAFETY: nothing was created from `instance` yet.
                unsafe { instance.destroy_instance(None) };
                return Err(error);
            }
        };
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(chosen.family)
            .queue_priorities(&priorities)];
        let names: Vec<_> = chosen.extensions.iter().map(|name| name.as_ptr()).collect();
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&names);
        // SAFETY: `chosen.physical` came from this instance, the family index
        // exists with at least one queue, and every enabled extension was
        // reported by the device (`choose`). The pointed-to arrays outlive the call.
        let device = match unsafe { instance.create_device(chosen.physical, &info, None) } {
            Ok(device) => device,
            Err(_) => {
                // SAFETY: no device exists, so nothing else uses the instance.
                unsafe { instance.destroy_instance(None) };
                return Err(ReadbackFallback::NoVulkanDevice);
            }
        };
        // SAFETY: queue 0 of `chosen.family` was requested at device creation.
        let queue = unsafe { device.get_device_queue(chosen.family, 0) };
        let memory_fd = khr::external_memory_fd::Device::new(&instance, &device);
        // SAFETY: `chosen.physical` belongs to `instance`.
        let memory = unsafe { instance.get_physical_device_memory_properties(chosen.physical) };
        let mut this = Self {
            entry: ManuallyDrop::new(entry),
            instance,
            physical: chosen.physical,
            device,
            queue,
            family: chosen.family,
            pool: vk::CommandPool::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            memory_fd,
            memory,
            host: Vec::new(),
            render_minor: chosen.render_minor,
            lost: Cell::new(false),
        };
        // From here on `Drop` releases whatever exists; null handles are no-ops.
        let pool = vk::CommandPoolCreateInfo::default()
            .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER)
            .queue_family_index(this.family);
        // SAFETY: valid create info for a family of this device.
        this.pool = unsafe { this.device.create_command_pool(&pool, None) }.map_err(allocation)?;
        let buffers = vk::CommandBufferAllocateInfo::default()
            .command_pool(this.pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: `this.pool` is a live pool of this device; the buffer is
        // freed with the pool.
        this.command = unsafe { this.device.allocate_command_buffers(&buffers) }
            .map_err(allocation)?
            .into_iter()
            .next()
            .ok_or_else(|| ReadbackFallback::Allocation("no command buffer".into()))?;
        // SAFETY: default create info; the fence is destroyed in `Drop`.
        this.fence = unsafe {
            this.device
                .create_fence(&vk::FenceCreateInfo::default(), None)
        }
        .map_err(allocation)?;
        Ok(this)
    }

    /// `/dev/dri/renderD<minor>` of the chosen device, for the GBM allocator.
    /// Whether this readback is unusable and must be rebuilt.
    pub(crate) fn is_lost(&self) -> bool {
        self.lost.get()
    }

    pub(crate) fn render_node(&self) -> PathBuf {
        PathBuf::from(format!("/dev/dri/renderD{}", self.render_minor))
    }

    /// Single-plane modifiers of `format` that this device can import from a
    /// dma-buf and copy from. The size limit is checked per call in `read`.
    pub(crate) fn modifiers(&self, format: PixelFormat) -> Vec<u64> {
        let Some(format) = vk_format(format) else {
            return Vec::new();
        };
        self.modifier_properties(format)
            .into_iter()
            .filter(|m| {
                m.drm_format_modifier_plane_count == 1
                    && m.drm_format_modifier_tiling_features
                        .contains(vk::FormatFeatureFlags::TRANSFER_SRC)
                    && self
                        .max_extent(
                            format,
                            m.drm_format_modifier,
                            vk::ImageUsageFlags::TRANSFER_SRC,
                        )
                        .is_some()
            })
            .map(|m| m.drm_format_modifier)
            .collect()
    }

    /// Every modifier the device lists for `format`, unfiltered.
    pub(super) fn modifier_properties(
        &self,
        format: vk::Format,
    ) -> Vec<vk::DrmFormatModifierPropertiesEXT> {
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default();
        let mut props = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: the device supports VK_EXT_image_drm_format_modifier, so the
        // chained list is a valid extension; with a null array the driver only
        // writes the count.
        unsafe {
            self.instance
                .get_physical_device_format_properties2(self.physical, format, &mut props)
        };
        let count = list.drm_format_modifier_count as usize;
        let mut modifiers = vec![vk::DrmFormatModifierPropertiesEXT::default(); count];
        let mut list = vk::DrmFormatModifierPropertiesListEXT::default()
            .drm_format_modifier_properties(&mut modifiers);
        let mut props = vk::FormatProperties2::default().push_next(&mut list);
        // SAFETY: as above; the array holds `count` entries and the driver
        // writes at most that many, lowering the count if it returns fewer.
        unsafe {
            self.instance
                .get_physical_device_format_properties2(self.physical, format, &mut props)
        };
        let written = list.drm_format_modifier_count as usize;
        modifiers.truncate(written);
        modifiers
    }

    /// Largest importable dma-buf image for `format` with `modifier` and
    /// `usage`, or `None` when the device cannot import such an image.
    pub(super) fn max_extent(
        &self,
        format: vk::Format,
        modifier: u64,
        usage: vk::ImageUsageFlags,
    ) -> Option<vk::Extent3D> {
        let mut drm = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
            .drm_format_modifier(modifier)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let mut external = vk::PhysicalDeviceExternalImageFormatInfo::default()
            .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
        let info = vk::PhysicalDeviceImageFormatInfo2::default()
            .format(format)
            .ty(vk::ImageType::TYPE_2D)
            .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
            .usage(usage)
            .push_next(&mut drm)
            .push_next(&mut external);
        let mut external_props = vk::ExternalImageFormatProperties::default();
        let mut props = vk::ImageFormatProperties2::default().push_next(&mut external_props);
        // SAFETY: every chained struct belongs to an extension the device
        // supports (image_drm_format_modifier, external memory in 1.1 core),
        // and all of them outlive the call.
        unsafe {
            self.instance.get_physical_device_image_format_properties2(
                self.physical,
                &info,
                &mut props,
            )
        }
        .ok()?;
        let extent = props.image_format_properties.max_extent;
        external_props
            .external_memory_properties
            .external_memory_features
            .contains(vk::ExternalMemoryFeatureFlags::IMPORTABLE)
            .then_some(extent)
    }
}

impl Drop for VulkanReadback {
    fn drop(&mut self) {
        // SAFETY: waiting idle first means no command buffer, buffer or fence
        // is in use. Children go before the device and the device before the
        // instance, and the loader last; null handles (a half-built value)
        // are valid no-ops. When the wait fails, work may still run: every
        // object and the loader are leaked instead (fail closed).
        unsafe {
            if self.device.device_wait_idle().is_err() {
                return;
            }
            for host in self.host.drain(..) {
                host.destroy(&self.device);
            }
            self.device.destroy_fence(self.fence, None);
            self.device.destroy_command_pool(self.pool, None);
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
            ManuallyDrop::drop(&mut self.entry);
        }
    }
}

fn allocation(error: vk::Result) -> ReadbackFallback {
    ReadbackFallback::Allocation(error.to_string())
}

/// Pick the physical device for `wanted`: one whose primary or render node is
/// a node of the compositor's GPU, or with no `wanted`, the only device with a
/// render node. CPU devices (lavapipe) have none, so they never count.
fn choose(
    instance: &ash::Instance,
    wanted: Option<u64>,
    api: u32,
) -> Result<Chosen, ReadbackFallback> {
    // SAFETY: `instance` is live; enumeration has no other preconditions.
    let devices = unsafe { instance.enumerate_physical_devices() }
        .map_err(|_| ReadbackFallback::NoVulkanDevice)?;
    if devices.is_empty() {
        return Err(ReadbackFallback::NoVulkanDevice);
    }
    let nodes = wanted.map(|dev| {
        let mut nodes = drm_nodes(dev);
        let own = (libc::major(dev), libc::minor(dev));
        if !nodes.contains(&own) {
            nodes.push(own);
        }
        nodes
    });
    let mut any_drm = false;
    let mut found = Vec::new();
    for physical in devices {
        // SAFETY: `physical` was enumerated from `instance`.
        let props = unsafe { instance.get_physical_device_properties(physical) };
        if props.api_version < vk::API_VERSION_1_1 {
            continue;
        }
        // SAFETY: as above.
        let Ok(extensions) = (unsafe { instance.enumerate_device_extension_properties(physical) })
        else {
            continue;
        };
        if !has(&extensions, ext::physical_device_drm::NAME) {
            continue;
        }
        any_drm = true;
        let (primary, render) = drm_nodes_of(instance, physical);
        let matches = match &nodes {
            Some(nodes) => [primary, render]
                .iter()
                .flatten()
                .any(|n| nodes.contains(n)),
            None => render.is_some(),
        };
        if matches {
            found.push((physical, props.api_version, extensions, render));
        }
    }
    if found.is_empty() {
        return Err(match (any_drm, wanted) {
            (false, _) => missing(ext::physical_device_drm::NAME),
            (true, Some(_)) => ReadbackFallback::DeviceMismatch,
            (true, None) => ReadbackFallback::NoVulkanDevice,
        });
    }
    if wanted.is_none() && found.len() > 1 {
        return Err(ReadbackFallback::DeviceMismatch);
    }
    // Two drivers may serve one GPU; take the first that has everything.
    let mut first_error = None;
    for (physical, device_api, extensions, render) in found {
        match check(instance, physical, api.min(device_api), &extensions, render) {
            Ok(chosen) => return Ok(chosen),
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    Err(first_error.unwrap_or(ReadbackFallback::NoVulkanDevice))
}

/// Check one matching device for extensions, a render node and a queue.
fn check(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
    api: u32,
    extensions: &[vk::ExtensionProperties],
    render: Option<Node>,
) -> Result<Chosen, ReadbackFallback> {
    let mut wanted: Vec<&'static CStr> = REQUIRED.to_vec();
    if api < vk::API_VERSION_1_2 {
        wanted.push(khr::image_format_list::NAME);
    }
    if let Some(name) = wanted.iter().copied().find(|name| !has(extensions, name)) {
        return Err(missing(name));
    }
    let (_, render_minor) = render.ok_or(ReadbackFallback::NoVulkanDevice)?;
    // SAFETY: `physical` was enumerated from `instance`.
    let families = unsafe { instance.get_physical_device_queue_family_properties(physical) };
    let usable = |flags: vk::QueueFlags| {
        families
            .iter()
            .position(|f| f.queue_count > 0 && f.queue_flags.intersects(flags))
    };
    // Graphics and compute families also transfer, with a 1x1x1 granularity.
    let family = usable(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
        .or_else(|| usable(vk::QueueFlags::TRANSFER))
        .ok_or(ReadbackFallback::NoVulkanDevice)?;
    Ok(Chosen {
        physical,
        family: u32::try_from(family).map_err(|_| ReadbackFallback::NoVulkanDevice)?,
        render_minor,
        extensions: wanted,
    })
}

/// A DRM node as `(major, minor)`.
type Node = (u32, u32);

/// The primary and render node Vulkan reports.
fn drm_nodes_of(
    instance: &ash::Instance,
    physical: vk::PhysicalDevice,
) -> (Option<Node>, Option<Node>) {
    let mut drm = vk::PhysicalDeviceDrmPropertiesEXT::default();
    let mut props = vk::PhysicalDeviceProperties2::default().push_next(&mut drm);
    // SAFETY: the caller checked that the device supports
    // VK_EXT_physical_device_drm, so the chained struct is valid; the
    // instance is at least 1.1, so the query is core.
    unsafe { instance.get_physical_device_properties2(physical, &mut props) };
    let node = |has: vk::Bool32, major: i64, minor: i64| {
        (has == vk::TRUE)
            .then(|| Some((u32::try_from(major).ok()?, u32::try_from(minor).ok()?)))
            .flatten()
    };
    (
        node(drm.has_primary, drm.primary_major, drm.primary_minor),
        node(drm.has_render, drm.render_major, drm.render_minor),
    )
}

fn has(extensions: &[vk::ExtensionProperties], name: &CStr) -> bool {
    extensions
        .iter()
        .any(|e| e.extension_name_as_c_str() == Ok(name))
}

fn missing(name: &'static CStr) -> ReadbackFallback {
    ReadbackFallback::MissingExtension(name.to_str().unwrap_or("unnamed extension"))
}
