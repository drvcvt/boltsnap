//! Hover, press and focus of every target, each a `QUICK` spring, so no
//! state switches hard.

use std::time::Instant;

use super::Target;
use crate::ui::motion::Spring;
use crate::ui::theme::QUICK;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Hover,
    Press,
    Focus,
}

#[derive(Default)]
pub struct Fades(Vec<(Target, Channel, Spring)>);

impl Fades {
    pub fn to(&mut self, target: Target, channel: Channel, to: f32, now: Instant) {
        match self
            .0
            .iter_mut()
            .find(|(t, c, _)| *t == target && *c == channel)
        {
            Some((_, _, spring)) => spring.retarget(to, QUICK, now),
            None => {
                let mut spring = Spring::settled(0.0, now);
                spring.retarget(to, QUICK, now);
                self.0.push((target, channel, spring));
            }
        }
        // What has faded out completely is the same as never lit.
        self.0
            .retain(|(_, _, spring)| !(spring.done(now) && spring.target() == 0.0));
    }

    pub fn value(&self, target: Target, channel: Channel, now: Instant) -> f32 {
        self.0
            .iter()
            .find(|(t, c, _)| *t == target && *c == channel)
            .map_or(0.0, |(_, _, spring)| spring.value(now).clamp(0.0, 1.0))
    }

    /// Drop every control's fades: control targets name blocks of one page,
    /// and a new page reuses the numbers.
    pub fn forget_controls(&mut self) {
        self.0
            .retain(|(target, _, _)| !matches!(target, Target::Control(_)));
    }

    pub fn moving(&self, now: Instant) -> bool {
        self.0.iter().any(|(_, _, spring)| !spring.done(now))
    }
}
