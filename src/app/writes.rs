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
