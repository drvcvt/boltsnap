use std::time::Instant;

use super::{Look, Outcome, Value, Widget, surface};
use crate::ui::text::Fonts;
use crate::ui::theme::{self, GAP_M, TextStyle};
use crate::ui::{Canvas, Effect, Key, Mods, Rect};

/// Caret and selection height, centred on the line.
const CARET_H: f32 = 16.0;
/// The field follows the row's room between these.
const MIN_W: f32 = 160.0;
const MAX_W: f32 = 360.0;

/// One line of text. Edits stay local until `Enter` or losing focus commits
/// them; only a commit that differs counts as a change.
pub struct TextField {
    text: String,
    committed: String,
    /// Byte indices; the selection runs between them.
    caret: usize,
    anchor: usize,
}

impl TextField {
    pub fn new(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            committed: text.to_owned(),
            caret: text.len(),
            anchor: text.len(),
        }
    }

    pub fn content(&self) -> &str {
        &self.text
    }

    fn selection(&self) -> std::ops::Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    fn inner(rect: Rect) -> Rect {
        Rect::new(rect.x + GAP_M, rect.y, rect.w - 2.0 * GAP_M, rect.h)
    }

    /// How far the text is shifted left so the caret stays inside.
    fn shift(&self, fonts: &Fonts, room: f32) -> f32 {
        let caret = fonts.width(TextStyle::Label, &self.text[..self.caret]);
        (caret - room).max(0.0)
    }

    fn index_at(&self, fonts: &Fonts, rect: Rect, x: f32) -> usize {
        let inner = Self::inner(rect);
        let x = x - inner.x + self.shift(fonts, inner.w);
        fonts
            .carets(TextStyle::Label, &self.text)
            .windows(2)
            .find(|pair| x < (pair[0].1 + pair[1].1) / 2.0)
            .map_or(self.text.len(), |pair| pair[0].0)
    }

    fn replace_selection(&mut self, with: &str) {
        let range = self.selection();
        self.text.replace_range(range.clone(), with);
        self.caret = range.start + with.len();
        self.anchor = self.caret;
    }

    fn insert(&mut self, text: &str) -> Outcome {
        let clean: String = text.chars().filter(|c| !c.is_control()).collect();
        self.replace_selection(&clean);
        Outcome::HANDLED
    }

    fn move_to(&mut self, index: usize, extend: bool) {
        self.caret = index;
        if !extend {
            self.anchor = index;
        }
    }

    fn previous(&self, from: usize, word: bool) -> usize {
        let mut chars = self.text[..from].char_indices().rev().peekable();
        if !word {
            return chars.next().map_or(0, |(i, _)| i);
        }
        let mut at = from;
        while let Some(&(i, c)) = chars.peek()
            && !c.is_alphanumeric()
        {
            at = i;
            chars.next();
        }
        while let Some(&(i, c)) = chars.peek()
            && c.is_alphanumeric()
        {
            at = i;
            chars.next();
        }
        at
    }

    fn next(&self, from: usize, word: bool) -> usize {
        let mut chars = self.text[from..].char_indices().peekable();
        let end = |i: usize| from + i;
        if !word {
            return chars.nth(1).map_or(self.text.len(), |(i, _)| end(i));
        }
        while chars.peek().is_some_and(|(_, c)| !c.is_alphanumeric()) {
            chars.next();
        }
        while chars.peek().is_some_and(|(_, c)| c.is_alphanumeric()) {
            chars.next();
        }
        chars.peek().map_or(self.text.len(), |&(i, _)| end(i))
    }

    fn delete(&mut self, forward: bool, word: bool) -> Outcome {
        if self.selection().is_empty() {
            self.anchor = if forward {
                self.next(self.caret, word)
            } else {
                self.previous(self.caret, word)
            };
        }
        self.replace_selection("");
        Outcome::HANDLED
    }

    fn select_word(&mut self, at: usize) {
        let word = |c: char| c.is_alphanumeric();
        self.anchor = self.text[..at]
            .char_indices()
            .rev()
            .take_while(|&(_, c)| word(c))
            .last()
            .map_or(at, |(i, _)| i);
        self.caret = self.text[at..]
            .char_indices()
            .find(|&(_, c)| !word(c))
            .map_or(self.text.len(), |(i, _)| at + i);
    }

    fn commit(&mut self) -> Outcome {
        let changed = self.text != self.committed;
        self.committed = self.text.clone();
        Outcome::changed_if(changed)
    }
}

