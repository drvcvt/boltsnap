//! Seat grab reset and version-aware input-object release.
use super::*;

impl Seat {
    /// Forget the current implicit grab; a new press starts the next one.
    pub fn end_grab(&mut self) {
        self.buttons.clear();
        self.touches.clear();
        self.grab_serial = None;
    }
}
pub(super) fn release_pointer(p: Option<wl_pointer::WlPointer>) {
    if let Some(p) = p.filter(|p| p.version() >= 3) {
        p.release();
    }
}
pub(super) fn release_touch(t: Option<wl_touch::WlTouch>) {
    if let Some(t) = t.filter(|t| t.version() >= 3) {
        t.release();
    }
}
