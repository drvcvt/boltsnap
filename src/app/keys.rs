//! The keyboard: Tab through the navigation entry and the controls, arrows
//! in the navigation, shortcuts for pages and closing.

use std::time::Instant;

use super::{AT_REST, App, Block, Target};
use crate::ui::theme::{GAP_S, QUICK};
use crate::ui::widgets::Outcome;
use crate::ui::{Effect, Key, Mods};

impl App {
    /// Tab stops in reading order: the current page's navigation entry, then
    /// every enabled row, its reset button (left of the control) first.
    pub fn tab_order(&self) -> Vec<Target> {
        let mut order = vec![Target::Nav(self.current)];
        for (index, block) in self.page().blocks.iter().enumerate() {
            if let Block::Row(row) = block
                && row.enabled
            {
                if row.reset {
                    order.push(Target::Reset(index));
                }
                order.push(Target::Control(index));
            }
        }
        order
    }

    pub(super) fn tab(&mut self, back: bool, now: Instant) -> Outcome {
        let order = self.tab_order();
        let at = self
            .focus
            .and_then(|focus| order.iter().position(|t| *t == focus));
        let next = match (at, back) {
            (None, false) => 0,
            (None, true) => order.len() - 1,
            (Some(at), false) => (at + 1) % order.len(),
            (Some(at), true) => (at + order.len() - 1) % order.len(),
        };
        self.keyed.retarget(1.0, QUICK, now);
        let outcome = self.set_focus(Some(order[next]), now);
        if let Target::Control(block) | Target::Reset(block) = order[next] {
            let rect = self.layout_at(0.0, now + AT_REST).blocks[block];
            self.scroll
                .reveal(rect.y - GAP_S, rect.bottom() + GAP_S, self.size.1, now);
        }
        outcome
    }

    pub(super) fn key(&mut self, key: Key, mods: Mods, now: Instant) -> Vec<Effect> {
        if mods.ctrl {
            match key {
                Key::Char('w' | 'q') => return vec![Effect::Close],
                Key::Char(digit @ '1'..='9') => {
                    self.show(digit as usize - '1' as usize, now);
                    return Vec::new();
                }
                _ => {}
            }
        }
        if key == Key::Tab {
            return self.tab(mods.shift, now).effect.into_iter().collect();
        }
        let outcome = match self.focus {
            Some(Target::Control(block)) => {
                let fonts = &self.fonts;
                match self.pages[self.current].blocks.get_mut(block) {
                    Some(Block::Row(row)) => row.control.key(key, mods, fonts, now),
                    _ => Outcome::IGNORED,
                }
            }
            Some(Target::Reset(block)) => match key {
                Key::Space | Key::Enter => Outcome {
                    effect: Some(Effect::Reset(block)),
                    ..Outcome::HANDLED
                },
                _ => Outcome::IGNORED,
            },
            Some(Target::Nav(_)) => {
                let to = match key {
                    Key::Up => self.current.checked_sub(1),
                    Key::Down => Some(self.current + 1).filter(|&to| to < self.pages.len()),
                    _ => None,
                };
                match to {
                    Some(to) => {
                        self.keyed.retarget(1.0, QUICK, now);
                        self.show(to, now);
                        self.set_focus(Some(Target::Nav(to)), now);
                        Outcome::HANDLED
                    }
                    None => Outcome::IGNORED,
                }
            }
            None => Outcome::IGNORED,
        };
        if !outcome.handled && key == Key::Escape {
            return vec![Effect::Close];
        }
        outcome.effect.into_iter().collect()
    }
}
