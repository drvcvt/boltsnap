use std::time::{Duration, Instant};

use super::{Look, Outcome, Value, Widget, icon_box, surface};
use crate::ui::icons::Icon;
use crate::ui::motion::{Scroll, Spring};
use crate::ui::text::Fonts;
use crate::ui::theme::{self, CTL_H, GAP_M, GAP_S, GAP_XS, ICON, QUICK, RADIUS, SNAP, TextStyle};
use crate::ui::{Canvas, Key, Mods, Rect};

/// The most entries an open list shows; more scroll inside it.
const VISIBLE: usize = 10;
/// Letters typed within this long of each other spell one word to jump to.
const TYPING: Duration = Duration::from_secs(1);

/// One choice out of a list. The list is drawn in the window, below the
/// control or above it when there is no room below; a long one scrolls in
/// itself, and typing jumps to the first entry that starts with the letters.
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
    /// How far the list is scrolled, in pixels.
    scroll: Scroll,
    /// What was typed to jump, and when the last letter came.
    typed: String,
    typed_at: Option<Instant>,
}

impl Dropdown {
    pub fn new(items: Vec<String>, chosen: usize, now: Instant) -> Self {
        let mut scroll = Scroll::new(now);
        scroll.set_max(items.len().saturating_sub(VISIBLE) as f32 * CTL_H, now);
        Self {
            chosen: chosen.min(items.len().saturating_sub(1)),
            highlight: 0,
            items,
            open: false,
            fade: Spring::settled(0.0, now),
            mark: Spring::settled(0.0, now),
            scroll,
            typed: String::new(),
            typed_at: None,
        }
    }

    /// The height the visible entries take, without the list's padding.
    fn view_h(&self) -> f32 {
        self.items.len().min(VISIBLE) as f32 * CTL_H
    }

    fn show(&mut self, now: Instant) -> Outcome {
        self.open = true;
        self.highlight = self.chosen;
        self.mark.set(self.chosen as f32, now);
        // Open where the chosen entry is, with it in the middle if it can be.
        let middle = (self.chosen as f32 + 0.5) * CTL_H - self.view_h() / 2.0;
        self.scroll.jump(middle, now);
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

    /// Entry `index` in `list`, scrolled `offset` pixels.
    fn entry(list: Rect, index: usize, offset: f32) -> Rect {
        Rect::new(
            list.x + GAP_XS,
            list.y + GAP_XS + index as f32 * CTL_H - offset,
            list.w - 2.0 * GAP_XS,
            CTL_H,
        )
    }

    /// Where entries may show: the list without its padding.
    fn view(list: Rect) -> Rect {
        list.inset(GAP_XS)
    }

    /// Mark `index` and scroll so it shows.
    fn mark_at(&mut self, index: usize, now: Instant) -> Outcome {
        self.highlight = index;
        self.mark.retarget(index as f32, SNAP, now);
        let top = index as f32 * CTL_H;
        self.scroll.reveal(top, top + CTL_H, self.view_h(), now);
        Outcome::HANDLED
    }

    fn entry_at(&self, list: Rect, (x, y): (f32, f32), now: Instant) -> Option<usize> {
        if !Self::view(list).contains(x, y) {
            return None;
        }
        let offset = self.scroll.offset(now);
        (0..self.items.len()).find(|&index| Self::entry(list, index, offset).contains(x, y))
    }

    /// The widest entry with its padding and check mark.
    fn content_w(&self, fonts: &Fonts) -> f32 {
        let widest = self
            .items
            .iter()
            .map(|item| fonts.width(TextStyle::Label, item))
            .fold(0.0, f32::max);
        (2.0 * GAP_M + widest + GAP_S + ICON + 2.0 * GAP_XS).ceil()
    }

    /// Letters came within `TYPING` of each other: a name is being typed.
    fn typing(&self, now: Instant) -> bool {
        self.typed_at
            .is_some_and(|at| now.saturating_duration_since(at) <= TYPING)
    }

    /// The first entry from the marked one on that starts with what was
    /// typed, ignoring case.
    fn find(&self, typed: &str) -> Option<usize> {
        let typed = typed.to_lowercase();
        let n = self.items.len();
        (0..n)
            .map(|step| (self.highlight + step) % n)
            .find(|&index| self.items[index].to_lowercase().starts_with(&typed))
    }
}

impl Widget for Dropdown {
    fn stretches(&self) -> bool {
        true
    }

