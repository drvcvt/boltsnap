//! P3: the replay buffer on a timeline: its length from "now" back, how much
//! it holds right now, and the part the memory cannot hold.

use std::time::Instant;

use super::super::{App, Bind, Block};
use super::Preview;
use crate::replay::settings::Settings;
use crate::settings;
use crate::ui::theme::{self, GAP_M, GAP_S, GLIDE, PANEL_RADIUS, TextStyle};
use crate::ui::{Canvas, Rect};

/// The line on top (what it holds, at what bitrate), the bar, the times.
const HEAD_H: f32 = 16.0;
const BAR_H: f32 = 16.0;
const LABEL_H: f32 = 14.0;
/// The bar is cells, one second each up to this many, then each a share.
const MOST_CELLS: u64 = 60;
const CELL_GAP: f32 = 2.0;
/// Below the radius steps: a cell is about 10 px wide.
const CELL_RADIUS: f32 = 2.0;
/// Held cells are the text's light, a little softer.
const HELD: f32 = 0.85;
/// Round steps for the marks; the smallest that keeps them to `MOST_MARKS`.
const STEPS: [u64; 8] = [5, 10, 15, 30, 60, 120, 300, 600];
const MOST_MARKS: u64 = 6;
/// Room each mark's time needs, so narrow windows show fewer.
const MARK_ROOM: f32 = 64.0;

/// The preview's height: the line on top, the bar, the times, gaps,
/// `GAP_M` around.
pub fn height() -> f32 {
    2.0 * GAP_M + HEAD_H + GAP_S + BAR_H + GAP_S + LABEL_H
}

/// The line on top inside the block `rect`.
pub fn head(rect: Rect) -> Rect {
    let inner = rect.inset(GAP_M);
    Rect::new(inner.x, inner.y, inner.w, HEAD_H)
}

/// The bar inside the block `rect`.
pub fn bar(rect: Rect) -> Rect {
    let head = head(rect);
    Rect::new(head.x, head.bottom() + GAP_S, head.w, BAR_H)
}

/// How many cells a buffer `length` seconds long is drawn as.
pub fn cells(length: f32) -> usize {
    (length.round().max(1.0) as u64).min(MOST_CELLS) as usize
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

/// What the bar shows: (length, seconds the memory holds, video bitrate in
/// kbit/s) as the Length and Memory rows hold them now.
fn shape(app: &App) -> Option<(f32, f32, u32)> {
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
    let settings = Settings {
        seconds: length as u64,
        memory_mib: memory as usize,
        bitrate_kbps: bitrate,
        ..Settings::default()
    };
    Some((
        length as f32,
        settings.seconds_held() as f32,
        settings.video_kbps(),
    ))
}

impl App {
    pub(super) fn sync_replay(&mut self, animate: bool, now: Instant) {
        let Some((length, held, kbps)) = shape(self) else {
            return;
        };
        self.preview.kbps = kbps;
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

    /// The buffer holds `seconds` now, or is not running. A full buffer
    /// drops its oldest one-second GOP at a time and so swings within its
    /// last second; that shows as full, not as the oldest cell dimming and
    /// lighting again every few seconds (each a glide redrawn at the
    /// display's rate).
    pub fn set_replay_held(&mut self, seconds: Option<f32>, now: Instant) {
        self.preview.running = seconds.is_some();
        let full = self
            .preview
            .length
            .target()
            .min(self.preview.holds.target());
        let to = seconds.map_or(0.0, |s| if s > full - 1.0 && s < full { full } else { s });
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

    /// The line on top: what the buffer holds now, and its bitrate.
    pub fn replay_head(&self, now: Instant) -> (String, String) {
        let held = if self.preview.running {
            format!(
                "Holds {}",
                time(self.preview.held.value(now).round() as u64)
            )
        } else {
            "Off".to_owned()
        };
        let mbit = self.preview.kbps as f32 / 1000.0;
        let rate = if mbit < 10.0 {
            format!("{mbit:.1}\u{a0}Mbit/s")
        } else {
            format!("{mbit:.0}\u{a0}Mbit/s")
        };
        (held, rate)
    }

    pub(super) fn draw_replay(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        let fonts = &self.fonts;
        let head = head(rect);
        let (held, rate) = self.replay_head(now);
        let baseline = fonts.baseline(TextStyle::Label, head.y, head.h);
        fonts.draw(
            canvas,
            TextStyle::Label,
            &held,
            (head.x, baseline),
            theme::TEXT,
            1.0,
        );
        let x = (head.right() - fonts.width(TextStyle::Label, &rate)).round();
        fonts.draw(
            canvas,
            TextStyle::Label,
            &rate,
            (x, baseline),
            theme::SUB,
            1.0,
        );
        // Cells from the right, the newest first: held ones light up one by
        // one as the fill glides, those the memory cannot hold stay dark.
        let bar = bar(rect);
        let length = self.preview.length.value(now).max(1.0);
        let n = cells(length);
        let per = length / n as f32;
        let w = (bar.w - (n - 1) as f32 * CELL_GAP) / n as f32;
        let held_cells = self.preview.held.value(now) / per;
        let kept_cells = self.preview.holds.value(now) / per;
        for back in 0..n {
            let x = bar.right() - (back + 1) as f32 * w - back as f32 * CELL_GAP;
            let cell = Rect::new(x, bar.y, w, BAR_H);
            let lit = (held_cells - back as f32).clamp(0.0, 1.0);
            let color = if back as f32 >= kept_cells.floor() && kept_cells < n as f32 {
                theme::WINDOW
            } else {
                theme::mix(theme::RAISE, theme::TEXT, HELD * lit)
            };
            canvas.fill(cell, CELL_RADIUS, color, 1.0);
        }
        // The times: now on the right, back to the length on the left.
        let step = step(length, bar.w);
        let line = Rect::new(bar.x, bar.bottom() + GAP_S, bar.w, LABEL_H);
        let baseline = fonts.baseline(TextStyle::Key, line.y, line.h);
        let label = |canvas: &mut Canvas, text: &str, x: f32, color| {
            let w = fonts.width(TextStyle::Key, text);
            let x = x.clamp(line.x, line.right() - w).round();
            fonts.draw(canvas, TextStyle::Key, text, (x, baseline), color, 1.0);
        };
        let now_w = fonts.width(TextStyle::Key, "now");
        label(canvas, "now", line.right() - now_w, theme::TEXT);
        let mut back = step;
        while (back as f32) <= length + 0.5 {
            let text = format!("\u{2212}{}", time(back));
            let w = fonts.width(TextStyle::Key, &text);
            let x = line.right() - back as f32 / length * line.w - w / 2.0;
            label(canvas, &text, x, theme::SUB);
            back += step;
        }
    }
}
