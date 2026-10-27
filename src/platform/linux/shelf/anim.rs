//! A card's appear and dismiss animations in the overlay, on the timing the
//! settings preview shares (`boltsnap::shelf_card`).

use boltsnap::shelf_card::{self, ease_out_cubic};

use super::cards::{self, animated_card_layout, animated_item_limit, overflow_card_opacity};
use super::{Daemon, Layout, QueueHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AnimKind {
    Appear,
    Disappear,
}

impl AnimKind {
    /// Per-kind duration in ms.
    pub(super) fn dur(self) -> u128 {
        match self {
            AnimKind::Appear => shelf_card::APPEAR_MS,
            AnimKind::Disappear => shelf_card::DISMISS_MS,
        }
    }
}

/// A running per-card animation. `start` is when it began; progress is
/// `elapsed / kind.dur()`, eased, until it reaches 1.0 and is retired.
pub(super) struct CardAnim {
    pub(super) id: u64,
    pub(super) kind: AnimKind,
    pub(super) start: std::time::Instant,
}

/// What starting an animation does.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Start {
    Animate,
    /// `[shelf] animations = false` and a dismissal: the card goes at once
    /// and the stack closes up.
    RemoveNow,
    /// `animations = false` and an arrival: the card is simply there.
    Nothing,
}

pub(super) fn anim_start(animations: bool, kind: AnimKind) -> Start {
    match (animations, kind) {
        (true, _) => Start::Animate,
        (false, AnimKind::Disappear) => Start::RemoveNow,
        (false, AnimKind::Appear) => Start::Nothing,
    }
}

impl Daemon {
    /// Begin an appear/dismiss animation for card `id`, replacing any existing
    /// animation on that card.
    pub(super) fn start_anim(&mut self, id: u64, kind: AnimKind) {
        self.anims.retain(|a| a.id != id);
        match anim_start(self.shelf_prefs.animations, kind) {
            Start::Animate => self.anims.push(CardAnim {
                id,
                kind,
                start: std::time::Instant::now(),
            }),
            Start::RemoveNow => {
                if self.remove_card_now(id) {
                    self.relayout();
                }
            }
            Start::Nothing => {}
        }
    }

    /// (scale, opacity) for card `id` from its in-flight animation, or (1.0, 1.0)
    /// when it is settled.
    pub(super) fn anim_factor(&self, id: u64) -> (f32, f32) {
        for a in &self.anims {
            if a.id != id {
                continue;
            }
            let t = (a.start.elapsed().as_millis() as f32 / a.kind.dur() as f32).clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            return match a.kind {
                AnimKind::Appear => shelf_card::appear(eased),
                AnimKind::Disappear => shelf_card::dismiss(eased),
            };
        }
        if let Some(position) = self.model.newest_first().position(|card| card.id == id) {
            let appear = self.active_appear_progress();
            let disappearances = self.active_disappear_progresses();
            let visible = self.shelf_prefs.visible;
            if position >= visible
                && position
                    < animated_item_limit(
                        self.model.len(),
                        visible,
                        appear.is_some(),
                        disappearances.len(),
                    )
                && let Some(opacity) =
                    overflow_card_opacity(position, visible, appear, &disappearances)
            {
                return (1.0, opacity);
            }
        }
        (1.0, 1.0)
    }

    fn active_appear_progress(&self) -> Option<f32> {
        let newest = self.model.newest_first().next()?.id;
        self.anims
            .iter()
            .rev()
            .find(|anim| anim.id == newest && anim.kind == AnimKind::Appear)
            .map(|anim| {
                ease_out_cubic(anim.start.elapsed().as_millis() as f32 / anim.kind.dur() as f32)
            })
    }

    fn active_disappear_progresses(&self) -> Vec<f32> {
        let mut progresses = self
            .anims
            .iter()
            .filter(|anim| anim.kind == AnimKind::Disappear)
            .map(|anim| {
                ease_out_cubic(anim.start.elapsed().as_millis() as f32 / anim.kind.dur() as f32)
            })
            .collect::<Vec<_>>();
        progresses.sort_by(|left, right| right.total_cmp(left));
        progresses
    }

    fn active_disappear_count(&self) -> usize {
        self.anims
            .iter()
            .filter(|animation| animation.kind == AnimKind::Disappear)
            .count()
    }

    pub(super) fn animating(&self) -> bool {
        !self.anims.is_empty()
    }

    /// Animations advance on the compositor's frame callbacks (the display's
    /// refresh rate). The loop timer only steps in when callbacks stop, e.g.
    /// while the shelf is not visible.
    pub(super) fn animation_timer_due(&self) -> bool {
        self.animating()
            && self
                .frame_requested
                .is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(100))
    }

    /// Advance animations one frame: retire finished ones (removing dismissed
    /// cards + their temp files), relayout if anything changed, and redraw.
    pub(super) fn tick_animations(&mut self, qh: &QueueHandle<Self>) {
        let done: Vec<(u64, AnimKind)> = self
            .anims
            .iter()
            .filter(|a| a.start.elapsed().as_millis() >= a.kind.dur())
            .map(|a| (a.id, a.kind))
            .collect();
        self.anims
            .retain(|a| a.start.elapsed().as_millis() < a.kind.dur());
        let mut removed = false;
        for (id, kind) in done {
            if kind == AnimKind::Disappear && self.remove_card_now(id) {
                removed = true;
            }
        }
        if removed {
            self.relayout();
        }
        self.draw(qh);
    }

    /// Layout for the current frame: disappearing slots collapse, while a sixth
    /// card remains in the fixed-height viewport just long enough to scroll and
    /// fade below it as the new card enters at the top.
    pub(super) fn animated_layout(&self) -> Layout {
        let cfg = &self.cfg;
        let appear_progress = self.active_appear_progress();
        let limit = animated_item_limit(
            self.model.len(),
            self.shelf_prefs.visible,
            appear_progress.is_some(),
            self.active_disappear_count(),
        );
        let items: Vec<(u64, u32, u32, f32)> = self
            .model
            .newest_first()
            .take(limit)
            .map(|t| {
                let collapse = self
                    .anims
                    .iter()
                    .find(|a| a.id == t.id && a.kind == AnimKind::Disappear)
                    .map(|a| {
                        let p = (a.start.elapsed().as_millis() as f32 / a.kind.dur() as f32)
                            .clamp(0.0, 1.0);
                        ease_out_cubic(p)
                    })
                    .unwrap_or(0.0);
                (t.id, t.thumb.width(), t.thumb.height(), collapse)
            })
            .collect();
        let top = self.shelf_prefs.corner.is_top();
        let layout =
            animated_card_layout(&items, cfg, self.shelf_prefs.visible, top, appear_progress);
        if self.active_disappear_count() == 0 {
            return layout;
        }
        cards::hold_size(layout, (self.width, self.height), top)
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimKind, Start, anim_start};

    #[test]
    fn without_animations_a_dismissal_removes_at_once_and_an_arrival_just_appears() {
        assert_eq!(anim_start(true, AnimKind::Appear), Start::Animate);
        assert_eq!(anim_start(true, AnimKind::Disappear), Start::Animate);
        assert_eq!(anim_start(false, AnimKind::Disappear), Start::RemoveNow);
        assert_eq!(anim_start(false, AnimKind::Appear), Start::Nothing);
    }
}
