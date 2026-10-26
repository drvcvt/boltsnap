use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, surface};
use crate::ui::icons::Icon;
use crate::ui::motion::Tween;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, ICON, KNOB_SLIDE, RADIUS, TextStyle};
use crate::ui::{Canvas, Key, Mods, Rect};

/// Gap between the control's edge and the chosen part's surface; the
/// surface's radius is the control's minus this (concentric).
const INSET: f32 = 3.0;

/// One choice: an icon, a label, or both.
pub struct Part {
    pub icon: Option<Icon>,
    pub label: Option<String>,
}

impl Part {
    pub fn text(label: &str) -> Self {
        Self {
            icon: None,
            label: Some(label.to_owned()),
        }
    }

    pub fn icon(icon: Icon) -> Self {
        Self {
            icon: Some(icon),
            label: None,
        }
    }

    fn content_width(&self, fonts: &Fonts) -> f32 {
        let label = self
            .label
            .as_deref()
            .map_or(0.0, |label| fonts.width(TextStyle::Label, label));
        let icon = if self.icon.is_some() { ICON } else { 0.0 };
        let gap = if self.icon.is_some() && self.label.is_some() {
            theme::GAP_S
        } else {
            0.0
        };
        icon + gap + label
    }
}

/// Two to four parts of equal width; the chosen part's surface slides.
pub struct Segment {
    parts: Vec<Part>,
    chosen: usize,
    /// The chosen surface's position, in parts.
    slide: Tween,
}

impl Segment {
    pub fn new(parts: Vec<Part>, chosen: usize, now: Instant) -> Self {
        let chosen = chosen.min(parts.len().saturating_sub(1));
        Self {
            parts,
            chosen,
            slide: Tween::settled(chosen as f32, now),
        }
    }

    fn part_width(&self, fonts: &Fonts) -> f32 {
        let widest = self
            .parts
            .iter()
            .map(|part| part.content_width(fonts))
            .fold(0.0, f32::max);
        (widest + 2.0 * GAP_M).ceil()
    }

    fn choose(&mut self, index: usize, now: Instant) -> Outcome {
        if index >= self.parts.len() || index == self.chosen {
            return Outcome::HANDLED;
        }
        self.chosen = index;
        self.slide.retarget(index as f32, KNOB_SLIDE, now);
        Outcome::CHANGED
    }

    fn part_rect(&self, rect: Rect, index: usize) -> Rect {
        let w = rect.w / self.parts.len() as f32;
        Rect::new(rect.x + index as f32 * w, rect.y, w, rect.h)
    }
}

impl Widget for Segment {
    fn width(&self, fonts: &Fonts) -> f32 {
        self.part_width(fonts) * self.parts.len() as f32
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, now: Instant) {
        surface(canvas, rect, &look);
        let part_w = rect.w / self.parts.len() as f32;
        let chosen = Rect::new(
            rect.x + self.slide.value(now) * part_w,
            rect.y,
            part_w,
            rect.h,
        )
        .inset(INSET);
        let chosen_fill = if look.enabled {
            theme::SELECTED
        } else {
            theme::HOVER
        };
        canvas.fill(chosen, RADIUS - INSET, chosen_fill, 1.0);
        for (index, part) in self.parts.iter().enumerate() {
            let cell = self.part_rect(rect, index);
            let color = if !look.enabled {
                theme::DIM
            } else if index == self.chosen || look.over(cell) {
                theme::TEXT
            } else {
                theme::SUB
            };
            let content = part.content_width(fonts);
            let mut x = cell.x + (cell.w - content) / 2.0;
            if let Some(icon) = part.icon {
                // The baseline puts the capitals' middle on the cell's middle,
                // so a centred icon sits on it too.
                icon.draw(canvas, icon_box(cell, x, ICON), color, 1.0);
                x += ICON + theme::GAP_S;
            }
            if let Some(label) = &part.label {
                let baseline = fonts.baseline(TextStyle::Label, cell.y, cell.h);
                fonts.draw(canvas, TextStyle::Label, label, (x, baseline), color, 1.0);
            }
        }
    }

    fn value(&self) -> Value {
        Value::Choice(self.chosen)
    }

    fn press(&mut self, rect: Rect, (x, _): (f32, f32), _: u8, _: &Fonts, now: Instant) -> Outcome {
        let index = ((x - rect.x) / (rect.w / self.parts.len() as f32)) as usize;
        self.choose(index.min(self.parts.len() - 1), now)
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, now: Instant) -> Outcome {
        match key {
            Key::Left if self.chosen > 0 => self.choose(self.chosen - 1, now),
            Key::Right => self.choose(self.chosen + 1, now),
            Key::Left => Outcome::HANDLED,
            _ => Outcome::IGNORED,
        }
    }

    fn animating(&self, now: Instant) -> bool {
        !self.slide.done(now)
    }
}
