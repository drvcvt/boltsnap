use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, light, surface};
use crate::ui::icons::Icon;
use crate::ui::motion::Spring;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, GAP_S, ICON, RADIUS, SNAP, TextStyle};
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
    slide: Spring,
    /// Each part's text, 0 dim to 1 bright, under the pointer.
    lit: Vec<Spring>,
}

impl Segment {
    pub fn new(parts: Vec<Part>, chosen: usize, now: Instant) -> Self {
        let chosen = chosen.min(parts.len().saturating_sub(1));
        Self {
            lit: vec![Spring::settled(0.0, now); parts.len()],
            parts,
            chosen,
            slide: Spring::settled(chosen as f32, now),
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
        self.slide.retarget(index as f32, SNAP, now);
        Outcome::CHANGED
    }

    /// Equal parts, except that a part whose content does not fit its share
    /// keeps its content plus `GAP_M` on each side and the others share the
    /// rest. Tighter than that, every part gets its content and an equal
    /// share of what is left; tighter still, the contents shrink alike. Each
    /// width changes steadily with `rect.w`, so a resize never jumps.
    pub(crate) fn part_rects(&self, fonts: &Fonts, rect: Rect) -> Vec<Rect> {
        let n = self.parts.len();
        let contents: Vec<f32> = self.parts.iter().map(|p| p.content_width(fonts)).collect();
        let floors: Vec<f32> = contents.iter().map(|c| c + 2.0 * GAP_M).collect();
        let total: f32 = contents.iter().sum();
        let widths: Vec<f32> = if rect.w >= floors.iter().sum::<f32>() {
            let mut kept = vec![false; n];
            loop {
                let free = rect.w - (0..n).filter(|&i| kept[i]).map(|i| floors[i]).sum::<f32>();
                let sharing = kept.iter().filter(|k| !**k).count();
                let share = free / sharing.max(1) as f32;
                let over: Vec<usize> = (0..n).filter(|&i| !kept[i] && floors[i] > share).collect();
                if over.is_empty() || sharing == 0 {
                    break (0..n)
                        .map(|i| if kept[i] { floors[i] } else { share })
                        .collect();
                }
                for i in over {
                    kept[i] = true;
                }
            }
        } else if rect.w >= total {
            let spare = (rect.w - total) / n as f32;
            contents.iter().map(|content| content + spare).collect()
        } else {
            let scale = rect.w / total.max(1.0);
            contents.iter().map(|content| content * scale).collect()
        };
        let mut x = rect.x;
        widths
            .into_iter()
            .map(|w| {
                let cell = Rect::new(x, rect.y, w, rect.h);
                x += w;
                cell
            })
            .collect()
    }
}

impl Widget for Segment {
    /// Equal parts if they fit, else the room down to each part's content
    /// with `GAP_M` around it, so a row stacks only when even that does not
    /// fit.
    fn width(&self, fonts: &Fonts, room: f32) -> f32 {
        let equal = self.part_width(fonts) * self.parts.len() as f32;
        let snug: f32 = self
            .parts
            .iter()
            .map(|part| (part.content_width(fonts) + 2.0 * GAP_M).ceil())
            .sum();
        // Between the two the segment takes the room, so its width follows
        // the window instead of jumping at a threshold.
        room.clamp(snug.min(equal), equal)
    }

    fn stretches(&self) -> bool {
        true
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, now: Instant) {
        // The track stays still: the part under the pointer lights its text
        // and the chosen surface slides; lighting the whole track as well
        // would answer on parts nobody meant (settings plan, 4i).
        surface(
            canvas,
            rect,
            &Look {
                hover: 0.0,
                press: 0.0,
                ..look
            },
        );
        let cells = self.part_rects(fonts, rect);
        // Between two parts the surface takes both position and width.
        let at = self.slide.value(now).clamp(0.0, (cells.len() - 1) as f32);
        let (from, to) = (cells[at.floor() as usize], cells[at.ceil() as usize]);
        let t = at.fract();
        let chosen = Rect::new(
            from.x + (to.x - from.x) * t,
            rect.y,
            from.w + (to.w - from.w) * t,
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
            let cell = cells[index];
            // A part brightens as the chosen surface arrives under it.
            let under = (1.0 - (at - index as f32).abs()).clamp(0.0, 1.0);
            let color = if !look.enabled {
                theme::DIM
            } else {
                theme::mix(
                    theme::SUB,
                    theme::TEXT,
                    under.max(self.lit[index].value(now)),
                )
            };
            // A label that still does not fit is cut, never drawn past its part.
            let room = cell.w - 2.0 * GAP_S;
            let label = part
                .label
                .as_deref()
                .map(|label| fonts.ellipsize(TextStyle::Label, label, room));
            let content = part.content_width(fonts).min(room);
            let mut x = cell.x + (cell.w - content) / 2.0;
            if let Some(icon) = part.icon {
                // The baseline puts the capitals' middle on the cell's middle,
                // so a centred icon sits on it too.
                icon.draw(canvas, icon_box(cell, x, ICON), color, 1.0);
                x += ICON + theme::GAP_S;
            }
            if let Some(label) = &label {
                let baseline = fonts.baseline(TextStyle::Label, cell.y, cell.h);
                fonts.draw(canvas, TextStyle::Label, label, (x, baseline), color, 1.0);
            }
        }
    }

    fn value(&self) -> Value {
        Value::Choice(self.chosen)
    }

    fn adopt(&mut self, value: &Value, now: Instant) {
        if let Value::Choice(index) = value {
            self.choose(*index, now);
        }
    }

    fn labels(&self) -> Vec<&str> {
        self.parts
            .iter()
            .filter_map(|part| part.label.as_deref())
            .collect()
    }

    fn press(
        &mut self,
        rect: Rect,
        (x, y): (f32, f32),
        _: u8,
        fonts: &Fonts,
        now: Instant,
    ) -> Outcome {
        let cells = self.part_rects(fonts, rect);
        match cells.iter().position(|cell| cell.contains(x, y)) {
            Some(index) => self.choose(index, now),
            None => Outcome::HANDLED,
        }
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, now: Instant) -> Outcome {
        match key {
            Key::Left if self.chosen > 0 => self.choose(self.chosen - 1, now),
            Key::Right => self.choose(self.chosen + 1, now),
            Key::Left => Outcome::HANDLED,
            _ => Outcome::IGNORED,
        }
    }

    fn hover(&mut self, rect: Rect, at: Option<(f32, f32)>, fonts: &Fonts, now: Instant) {
        let cells = self.part_rects(fonts, rect);
        light(
            &mut self.lit,
            |index| at.is_some_and(|(x, y)| cells[index].contains(x, y)),
            now,
        );
    }

    fn animating(&self, now: Instant) -> bool {
        !self.slide.done(now) || self.lit.iter().any(|lit| !lit.done(now))
    }
}
