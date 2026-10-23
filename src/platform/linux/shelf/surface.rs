//! The shelf's layer surface lives only while the shelf shows something.
//!
//! An overlay layer that always exists, even at 1x1, keeps Hyprland from
//! giving a fullscreen window its output alone (`isSolitaryBlocked`,
//! `SC_OVERLAYS` in Hyprland 0.56): no tearing and no direct scanout for games
//! on that monitor. `place_on_output` already creates the surface when a card
//! arrives; this drops it again once the last card is gone, and lets the
//! appear animation of a fresh surface start with its first frame.

use super::{AnimKind, Daemon};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Release {
    /// A card or a running animation (the last dismiss) still draws on it.
    Keep,
    Drop,
    /// A drag whose end event never arrived still uses the surface as its
    /// origin. Its card and file are gone, so nothing is left to protect.
    DropAndEndDrag,
}

pub(super) fn release_plan(cards: usize, animations: usize, dragging: bool) -> Release {
    if cards > 0 || animations > 0 {
        Release::Keep
    } else if dragging {
        Release::DropAndEndDrag
    } else {
        Release::Drop
    }
}

impl Daemon {
    /// Drops the shelf surface when nothing needs it. True when the shelf has
    /// no surface afterwards, so there is nothing to draw.
    pub(super) fn release_empty_surface(&mut self) -> bool {
        match release_plan(
            self.model.len(),
            self.anims.len(),
            self.drag_source.is_some(),
        ) {
            Release::Keep => return false,
            Release::Drop => {}
            // Not clear_drag(): its redraw would land back here.
            Release::DropAndEndDrag => self.forget_drag(),
        }
        // Dropping the LayerSurface destroys the layer, which unmaps it. The
        // next card creates a new one on its requested output, else on this
        // one: `output_name` stays for that.
        self.layer = None;
        self.shelf_configured = false;
        self.frame_requested = None;
        true
    }

    /// Ends the drag state without redrawing. A late `cancelled` or
    /// `dnd_finished` for the dropped source then finds nothing to act on.
    pub(super) fn forget_drag(&mut self) {
        self.drag_source = None;
        self.drag_card = None;
        self.icon_surface = None;
        self.drag_icon_buffer = None;
        self.drag_icon_pool = None;
        self.drop_ok = false;
        self.press = None;
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
    use super::{Release, release_plan};

    #[test]
    fn an_empty_idle_shelf_drops_its_surface() {
        assert_eq!(release_plan(0, 0, false), Release::Drop);
    }

    #[test]
    fn a_card_or_an_animation_keeps_the_surface() {
        assert_eq!(release_plan(1, 0, false), Release::Keep);
        assert_eq!(release_plan(1, 0, true), Release::Keep);
        // The last card's dismiss animation still draws on it.
        assert_eq!(release_plan(0, 1, false), Release::Keep);
        assert_eq!(release_plan(0, 1, true), Release::Keep);
    }

    #[test]
    fn a_stale_drag_does_not_keep_an_empty_shelf() {
        assert_eq!(release_plan(0, 0, true), Release::DropAndEndDrag);
    }
}
