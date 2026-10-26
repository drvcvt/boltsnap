//! The Boltsnap window's content: navigation on the left, one page of
//! grouped rows on the right. Knows nothing about the windowing system; the
//! platform feeds it [`Input`] and shows what [`App::draw`] paints.

mod draw;
mod fades;
pub mod gallery;
mod layout;
pub mod measure;
pub mod pages;

#[cfg(test)]
mod gates;
#[cfg(test)]
mod tests;

use std::time::Instant;

pub use fades::Channel;
pub use layout::{Layout, RowParts};

use crate::ui::icons::Icon;
use crate::ui::motion::{Scroll, Spring};
use crate::ui::text::Fonts;
use crate::ui::theme::{GAP_S, GLIDE, PAGE_RISE, Rgb, SNAP};
use crate::ui::widgets::{self, Outcome, Widget};
use crate::ui::{Effect, Input, Key, Mods, Rect};

pub struct Row {
    pub icon: Icon,
    pub label: String,
    pub hint: Option<String>,
    pub control: Box<dyn Widget>,
    pub enabled: bool,
}

/// A page is a list of blocks. Rows that follow each other share one group
/// surface; a heading names the group below it.
pub enum Block {
    Heading(String),
    Row(Row),
    Banner {
        icon: Icon,
        text: String,
        color: Rgb,
    },
}

pub struct Page {
    pub name: String,
    pub icon: Icon,
    pub blocks: Vec<Block>,
}

/// Something the pointer or the keyboard can be on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A navigation entry, by page.
    Nav(usize),
    /// A row's control, by block on the current page.
    Control(usize),
}

pub struct App {
    pub fonts: Fonts,
    pages: Vec<Page>,
    current: usize,
    /// The navigation selection's position, in entries.
    nav: Spring,
    /// 0 to 1 while the current page comes in.
    page_in: Spring,
    size: (f32, f32),
    scroll: Scroll,
    focus: Option<Target>,
    pointer: Option<(f32, f32)>,
    /// The control that keeps the pointer from press to release.
    capture: Option<Target>,
    hover: Option<Target>,
    fades: fades::Fades,
    /// Spatial motion off: a page comes in without rising.
    pub reduced_motion: bool,
}

impl App {
    pub fn new(fonts: Fonts, pages: Vec<Page>, size: (f32, f32), now: Instant) -> Self {
        let mut app = Self {
            fonts,
            pages,
            current: 0,
            nav: Spring::settled(0.0, now),
            page_in: Spring::settled(1.0, now),
            size,
            scroll: Scroll::new(now),
            focus: None,
            pointer: None,
            capture: None,
            hover: None,
            fades: fades::Fades::default(),
            reduced_motion: false,
        };
        app.resize(size, now);
        app
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn focus(&self) -> Option<Target> {
        self.focus
    }

    pub fn page(&self) -> &Page {
        &self.pages[self.current]
    }

    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// The value of the control in `block` on the current page.
    pub fn value(&self, block: usize) -> Option<widgets::Value> {
        Some(self.row(block)?.control.value())
    }

    /// A resize relayouts at once; it is on the no-animation list because it
    /// follows the compositor.
    pub fn resize(&mut self, size: (f32, f32), now: Instant) {
        self.size = size;
        let height = self.layout_at(0.0, 0.0).height;
        self.scroll.set_max(height - size.1, now);
    }

    /// The page by its navigation name, case-insensitive.
    pub fn show_named(&mut self, name: &str, now: Instant) -> bool {
        let Some(index) = self
            .pages
            .iter()
            .position(|page| page.name.eq_ignore_ascii_case(name))
        else {
            return false;
        };
        self.show(index, now);
        true
    }

    fn show(&mut self, index: usize, now: Instant) {
        if index >= self.pages.len() || index == self.current {
            return;
        }
        self.blur(now);
        self.unhover(now);
        self.fades.forget_controls();
        self.current = index;
        self.nav.retarget(index as f32, SNAP, now);
        self.page_in.set(0.0, now);
        self.page_in.retarget(1.0, GLIDE, now);
        self.scroll = Scroll::new(now);
        self.capture = None;
        self.resize(self.size, now);
    }

    /// How far below its place the page is while it comes in.
    fn rise(&self, now: Instant) -> f32 {
        if self.reduced_motion {
            0.0
        } else {
            PAGE_RISE * (1.0 - self.page_in.value(now))
        }
    }

    /// 0 to 1 while the current page fades in.
    pub fn page_alpha(&self, now: Instant) -> f32 {
        self.page_in.value(now).clamp(0.0, 1.0)
    }

    fn row(&self, block: usize) -> Option<&Row> {
        match self.page().blocks.get(block)? {
            Block::Row(row) => Some(row),
            _ => None,
        }
    }

    fn control(&mut self, block: usize) -> Option<&mut dyn Widget> {
        match self.pages[self.current].blocks.get_mut(block)? {
            Block::Row(row) => Some(row.control.as_mut()),
            _ => None,
        }
    }

    fn control_rect(&self, block: usize, now: Instant) -> Option<Rect> {
        let rect = *self.block_rects(now).get(block)?;
        Some(self.row_parts(rect, self.row(block)?).control)
    }

    fn target_at(&self, (x, y): (f32, f32), now: Instant) -> Option<Target> {
        if x < self.nav_w() {
            return (0..self.pages.len())
                .find(|&index| self.nav_slot(index).contains(x, y))
                .map(Target::Nav);
        }
        let rects = self.block_rects(now);
        self.page()
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match block {
                Block::Row(row)
                    if row.enabled && self.row_parts(rects[index], row).control.contains(x, y) =>
                {
                    Some(Target::Control(index))
                }
                _ => None,
            })
    }

