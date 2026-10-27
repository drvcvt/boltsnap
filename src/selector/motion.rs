//! The toolbar's motion: every visible change of it is a `QUICK` spring
//! (`/ui` motion). The platform hands in what should be (layout, pointer,
//! toggles) at the moment it changes; drawing samples what is.

use std::time::Instant;

use boltsnap::ui::motion::Spring;
use boltsnap::ui::theme::QUICK;

use super::render::toolbar::{ControlLook, Layout, Look};

/// What the toolbar should show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Targets {
    pub hovered: Option<usize>,
    pub pressed: Option<usize>,
    /// Frame and Audio switched on.
    pub on: [bool; 4],
    /// Clip has a buffer to clip from (the others always do).
    pub enabled: [bool; 4],
}

/// One toolbar as drawn this frame, with its opacity.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    pub layout: Layout,
    pub look: Look,
    pub alpha: f32,
}

/// Everything of the toolbar a frame draws: older toolbars fading out
/// where they were, then the current one.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Bar {
    pub ghosts: Vec<Drawn>,
    pub current: Option<Drawn>,
}

impl Bar {
    pub fn all(&self) -> impl Iterator<Item = &Drawn> {
        self.ghosts.iter().chain(self.current.iter())
    }
}

struct Shown {
    layout: Layout,
    alpha: Spring,
}

struct Ghost {
    layout: Layout,
    look: Look,
    alpha: Spring,
}

/// At most this many toolbars fade out at once; a fifth switch within one
/// `QUICK` drops the oldest, which is nearly gone by then.
const GHOSTS: usize = 4;

pub struct BarMotion {
    current: Option<Shown>,
    ghosts: Vec<Ghost>,
    /// Per control: hover, press, on, enabled.
    controls: [[Spring; 4]; 4],
}

fn unit(on: bool) -> f32 {
    f32::from(u8::from(on))
}

fn fade(from: f32, to: f32, now: Instant) -> Spring {
    let mut spring = Spring::settled(from, now);
    spring.retarget(to, QUICK, now);
    spring
}

impl BarMotion {
    /// Toggles and Clip as they open, without fading to them.
    pub fn new(targets: &Targets, now: Instant) -> Self {
        Self {
            current: None,
            ghosts: Vec::new(),
            controls: std::array::from_fn(|i| {
                [0.0, 0.0, unit(targets.on[i]), unit(targets.enabled[i])]
                    .map(|v| Spring::settled(v, now))
            }),
        }
    }

    fn look(&self, now: Instant) -> Look {
        Look {
            controls: std::array::from_fn(|i| {
                let [hover, press, on, enabled] =
                    self.controls[i].map(|s| s.value(now).clamp(0.0, 1.0));
                ControlLook {
                    hover,
                    press,
                    on,
                    enabled,
                }
            }),
        }
    }

    /// `layout` is the toolbar there should be (`None`: none). The same
    /// arrangement follows it at once, as the selection is dragged; another
    /// arrangement (strip, grid, column; above, below, inside) or a toolbar
    /// coming back fades in while the old one fades out where it was.
    pub fn update(&mut self, layout: Option<Layout>, targets: &Targets, now: Instant) {
        for (i, springs) in self.controls.iter_mut().enumerate() {
            let to = [
                unit(targets.hovered == Some(i)),
                unit(targets.pressed == Some(i)),
                unit(targets.on[i]),
                unit(targets.enabled[i]),
            ];
            for (spring, to) in springs.iter_mut().zip(to) {
                spring.retarget(to, QUICK, now);
            }
        }
        match (layout, self.current.as_mut()) {
            (Some(layout), Some(shown))
                if shown.alpha.target() == 1.0 && same_arrangement(&shown.layout, &layout) =>
            {
                shown.layout = layout;
            }
            (Some(layout), _) => {
                if let Some(old) = self.current.take() {
                    let look = self.look(now);
                    let mut alpha = old.alpha;
                    alpha.retarget(0.0, QUICK, now);
                    self.ghosts.push(Ghost {
                        layout: old.layout,
                        look,
                        alpha,
                    });
                }
                self.current = Some(Shown {
                    layout,
                    alpha: fade(0.0, 1.0, now),
                });
            }
            (None, Some(shown)) => shown.alpha.retarget(0.0, QUICK, now),
            (None, None) => {}
        }
        self.ghosts
            .retain(|ghost| !ghost.alpha.done(now) || ghost.alpha.target() != 0.0);
        if self.ghosts.len() > GHOSTS {
            self.ghosts.drain(..self.ghosts.len() - GHOSTS);
        }
    }

    /// The toolbar that takes input: the current one, unless it is going.
    pub fn layout(&self) -> Option<&Layout> {
        self.current
            .as_ref()
            .filter(|shown| shown.alpha.target() == 1.0)
            .map(|shown| &shown.layout)
    }

    pub fn sample(&self, now: Instant) -> Bar {
        let look = self.look(now);
        let visible = |alpha: f32| alpha > 0.0;
        Bar {
            ghosts: self
                .ghosts
                .iter()
                .map(|ghost| Drawn {
                    layout: ghost.layout,
                    look: ghost.look,
                    alpha: ghost.alpha.value(now).clamp(0.0, 1.0),
                })
                .filter(|drawn| visible(drawn.alpha))
                .collect(),
            current: self
                .current
                .as_ref()
                .map(|shown| Drawn {
                    layout: shown.layout,
                    look,
                    alpha: shown.alpha.value(now).clamp(0.0, 1.0),
                })
                .filter(|drawn| visible(drawn.alpha)),
        }
    }

    /// A spring still moves something that is (or was just) on screen.
    pub fn moving(&self, now: Instant) -> bool {
        let shown = self.current.as_ref();
        let current = shown.is_some_and(|s| !s.alpha.done(now));
        let controls = shown.is_some_and(|s| s.alpha.target() > 0.0 || !s.alpha.done(now))
            && self.controls.iter().flatten().any(|s| !s.done(now));
        current || controls || self.ghosts.iter().any(|g| !g.alpha.done(now))
    }
}

/// Strip, grid or column, and the same side of the selection.
fn same_arrangement(a: &Layout, b: &Layout) -> bool {
    a.columns == b.columns && a.placement == b.placement
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
