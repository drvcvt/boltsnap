//! Vulkan readback of vendor-modifier dma-bufs (tier 2). `libvulkan.so.1` is
//! loaded at runtime; a missing loader is a fallback cause, never a crash.
use crate::{PixelFormat, capture::ReadbackFallback};
use ash::vk;

mod device;
mod host;
mod readback;
#[cfg(test)]
mod tests;

#[allow(unused_imports, reason = "used by the Vulkan tier, plan step 4")]
pub(crate) use device::VulkanReadback;

/// Load the Vulkan loader, or say why it is unavailable.
pub(crate) fn load() -> Result<ash::Entry, ReadbackFallback> {
    // SAFETY: `Entry::load` dlopens the system loader and resolves
    // `vkGetInstanceProcAddr`. The returned `Entry` keeps the library mapped
    // for as long as it, and every object created through it, lives; libway
    // never unloads it while Vulkan objects exist.
    unsafe { ash::Entry::load() }
        .map_err(|error| ReadbackFallback::NoVulkanLoader(error.to_string()))
}

/// The Vulkan format with the same bytes as a DRM format, for the formats the
/// GPU tiers read back. DRM ARGB/XRGB8888 are B,G,R,A/X in memory, which is
/// `B8G8R8A8_UNORM`; X is read as is, never forced opaque.
pub(super) fn vk_format(format: PixelFormat) -> Option<vk::Format> {
    match format {
        PixelFormat::Argb8888 | PixelFormat::Xrgb8888 => Some(vk::Format::B8G8R8A8_UNORM),
        _ => None,
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    /// Either outcome is fine; the point is that a missing loader is an error
    /// value, not a panic or an abort.
    #[test]
    fn loading_vulkan_never_panics() {
        match load() {
            Ok(_) => {}
            Err(ReadbackFallback::NoVulkanLoader(message)) => assert!(!message.is_empty()),
            Err(other) => panic!("unexpected cause {other:?}"),
        }
    }

    #[test]
    fn only_argb_and_xrgb_map_to_bgra() {
        let bgra = Some(vk::Format::B8G8R8A8_UNORM);
        assert!(vk_format(PixelFormat::Argb8888) == bgra);
        assert!(vk_format(PixelFormat::Xrgb8888) == bgra);
        for other in [
            PixelFormat::Abgr8888,
            PixelFormat::Xbgr8888,
            PixelFormat::Rgb888,
            PixelFormat::Bgr888,
            PixelFormat::Argb2101010,
            PixelFormat::Xrgb2101010,
            PixelFormat::Abgr2101010,
            PixelFormat::Xbgr2101010,
        ] {
            assert!(vk_format(other).is_none(), "{other:?}");
        }
    }

    #[test]
    fn readback_is_send() {
        fn send<T: Send>() {}
        send::<VulkanReadback>();
    }
}
