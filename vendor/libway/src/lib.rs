//! Wayland capture with explicit deadlines, immutable completed frames and optional DMA-BUF.
//! The default build needs no GPU libraries and does not open surfaces or portal dialogs.
mod buffer;
mod capture;
#[cfg(feature = "image")]
mod compose;
mod error;
#[cfg(feature = "gpu")]
pub mod gpu;
mod types;
pub use buffer::{CpuBuffer, Frame, FrameStorage, PixelFormat};
pub use capture::{BufferKind, Connection, CursorEvent, CursorStream, Stream};
#[cfg(feature = "image")]
pub use compose::DesktopCapture;
pub use error::{Error, Result};
pub use types::*;
