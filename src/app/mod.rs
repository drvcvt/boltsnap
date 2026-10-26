//! The Boltsnap window's content: navigation on the left, one page of
//! grouped rows on the right. Knows nothing about the windowing system; the
//! platform feeds it [`Input`] and shows what [`App::draw`] paints.

mod draw;
mod fades;
pub mod gallery;
mod keys;
mod layout;
pub mod measure;
pub mod pages;
mod writes;

#[cfg(test)]
mod tests;

use std::time::Instant;

pub use fades::Channel;
pub use layout::{AT_REST, Layout, RowParts};
pub use writes::{Bind, Encode, Write};

use crate::ui::icons::Icon;
use crate::ui::motion::{Scroll, Spring};
use crate::ui::text::Fonts;
use crate::ui::theme::{GLIDE, PAGE_RISE, QUICK, Rgb, SNAP};
use crate::ui::widgets::{self, Outcome, Widget};
use crate::ui::{Effect, Input, Rect};

pub struct Row {
    pub icon: Icon,
    pub label: String,
    pub hint: Option<String>,
    pub control: Box<dyn Widget>,
    pub enabled: bool,
    pub bind: Bind,
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
    /// 0 with icons only, 1 with names (`layout::reshape`).
    nav_open: Spring,
    /// Per block of the current page: 0 beside its name, 1 stacked.
    stacked: Vec<Spring>,
    /// 0 to 1: the scroll thumb, there while the page is longer than the window.
    thumb: Spring,
    /// 1 while focus came from the keyboard, 0 after a click: the focus
    /// outline shows only for the first (like `:focus-visible`).
    keyed: Spring,
    size: (f32, f32),
    scroll: Scroll,
    focus: Option<Target>,
    pointer: Option<(f32, f32)>,
    /// The control that keeps the pointer from press to release.
    capture: Option<Target>,
    hover: Option<Target>,
    fades: fades::Fades,
    /// Per block of the current page: the value its row last reported.
    sent: Vec<Option<widgets::Value>>,
    sent_at: Option<Instant>,
    /// Spatial motion off: a page comes in without rising, the navigation
    /// and rows change shape at once.
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
            nav_open: Spring::settled(
                if size.0 < crate::ui::theme::NAV_NARROW_BELOW {
                    0.0
                } else {
                    1.0
                },
                now,
            ),
            stacked: Vec::new(),
            thumb: Spring::settled(0.0, now),
            keyed: Spring::settled(0.0, now),
            size,
            scroll: Scroll::new(now),
            focus: None,
            pointer: None,
            capture: None,
            hover: None,
            fades: fades::Fades::default(),
            sent: Vec::new(),
            sent_at: None,
            reduced_motion: false,
        };
        app.fades.set(Target::Nav(0), Channel::Current, 1.0, now);
        app.resize(size, now);
        app.thumb.set(app.thumb.target(), now);
        app.remember_values();
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

    /// The layout follows the window at once (it follows the compositor);
    /// what crosses a threshold moves on springs (`layout::reshape`).
    pub fn resize(&mut self, size: (f32, f32), now: Instant) {
        self.size = size;
        self.reshape(now);
        let height = self.layout_at(0.0, 0.0, now + AT_REST).height;
        self.scroll.set_max(height - size.1, now);
        let thumb = if self.scroll.max() > 0.0 { 1.0 } else { 0.0 };
        self.thumb.retarget(thumb, QUICK, now);
    }

    /// 0 to 1: how much the focus outline shows (see `keyed`).
    pub fn keyed(&self, now: Instant) -> f32 {
        self.keyed.value(now).clamp(0.0, 1.0)
    }

    /// 0 to 1: how much of the scroll thumb shows.
    pub fn thumb_alpha(&self, now: Instant) -> f32 {
        self.thumb.value(now).clamp(0.0, 1.0)
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
        self.fades
            .to(Target::Nav(self.current), Channel::Current, 0.0, now);
        self.fades
            .to(Target::Nav(index), Channel::Current, 1.0, now);
        self.current = index;
        // The new page's rows start where they belong.
        self.stacked.clear();
        self.nav.retarget(index as f32, SNAP, now);
        self.page_in.set(0.0, now);
        self.page_in.retarget(1.0, GLIDE, now);
        self.scroll = Scroll::new(now);
        self.capture = None;
        self.resize(self.size, now);
        self.remember_values();
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
        if x < self.nav_w(now) {
            return (0..self.pages.len())
                .find(|&index| self.nav_slot(index, now).contains(x, y))
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

    /// React to `input`; a row whose value changed reports it as
    /// [`Effect::Changed`] (`writes.rs`).
    pub fn handle(&mut self, input: Input, now: Instant) -> Vec<Effect> {
        let mut effects = self.react(input, now);
        effects.extend(self.changes(now));
        effects
    }

    fn react(&mut self, input: Input, now: Instant) -> Vec<Effect> {
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
        self.keyed.retarget(0.0, QUICK, now);
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

    pub fn animating(&self, now: Instant) -> bool {
        !self.nav.done(now)
            || !self.page_in.done(now)
            || !self.nav_open.done(now)
            || !self.thumb.done(now)
            || !self.keyed.done(now)
            || self.stacked.iter().any(|spring| !spring.done(now))
            || self.scroll.moving(now)
            || self.fades.moving(now)
            || self.page().blocks.iter().any(|block| match block {
                Block::Row(row) => row.control.animating(now),
                _ => false,
            })
    }
}