impl Widget for TextField {
    fn width(&self, _: &Fonts, room: f32) -> f32 {
        room.clamp(MIN_W, MAX_W)
    }

    fn stretches(&self) -> bool {
        true
    }

    fn draw(&self, canvas: &mut Canvas, fonts: &Fonts, rect: Rect, look: Look, _: Instant) {
        // A press places the caret; the surface does not flash for it.
        surface(canvas, rect, &Look { press: 0.0, ..look });
        let inner = Self::inner(rect);
        let shift = self.shift(fonts, inner.w);
        let x_of =
            |index: usize| inner.x + fonts.width(TextStyle::Label, &self.text[..index]) - shift;
        let line = Rect::new(inner.x, rect.y + (rect.h - CARET_H) / 2.0, inner.w, CARET_H);
        let range = self.selection();
        if look.focused && !range.is_empty() {
            let (left, right) = (
                x_of(range.start).max(inner.x),
                x_of(range.end).min(inner.right()),
            );
            canvas.fill(
                Rect::new(left, line.y, right - left, line.h),
                0.0,
                theme::SELECTED,
                1.0,
            );
        }
        let baseline = fonts.baseline(TextStyle::Label, rect.y, rect.h);
        if !look.focused {
            // At rest a long path shows its start and its end.
            let text = fonts.ellipsize_middle(TextStyle::Label, &self.text, inner.w);
            fonts.draw(
                canvas,
                TextStyle::Label,
                &text,
                (inner.x, baseline),
                look.text(),
                1.0,
            );
            return;
        }
        fonts.draw_clipped(
            canvas,
            TextStyle::Label,
            &self.text,
            (inner.x - shift, baseline),
            look.text(),
            1.0,
            inner,
        );
        if look.focused {
            // One device pixel wide, on a whole pixel.
            let x = (x_of(self.caret) * canvas.scale).round() / canvas.scale;
            canvas.fill(
                Rect::new(x, line.y, 1.0 / canvas.scale, line.h),
                0.0,
                theme::TEXT,
                1.0,
            );
        }
    }

    fn value(&self) -> Value {
        Value::Text(self.committed.clone())
    }

    fn press(
        &mut self,
        rect: Rect,
        (x, _): (f32, f32),
        clicks: u8,
        fonts: &Fonts,
        _: Instant,
    ) -> Outcome {
        let index = self.index_at(fonts, rect, x);
        if clicks >= 2 {
            self.select_word(index);
        } else {
            self.move_to(index, false);
        }
        Outcome::HANDLED
    }

    fn drag(&mut self, rect: Rect, (x, _): (f32, f32), fonts: &Fonts) -> Outcome {
        self.caret = self.index_at(fonts, rect, x);
        Outcome::HANDLED
    }

    fn key(&mut self, key: Key, mods: Mods, _: &Fonts, _: Instant) -> Outcome {
        let range = self.selection();
        match key {
            Key::Left if !mods.shift && !range.is_empty() => self.move_to(range.start, false),
            Key::Right if !mods.shift && !range.is_empty() => self.move_to(range.end, false),
            Key::Left => self.move_to(self.previous(self.caret, mods.ctrl), mods.shift),
            Key::Right => self.move_to(self.next(self.caret, mods.ctrl), mods.shift),
            Key::Home => self.move_to(0, mods.shift),
            Key::End => self.move_to(self.text.len(), mods.shift),
            Key::Backspace => return self.delete(false, mods.ctrl),
            Key::Delete => return self.delete(true, mods.ctrl),
            Key::Space => return self.insert(" "),
            Key::Enter => return self.commit(),
            Key::Char('a') => {
                self.anchor = 0;
                self.caret = self.text.len();
            }
            Key::Char('c') | Key::Char('x') if range.is_empty() => {}
            Key::Char('c') => {
                return Outcome {
                    effect: Some(Effect::Copy(self.text[range].to_owned())),
                    ..Outcome::HANDLED
                };
            }
            Key::Char('x') => {
                let copied = self.text[range].to_owned();
                self.replace_selection("");
                return Outcome {
                    effect: Some(Effect::Copy(copied)),
                    ..Outcome::HANDLED
                };
            }
            Key::Char('v') => {
                return Outcome {
                    effect: Some(Effect::Paste),
                    ..Outcome::HANDLED
                };
            }
            _ => return Outcome::IGNORED,
        }
        Outcome::HANDLED
    }

    fn text(&mut self, text: &str) -> Outcome {
        self.insert(text)
    }

    fn blur(&mut self, _: Instant) -> Outcome {
        self.anchor = self.caret;
        self.commit()
    }
}
