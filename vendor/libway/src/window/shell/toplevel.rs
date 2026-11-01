//! Toplevel size limits (`xdg_toplevel.set_min_size` / `set_max_size`). The
//! requests are double-buffered and libway sends no commit for them: they apply
//! with the window's next presented frame, CPU or native, whose commit follows
//! them on the wire (R3 plan, D2).
use super::{Host, state::WindowState, types::WindowId};
use crate::{Error, Result};

/// Logical size limits of a toplevel. 0 on an axis means no limit there, which
/// is also the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SizeLimits {
    /// Smallest logical width, 0 for none.
    pub min_width: u32,
    /// Smallest logical height, 0 for none.
    pub min_height: u32,
    /// Largest logical width, 0 for none.
    pub max_width: u32,
    /// Largest logical height, 0 for none.
    pub max_height: u32,
}

impl SizeLimits {
    fn min(self) -> (i32, i32) {
        (self.min_width as i32, self.min_height as i32)
    }

    fn max(self) -> (i32, i32) {
        (self.max_width as i32, self.max_height as i32)
    }

    fn validate(self) -> Result<()> {
        let sides = [
            self.min_width,
            self.min_height,
            self.max_width,
            self.max_height,
        ];
        if sides.iter().any(|side| *side > i32::MAX as u32) {
            return Err(Error::InvalidDimensions);
        }
        let inverted = |min: u32, max: u32| min != 0 && max != 0 && max < min;
        if inverted(self.min_width, self.max_width) || inverted(self.min_height, self.max_height) {
            return Err(Error::InvalidInput("maximum size below minimum size"));
        }
        Ok(())
    }
}

impl Host {
    /// Replace the size limits of a toplevel. Only the changed requests are
    /// sent, and they apply with the window's next presented frame; libway
    /// commits nothing for them. The compositor may ignore them.
    ///
    /// Errors: `InvalidDimensions` for a side above `i32::MAX`; `InvalidInput`
    /// for a maximum below the minimum on an axis where both are set, a layer,
    /// or a retired or foreign window. Nothing is sent on error.
    pub fn set_size_limits(&mut self, window: WindowId, limits: SizeLimits) -> Result<()> {
        self.coordinator
            .mutate(|state| state.set_size_limits(window, limits))
    }

    /// The size limits last set with [`Host::set_size_limits`], all 0 before.
    /// A layer, or a retired or foreign window, returns `InvalidInput`.
    pub fn size_limits(&self, window: WindowId) -> Result<SizeLimits> {
        if self.coordinator.failure().is_some() {
            return Err(Error::SessionStopped);
        }
        let state = &self.coordinator.state;
        state.ensure_live()?;
        let record = state.record(window)?;
        record.native.toplevel()?;
        Ok(record.limits)
    }
}

impl WindowState {
    pub(crate) fn set_size_limits(&mut self, window: WindowId, limits: SizeLimits) -> Result<()> {
        self.ensure_active()?;
        let record = self.record(window)?;
        let toplevel = record.native.toplevel()?;
        limits.validate()?;
        let old = record.limits;
        // Never let a strict compositor see max below min between requests:
        // first lower the minimum to the per-axis smaller of old and new (0, no
        // limit, is the smallest), which fits the old maximum; then the new
        // maximum, which fits that minimum; then the new minimum.
        let lower = |old: u32, new: u32| old.min(new);
        let interim = SizeLimits {
            min_width: lower(old.min_width, limits.min_width),
            min_height: lower(old.min_height, limits.min_height),
            ..old
        };
        if interim.min() != old.min() {
            let (width, height) = interim.min();
            toplevel.set_min_size(width, height);
        }
        if limits.max() != old.max() {
            let (width, height) = limits.max();
            toplevel.set_max_size(width, height);
        }
        if limits.min() != interim.min() {
            let (width, height) = limits.min();
            toplevel.set_min_size(width, height);
        }
        self.windows
            .get_mut(&window)
            .expect("validated record")
            .limits = limits;
        self.requested()
    }
}
