//! The Boltsnap window's content: navigation on the left, one page of rows
//! on the right. Knows nothing about the windowing system; the platform feeds
//! it [`Input`] and shows what [`App::draw`] paints.

mod draw;
pub mod gallery;

#[cfg(test)]
mod tests;

use std::time::Instant;

use crate::ui::icons::Icon;
use crate::ui::motion::{Scroll, Tween};
use crate::ui::text::Fonts;
use crate::ui::theme::{
    CTL_H, EDGE, GAP_M, GAP_S, GAP_XS, GROUP_GAP, HOVER_FADE, ICON, NAV_SLIDE, NAV_W, ROW_H, Rgb,
    TextStyle,
};
use crate::ui::widgets::{self, Outcome, Widget};
use crate::ui::{Effect, Input, Key, Mods, Rect};

/// Hint line under a row's name.
const LABEL_LINE: f32 = 16.0;
const HINT_LINE: f32 = 14.0;

pub struct Row {
    pub icon: Icon,
    pub label: String,
    pub hint: Option<String>,
    pub control: Box<dyn Widget>,
    pub enabled: bool,
}

pub enum Block {
    Heading(String),
    Row(Row),
    Banner {
        icon: Icon,
        text: String,
        color: Rgb,
    },
    /// A line of text in one style, with a note on the right.
    Specimen {
        style: TextStyle,
        text: String,
        note: String,
    },
}

impl Block {
    fn height(&self) -> f32 {
        match self {
            Block::Heading(_) | Block::Specimen { .. } => CTL_H,
            Block::Row(_) | Block::Banner { .. } => ROW_H,
        }
    }
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

/// Where a row's parts go.
#[derive(Clone, Copy, Debug)]
pub struct RowParts {
    pub icon: Rect,
    pub label: Rect,
    pub hint: Option<Rect>,
    pub control: Rect,
}

pub struct App {
    pub fonts: Fonts,
    pages: Vec<Page>,
    current: usize,
    /// The navigation selection's position, in entries.
    nav: Tween,
    size: (f32, f32),
    scroll: Scroll,
    focus: Option<Target>,
    pointer: Option<(f32, f32)>,
    /// The control that keeps the pointer from press to release.
    capture: Option<usize>,
    hover: Option<Target>,
    fades: Vec<(Target, Tween)>,
}

impl App {
    pub fn new(fonts: Fonts, pages: Vec<Page>, size: (f32, f32), now: Instant) -> Self {
        let mut app = Self {
            fonts,
            pages,
            current: 0,
            nav: Tween::settled(0.0, now),
            size,
            scroll: Scroll::new(now),
            focus: None,
            pointer: None,
            capture: None,
            hover: None,
            fades: Vec::new(),
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

    /// The value of the control in `block` on the current page.
    pub fn value(&self, block: usize) -> Option<widgets::Value> {
        Some(self.row(block)?.control.value())
    }

    pub fn resize(&mut self, size: (f32, f32), now: Instant) {
        self.size = size;
        let content = self.content_height();
        self.scroll.set_max(content - size.1, now);
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
        self.nav.set(index as f32, now);
        true
    }

    fn show(&mut self, index: usize, now: Instant) {
        if index >= self.pages.len() || index == self.current {
            return;
        }
        self.blur(now);
        self.current = index;
        self.nav.retarget(index as f32, NAV_SLIDE, now);
        self.scroll = Scroll::new(now);
        self.capture = None;
        self.resize(self.size, now);
    }

    pub fn nav_rect(index: usize) -> Rect {
        Rect::new(
            GAP_S,
            GAP_S + index as f32 * (CTL_H + GAP_XS),
            NAV_W - 2.0 * GAP_S,
            CTL_H,
        )
    }

    fn content_height(&self) -> f32 {
        self.block_tops(0.0).last().map_or(0.0, |&(top, h)| top + h) + EDGE
    }

    /// Each block's (top, height) in page coordinates, `scroll` subtracted.
    fn block_tops(&self, scroll: f32) -> Vec<(f32, f32)> {
        let mut y = EDGE - scroll;
        self.page()
            .blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                match block {
                    Block::Heading(_) if index > 0 => y += GROUP_GAP,
                    Block::Banner { .. } if index > 0 => y += GAP_S,
                    _ => {}
                }
                let top = y;
                y += block.height();
                if matches!(block, Block::Banner { .. }) {
                    y += GAP_S;
                }
                (top, block.height())
            })
            .collect()
    }

    /// Each block's rect in the window this frame.
    pub fn block_rects(&self, now: Instant) -> Vec<Rect> {
        let x = NAV_W + EDGE;
        let w = self.size.0 - NAV_W - 2.0 * EDGE;
        self.block_tops(self.scroll.offset(now))
            .into_iter()
            .map(|(top, h)| Rect::new(x, top, w, h))
            .collect()
    }