    /// The pointer is on `target` now; parts inside a control follow it.
    fn set_hover(&mut self, target: Option<Target>, now: Instant) {
        if target != self.hover {
            if let Some(old) = self.hover {
                self.fades.to(old, Channel::Hover, 0.0, now);
            }
            if let Some(new) = target {
                self.fades.to(new, Channel::Hover, 1.0, now);
            }
            self.part_hover(self.hover, None, now);
            self.hover = target;
        }
        self.part_hover(target, self.pointer, now);
    }

    fn part_hover(&mut self, target: Option<Target>, at: Option<(f32, f32)>, now: Instant) {
        let Some(Target::Control(block)) = target else {
            return;
        };
        if let Some(rect) = self.control_rect(block, now)
            && let Some(Block::Row(row)) = self.pages[self.current].blocks.get_mut(block)
        {
            row.control.hover(rect, at, &self.fonts, now);
        }
    }

    fn unhover(&mut self, now: Instant) {
        self.set_hover(None, now);
    }

    /// 0 to 1 for how much `target` shows `channel` this frame.
    pub fn fade(&self, target: Target, channel: Channel, now: Instant) -> f32 {
        self.fades.value(target, channel, now)
    }

    /// Leave the focused control, committing what it holds.
    fn blur(&mut self, now: Instant) -> Outcome {
        let Some(old) = self.focus.take() else {
            return Outcome::IGNORED;
        };
        self.fades.to(old, Channel::Focus, 0.0, now);
        let Target::Control(block) = old else {
            return Outcome::IGNORED;
        };
        let Some(control) = self.control(block) else {
            return Outcome::IGNORED;
        };
        control.close_overlay(now);
        control.blur(now)
    }

    fn set_focus(&mut self, target: Option<Target>, now: Instant) -> Outcome {
        if target == self.focus {
            return Outcome::IGNORED;
        }
        let outcome = self.blur(now);
        self.focus = target;
        if let Some(target) = target {
            self.fades.to(target, Channel::Focus, 1.0, now);
        }
        outcome
    }

