//! P2/V1: the monitors to scale in their real arrangement, each a monitor on
//! its stand with its name under it. Those a recording takes are on (a
//! lighter screen surface); a line under them says what the recording makes. Follows the Display and
//! "Both displays as" rows.

use std::time::Instant;

use super::super::pages::Screen;
use super::super::{App, Bind, Block};
use super::Preview;
use crate::settings;
use crate::ui::motion::Spring;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, GAP_S, GAP_XS, PANEL_RADIUS, QUICK, RADIUS, TextStyle};
use crate::ui::{Canvas, Rect};

/// The tallest the monitors get.
const MOST_H: f32 = 160.0;
/// Between neighbours, so two monitors read as two.
const SEAM: f32 = GAP_XS;
/// A monitor's frame around its screen, and their corners (concentric).
const BEZEL: f32 = 3.0;
const BODY_RADIUS: f32 = RADIUS - GAP_XS;
const SCREEN_RADIUS: f32 = BODY_RADIUS - BEZEL;
/// The stand: a neck, then the foot, as shares of the monitor's width.
const NECK_H: f32 = 5.0;
const FOOT_H: f32 = 3.0;
const STAND_H: f32 = NECK_H + FOOT_H;
const NECK: f32 = 0.08;
const FOOT: f32 = 0.28;
/// Lines under a monitor: its name (`Hint`, capitals 9 with 3 above and
/// below) and its connector (`Key`, 8 with 3); the line under all (`Label`).
const NAME_H: f32 = 15.0;
const PORT_H: f32 = 14.0;
const LINE_H: f32 = 16.0;
const MOST_LINES: usize = 2;
/// A screen that is on: brighter than `SELECTED`, so
/// half on (Focused) still stands apart from its `RAISE` frame.
const LIT: f32 = 0.3;

const FOCUSED: &str = "The one with focus when you start";
const MISSING: &str = "Not connected, the focused one records";
const ONE: &str = "One clip";

/// What a recording of each of `n` monitors separately makes.
fn clips(n: usize) -> String {
    match n {
        0 | 1 => ONE.to_owned(),
        2 => "Two clips".to_owned(),
        3 => "Three clips".to_owned(),
        n => format!("{n} clips"),
    }
}

/// Every line the preview can show for `n` monitors, for its height.
pub fn every_caption(n: usize) -> [String; 4] {
    [FOCUSED.into(), MISSING.into(), ONE.into(), clips(n)]
}

/// The preview's height for monitors `screens` in a column `width` wide.
pub fn height(fonts: &Fonts, width: f32, screens: &[Screen]) -> f32 {
    let Some(bounds) = bounds(screens) else {
        return 0.0;
    };
    let inner = (width - 2.0 * GAP_M).max(0.0);
    let lines = every_caption(screens.len())
        .iter()
        .map(|text| fonts.wrap(TextStyle::Label, text, inner, MOST_LINES).len())
        .max()
        .unwrap_or(1);
    2.0 * GAP_M
        + monitors_h(inner, bounds)
        + STAND_H
        + GAP_S
        + NAME_H
        + PORT_H
        + GAP_S
        + lines as f32 * LINE_H
}

fn monitors_h(inner_w: f32, bounds: Rect) -> f32 {
    (inner_w * bounds.h / bounds.w).min(MOST_H).floor()
}

fn bounds(screens: &[Screen]) -> Option<Rect> {
    let left = screens.iter().map(|s| s.x).reduce(f32::min)?;
    let top = screens.iter().map(|s| s.y).reduce(f32::min)?;
    let right = screens.iter().map(|s| s.x + s.w).reduce(f32::max)?;
    let bottom = screens.iter().map(|s| s.y + s.h).reduce(f32::max)?;
    (right > left && bottom > top).then(|| Rect::new(left, top, right - left, bottom - top))
}

/// The monitors together inside the block `rect`, scaled to fit `GAP_M` in
/// and at most `MOST_H` tall, centred; with the scale.
fn area(rect: Rect, screens: &[Screen]) -> Option<(Rect, f32)> {
    let bounds = bounds(screens)?;
    let inner = rect.inset(GAP_M);
    let room = monitors_h(inner.w, bounds);
    let scale = (inner.w / bounds.w).min(room / bounds.h);
    let (w, h) = (bounds.w * scale, bounds.h * scale);
    let at = Rect::new(
        inner.x + (inner.w - w) / 2.0,
        inner.y + (room - h) / 2.0,
        w,
        h,
    );
    Some((at, scale))
}

/// Where each monitor goes in the preview block `rect`: its frame, `SEAM`
/// from its neighbours, on whole pixels.
pub fn places(rect: Rect, screens: &[Screen]) -> Vec<Rect> {
    let (Some(bounds), Some((at, scale))) = (bounds(screens), area(rect, screens)) else {
        return Vec::new();
    };
    screens
        .iter()
        .map(|s| {
            let x = (at.x + (s.x - bounds.x) * scale).round();
            let y = (at.y + (s.y - bounds.y) * scale).round();
            let right = (at.x + (s.x + s.w - bounds.x) * scale).round();
            let bottom = (at.y + (s.y + s.h - bounds.y) * scale).round();
            Rect::new(x, y, right - x, bottom - y).inset(SEAM / 2.0)
        })
        .collect()
}

