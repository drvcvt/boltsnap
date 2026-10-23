//! The shelf's layer surface lives only while the shelf shows something.
//!
//! An overlay layer that always exists, even at 1x1, keeps Hyprland from
//! giving a fullscreen window its output alone (`isSolitaryBlocked`,
//! `SC_OVERLAYS` in Hyprland 0.56): no tearing and no direct scanout for games
//! on that monitor. `place_on_output` already creates the surface when a card
//! arrives; this drops it again once the last card is gone, and lets the
//! appear animation of a fresh surface start with its first frame.

use super::{AnimKind, Daemon};

/// Whether the shelf needs its surface: a card to show, an animation still
/// running (the last dismiss), or a drag that uses the surface as its origin.
pub(super) fn shelf_surface_needed(cards: usize, animations: usize, dragging: bool) -> bool {
    cards > 0 || animations > 0 || dragging
}

impl Daemon {
    /// Drops the shelf surface when nothing needs it. True when the shelf has
    /// no surface afterwards, so there is nothing to draw.
    pub(super) fn release_empty_surface(&mut self) -> bool {
        if shelf_surface_needed(
            self.model.len(),
            self.anims.len(),
            self.drag_source.is_some(),
        ) {
            return false;
        }
        // Dropping the LayerSurface destroys the layer, which unmaps it. The
        // next card creates a new one on the then-focused output.
        self.layer = None;
        self.output_name = None;
        self.shelf_configured = false;
        self.frame_requested = None;
        true
    }

    /// A configure arrived: buffers may be attached from now on. A fresh
    /// surface waited a roundtrip for its first one; appear animations started
    /// meanwhile begin now, so the first frame shows them from the start.
    pub(super) fn mark_shelf_configured(&mut self) {
        if !self.shelf_configured {
            let now = std::time::Instant::now();
            for anim in self.anims.iter_mut().filter(|a| a.kind == AnimKind::Appear) {
                anim.start = now;
            }
        }
        self.shelf_configured = true;
        self.shelf_pending_draw = false;
    }
}

#[cfg(test)]
mod tests {
    use super::shelf_surface_needed;

    #[test]
    fn an_empty_idle_shelf_needs_no_surface() {
        assert!(!shelf_surface_needed(0, 0, false));
    }

    #[test]
    fn a_card_an_animation_or_a_drag_keeps_the_surface() {
        assert!(shelf_surface_needed(1, 0, false));
        // The last card's dismiss animation still draws on it.
        assert!(shelf_surface_needed(0, 1, false));
        // A drag uses the shelf surface as its origin until it ends.
        assert!(shelf_surface_needed(0, 0, true));
    }
}
