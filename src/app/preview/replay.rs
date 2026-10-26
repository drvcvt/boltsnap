//! P3: the replay buffer on a timeline: its length from "now" back, how much
//! it holds right now, and the part the memory cannot hold.

use std::time::Instant;

use super::super::{App, Bind, Block};
use super::Preview;
use crate::replay::settings::Settings;
use crate::settings;
use crate::ui::theme::{self, GAP_M, GAP_S, GLIDE, PANEL_RADIUS, TextStyle};
use crate::ui::{Canvas, Rect};

/// The bar and the line of times under it.
const BAR_H: f32 = 10.0;
const LABEL_H: f32 = 14.0;
/// Round steps for the marks; the smallest that keeps them to `MOST_MARKS`.
const STEPS: [u64; 8] = [5, 10, 15, 30, 60, 120, 300, 600];
const MOST_MARKS: u64 = 6;
/// Room each mark's time needs, so narrow windows show fewer.
const MARK_ROOM: f32 = 64.0;

/// The preview's height: the bar, a gap, the times, `GAP_M` around.
pub fn height() -> f32 {
    2.0 * GAP_M + BAR_H + GAP_S + LABEL_H
}

/// The bar inside the block `rect`.
pub fn bar(rect: Rect) -> Rect {
    let inner = rect.inset(GAP_M);
    Rect::new(inner.x, inner.y, inner.w, BAR_H)
}

/// Seconds between marks for a buffer `length` long on a bar `width` wide.
pub fn step(length: f32, width: f32) -> u64 {
    let most = MOST_MARKS.min((width / MARK_ROOM).floor().max(1.0) as u64);
    STEPS
        .into_iter()
        .find(|step| (length / *step as f32).ceil() as u64 <= most)
        .unwrap_or(STEPS[STEPS.len() - 1])
}

/// "1 min", "30 s": as the Length row writes them.
fn time(seconds: u64) -> String {
    if seconds >= 60 && seconds.is_multiple_of(60) {
        format!("{}\u{a0}min", seconds / 60)
    } else {
        format!("{seconds}\u{a0}s")
    }
}

/// What the bar shows: (length, seconds the memory holds) as the Length and
/// Memory rows hold them now.
fn shape(app: &App) -> Option<(f32, f32)> {
    let mut bitrate = None;
    let (mut length, mut memory) = (None, None);
    for block in &app.page().blocks {
        match block {
            Block::Preview(Preview::Replay { bitrate_kbps }) => bitrate = Some(*bitrate_kbps),
            Block::Row(row) => {
                let Bind::Key(key, encode) = &row.bind else {
                    continue;
                };
                let value = encode(&row.control.value());
                match (*key, value) {
                    ("replay.duration_seconds", Some(settings::Value::Int(s))) => length = Some(s),
                    ("replay.memory_mib", Some(settings::Value::Int(m))) => memory = Some(m),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    let (bitrate, length, memory) = (bitrate?, length?, memory?);
    let held = Settings {
        seconds: length as u64,
        memory_mib: memory as usize,
        bitrate_kbps: bitrate,
        ..Settings::default()
    }
    .seconds_held();
    Some((length as f32, held as f32))
}

impl App {
    pub(super) fn sync_replay(&mut self, animate: bool, now: Instant) {
        let Some((length, held)) = shape(self) else {
            return;
        };
        let animate = animate && !self.reduced_motion;
        for (spring, to) in [
            (&mut self.preview.length, length),
            (&mut self.preview.holds, held),
        ] {
            if animate {
                spring.retarget(to, GLIDE, now);
            } else {
                spring.set(to, now);
            }
        }
    }

    /// The buffer holds `seconds` now, or is not running.
    pub fn set_replay_held(&mut self, seconds: Option<f32>, now: Instant) {
        let to = seconds.unwrap_or(0.0);
        if self.reduced_motion {
            self.preview.held.set(to, now);
        } else {
            self.preview.held.retarget(to, GLIDE, now);
        }
    }

    pub(super) fn replay_moving(&self, now: Instant) -> bool {
        [self.preview.length, self.preview.holds, self.preview.held]
            .iter()
            .any(|spring| !spring.done(now))
    }

    /// The bar's parts now: (filled from the right, the part the memory
    /// cannot hold on the left), as fractions of its width.
    pub fn replay_bar(&self, now: Instant) -> (f32, f32) {
        let length = self.preview.length.value(now).max(1.0);
        let filled = (self.preview.held.value(now) / length).clamp(0.0, 1.0);
        let lost = (1.0 - self.preview.holds.value(now) / length).clamp(0.0, 1.0);
        (filled, lost)
    }

    pub(super) fn draw_replay(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        let bar = bar(rect);
        let radius = BAR_H / 2.0;
        canvas.fill(bar, radius, theme::RAISE, 1.0);
        let (filled, lost) = self.replay_bar(now);
        if lost > 0.0 {
            let w = (bar.w * lost).max(BAR_H);
            canvas.fill(Rect { w, ..bar }, radius, theme::WINDOW, 1.0);
        }
        if filled > 0.0 {
            let w = (bar.w * filled).max(BAR_H);
            canvas.fill(
                Rect::new(bar.right() - w, bar.y, w, BAR_H),
                radius,
                theme::SELECTED,
                1.0,
            );
        }
        // The times: now on the right, back to the length on the left.
        let length = self.preview.length.value(now).max(1.0);
        let step = step(length, bar.w);
        let line = Rect::new(bar.x, bar.bottom() + GAP_S, bar.w, LABEL_H);
        let baseline = self.fonts.baseline(TextStyle::Key, line.y, line.h);
        let label = |canvas: &mut Canvas, text: &str, x: f32| {
            let w = self.fonts.width(TextStyle::Key, text);
            let x = x.clamp(line.x, line.right() - w).round();
            self.fonts
                .draw(canvas, TextStyle::Key, text, (x, baseline), theme::SUB, 1.0);
        };
        let now_w = self.fonts.width(TextStyle::Key, "now");
        label(canvas, "now", line.right() - now_w);
        let mut back = step;
        while (back as f32) <= length + 0.5 {
            let text = format!("\u{2212}{}", time(back));
            let w = self.fonts.width(TextStyle::Key, &text);
            let x = line.right() - back as f32 / length * line.w - w / 2.0;
            label(canvas, &text, x);
            back += step;
        }
    }
}
