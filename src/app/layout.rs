//! Where everything goes at a window size: the navigation column, the page's
//! column of groups, each row's parts. Rows put their control below the name
//! when both do not fit side by side (settings plan, Phase 4b).

use std::time::Instant;

use super::{App, Block, Row};
use crate::ui::Rect;
use crate::ui::theme::{
    CONTENT_MAX, CTL_H, GAP_L, GAP_M, GAP_S, GAP_XL, GAP_XS, ICON, LABEL_MIN, NAV_NARROW_BELOW,
    NAV_NARROW_W, NAV_W, ROW_H,
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

/// The row's inner line, `GAP_M` in from the group's sides.
fn inner(rect: Rect) -> Rect {
    Rect::new(rect.x + GAP_M, rect.y, rect.w - 2.0 * GAP_M, rect.h)
}

/// Where the name starts, after the row's icon.
fn text_x(inner: Rect) -> f32 {
    inner.x + ICON + GAP_S
}

/// The control's width beside the name, or `None` when it does not fit
/// there and the row stacks.
fn beside(row: &Row, fonts: &crate::ui::text::Fonts, inner: Rect) -> Option<f32> {
    let room = inner.right() - text_x(inner) - LABEL_MIN - GAP_M;
    let width = row.control.width(fonts, room);
    (width <= room).then_some(width)
}

impl App {
    pub fn narrow(&self) -> bool {
        self.size.0 < NAV_NARROW_BELOW
    }

    pub fn nav_w(&self) -> f32 {
        if self.narrow() { NAV_NARROW_W } else { NAV_W }
    }

    /// A navigation entry's surface. Entries follow each other with no gap
    /// in their hit areas ([`App::nav_slot`]); the surface sits inside.
    pub fn nav_rect(&self, index: usize) -> Rect {
        let slot = self.nav_slot(index);
        Rect::new(slot.x, slot.y + GAP_XS / 2.0, slot.w, CTL_H)
    }

    /// An entry's hit area. The first entry's surface is on the title's line.
    pub fn nav_slot(&self, index: usize) -> Rect {
        let pitch = CTL_H + GAP_XS;
        Rect::new(
            GAP_S,
            GAP_XL - GAP_XS / 2.0 + index as f32 * pitch,
            self.nav_w() - 2.0 * GAP_S,
            pitch,
        )
    }

    /// The page's column: centred right of the navigation, at most
    /// `CONTENT_MAX` wide, `GAP_XL` from the edges at least.
    pub fn column(&self) -> (f32, f32) {
        let area = self.size.0 - self.nav_w();
        let w = (area - 2.0 * GAP_XL).clamp(0.0, CONTENT_MAX);
        (self.nav_w() + (area - w) / 2.0, w)
    }

    fn row_height(&self, row: &Row, width: f32) -> f32 {
        let line = inner(Rect::new(0.0, 0.0, width, ROW_H));
        if beside(row, &self.fonts, line).is_some() {
            ROW_H
        } else {
            STACKED_H
        }
    }

    /// The layout `scroll` pixels down the page, `rise` pixels lower while a
    /// new page comes in.
    pub fn layout_at(&self, scroll: f32, rise: f32) -> Layout {
        let (x, w) = self.column();
        let top = rise - scroll;
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
                Block::Row(row) => {
                    if panel.is_none() {
                        panel = Some(y);
                        y += GAP_XS;
                    }
                    self.row_height(row, w)
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
        self.layout_at(self.scroll.offset(now), self.rise(now))
    }

    pub fn block_rects(&self, now: Instant) -> Vec<Rect> {
        self.layout(now).blocks
    }

    pub fn row_parts(&self, rect: Rect, row: &Row) -> RowParts {
        let line = inner(rect);
        let text_x = text_x(line);
        let (label_block, control) = match beside(row, &self.fonts, line) {
            Some(width) => {
                let control = Rect::new(
                    line.right() - width,
                    rect.y + (rect.h - CTL_H) / 2.0,
                    width,
                    CTL_H,
                );
                let label = Rect::new(
                    text_x,
                    rect.y,
                    (control.x - GAP_M - text_x).max(0.0),
                    rect.h,
                );
                (label, control)
            }
            None => {
                let label = Rect::new(text_x, rect.y + GAP_S, line.right() - text_x, CTL_H);
                let full = line.right() - text_x;
                let width = if row.control.stretches() {
                    full
                } else {
                    row.control.width(&self.fonts, full).min(full)
                };
                let control = Rect::new(line.right() - width, label.bottom() + GAP_S, width, CTL_H);
                (label, control)
            }
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
    pub(super) fn bounds(&self) -> Rect {
        let x = self.nav_w();
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
                        .overlay(anchor, self.bounds(), &self.fonts, now)?;
                    Some((index, list))
                }
                _ => None,
            })
    }
}