fn across(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right()
}

/// Each monitor's stand (neck, foot), unless another monitor is below it.
fn stands(places: &[Rect]) -> Vec<Option<(Rect, Rect)>> {
    places
        .iter()
        .map(|body| {
            let below = places
                .iter()
                .any(|other| other.y > body.y && across(*body, *other));
            if below {
                return None;
            }
            let middle = body.x + body.w / 2.0;
            let part = |share: f32, least: f32, y: f32, h: f32| {
                let w = (body.w * share).round().max(least);
                Rect::new((middle - w / 2.0).round(), y, w, h)
            };
            Some((
                part(NECK, 4.0, body.bottom(), NECK_H),
                part(FOOT, 12.0, body.bottom() + NECK_H, FOOT_H),
            ))
        })
        .collect()
}

/// Between neighbouring monitors, for one clip over both: the seam and
/// `BODY_RADIUS` into each body, over the height (or width) they share, so
/// their facing corners fill square and the two read as one double monitor.
/// Their screens keep their frames.
fn joins(places: &[Rect]) -> Vec<Rect> {
    let mut joins = Vec::new();
    for (i, a) in places.iter().enumerate() {
        for b in &places[i + 1..] {
            let (left, right) = if a.x <= b.x { (a, b) } else { (b, a) };
            let (top, bottom) = if a.y <= b.y { (a, b) } else { (b, a) };
            if (right.x - left.right() - SEAM).abs() <= 1.0 {
                let y = left.y.max(right.y);
                let h = left.bottom().min(right.bottom()) - y;
                let x = left.right() - BODY_RADIUS;
                joins.push(Rect::new(x, y, right.x + BODY_RADIUS - x, h));
            } else if (bottom.y - top.bottom() - SEAM).abs() <= 1.0 {
                let x = top.x.max(bottom.x);
                let w = top.right().min(bottom.right()) - x;
                let y = top.bottom() - BODY_RADIUS;
                joins.push(Rect::new(x, y, w, bottom.y + BODY_RADIUS - y));
            }
        }
    }
    joins.retain(|join| join.w > 0.0 && join.h > 0.0);
    joins
}

/// The top of the names' lines in the block `rect`.
fn names_top(rect: Rect, screens: &[Screen]) -> f32 {
    let inner = rect.inset(GAP_M);
    let room = bounds(screens).map_or(0.0, |bounds| monitors_h(inner.w, bounds));
    inner.y + room + STAND_H + GAP_S
}