    fn width(&self, fonts: &Fonts, _: f32) -> f32 {
        // Short lists keep one width so their left edges line up; a long
        // entry shows in full in the open list, not here.
        self.content_w(fonts)
            .clamp(theme::MENU_MIN, theme::MENU_MAX)
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

    fn adopt(&mut self, value: &Value, now: Instant) {
        if let Value::Choice(index) = value
            && *index < self.items.len()
        {
            self.chosen = *index;
            self.mark.set(*index as f32, now);
        }
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
            Key::Enter => self.choose(self.highlight, now),
            // Space is part of a name being typed, else it chooses.
            Key::Space if self.typing(now) => self.text(" ", now),
            Key::Space => self.choose(self.highlight, now),
            Key::Escape => {
                self.hide(now);
                Outcome::HANDLED
            }
            _ => Outcome::IGNORED,
        }
    }

    fn text(&mut self, text: &str, now: Instant) -> Outcome {
        if text.trim().is_empty() && self.typed.is_empty() {
            return Outcome::IGNORED;
        }
        if !self.open {
            self.show(now);
        }
        if !self.typing(now) {
            self.typed.clear();
        }
        self.typed.push_str(text);
        self.typed_at = Some(now);
        match self.find(&self.typed.clone()) {
            Some(index) => self.mark_at(index, now),
            None => Outcome::HANDLED,
        }
    }

    fn animating(&self, now: Instant) -> bool {
        !self.fade.done(now) || !self.mark.done(now) || self.scroll.moving(now)
    }

    fn overlay(&self, anchor: Rect, bounds: Rect, fonts: &Fonts, now: Instant) -> Option<Rect> {
        if self.fade.value(now) <= 0.0 && !self.open {
            return None;
        }
        let h = self.view_h() + 2.0 * GAP_XS;
        let below = anchor.bottom() + GAP_XS;
        let y = if below + h <= bounds.bottom() || anchor.y - GAP_XS - h < bounds.y {
            below
        } else {
            anchor.y - GAP_XS - h
        };
        // As wide as its widest entry, at least the control, at most the room.
        let w = self.content_w(fonts).max(anchor.w).min(bounds.w);
        let x = anchor.x.min(bounds.right() - w).max(bounds.x);
        Some(Rect::new(x, y, w, h))
    }

    fn overlay_open(&self) -> bool {
        self.open
    }

    fn draw_overlay(&self, canvas: &mut Canvas, fonts: &Fonts, list: Rect, now: Instant) {
        let alpha = self.fade.value(now).clamp(0.0, 1.0);
        canvas.fill(list, RADIUS, theme::HOVER, alpha);
        let view = Self::view(list);
        let offset = self.scroll.offset(now);
        let mark = Rect {
            y: Self::entry(list, 0, offset).y + self.mark.value(now) * CTL_H,
            ..Self::entry(list, 0, offset)
        };
        // The mark stays inside the list while it scrolls past an edge.
        let top = mark.y.max(view.y);
        let bottom = mark.bottom().min(view.bottom());
        if bottom > top {
            let mark = Rect::new(mark.x, top, mark.w, bottom - top);
            canvas.fill(mark, RADIUS - GAP_XS, theme::SELECTED, alpha);
        }
        let first = (offset / CTL_H).floor() as usize;
        let last = (((offset + self.view_h()) / CTL_H).ceil() as usize).min(self.items.len());
        for index in first..last {
            let entry = Self::entry(list, index, offset);
            let check_x = entry.right() - GAP_M - ICON;
            let x = entry.x + GAP_M - GAP_XS;
            let item = fonts.ellipsize(TextStyle::Label, &self.items[index], check_x - x - GAP_S);
            let baseline = fonts.baseline(TextStyle::Label, entry.y, entry.h);
            fonts.draw_clipped(
                canvas,
                TextStyle::Label,
                &item,
                (x, baseline),
                theme::TEXT,
                alpha,
                view,
            );
            let whole = entry.y >= view.y && entry.bottom() <= view.bottom();
            if index == self.chosen && whole {
                Icon::Check.draw(canvas, icon_box(entry, check_x, ICON), theme::TEXT, alpha);
            }
        }
    }

    fn overlay_hover(&mut self, list: Rect, at: (f32, f32), now: Instant) {
        if self.open
            && let Some(index) = self.entry_at(list, at, now)
            && index != self.highlight
        {
            self.highlight = index;
            self.mark.retarget(index as f32, SNAP, now);
        }
    }

    fn overlay_wheel(&mut self, notches: i32, pixels: f32, now: Instant) -> bool {
        if !self.open || self.scroll.max() <= 0.0 {
            return false;
        }
        self.scroll.wheel(notches, pixels, now);
        true
    }

    fn overlay_press(&mut self, list: Rect, at: (f32, f32), now: Instant) -> Outcome {
        if !self.open {
            return Outcome::IGNORED;
        }
        match self.entry_at(list, at, now) {
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
