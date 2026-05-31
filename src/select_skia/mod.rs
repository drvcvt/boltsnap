//! New region selector on raw SCTK (wlr-layer-shell) + tiny-skia, behind `--new`.
//! Parallel to `src/select.rs` (egui); same public signature so it is a drop-in.

mod font;
mod render;

use image::RgbaImage;

use crate::DynResult;

/// Drop-in replacement for `crate::select::run_select_with_parallel_capture`.
/// Until the SCTK driver lands (Task 8) this forwards to the egui selector so
/// the build stays green and `--new` is harmless.
pub fn run_select_with_parallel_capture<F>(capture: F) -> DynResult<Option<RgbaImage>>
where
    F: FnOnce() -> Result<RgbaImage, String> + Send + 'static,
{
    crate::select::run_select_with_parallel_capture(capture)
}