/// Where each monitor's name and connector go: under it, as wide as it.
/// Monitors above one another share the width equally instead, in order of
/// where they are, so no names lie on top of each other.
pub fn name_places(rect: Rect, screens: &[Screen]) -> Vec<Rect> {
    let places = places(rect, screens);
    let (y, h) = (names_top(rect, screens), NAME_H + PORT_H);
    let stacked = places
        .iter()
        .enumerate()
        .any(|(i, a)| places[i + 1..].iter().any(|b| across(*a, *b)));
    if !stacked {
        return places.iter().map(|p| Rect::new(p.x, y, p.w, h)).collect();
    }
    let inner = rect.inset(GAP_M);
    let mut order: Vec<usize> = (0..places.len()).collect();
    order.sort_by(|&a, &b| {
        (places[a].x, places[a].y)
            .partial_cmp(&(places[b].x, places[b].y))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let w = (inner.w / places.len() as f32).floor();
    let mut slots = vec![Rect::new(0.0, 0.0, 0.0, 0.0); places.len()];
    for (slot, index) in order.into_iter().enumerate() {
        slots[index] = Rect::new(inner.x + slot as f32 * w, y, w, h);
    }
    slots
}

/// Where the line under everything goes in the block `rect`.
pub fn caption_place(rect: Rect, screens: &[Screen]) -> Rect {
    let inner = rect.inset(GAP_M);
    let y = names_top(rect, screens) + NAME_H + PORT_H + GAP_S;
    Rect::new(inner.x, y, inner.w, inner.bottom() - y)
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

/// The monitor a target names, when it is connected.
fn named<'a>(screens: &'a [Screen], target: &str) -> Option<&'a Screen> {
    let name = target.strip_prefix("output:")?;
    screens.iter().find(|screen| screen.name == name)
}

/// How lit each monitor is for `target`, and the bridge between them.
fn lights(screens: &[Screen], target: &str, combined: bool) -> (Vec<f32>, f32) {
    let output = named(screens, target);
    let lit = screens
        .iter()
        .map(|screen| match target {
            "both" => 1.0,
            _ if target.starts_with("output:") && output.is_some() => {
                f32::from(u8::from(output.is_some_and(|o| o.name == screen.name)))
            }
            // Whichever has focus when the recording starts: any of them.
            // A monitor that is not connected falls back to that.
            _ => 0.5,
        })
        .collect();
    let bridge = f32::from(u8::from(target == "both" && combined && screens.len() > 1));
    (lit, bridge)
}

/// The line under the monitors: what the recording makes.
fn caption(screens: &[Screen], target: &str, combined: bool) -> String {
    match target {
        "focused" => FOCUSED.into(),
        "both" if combined => ONE.into(),
        "both" => clips(screens.len()),
        _ if named(screens, target).is_some() => ONE.into(),
        _ => MISSING.into(),
    }
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
        let text = caption(screens, &target, combined);
        let animate = animate && !self.reduced_motion;
        let captions = &mut self.preview.captions;
        if self.preview.lit.len() != lit.len() || !animate {
            self.preview.lit = lit.iter().map(|on| Spring::settled(*on, now)).collect();
            self.preview.bridge = Spring::settled(bridge, now);
            *captions = vec![(text, Spring::settled(1.0, now))];
            return;
        }
        for (spring, on) in self.preview.lit.iter_mut().zip(lit) {
            spring.retarget(on, QUICK, now);
        }
        self.preview.bridge.retarget(bridge, QUICK, now);
        // The old line fades out where it is while the new one fades in.
        for (old, spring) in captions.iter_mut() {
            spring.retarget(f32::from(u8::from(*old == text)), QUICK, now);
        }
        if !captions.iter().any(|(old, _)| *old == text) {
            let mut spring = Spring::settled(0.0, now);
            spring.retarget(1.0, QUICK, now);
            captions.push((text, spring));
        }
        captions.retain(|(_, spring)| !(spring.done(now) && spring.target() == 0.0));
    }

    pub(super) fn displays_moving(&self, now: Instant) -> bool {
        !self.preview.bridge.done(now)
            || self.preview.lit.iter().any(|s| !s.done(now))
            || self.preview.captions.iter().any(|(_, s)| !s.done(now))
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

    /// The lines under the monitors now, with how much each shows.
    pub fn display_captions(&self, now: Instant) -> Vec<(String, f32)> {
        self.preview
            .captions
            .iter()
            .map(|(text, spring)| (text.clone(), spring.value(now).clamp(0.0, 1.0)))
            .filter(|(_, shown)| *shown > 0.0)
            .collect()
    }

    pub(super) fn draw_displays(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        let screens = self.screens().unwrap_or_default();
        if bounds(screens).is_none() {
            return;
        }
        let (lit, bridge) = self.display_lights(rect, now);
        let bodies: Vec<Rect> = lit.iter().map(|(place, _)| *place).collect();
        let light = |canvas: &mut Canvas, clip: Rect, radius: f32, on: f32| {
            canvas.fill(
                clip,
                radius,
                theme::mix(theme::SELECTED, theme::SUB, LIT),
                on,
            );
        };
        for (body, stand) in bodies.iter().zip(stands(&bodies)) {
            canvas.fill(*body, BODY_RADIUS, theme::RAISE, 1.0);
            if let Some((neck, foot)) = stand {
                canvas.fill(neck, 0.0, theme::RAISE, 1.0);
                canvas.fill(foot, FOOT_H / 2.0, theme::RAISE, 1.0);
            }
        }
        // Combined: one clip over all of them, their bodies joined into one.
        if bridge > 0.0 {
            for join in joins(&bodies) {
                canvas.fill(join, 0.0, theme::RAISE, bridge);
            }
        }
        for (body, on) in &lit {
            let screen = body.inset(BEZEL);
            canvas.fill(screen, SCREEN_RADIUS, theme::WINDOW, 1.0);
            light(canvas, screen, SCREEN_RADIUS, *on);
        }
        let fonts = &self.fonts;
        for ((place, (_, on)), screen) in name_places(rect, screens).iter().zip(&lit).zip(screens) {
            let mut line = |style, text: &str, y: f32, h: f32, color| {
                let text = fonts.ellipsize(style, text, place.w);
                let x = place.x + (place.w - fonts.width(style, &text)) / 2.0;
                let baseline = fonts.baseline(style, y, h);
                fonts.draw(canvas, style, &text, (x.round(), baseline), color, 1.0);
            };
            let name = theme::mix(theme::SUB, theme::TEXT, *on);
            line(TextStyle::Hint, &screen.label, place.y, NAME_H, name);
            if screen.label != screen.name {
                let y = place.y + NAME_H;
                line(TextStyle::Key, &screen.name, y, PORT_H, theme::SUB);
            }
        }
        let place = caption_place(rect, screens);
        for (text, shown) in self.display_captions(now) {
            let lines = fonts.wrap(TextStyle::Label, &text, place.w, MOST_LINES);
            for (index, line) in lines.iter().enumerate() {
                let y = place.y + index as f32 * LINE_H;
                let x = place.x + (place.w - fonts.width(TextStyle::Label, line)) / 2.0;
                let baseline = fonts.baseline(TextStyle::Label, y, LINE_H);
                fonts.draw(
                    canvas,
                    TextStyle::Label,
                    line,
                    (x.round(), baseline),
                    theme::TEXT,
                    shown,
                );
            }
        }
    }
}