    pub fn row_parts(&self, rect: Rect, row: &Row) -> RowParts {
        let control_w = row.control.width(&self.fonts);
        let control = Rect::new(
            rect.right() - control_w,
            rect.y + (rect.h - CTL_H) / 2.0,
            control_w,
            CTL_H,
        );
        let text_x = rect.x + ICON + GAP_S;
        let text_w = (control.x - GAP_M - text_x).max(0.0);
        let (label, hint) = if row.hint.is_some() {
            let top = rect.y + (rect.h - LABEL_LINE - HINT_LINE) / 2.0;
            (
                Rect::new(text_x, top, text_w, LABEL_LINE),
                Some(Rect::new(text_x, top + LABEL_LINE, text_w, HINT_LINE)),
            )
        } else {
            (Rect::new(text_x, rect.y, text_w, rect.h), None)
        };
        RowParts {
            icon: widgets::icon_box(label, rect.x, ICON),
            label,
            hint,
            control,
        }
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

    fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.0, self.size.1).inset(GAP_S)
    }

    /// The open list, if a control has one: (block, list rect).
    fn open_overlay(&self, now: Instant) -> Option<(usize, Rect)> {
        let rects = self.block_rects(now);
        self.page()
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match block {
                Block::Row(row) if row.control.overlay_open() => {
                    let anchor = self.row_parts(rects[index], row).control;
                    let list = row
                        .control
                        .overlay(anchor, self.bounds(), &self.fonts, now)?;
                    Some((index, list))
                }
                _ => None,
            })
    }

    fn target_at(&self, (x, y): (f32, f32), now: Instant) -> Option<Target> {
        if x < NAV_W {
            return (0..self.pages.len())
                .find(|&index| Self::nav_rect(index).contains(x, y))
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

    fn set_hover(&mut self, target: Option<Target>, now: Instant) {
        if target == self.hover {
            return;
        }
        for (to, which) in [(0.0, self.hover), (1.0, target)] {
            let Some(which) = which else {
                continue;
            };
            match self.fades.iter_mut().find(|(t, _)| *t == which) {
                Some((_, fade)) => fade.retarget(to, HOVER_FADE, now),
                None => {
                    let mut fade = Tween::settled(0.0, now);
                    fade.retarget(to, HOVER_FADE, now);
                    self.fades.push((which, fade));
                }
            }
        }
        self.hover = target;
        self.fades
            .retain(|(t, fade)| Some(*t) == target || !(fade.done(now) && fade.target() == 0.0));
    }

    fn hover_of(&self, target: Target, now: Instant) -> f32 {
        self.fades
            .iter()
            .find(|(t, _)| *t == target)
            .map_or(0.0, |(_, fade)| fade.value(now).clamp(0.0, 1.0))
    }

    /// Leave the focused control, committing what it holds.
    fn blur(&mut self, now: Instant) -> Outcome {
        let Some(Target::Control(block)) = self.focus.take() else {
            self.focus = None;
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
            let (top, h) = self.block_tops(0.0)[block];
            self.scroll
                .reveal(top - GAP_S, top + h + GAP_S, self.size.1, now);
        }
        outcome
    }

    pub fn handle(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let outcome = match input {
            Input::Move { x, y } => {
                self.pointer = Some((x, y));
                if let Some(block) = self.capture
                    && let Some(rect) = self.control_rect(block, now)
                {
                    let fonts = &self.fonts;
                    let control = match self.pages[self.current].blocks.get_mut(block) {
                        Some(Block::Row(row)) => row.control.as_mut(),
                        _ => return Vec::new(),
                    };
                    control.drag(rect, (x, y), fonts);
                }
                let target = self.target_at((x, y), now);
                self.set_hover(target, now);
                Outcome::HANDLED
            }
            Input::Leave => {
                self.pointer = None;
                self.set_hover(None, now);
                Outcome::HANDLED
            }
            Input::Press { x, y, clicks } => self.press((x, y), clicks, now),
            Input::Release => {
                if let Some(block) = self.capture.take()
                    && let Some(control) = self.control(block)
                {
                    control.release();
                }
                Outcome::HANDLED
            }
            Input::Wheel { notches, pixels } => {
                if self.open_overlay(now).is_none() {
                    self.scroll.wheel(notches, pixels, now);
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
        match self.target_at(at, now) {
            Some(Target::Nav(index)) => {
                self.show(index, now);
                self.focus = None;
                Outcome::HANDLED
            }
            Some(Target::Control(block)) => {
                self.set_focus(Some(Target::Control(block)), now);
                let rect = self.control_rect(block, now).expect("a control's row");
                let fonts = &self.fonts;
                let Some(Block::Row(row)) = self.pages[self.current].blocks.get_mut(block) else {
                    return Outcome::IGNORED;
                };
                let outcome = row.control.press(rect, at, clicks, fonts, now);
                if outcome.handled {
                    self.capture = Some(block);
                }
                outcome
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
            Some(Target::Nav(_)) => match key {
                Key::Up if self.current > 0 => {
                    self.show(self.current - 1, now);
                    self.focus = Some(Target::Nav(self.current));
                    Outcome::HANDLED
                }
                Key::Down => {
                    self.show(self.current + 1, now);
                    self.focus = Some(Target::Nav(self.current));
                    Outcome::HANDLED
                }
                _ => Outcome::IGNORED,
            },
            None => Outcome::IGNORED,
        };
        if !outcome.handled && key == Key::Escape {
            return vec![Effect::Close];
        }
        outcome.effect.into_iter().collect()
    }

    pub fn animating(&self, now: Instant) -> bool {
        !self.nav.done(now)
            || self.scroll.moving(now)
            || self.fades.iter().any(|(_, fade)| !fade.done(now))
            || self.page().blocks.iter().any(|block| match block {
                Block::Row(row) => row.control.animating(now),
                _ => false,
            })
    }
}
