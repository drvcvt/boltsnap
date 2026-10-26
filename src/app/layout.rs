//! Where everything goes at a window size: the navigation column, the page's
//! column of groups, each row's parts. Rows put their control below the name
//! when both do not fit side by side (settings plan, Phase 4b). Crossing a
//! threshold is a state change: the navigation opens and closes and rows
//! stack on springs (Phase 4c).

use std::time::{Duration, Instant};

use super::{App, Block, Row};
use crate::ui::Rect;
use crate::ui::motion::Spring;
use crate::ui::theme::{
    CONTENT_MAX, CTL_H, GAP_L, GAP_M, GAP_S, GAP_XL, GAP_XS, GLIDE, ICON, LABEL_MIN,
    NAV_NARROW_BELOW, NAV_NARROW_W, NAV_W, ROW_H,
};
use crate::ui::widgets::{self, Widget};

/// A text line: a group heading, a row's name above its hint.
pub const LINE: f32 = 16.0;
const HINT_LINE: f32 = 14.0;
/// A stacked row: the name's line, then the control, each `CTL_H`.
const STACKED_H: f32 = 2.0 * CTL_H + 3.0 * GAP_S;

/// Where a row's parts go.
#[derive(Clone, Copy, Debug)]
pub struct RowParts {
    pub icon: Rect,
    pub label: Rect,
    pub hint: Option<Rect>,
    pub control: Rect,
}

/// The page at the current size and scroll, in window coordinates.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    pub title: Rect,
    /// One per block: a heading's line, a banner, a row.
    pub blocks: Vec<Rect>,
    /// The surface behind each run of rows.
    pub panels: Vec<Rect>,
    /// From the top of the page to below its last block, unscrolled.
    pub height: f32,
}

/// Far enough ahead that every layout spring is at rest: the slowest,
/// `GLIDE`, settles within ten of its durations.
pub const AT_REST: Duration = Duration::from_secs(10);

