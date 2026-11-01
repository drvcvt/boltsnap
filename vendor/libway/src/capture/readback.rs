//! Which buffer path a desktop tile capture used, and why a faster one was not.
use crate::PixelFormat;

/// The buffer path a tile capture used.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum TileReadback {
    /// Shared memory filled by the compositor, as `capture_desktop_tiles`.
    #[default]
    Shm,
    /// A linear dma-buf read through `mmap` (needs the `vulkan` feature's GPU
    /// support, not Vulkan itself).
    LinearDmabuf,
    /// A dma-buf with a vendor modifier, copied to the CPU through Vulkan.
    Vulkan,
}

/// Why a faster tier was not used. The first cause on a connection is kept
/// until it is reset.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadbackFallback {
    /// `libvulkan.so.1` could not be loaded; the loader's message.
    NoVulkanLoader(String),
    /// Vulkan loaded, but no usable physical device or driver exists.
    NoVulkanDevice,
    /// A required Vulkan device extension is missing.
    MissingExtension(&'static str),
    /// The compositor offered no dma-buf buffers for this capture.
    NoDmabuf,
    /// No GPU matches the compositor's dma-buf device.
    DeviceMismatch,
    /// No offered modifier can be allocated and read back (for example
    /// only multi-plane or non-copyable ones).
    NoUsableModifier,
    /// The capture format has no dma-buf readback here.
    UnsupportedFormat(PixelFormat),
    /// Allocating the dma-buf failed.
    Allocation(String),
    /// The compositor or Vulkan refused the dma-buf.
    Import(String),
    /// Reading the dma-buf back failed.
    Copy(String),
    /// The tier ran out of time before the deadline.
    Timeout,
}

impl ReadbackFallback {
    /// A cause that will not change until the system does (missing library,
    /// extension, dma-buf support, device or modifier): the tier is not tried
    /// again on this connection until `reset_readback`, or for
    /// `DeviceMismatch` until the compositor names another device. Other
    /// causes are retried next call.
    pub fn is_structural(&self) -> bool {
        matches!(
            self,
            Self::NoVulkanLoader(_)
                | Self::NoVulkanDevice
                | Self::NoDmabuf
                | Self::MissingExtension(_)
                | Self::DeviceMismatch
                | Self::NoUsableModifier
                | Self::UnsupportedFormat(_)
        )
    }
}

impl std::fmt::Display for ReadbackFallback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoVulkanLoader(error) => write!(f, "Vulkan loader unavailable: {error}"),
            Self::NoVulkanDevice => f.write_str("no usable Vulkan device"),
            Self::MissingExtension(name) => write!(f, "missing Vulkan extension {name}"),
            Self::NoDmabuf => f.write_str("the compositor offered no dma-buf"),
            Self::DeviceMismatch => f.write_str("no GPU matches the compositor's dma-buf device"),
            Self::NoUsableModifier => f.write_str("no usable dma-buf modifier"),
            Self::UnsupportedFormat(format) => write!(f, "no dma-buf readback for {format:?}"),
            Self::Allocation(error) => write!(f, "dma-buf allocation failed: {error}"),
            Self::Import(error) => write!(f, "dma-buf import failed: {error}"),
            Self::Copy(error) => write!(f, "dma-buf readback failed: {error}"),
            Self::Timeout => f.write_str("dma-buf readback timed out"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_system_causes_are_structural() {
        assert!(ReadbackFallback::NoVulkanLoader("x".into()).is_structural());
        assert!(ReadbackFallback::MissingExtension("VK_EXT_x").is_structural());
        assert!(ReadbackFallback::NoDmabuf.is_structural());
        assert!(!ReadbackFallback::Timeout.is_structural());
        assert!(!ReadbackFallback::Import("refused".into()).is_structural());
        assert_eq!(
            ReadbackFallback::MissingExtension("VK_EXT_queue_family_foreign").to_string(),
            "missing Vulkan extension VK_EXT_queue_family_foreign"
        );
    }
}
