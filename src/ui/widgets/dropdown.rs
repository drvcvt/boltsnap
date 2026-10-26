use std::time::Instant;

use super::{Look, Outcome, Value, Widget, icon_box, surface};
use crate::ui::icons::Icon;
use crate::ui::motion::Spring;
use crate::ui::text::Fonts;
use crate::ui::theme::{self, CTL_H, GAP_M, GAP_S, GAP_XS, ICON, QUICK, RADIUS, SNAP, TextStyle};
use crate::ui::{Canvas, Key, Mods, Rect};

/// One choice out of a short list. The list is drawn in the window, below
/// the control or above it when there is no room below.
pub struct Dropdown {
    items: Vec<String>,
    chosen: usize,
    /// Takes input; a closing list still fades but no longer does.
    open: bool,
    /// 0 closed, 1 open.
    fade: Spring,
    /// The marked entry, by keyboard or pointer; its surface glides there.
    highlight: usize,
    mark: Spring,
}

impl Dropdown {
    pub fn new(items: Vec<String>, chosen: usize, now: Instant) -> Self {
        Self {
            chosen: chosen.min(items.len().saturating_sub(1)),
            highlight: 0,
            items,
            open: false,
            fade: Spring::settled(0.0, now),
            mark: Spring::settled(0.0, now),
        }
    }

    fn show(&mut self, now: Instant) -> Outcome {
        self.open = true;
        self.highlight = self.chosen;
        self.mark.set(self.chosen as f32, now);
        self.fade.retarget(1.0, QUICK, now);
        Outcome::HANDLED
    }

    fn hide(&mut self, now: Instant) {
        self.open = false;
        self.fade.retarget(0.0, QUICK, now);
    }

    fn choose(&mut self, index: usize, now: Instant) -> Outcome {
        self.hide(now);
        let changed = index != self.chosen;
        self.chosen = index;
        Outcome::changed_if(changed)
    }

    fn entry(list: Rect, index: usize) -> Rect {
        Rect::new(
            list.x + GAP_XS,
            list.y + GAP_XS + index as f32 * CTL_H,
            list.w - 2.0 * GAP_XS,
            CTL_H,
        )
    }

    fn mark_at(&mut self, index: usize, now: Instant) -> Outcome {
        self.highlight = index;
        self.mark.retarget(index as f32, SNAP, now);
        Outcome::HANDLED
    }

    fn entry_at(&self, list: Rect, (x, y): (f32, f32)) -> Option<usize> {
        (0..self.items.len()).find(|&index| Self::entry(list, index).contains(x, y))
    }
}

impl Widget for Dropdown {
    fn stretches(&self) -> bool {
        true
    }

    fn width(&self, fonts: &Fonts, _: f32) -> f32 {
        let widest = self
            .items
            .iter()
            .map(|item| fonts.width(TextStyle::Label, item))
            .fold(0.0, f32::max);
        // Entry text, the gap and the check mark must fit in the list too.
        (2.0 * GAP_M + widest + GAP_S + ICON + 2.0 * GAP_XS).ceil()
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, _: Instant) {
        surface(canvas, rect, &look);
        let chevron_x = rect.right() - GAP_M - ICON;
        fonts.draw_in(
            canvas,
            TextStyle::Label,
            &self.items[self.chosen],
            Rect::new(
                rect.x + GAP_M,
                rect.y,
                chevron_x - GAP_S - rect.x - GAP_M,
                rect.h,
            ),
            look.text(),
            1.0,
        );
        let chevron = if look.enabled { theme::SUB } else { theme::DIM };
        Icon::ChevronDown.draw(canvas, icon_box(rect, chevron_x, ICON), chevron, 1.0);
    }

    fn value(&self) -> Value {
        Value::Choice(self.chosen)
    }

    fn labels(&self) -> Vec<&str> {
        self.items.iter().map(String::as_str).collect()
    }

    fn press(&mut self, _: Rect, _: (f32, f32), _: u8, _: &Fonts, now: Instant) -> Outcome {
        if self.open {
            self.hide(now);
            Outcome::HANDLED
        } else {
            self.show(now)
        }
    }

    fn key(&mut self, key: Key, _: Mods, _: &Fonts, now: Instant) -> Outcome {
        if !self.open {
            return match key {
                Key::Enter | Key::Space | Key::Up | Key::Down => self.show(now),
                _ => Outcome::IGNORED,
            };
        }
        match key {
            Key::Up => self.mark_at(self.highlight.saturating_sub(1), now),
            Key::Down => self.mark_at((self.highlight + 1).min(self.items.len() - 1), now),
            Key::Home => self.mark_at(0, now),
            Key::End => self.mark_at(self.items.len() - 1, now),
            Key::Enter | Key::Space => self.choose(self.highlight, now),
            Key::Escape => {
                self.hide(now);
                Outcome::HANDLED
            }
            _ => Outcome::IGNORED,
        }
    }

    fn animating(&self, now: Instant) -> bool {
        !self.fade.done(now) || !self.mark.done(now)
    }

    fn overlay(&self, anchor: Rect, bounds: Rect, _: &Fonts, now: Instant) -> Option<Rect> {
        if self.fade.value(now) <= 0.0 && !self.open {
            return None;
        }
        let h = self.items.len() as f32 * CTL_H + 2.0 * GAP_XS;
        let below = anchor.bottom() + GAP_XS;
        let y = if below + h <= bounds.bottom() || anchor.y - GAP_XS - h < bounds.y {
            below
        } else {
            anchor.y - GAP_XS - h
        };
        let x = anchor.x.min(bounds.right() - anchor.w).max(bounds.x);
        Some(Rect::new(x, y, anchor.w, h))
    }

    fn overlay_open(&self) -> bool {
        self.open
    }

    fn draw_overlay(&self, canvas: &mut Canvas, fonts: &Fonts, list: Rect, now: Instant) {
        let alpha = self.fade.value(now).clamp(0.0, 1.0);
        canvas.fill(list, RADIUS, theme::HOVER, alpha);
        let mark = Rect {
            y: Self::entry(list, 0).y + self.mark.value(now) * CTL_H,
            ..Self::entry(list, 0)
        };
        canvas.fill(mark, RADIUS - GAP_XS, theme::SELECTED, alpha);
        for (index, item) in self.items.iter().enumerate() {
            let entry = Self::entry(list, index);
            let check_x = entry.right() - GAP_M - ICON;
            fonts.draw_in(
                canvas,
                TextStyle::Label,
                item,
                Rect::new(
                    entry.x + GAP_M - GAP_XS,
                    entry.y,
                    check_x - entry.x - GAP_M,
                    entry.h,
                ),
                theme::TEXT,
                alpha,
            );
            if index == self.chosen {
                Icon::Check.draw(canvas, icon_box(entry, check_x, ICON), theme::TEXT, alpha);
            }
        }
    }

    fn overlay_hover(&mut self, list: Rect, at: (f32, f32), now: Instant) {
        if self.open
            && let Some(index) = self.entry_at(list, at)
            && index != self.highlight
        {
            self.mark_at(index, now);
        }
    }

    fn overlay_press(&mut self, list: Rect, at: (f32, f32), now: Instant) -> Outcome {
        if !self.open {
            return Outcome::IGNORED;
        }
        match self.entry_at(list, at) {
            Some(index) => self.choose(index, now),
            None if list.contains(at.0, at.1) => Outcome::HANDLED,
            None => Outcome::IGNORED,
        }
    }

    fn close_overlay(&mut self, now: Instant) {
        if self.open {
            self.hide(now);
        }
    }
}
