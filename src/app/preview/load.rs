//! 5i: what the running replay buffer costs, as measured by the platform
//! (CPU of the recorder with its helpers and of the worker, their memory),
//! at the bottom of the Replay page. The numbers change as they are
//! measured, without a glide (they are readings, not a moving shape).

use super::super::App;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, PANEL_RADIUS, TextStyle};
use crate::ui::{Canvas, Rect};

/// One line of `Label`, its capitals in the middle.
const LINE_H: f32 = 16.0;

/// The replay buffer's measured cost.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplayLoad {
    /// Percent of one core over the last second; `None` until two readings.
    pub cpu_percent: Option<f32>,
    pub memory_bytes: u64,
}

/// The widest each side gets, so the block keeps its height as the numbers
/// change.
const WIDEST: (&str, &str) = ("Before your changes", "1000% of one core, 99999\u{a0}MB");

/// Both sides on one line in a column `width` wide, else the numbers under.
fn one_line(fonts: &Fonts, width: f32) -> bool {
    let width_of = |text| fonts.width(TextStyle::Label, text);
    width_of(WIDEST.0) + GAP_M + width_of(WIDEST.1) <= width - 2.0 * GAP_M
}

pub fn height(fonts: &Fonts, width: f32) -> f32 {
    let lines = if one_line(fonts, width) { 1.0 } else { 2.0 };
    2.0 * GAP_M + lines * LINE_H
}

impl App {
    /// The running buffer's cost now, `None` while it is off.
    pub fn set_replay_load(&mut self, load: Option<ReplayLoad>) {
        self.preview.load = load;
    }

    /// The line's two sides: what it is, and the numbers. `stale`: the buffer
    /// runs with the settings from before the page's changes.
    pub fn replay_load_text(&self, stale: bool) -> (String, String) {
        let Some(load) = self.preview.load else {
            return ("Off, nothing in use".into(), String::new());
        };
        let what = if stale {
            "Before your changes"
        } else {
            "In use now"
        };
        let memory = format!("{}\u{a0}MB", (load.memory_bytes as f64 / 1e6).round());
        let numbers = match load.cpu_percent {
            Some(cpu) => format!("{}% of one core, {memory}", cpu.round()),
            None => memory,
        };
        (what.into(), numbers)
    }

    pub(super) fn draw_load(&self, canvas: &mut Canvas, rect: Rect, stale: bool) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        let fonts = &self.fonts;
        let inner = rect.inset(GAP_M);
        let first = fonts.baseline(TextStyle::Label, inner.y, LINE_H).round();
        let (what, numbers) = self.replay_load_text(stale);
        fonts.draw(
            canvas,
            TextStyle::Label,
            &what,
            (inner.x, first),
            theme::SUB,
            1.0,
        );
        // Right of it, or under it, left-aligned, in a narrow column.
        let (x, baseline) = if one_line(fonts, rect.w) {
            let x = inner.right() - fonts.width(TextStyle::Label, &numbers);
            (x.round(), first)
        } else {
            (inner.x, first + LINE_H)
        };
        fonts.draw(
            canvas,
            TextStyle::Label,
            &numbers,
            (x, baseline),
            theme::TEXT,
            1.0,
        );
    }
}
