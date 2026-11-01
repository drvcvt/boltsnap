//! Shared ownership of the Vulkan loader, instance and logical device, so
//! that memory handed out with a tile can outlive the readback that made it.
use ash::vk;
use std::{
    mem::ManuallyDrop,
    sync::atomic::{AtomicBool, Ordering},
};

/// The loader, instance and device. Destroyed when the last holder drops:
/// the readback and every leased host buffer hold an `Arc` of it and destroy
/// their own child objects first.
pub(crate) struct DeviceOwner {
    /// Keeps `libvulkan` mapped until the device and instance are gone.
    entry: ManuallyDrop<ash::Entry>,
    pub(super) instance: ash::Instance,
    pub(super) device: ash::Device,
    /// Set when submitted work could not be drained: then nothing created
    /// from this device is ever destroyed or freed again (fail closed).
    leaking: AtomicBool,
}

impl DeviceOwner {
    pub(super) fn new(entry: ash::Entry, instance: ash::Instance, device: ash::Device) -> Self {
        Self {
            entry: ManuallyDrop::new(entry),
            instance,
            device,
            leaking: AtomicBool::new(false),
        }
    }

    /// Give up on the device: from now on every object of it is leaked.
    pub(super) fn leak(&self) {
        self.leaking.store(true, Ordering::Release);
    }

    pub(super) fn is_leaking(&self) -> bool {
        self.leaking.load(Ordering::Acquire)
    }

    /// `vkDeviceWaitIdle`.
    pub(super) fn wait_idle(&self) -> Result<(), vk::Result> {
        // SAFETY: the device is live while `self` exists. Its queue is used
        // only by the readback, which calls this from its own `&mut` paths.
        unsafe { self.device.device_wait_idle() }
    }
}

impl Drop for DeviceOwner {
    fn drop(&mut self) {
        if self.is_leaking() {
            return;
        }
        // SAFETY: this is the last holder, and every holder destroyed its own
        // child objects before letting go, so the device has no children
        // left. The device goes before the instance, and the loader last.
        unsafe {
            self.device.destroy_device(None);
            self.instance.destroy_instance(None);
            ManuallyDrop::drop(&mut self.entry);
        }
    }
}
