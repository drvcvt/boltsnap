//! P2: the monitors to scale in their real arrangement, the ones a
//! recording takes lit, following the Display and "Both displays as" rows.

use std::time::Instant;

use super::super::pages::Screen;
use super::super::{App, Bind, Block};
use super::Preview;
use crate::settings;
use crate::ui::motion::Spring;
use crate::ui::theme::{self, GAP_M, GAP_XS, PANEL_RADIUS, QUICK, RADIUS, TextStyle};
use crate::ui::{Canvas, Rect};

/// The tallest the arrangement gets.
const MOST_H: f32 = 200.0;
/// Between neighbours, so two monitors read as two.
const SEAM: f32 = 2.0;

/// The preview's height for monitors `screens` in a column `width` wide.
pub fn height(width: f32, screens: &[Screen]) -> f32 {
    let Some(bounds) = bounds(screens) else {
        return 0.0;
    };
    let inner = (width - 2.0 * GAP_M).max(0.0);
    (inner * bounds.h / bounds.w).min(MOST_H) + 2.0 * GAP_M
}

fn bounds(screens: &[Screen]) -> Option<Rect> {
    let left = screens.iter().map(|s| s.x).reduce(f32::min)?;
    let top = screens.iter().map(|s| s.y).reduce(f32::min)?;
    let right = screens.iter().map(|s| s.x + s.w).reduce(f32::max)?;
    let bottom = screens.iter().map(|s| s.y + s.h).reduce(f32::max)?;
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

/// Where each monitor goes in the preview block `rect`: scaled to fit
/// `GAP_M` inside it, centred, `SEAM` apart, on whole pixels.
pub fn places(rect: Rect, screens: &[Screen]) -> Vec<Rect> {
    let Some(bounds) = bounds(screens) else {
        return Vec::new();
    };
    let inner = rect.inset(GAP_M);
    let scale = (inner.w / bounds.w).min(inner.h / bounds.h);
    let (w, h) = (bounds.w * scale, bounds.h * scale);
    let (x0, y0) = (inner.x + (inner.w - w) / 2.0, inner.y + (inner.h - h) / 2.0);
    screens
        .iter()
        .map(|s| {
            let x = (x0 + (s.x - bounds.x) * scale).round();
            let y = (y0 + (s.y - bounds.y) * scale).round();
            let right = (x0 + (s.x + s.w - bounds.x) * scale).round();
            let bottom = (y0 + (s.y + s.h - bounds.y) * scale).round();
            Rect::new(x, y, right - x, bottom - y).inset(SEAM / 2.0)
        })
        .collect()
}

/// What a recording takes now: (`record_default_target` as the Display row
/// would write it, "Both displays as" combined).
fn target(app: &App) -> Option<(String, bool)> {
    let mut target = None;
    let mut combined = false;
    for block in &app.page().blocks {
        let Block::Row(row) = block else { continue };
        match &row.bind {
            Bind::Key("record_default_target", encode) => {
                if let Some(settings::Value::Text(value)) = encode(&row.control.value()) {
                    target = Some(value);
                }
            }
            Bind::Key("record_both_mode", encode) => {
                combined =
                    encode(&row.control.value()) == Some(settings::Value::Text("combined".into()));
            }
            _ => {}
        }
    }
    target.map(|target| (target, combined))
}

/// How lit each monitor is for `target`, and the bridge between them.
fn lights(screens: &[Screen], target: &str, combined: bool) -> (Vec<f32>, f32) {
    let lit = screens
        .iter()
        .map(|screen| match target {
            // Whichever has focus when the recording starts: any of them.
            "focused" => 0.5,
            "both" => 1.0,
            name => f32::from(u8::from(name.strip_prefix("output:") == Some(&screen.name))),
        })
        .collect();
    let bridge = f32::from(u8::from(target == "both" && combined && screens.len() > 1));
    (lit, bridge)
}

impl App {
    fn screens(&self) -> Option<&[Screen]> {
        self.page().blocks.iter().find_map(|block| match block {
            Block::Preview(Preview::Displays(screens)) => Some(screens.as_slice()),
            _ => None,
        })
    }

    pub(super) fn sync_displays(&mut self, animate: bool, now: Instant) {
        let (Some(screens), Some((target, combined))) = (self.screens(), target(self)) else {
            return;
        };
        let (lit, bridge) = lights(screens, &target, combined);
        let animate = animate && !self.reduced_motion;
        if self.preview.lit.len() != lit.len() || !animate {
            self.preview.lit = lit.iter().map(|on| Spring::settled(*on, now)).collect();
            self.preview.bridge = Spring::settled(bridge, now);
            return;
        }
        for (spring, on) in self.preview.lit.iter_mut().zip(lit) {
            spring.retarget(on, QUICK, now);
        }
        self.preview.bridge.retarget(bridge, QUICK, now);
    }

    pub(super) fn displays_moving(&self, now: Instant) -> bool {
        !self.preview.bridge.done(now) || self.preview.lit.iter().any(|s| !s.done(now))
    }

    /// Each monitor's place in the block and how lit it is now.
    pub fn display_lights(&self, rect: Rect, now: Instant) -> (Vec<(Rect, f32)>, f32) {
        let screens = self.screens().unwrap_or_default();
        let lit = places(rect, screens)
            .into_iter()
            .zip(&self.preview.lit)
            .map(|(place, spring)| (place, spring.value(now).clamp(0.0, 1.0)))
            .collect();
        (lit, self.preview.bridge.value(now).clamp(0.0, 1.0))
    }

    pub(super) fn draw_displays(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        let screens = self.screens().unwrap_or_default();
        let (lit, bridge) = self.display_lights(rect, now);
        let radius = RADIUS - GAP_XS;
        // Combined: one clip over all of them, the seams closed.
        if bridge > 0.0
            && let Some(first) = lit.first()
        {
            let all = lit.iter().fold(first.0, |a, (b, _)| {
                let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                Rect::new(
                    x,
                    y,
                    a.right().max(b.right()) - x,
                    a.bottom().max(b.bottom()) - y,
                )
            });
            canvas.fill(all, radius, theme::SELECTED, bridge);
        }
        for ((place, on), screen) in lit.iter().zip(screens) {
            canvas.fill(
                *place,
                radius,
                theme::mix(theme::RAISE, theme::SELECTED, *on),
                1.0,
            );
            let color = theme::mix(theme::SUB, theme::TEXT, *on);
            let name = self
                .fonts
                .ellipsize(TextStyle::Hint, &screen.label, place.w - 2.0 * GAP_M);
            let x = place.x + (place.w - self.fonts.width(TextStyle::Hint, &name)) / 2.0;
            let baseline = self.fonts.baseline(TextStyle::Hint, place.y, place.h);
            self.fonts.draw(
                canvas,
                TextStyle::Hint,
                &name,
                (x.round(), baseline),
                color,
                1.0,
            );
        }
    }
}
