//! Blocks that come and go on the shown page open and close their room
//! instead of making everything below them jump (settings plan, Phase 5l):
//! a new block's room opens on `GLIDE` and its content comes in over the last
//! quarter of that; a block that goes fades over the first quarter while its
//! room closes.

use std::time::Instant;

use super::{App, Block, Page};
use crate::ui::motion::Spring;
use crate::ui::theme::GLIDE;

/// What a block is across a reload: its kind and name. A row whose value or
/// hint changed is the same row; a banner with other words is another.
#[derive(PartialEq)]
enum Key<'a> {
    Heading(&'a str),
    Row(&'a str),
    Banner(&'a str),
    Preview(std::mem::Discriminant<super::Preview>),
}

fn key(block: &Block) -> Key<'_> {
    match block {
        Block::Heading(text) => Key::Heading(text),
        Block::Row(row) => Key::Row(&row.label),
        Block::Banner { text, .. } => Key::Banner(text),
        Block::Preview(preview) => Key::Preview(std::mem::discriminant(preview)),
    }
}

/// A block that left the shown page, its room closing.
pub(super) struct Gone {
    pub block: Block,
    /// The page's block it sits before (the page's length: at the end).
    pub at: usize,
    pub room: Spring,
}

/// How much of a block's content shows at `room` (0 to 1) of its room: none
/// until three quarters are open, all when it is.
pub(super) fn ink(room: f32) -> f32 {
    ((room - 0.75) / 0.25).clamp(0.0, 1.0)
}

impl App {
    /// 0 to 1: how much of its room block `index` of the shown page takes.
    pub(super) fn room(&self, index: usize, now: Instant) -> f32 {
        self.rooms
            .get(index)
            .map_or(1.0, |spring| spring.value(now).clamp(0.0, 1.0))
    }

    /// `pages` are about to replace the current ones: on the shown page,
    /// blocks that stay keep their room, new ones open theirs, the ones that
    /// went close theirs. Blocks still leaving from before go at once.
    pub(super) fn arrive(&mut self, pages: &[Page], now: Instant) {
        self.gone.clear();
        let (Some(old), Some(new)) = (self.pages.get_mut(self.current), pages.get(self.current))
        else {
            self.rooms.clear();
            return;
        };
        let old_blocks = std::mem::take(&mut old.blocks);
        // In order: each new block takes the next old one with its key.
        let mut matched: Vec<Option<usize>> = Vec::with_capacity(new.blocks.len());
        let mut next = 0;
        for block in &new.blocks {
            let found = (next..old_blocks.len()).find(|&i| key(&old_blocks[i]) == key(block));
            if let Some(i) = found {
                next = i + 1;
            }
            matched.push(found);
        }
        let animate = !self.reduced_motion;
        let mut rooms = Vec::with_capacity(new.blocks.len());
        for found in &matched {
            rooms.push(match found {
                Some(i) => self
                    .rooms
                    .get(*i)
                    .copied()
                    .unwrap_or_else(|| Spring::settled(1.0, now)),
                None if animate => {
                    let mut spring = Spring::settled(0.0, now);
                    spring.retarget(1.0, GLIDE, now);
                    spring
                }
                None => Spring::settled(1.0, now),
            });
        }
        if animate {
            for (i, block) in old_blocks.into_iter().enumerate() {
                if matched.contains(&Some(i)) {
                    continue;
                }
                // Before the first new block that came from after it.
                let at = matched
                    .iter()
                    .position(|found| found.is_some_and(|j| j > i))
                    .unwrap_or(new.blocks.len());
                let mut room = self
                    .rooms
                    .get(i)
                    .copied()
                    .unwrap_or_else(|| Spring::settled(1.0, now));
                room.retarget(0.0, GLIDE, now);
                self.gone.push(Gone { block, at, room });
            }
        }
        self.rooms = rooms;
    }

    /// Whether a block is still opening or closing.
    pub(super) fn rooms_moving(&self, now: Instant) -> bool {
        self.rooms.iter().any(|spring| !spring.done(now))
            || self.gone.iter().any(|gone| !gone.room.done(now))
    }
}
