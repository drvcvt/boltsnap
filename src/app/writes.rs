//! What the window saves: each row is bound to a key of `config.toml` or to
//! the replay buffer, and a value that changed goes out as a [`Write`]
//! (settings plan, Phase 5b).

use std::time::{Duration, Instant};

use super::{App, Block, Page, Target};
use crate::settings;
use crate::ui::Effect;
use crate::ui::widgets::Value;

/// While a control is dragged it reports at most this often; the last value
/// always goes out on release.
const THROTTLE: Duration = Duration::from_millis(100);

/// A control's value as the value of its key, or `None` for one it cannot be.
pub type Encode = Box<dyn Fn(&Value) -> Option<settings::Value>>;

pub enum Bind {
    /// Shows a value and writes nothing.
    None,
    /// A key of `config.toml`.
    Key(&'static str, Encode),
    /// Starts or stops the replay buffer.
    ReplayBuffer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Write {
    Key(&'static str, settings::Value),
    Replay(bool),
}

impl App {
    /// Take the current page's values as reported.
    pub(super) fn remember_values(&mut self) {
        self.sent = self.pages[self.current]
            .blocks
            .iter()
            .map(|block| match block {
                Block::Row(row) => Some(row.control.value()),
                _ => None,
            })
            .collect();
    }

    /// [`Effect::Changed`] for every row whose value moved since it last
    /// reported.
    pub(super) fn changes(&mut self, now: Instant) -> Vec<Effect> {
        if self.capture.is_some()
            && self
                .sent_at
                .is_some_and(|at| now.saturating_duration_since(at) < THROTTLE)
        {
            return Vec::new();
        }
        let mut effects = Vec::new();
        for (index, block) in self.pages[self.current].blocks.iter().enumerate() {
            let Block::Row(row) = block else { continue };
            let value = row.control.value();
            if let Some(sent) = self.sent.get_mut(index)
                && sent.as_ref() != Some(&value)
            {
                *sent = Some(value);
                effects.push(Effect::Changed(index));
            }
        }
        if !effects.is_empty() {
            self.sent_at = Some(now);
        }
        effects
    }

    /// What the row in `block` of the current page writes for its value now.
    pub fn write(&self, block: usize) -> Option<Write> {
        let row = self.row(block).filter(|row| row.enabled)?;
        match &row.bind {
            Bind::None => None,
            Bind::Key(key, encode) => Some(Write::Key(key, encode(&row.control.value())?)),
            Bind::ReplayBuffer => match row.control.value() {
                Value::Bool(on) => Some(Write::Replay(on)),
                _ => None,
            },
        }
    }

    /// The window closes: a control being edited commits, and what changed
    /// reports.
    pub fn finish(&mut self, now: Instant) -> Vec<Effect> {
        self.capture = None;
        self.blur(now);
        self.changes(now)
    }

    /// The pages again with the values in effect, after they changed
    /// elsewhere. Same blocks and rows (names and entries) → each row takes
    /// its new value through [`Widget::adopt`], except the focused and the
    /// dragged row and those `busy(page, block)` says still have a write on
    /// its way; what a row takes counts as reported, so nothing writes it
    /// back. Anything else changed → [`App::reload`].
    pub fn update(&mut self, pages: Vec<Page>, now: Instant, busy: impl Fn(usize, usize) -> bool) {
        if !same_shape(&self.pages, &pages) {
            self.reload(pages, now);
            return;
        }
        let (current, focus, capture) = (self.current, self.focus, self.capture);
        for (page, (old, new)) in self.pages.iter_mut().zip(pages).enumerate() {
            for (index, pair) in old.blocks.iter_mut().zip(new.blocks).enumerate() {
                let (old, new) = match pair {
                    (Block::Row(old), Block::Row(new)) => (old, new),
                    (old, new) => {
                        *old = new;
                        continue;
                    }
                };
                old.hint = new.hint;
                old.enabled = new.enabled;
                old.bind = new.bind;
                let held = Some(Target::Control(index));
                if page == current && (focus == held || capture == held || busy(page, index)) {
                    continue;
                }
                let value = new.control.value();
                if old.control.value() != value {
                    old.control.adopt(&value, now);
                    if page == current
                        && let Some(sent) = self.sent.get_mut(index)
                    {
                        *sent = Some(value);
                    }
                }
            }
        }
        // A hint that came or went changes heights.
        self.resize(self.size, now);
    }

    /// Fresh pages, after a write was refused or the state changed: the same
    /// page stays shown at the same scroll, controls show the new values and
    /// none keeps focus.
    pub fn reload(&mut self, pages: Vec<Page>, now: Instant) {
        if pages.is_empty() {
            return;
        }
        if matches!(self.focus, Some(Target::Control(_))) {
            self.focus = None;
        }
        self.hover = None;
        self.capture = None;
        self.fades.forget_controls();
        self.current = self.current.min(pages.len() - 1);
        self.pages = pages;
        self.stacked.clear();
        self.resize(self.size, now);
        self.remember_values();
        if let Some(at) = self.pointer {
            let target = self.target_at(at, now);
            self.set_hover(target, now);
        }
    }
}

/// The same pages, blocks of the same kinds, rows with the same names and
/// entries: then values can change in place.
fn same_shape(old: &[Page], new: &[Page]) -> bool {
    old.len() == new.len()
        && old.iter().zip(new).all(|(old, new)| {
            old.blocks.len() == new.blocks.len()
                && old.blocks.iter().zip(&new.blocks).all(|pair| match pair {
                    (Block::Row(a), Block::Row(b)) => {
                        a.label == b.label && a.control.labels() == b.control.labels()
                    }
                    (Block::Heading(a), Block::Heading(b)) => a == b,
                    (Block::Banner { .. }, Block::Banner { .. }) => true,
                    _ => false,
                })
        })
}