fn lerp(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

/// Between two rectangles, on whole pixels (see [`App::column`]).
fn lerp_rect(from: Rect, to: Rect, t: f32) -> Rect {
    Rect::new(
        lerp(from.x, to.x, t).round(),
        lerp(from.y, to.y, t).round(),
        lerp(from.w, to.w, t).round(),
        lerp(from.h, to.h, t).round(),
    )
}

/// To `to` on `GLIDE`, or at once.
fn move_to(spring: &mut Spring, to: f32, animate: bool, now: Instant) {
    if animate {
        spring.retarget(to, GLIDE, now);
    } else {
        spring.set(to, now);
    }
}

/// The row's inner line, `GAP_M` in from the group's sides.
fn inner(rect: Rect) -> Rect {
    Rect::new(rect.x + GAP_M, rect.y, rect.w - 2.0 * GAP_M, rect.h)
}

/// Where the name starts, after the row's icon.
fn text_x(inner: Rect) -> f32 {
    inner.x + ICON + GAP_S
}

/// The room for the control beside the name.
fn room_beside(inner: Rect) -> f32 {
    inner.right() - text_x(inner) - LABEL_MIN - GAP_M
}

/// The control's width beside the name, or `None` when it does not fit
/// there and the row stacks.
fn beside(row: &Row, fonts: &crate::ui::text::Fonts, inner: Rect) -> Option<f32> {
    let room = room_beside(inner);
    let width = row.control.width(fonts, room);
    (width <= room).then_some(width)
}

impl App {
    /// Where the navigation is going: icons only below `NAV_NARROW_BELOW`.
    pub fn narrow(&self) -> bool {
        self.size.0 < NAV_NARROW_BELOW
    }

    /// 0 with icons only, 1 with names.
    pub fn nav_open(&self, now: Instant) -> f32 {
        self.nav_open.value(now).clamp(0.0, 1.0)
    }

    pub fn nav_w(&self, now: Instant) -> f32 {
        lerp(NAV_NARROW_W, NAV_W, self.nav_open(now))
    }

    /// A navigation entry's surface. Entries follow each other with no gap
    /// in their hit areas ([`App::nav_slot`]); the surface sits inside.
    pub fn nav_rect(&self, index: usize, now: Instant) -> Rect {
        let slot = self.nav_slot(index, now);
        Rect::new(slot.x, slot.y + GAP_XS / 2.0, slot.w, CTL_H)
    }

    /// An entry's hit area. The first entry's surface is on the title's line.
    pub fn nav_slot(&self, index: usize, now: Instant) -> Rect {
        let pitch = CTL_H + GAP_XS;
        Rect::new(
            GAP_S,
            GAP_XL - GAP_XS / 2.0 + index as f32 * pitch,
            self.nav_w(now) - 2.0 * GAP_S,
            pitch,
        )
    }

    /// The page's column right of a navigation `nav_w` wide: centred, at most
    /// `CONTENT_MAX` wide, `GAP_XL` from the edges at least.
    fn column_beside(&self, nav_w: f32) -> (f32, f32) {
        let area = self.size.0 - nav_w;
        let w = (area - 2.0 * GAP_XL).clamp(0.0, CONTENT_MAX);
        (nav_w + (area - w) / 2.0, w)
    }

    /// On whole pixels, like everything that moves the page: text sits on
    /// whole pixels, and what carries it must move in the same steps.
    pub fn column(&self, now: Instant) -> (f32, f32) {
        let (x, w) = self.column_beside(self.nav_w(now));
        (x.round(), w.round())
    }

    /// Point the layout's springs at the window's size: the navigation opens
    /// or closes, each row sits beside its name or stacks. A row new to the
    /// springs (a page just shown) starts where it belongs.
    pub(super) fn reshape(&mut self, now: Instant) {
        let animate = !self.reduced_motion;
        let open = if self.narrow() { 0.0 } else { 1.0 };
        move_to(&mut self.nav_open, open, animate, now);
        let nav_w = if self.narrow() { NAV_NARROW_W } else { NAV_W };
        let line = inner(Rect::new(0.0, 0.0, self.column_beside(nav_w).1, ROW_H));
        let stacked: Vec<f32> = self.pages[self.current]
            .blocks
            .iter()
            .map(|block| match block {
                Block::Row(row) if beside(row, &self.fonts, line).is_none() => 1.0,
                _ => 0.0,
            })
            .collect();
        for (index, to) in stacked.into_iter().enumerate() {
            match self.stacked.get_mut(index) {
                Some(spring) => move_to(spring, to, animate, now),
                None => self.stacked.push(Spring::settled(to, now)),
            }
        }
    }

    /// The layout `scroll` pixels down the page, `rise` pixels lower while a
    /// new page comes in.
    pub fn layout_at(&self, scroll: f32, rise: f32, now: Instant) -> Layout {
        let (x, w) = self.column(now);
        // Scroll and rise glide in fractions; the page moves in whole pixels
        // so its text never shifts against its surfaces.
        let top = (rise - scroll).round();
        let mut y = GAP_XL;
        let title = Rect::new(x, top + y, w, CTL_H);
        y += CTL_H + GAP_L;
        let mut layout = Layout {
            title,
            ..Layout::default()
        };
        let mut panel: Option<f32> = None;
        let mut first = true;
        let blocks = &self.page().blocks;
        for (index, block) in blocks.iter().enumerate() {
            let in_panel = matches!(block, Block::Row(_));
            if !in_panel && let Some(start) = panel.take() {
                y += GAP_XS;
                layout.panels.push(Rect::new(x, top + start, w, y - start));
            }
            if !first && panel.is_none() {
                // A heading sits close to its group; everything else is a
                // group apart.
                let after_heading = matches!(blocks[index - 1], Block::Heading(_));
                y += if after_heading { GAP_S } else { GAP_XL };
            }
            let h = match block {
                Block::Heading(_) => LINE,
                Block::Banner { .. } => ROW_H,
                Block::Row(_) => {
                    if panel.is_none() {
                        panel = Some(y);
                        y += GAP_XS;
                    }
                    let stacked = self.stacked.get(index).map_or(0.0, |s| s.value(now));
                    lerp(ROW_H, STACKED_H, stacked.clamp(0.0, 1.0)).round()
                }
            };
            layout.blocks.push(Rect::new(x, top + y, w, h));
            y += h;
            first = false;
        }
        if let Some(start) = panel {
            y += GAP_XS;
            layout.panels.push(Rect::new(x, top + start, w, y - start));
        }
        layout.height = y + GAP_XL;
        layout
    }

    pub fn layout(&self, now: Instant) -> Layout {
        self.layout_at(self.scroll.offset(now), self.rise(now), now)
    }

    pub fn block_rects(&self, now: Instant) -> Vec<Rect> {
        self.layout(now).blocks
    }

    /// A row's parts. How far it has stacked is in its height, which the
    /// layout takes between `ROW_H` and `STACKED_H` from the row's spring.
    pub fn row_parts(&self, rect: Rect, row: &Row) -> RowParts {
        let line = inner(rect);
        let text_x = text_x(line);
        let side = {
            let width = row.control.width(&self.fonts, room_beside(line));
            let control = Rect::new(
                line.right() - width,
                rect.y + (ROW_H - CTL_H) / 2.0,
                width,
                CTL_H,
            );
            let label = Rect::new(text_x, rect.y, (control.x - GAP_M - text_x).max(0.0), ROW_H);
            (label, control)
        };
        let stacked = {
            let label = Rect::new(text_x, rect.y + GAP_S, line.right() - text_x, CTL_H);
            let full = line.right() - text_x;
            let width = if row.control.stretches() {
                full
            } else {
                row.control.width(&self.fonts, full).min(full)
            };
            let control = Rect::new(line.right() - width, label.bottom() + GAP_S, width, CTL_H);
            (label, control)
        };
        let t = ((rect.h - ROW_H) / (STACKED_H - ROW_H)).clamp(0.0, 1.0);
        let (label_block, control) = if t == 0.0 {
            side
        } else if t == 1.0 {
            stacked
        } else {
            (
                lerp_rect(side.0, stacked.0, t),
                lerp_rect(side.1, stacked.1, t),
            )
        };
        let (label, hint) = if row.hint.is_some() {
            let y = label_block.y + (label_block.h - LINE - HINT_LINE) / 2.0;
            (
                Rect::new(label_block.x, y, label_block.w, LINE),
                Some(Rect::new(label_block.x, y + LINE, label_block.w, HINT_LINE)),
            )
        } else {
            (label_block, None)
        };
        RowParts {
            icon: widgets::icon_box(label, line.x, ICON),
            label,
            hint,
            control,
        }
    }

    /// Where lists may open: the page area, off the navigation.
    pub(super) fn bounds(&self, now: Instant) -> Rect {
        let x = self.nav_w(now);
        Rect::new(x, 0.0, self.size.0 - x, self.size.1).inset(GAP_S)
    }

    /// The open list, if a control has one: (block, list rect).
    pub(super) fn open_overlay(&self, now: Instant) -> Option<(usize, Rect)> {
        self.overlay_where(now, |control| control.overlay_open())
    }

    /// The list being drawn, also while it fades out.
    pub(super) fn visible_overlay(&self, now: Instant) -> Option<(usize, Rect)> {
        self.overlay_where(now, |_| true)
    }

    fn overlay_where(
        &self,
        now: Instant,
        wanted: impl Fn(&dyn Widget) -> bool,
    ) -> Option<(usize, Rect)> {
        let rects = self.block_rects(now);
        self.page()
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match block {
                Block::Row(row) if wanted(row.control.as_ref()) => {
                    let anchor = self.row_parts(rects[index], row).control;
                    let list = row
                        .control
                        .overlay(anchor, self.bounds(now), &self.fonts, now)?;
                    Some((index, list))
                }
                _ => None,
            })
    }
}