    /// Tab stops in reading order: the current page's navigation entry, then
    /// every enabled control.
    pub fn tab_order(&self) -> Vec<Target> {
        let mut order = vec![Target::Nav(self.current)];
        order.extend(
            self.page()
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, block)| matches!(block, Block::Row(row) if row.enabled))
                .map(|(index, _)| Target::Control(index)),
        );
        order
    }

    fn tab(&mut self, back: bool, now: Instant) -> Outcome {
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
        let outcome = self.set_focus(Some(order[next]), now);
        if let Target::Control(block) = order[next] {
            let rect = self.layout_at(0.0, 0.0).blocks[block];
            self.scroll
                .reveal(rect.y - GAP_S, rect.bottom() + GAP_S, self.size.1, now);
        }
        outcome
    }

    pub fn handle(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let outcome = match input {
            Input::Move { x, y } => self.moved((x, y), now),
            Input::Leave => {
                self.pointer = None;
                self.unhover(now);
                Outcome::HANDLED
            }
            Input::Press { x, y, clicks } => self.press((x, y), clicks, now),
            Input::Release => {
                if let Some(target) = self.capture.take() {
                    self.fades.to(target, Channel::Press, 0.0, now);
                    if let Target::Control(block) = target
                        && let Some(control) = self.control(block)
                    {
                        control.release(now);
                    }
                }
                Outcome::HANDLED
            }
            Input::Wheel { notches, pixels } => {
                if self.open_overlay(now).is_none() {
                    self.scroll.wheel(notches, pixels, now);
                    // The page moves under a still pointer.
                    if let Some(at) = self.pointer {
                        let target = self.target_at(at, now);
                        self.set_hover(target, now);
                    }
                }
                Outcome::HANDLED
            }
            Input::Key { key, mods } => return self.key(key, mods, now),
            Input::Text(text) | Input::Pasted(text) => match self.focus {
                Some(Target::Control(block)) => self
                    .control(block)
                    .map_or(Outcome::IGNORED, |control| control.text(&text)),
                _ => Outcome::IGNORED,
            },
        };
        outcome.effect.into_iter().collect()
    }

    fn moved(&mut self, at: (f32, f32), now: Instant) -> Outcome {
        self.pointer = Some(at);
        if let Some(Target::Control(block)) = self.capture
            && let Some(rect) = self.control_rect(block, now)
        {
            let fonts = &self.fonts;
            if let Some(Block::Row(row)) = self.pages[self.current].blocks.get_mut(block) {
                row.control.drag(rect, at, fonts);
            }
        }
        if let Some((block, list)) = self.open_overlay(now)
            && let Some(control) = self.control(block)
        {
            control.overlay_hover(list, at, now);
        }
        let target = self.target_at(at, now);
        self.set_hover(target, now);
        Outcome::HANDLED
    }

    fn press(&mut self, at: (f32, f32), clicks: u8, now: Instant) -> Outcome {
        if let Some((block, list)) = self.open_overlay(now) {
            let anchor = self.control_rect(block, now);
            let control = self.control(block).expect("overlay belongs to a row");
            let outcome = control.overlay_press(list, at, now);
            if !outcome.handled {
                // A press beside an open list only closes it, also on its
                // own control (which would otherwise open it again).
                control.close_overlay(now);
                if anchor.is_none_or(|anchor| !anchor.contains(at.0, at.1)) {
                    return self.set_focus(None, now);
                }
            }
            return outcome;
        }
        let target = self.target_at(at, now);
        if let Some(target) = target {
            self.capture = Some(target);
            self.fades.to(target, Channel::Press, 1.0, now);
        }
        match target {
            Some(Target::Nav(index)) => {
                self.show(index, now);
                self.set_focus(None, now);
                Outcome::HANDLED
            }
            Some(Target::Control(block)) => {
                self.set_focus(Some(Target::Control(block)), now);
                let rect = self.control_rect(block, now).expect("a control's row");
                let fonts = &self.fonts;
                let Some(Block::Row(row)) = self.pages[self.current].blocks.get_mut(block) else {
                    return Outcome::IGNORED;
                };
                row.control.press(rect, at, clicks, fonts, now)
            }
            None => self.set_focus(None, now),
        }
    }

    fn key(&mut self, key: Key, mods: Mods, now: Instant) -> Vec<Effect> {
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
            Some(Target::Nav(_)) => {
                let to = match key {
                    Key::Up => self.current.checked_sub(1),
                    Key::Down => Some(self.current + 1).filter(|&to| to < self.pages.len()),
                    _ => None,
                };
                match to {
                    Some(to) => {
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

    pub fn animating(&self, now: Instant) -> bool {
        !self.nav.done(now)
            || !self.page_in.done(now)
            || self.scroll.moving(now)
            || self.fades.moving(now)
            || self.page().blocks.iter().any(|block| match block {
                Block::Row(row) => row.control.animating(now),
                _ => false,
            })
    }
}
