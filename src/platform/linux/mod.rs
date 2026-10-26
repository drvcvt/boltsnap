pub mod app;
pub(crate) mod buffers;
pub mod capture;
pub mod clipboard;
pub(crate) mod cursor_track;
pub(crate) mod files;
pub mod gsr;
pub(crate) mod hypr;
pub mod ipc;
pub mod paths;
pub mod replay;
pub mod select_skia;
pub mod shelf;
pub(crate) mod timing;
pub mod tray;

// Screenshot capture and its helpers live together in `capture/`; the old
// paths stay valid for their callers.
pub(crate) use capture::image_transfer;
pub use capture::portal;
