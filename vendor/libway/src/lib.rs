//! Wayland capture with explicit deadlines, immutable completed frames and optional DMA-BUF,
//! plus optional drag-and-drop. The default build needs no GPU libraries and does not open
//! surfaces or portal dialogs.
//!
//! Without default features, shared data types and [`Display`] remain available.
//! `capture` enables the dedicated capture connection; `image` adds upright images and
//! desktop composition. `gpu` adds thread-local GBM allocations. `dnd` enables drag/drop;
//! `foreign-display` additionally enables unsafe guest attachment to libwayland displays.
#![warn(missing_docs)]
#[cfg_attr(not(feature = "capture"), allow(dead_code))]
mod buffer;
#[cfg(feature = "capture")]
mod capture;
#[cfg(feature = "image")]
mod compose;
mod display;
#[cfg(feature = "dnd")]
pub mod dnd;
mod error;
#[cfg(feature = "gpu")]
pub mod gpu;
#[cfg_attr(not(feature = "capture"), allow(dead_code))]
mod types;
pub use buffer::{CpuBuffer, Frame, FrameStorage, PixelFormat};
#[cfg(feature = "capture")]
pub use capture::{BufferKind, Connection, CursorEvent, CursorStream, Stream};
#[cfg(feature = "image")]
pub use compose::DesktopCapture;
pub use display::{Display, SurfaceHandle};
pub use error::{Error, Result, TransferError};
pub use types::*;
