//! The toolbar's motion: every visible change of it is a `QUICK` spring
//! (`/ui` motion). The platform hands in what should be (layout, pointer,
//! toggles) at the moment it changes; drawing samples what is.

use std::time::{Duration, Instant};

use boltsnap::ui::motion::Spring;
use boltsnap::ui::theme::QUICK;

use super::render::toolbar::{self, CLIP, ControlLook, FRAME, Layout, Look};

/// What the toolbar should show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Targets {
    pub hovered: Option<usize>,
    pub pressed: Option<usize>,
    /// Frame and Audio switched on.
    pub on: [bool; 4],
    /// Clip has a buffer to clip from (the others always do).
    pub enabled: [bool; 4],
    /// Why Clip has none, said while the pointer is on it.
    pub reason: Option<&'static str>,
    /// The clip's size in pixels, if the selection makes one.
    pub size: Option<(u32, u32)>,
}

/// How long the pointer rests on an icon-only control before its name
/// shows: sweeping over the toolbar shows none.
pub const NAME_DELAY: Duration = Duration::from_millis(400);

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
    /// Why Clip cannot be pressed, beside the current toolbar, with its
    /// opacity (already times the toolbar's).
    pub hint: Option<(&'static str, f32)>,
    /// Names of icon-only controls beside them, with their opacity: the one
    /// showing and any fading out.
    pub tips: Vec<(usize, &'static str, f32)>,
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

/// A control's name: it shows from `from` on.
struct Tip {
    control: usize,
    text: &'static str,
    alpha: Spring,
    from: Instant,
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
    /// The reason beside the toolbar; the text stays while it fades out.
    hint: Spring,
    hint_text: Option<&'static str>,
    /// The last is the one showing or waiting to, unless it fades out.
    tips: Vec<Tip>,
    size: Option<(u32, u32)>,
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
            hint: Spring::settled(0.0, now),
            hint_text: None,
            tips: Vec::new(),
            size: targets.size,
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
            size: self.size,
        }
    }

    /// The name of the icon-only control under the pointer: after
    /// `NAME_DELAY` at rest; at once while another one shows (they swap);
    /// out with `QUICK` when the pointer leaves. A name whose state changes
    /// crossfades to the new word.
    fn update_tips(&mut self, hovered: Option<usize>, on: &[bool; 4], now: Instant) {
        let fade_out = |tip: &mut Tip| {
            if tip.alpha.target() != 0.0 {
                tip.alpha.retarget(0.0, QUICK, now);
            }
        };
        let want = hovered
            .filter(|&i| i >= FRAME)
            .and_then(|i| Some((i, toolbar::name(i, on[i])?)));
        let current = self.tips.last().filter(|tip| tip.alpha.target() == 1.0);
        let (showing, same) = match (current, want) {
            (Some(tip), Some((i, text))) => (now >= tip.from, tip.control == i && tip.text == text),
            _ => (false, false),
        };
        if same {
            return;
        }
        let waited = current.filter(|tip| tip.control == want.map_or(usize::MAX, |w| w.0));
        let from = match (showing, waited) {
            (true, _) => now,
            // The word changed while waiting: the wait goes on.
            (false, Some(tip)) => tip.from,
            (false, None) => now + NAME_DELAY,
        };
        self.tips.iter_mut().for_each(fade_out);
        if let Some((control, text)) = want {
            let mut alpha = Spring::settled(0.0, now);
            alpha.retarget(1.0, QUICK, from);
            self.tips.push(Tip {
                control,
                text,
                alpha,
                from,
            });
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
        self.size = targets.size;
        let hovered = targets.hovered.filter(|_| layout.is_some());
        self.update_tips(hovered, &targets.on, now);
        self.tips
            .retain(|tip| !tip.alpha.done(now) || tip.alpha.target() != 0.0);
        let hint = targets.hovered == Some(CLIP) && !targets.enabled[CLIP] && layout.is_some();
        let hint = hint.then_some(targets.reason).flatten();
        if hint.is_some() {
            self.hint_text = hint;
        }
        self.hint.retarget(unit(hint.is_some()), QUICK, now);
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
            hint: self
                .current
                .as_ref()
                .zip(self.hint_text)
                .and_then(|(shown, text)| {
                    let alpha = self.hint.value(now).clamp(0.0, 1.0)
                        * shown.alpha.value(now).clamp(0.0, 1.0);
                    visible(alpha).then_some((text, alpha))
                }),
            tips: self
                .current
                .as_ref()
                .map(|shown| {
                    let bar = shown.alpha.value(now).clamp(0.0, 1.0);
                    self.tips
                        .iter()
                        .map(|tip| {
                            let alpha = tip.alpha.value(now).clamp(0.0, 1.0) * bar;
                            (tip.control, tip.text, alpha)
                        })
                        .filter(|tip| visible(tip.2))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// A spring still moves something that is (or was just) on screen.
    pub fn moving(&self, now: Instant) -> bool {
        let shown = self.current.as_ref();
        let current = shown.is_some_and(|s| !s.alpha.done(now));
        let controls = shown.is_some_and(|s| s.alpha.target() > 0.0 || !s.alpha.done(now))
            && self.controls.iter().flatten().any(|s| !s.done(now));
        current
            || controls
            || (shown.is_some() && !self.hint.done(now))
            || (shown.is_some() && self.tips.iter().any(|t| !t.alpha.done(now)))
            || self.ghosts.iter().any(|g| !g.alpha.done(now))
    }
}

/// The same rows, and the same side of the selection.
fn same_arrangement(a: &Layout, b: &Layout) -> bool {
    a.arrangement == b.arrangement && a.placement == b.placement
}

#[cfg(test)]
#[path = "motion_tests.rs"]
mod tests;
