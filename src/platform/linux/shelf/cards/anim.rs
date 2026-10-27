//! A card's appear and dismiss animations in the overlay, on the timing the
//! settings preview shares (`boltsnap::shelf_card`), and its hover fade
//! (`[shelf] hover_fade`).

use boltsnap::shelf_card::{self, ease_out_cubic};

use super::cards::{self, animated_card_layout, animated_item_limit, overflow_card_opacity};
use super::{Daemon, Layout, QueueHandle};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AnimKind {
    Appear,
    Disappear,
    /// Into and out of the hover look: neither scales nor moves the card.
    HoverIn,
    HoverOut,
}

impl AnimKind {
    /// Per-kind duration in ms.
    pub(super) fn dur(self) -> u128 {
        match self {
            AnimKind::Appear => shelf_card::APPEAR_MS,
            AnimKind::Disappear => shelf_card::DISMISS_MS,
            AnimKind::HoverIn | AnimKind::HoverOut => shelf_card::HOVER_MS,
        }
    }

    fn hover(self) -> bool {
        matches!(self, AnimKind::HoverIn | AnimKind::HoverOut)
    }
}

/// How far a hover fade `kind` has come after `ms`, 0 to 1: linear, so a
/// fade turned around midway goes on from where it is.
pub(super) fn hover_lift(kind: AnimKind, ms: f32) -> f32 {
    let t = (ms / shelf_card::HOVER_MS as f32).clamp(0.0, 1.0);
    if kind == AnimKind::HoverIn {
        t
    } else {
        1.0 - t
    }
}

/// How long ago a hover fade `kind` must have begun to stand at `lift` now.
pub(super) fn hover_begun(kind: AnimKind, lift: f32) -> f32 {
    let done = if kind == AnimKind::HoverIn {
        lift
    } else {
        1.0 - lift
    };
    done.clamp(0.0, 1.0) * shelf_card::HOVER_MS as f32
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
        // A hover fade has its own switch, `hover_fade`.
        (false, AnimKind::HoverIn | AnimKind::HoverOut) => Start::Animate,
    }
}

impl Daemon {
    /// The pointer is on card `to` now (or on none). With `[shelf]
    /// hover_fade` the old card fades out and the new one in, each from
    /// where it stands; without, the change is at once.
    pub(super) fn set_hovered(&mut self, to: Option<u64>) {
        if to == self.hovered {
            return;
        }
        if self.shelf_prefs.hover_fade {
            let lifts = self.hover_lifts();
            let lift = |id: u64| {
                lifts
                    .iter()
                    .find(|(card, _)| *card == id)
                    .map_or(0.0, |(_, l)| *l)
            };
            let now = std::time::Instant::now();
            for (id, kind) in [(self.hovered, AnimKind::HoverOut), (to, AnimKind::HoverIn)] {
                let Some(id) = id else { continue };
                let from = lift(id);
                self.anims.retain(|a| !(a.id == id && a.kind.hover()));
                let begun =
                    std::time::Duration::from_micros((hover_begun(kind, from) * 1000.0) as u64);
                self.anims.push(CardAnim {
                    id,
                    kind,
                    start: now.checked_sub(begun).unwrap_or(now),
                });
            }
        }
        self.hovered = to;
    }

    /// How far each card's hover has come, for the painter: fading cards by
    /// their fade, the hovered one at 1 once it is in.
    pub(super) fn hover_lifts(&self) -> Vec<(u64, f32)> {
        let mut lifts: Vec<(u64, f32)> = self
            .anims
            .iter()
            .filter(|a| a.kind.hover())
            .map(|a| {
                (
                    a.id,
                    hover_lift(a.kind, a.start.elapsed().as_secs_f32() * 1000.0),
                )
            })
            .collect();
        if let Some(id) = self.hovered
            && !lifts.iter().any(|(card, _)| *card == id)
        {
            lifts.push((id, 1.0));
        }
        lifts
    }

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
            if a.id != id || a.kind.hover() {
                continue;
            }
            let t = (a.start.elapsed().as_millis() as f32 / a.kind.dur() as f32).clamp(0.0, 1.0);
            let eased = ease_out_cubic(t);
            return match a.kind {
                AnimKind::Appear => shelf_card::appear(eased),
                AnimKind::Disappear => shelf_card::dismiss(eased),
                // Skipped above: a hover fade does not scale or fade the card.
                AnimKind::HoverIn | AnimKind::HoverOut => (1.0, 1.0),
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

    /// Also while a replay clip is being written: its arc turns.
    pub(super) fn animating(&self) -> bool {
        !self.anims.is_empty() || self.pending.active()
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
        self.pending.retire(std::time::Instant::now());
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

    #[test]
    fn a_hover_fade_goes_both_ways_and_turns_around_where_it_stands() {
        use super::{hover_begun, hover_lift};
        assert_eq!(hover_lift(AnimKind::HoverIn, 0.0), 0.0);
        assert_eq!(hover_lift(AnimKind::HoverIn, 60.0), 0.5);
        assert_eq!(hover_lift(AnimKind::HoverIn, 500.0), 1.0);
        assert_eq!(hover_lift(AnimKind::HoverOut, 30.0), 0.75);
        // Out at 0.75, turned in: the fade in begins as if 90 ms ago.
        let begun = hover_begun(AnimKind::HoverIn, 0.75);
        assert_eq!(hover_lift(AnimKind::HoverIn, begun), 0.75);
        let begun = hover_begun(AnimKind::HoverOut, 0.25);
        assert_eq!(hover_lift(AnimKind::HoverOut, begun), 0.25);
        // Never more than the 150 ms asked for.
        const { assert!(boltsnap::shelf_card::HOVER_MS <= 150) };
    }
}
