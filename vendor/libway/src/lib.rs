//! Wayland capture with explicit deadlines, immutable completed frames and optional DMA-BUF,
//! plus optional drag-and-drop and owned windows. The default build needs no GPU libraries
//! and does not open surfaces or portal dialogs implicitly. With the `window` feature,
//! `window::Host` explicitly creates caller-driven owned windows with bounded CPU
//! shared-memory presentation.
//!
//! Without default features, shared data types and [`Display`] remain available.
//! `capture` enables the dedicated capture connection; `image` adds upright images and
//! desktop composition. `gpu` adds thread-local GBM allocations. `dnd` enables drag/drop;
//! `foreign-display` additionally enables unsafe guest attachment to libwayland displays.
//! `window` enables owned windows. `owned-native-system` adds dynamic system transport
//! and standard native handles on exclusive owned presenter claims to `window`, without
//! enabling drag-and-drop.
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
#[cfg(feature = "window")]
mod runtime;
#[cfg_attr(not(feature = "capture"), allow(dead_code))]
mod types;
#[cfg(feature = "window")]
pub mod window;
pub use buffer::{CpuBuffer, Frame, FrameStorage, PixelFormat};
#[cfg(feature = "capture")]
pub use capture::{
    BufferKind, Connection, CursorEvent, CursorStream, DesktopTile, DesktopTiles, ReadbackFallback,
    Stream, TileReadback,
};
#[cfg(feature = "image")]
pub use compose::DesktopCapture;
pub use display::{Display, SurfaceHandle};
pub use error::{Error, Result, TransferError};
pub use types::*;

#[cfg(test)]
extern crate self as libway;
// The integration tests' Wayland test server, for the runtime's tests and
// the crate-private capture paths.
#[cfg(all(test, any(feature = "window", feature = "capture")))]
#[path = "../tests/support/mod.rs"]
mod support;
