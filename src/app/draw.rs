//! Painting the navigation and the current page.

use std::time::Instant;

use super::{App, Block, Target};
use crate::ui::theme::{self, CTL_H, GAP_M, GAP_S, GAP_XS, ICON, RADIUS, TextStyle};
use crate::ui::widgets::{self, Look};
use crate::ui::{Canvas, Rect};

impl App {
    pub fn draw(&self, canvas: &mut Canvas, now: Instant) {
        canvas.clear(theme::WINDOW);
        self.draw_nav(canvas, now);
        let rects = self.block_rects(now);
        for (index, block) in self.page().blocks.iter().enumerate() {
            let rect = rects[index];
            if rect.bottom() < 0.0 || rect.y > self.size.1 {
                continue;
            }
            self.draw_block(canvas, index, block, rect, now);
        }
        if let Some((block, list)) = self.visible_overlay(now) {
            let row = self.row(block).expect("overlay belongs to a row");
            row.control
                .draw_overlay(canvas, &self.fonts, list, self.pointer, now);
        }
    }

    /// The list being drawn, also while it fades out.
    fn visible_overlay(&self, now: Instant) -> Option<(usize, Rect)> {
        let rects = self.block_rects(now);
        self.page()
            .blocks
            .iter()
            .enumerate()
            .find_map(|(index, block)| match block {
                Block::Row(row) => {
                    let anchor = self.row_parts(rects[index], row).control;
                    let list = row
                        .control
                        .overlay(anchor, self.bounds(), &self.fonts, now)?;
                    Some((index, list))
                }
                _ => None,
            })
    }

    fn draw_nav(&self, canvas: &mut Canvas, now: Instant) {
        let slot = Self::nav_rect(0);
        let selection = Rect {
            y: slot.y + self.nav.value(now) * (CTL_H + GAP_XS),
            ..slot
        };
        let focused = matches!(self.focus, Some(Target::Nav(_)));
        let fill = if focused { theme::HOVER } else { theme::RAISE };
        canvas.fill(selection, RADIUS, fill, 1.0);
        for (index, page) in self.pages.iter().enumerate() {
            let rect = Self::nav_rect(index);
            let lit = if index == self.current {
                1.0
            } else {
                self.hover_of(Target::Nav(index), now)
            };
            let color = theme::mix(theme::SUB, theme::TEXT, lit);
            let line = rect.inset(GAP_S);
            let line = Rect::new(line.x, rect.y, line.w, rect.h);
            page.icon
                .draw(canvas, widgets::icon_box(line, line.x, ICON), color, 1.0);
            let x = line.x + ICON + GAP_S;
            self.fonts.draw_in(
                canvas,
                TextStyle::Label,
                &page.name,
                Rect::new(x, rect.y, line.right() - x, rect.h),
                color,
                1.0,
            );
        }
    }

    fn draw_block(
        &self,
        canvas: &mut Canvas,
        index: usize,
        block: &Block,
        rect: Rect,
        now: Instant,
    ) {
        let fonts = &self.fonts;
        match block {
            Block::Heading(text) => {
                fonts.draw_in(canvas, TextStyle::Heading, text, rect, theme::SUB, 1.0);
            }
            Block::Banner { icon, text, color } => {
                widgets::draw_banner(canvas, fonts, rect, *icon, text, *color);
            }
            Block::Specimen { style, text, note } => {
                let note_w = fonts.width(TextStyle::Key, note);
                fonts.draw_in(
                    canvas,
                    *style,
                    text,
                    Rect::new(rect.x, rect.y, rect.w - note_w - GAP_M, rect.h),
                    theme::TEXT,
                    1.0,
                );
                let baseline = fonts.baseline(TextStyle::Key, rect.y, rect.h);
                fonts.draw(
                    canvas,
                    TextStyle::Key,
                    note,
                    (rect.right() - note_w, baseline),
                    theme::SUB,
                    1.0,
                );
            }
            Block::Row(row) => {
                let parts = self.row_parts(rect, row);
                let (text, sub) = if row.enabled {
                    (theme::TEXT, theme::SUB)
                } else {
                    (theme::DIM, theme::DIM)
                };
                row.icon.draw(canvas, parts.icon, sub, 1.0);
                fonts.draw_in(canvas, TextStyle::Label, &row.label, parts.label, text, 1.0);
                if let (Some(hint), Some(line)) = (&row.hint, parts.hint) {
                    fonts.draw_in(canvas, TextStyle::Hint, hint, line, sub, 1.0);
                }
                let target = Target::Control(index);
                let look = Look {
                    hover: self.hover_of(target, now),
                    focus: self.focus == Some(target),
                    enabled: row.enabled,
                    pointer: self.pointer.filter(|&(x, y)| parts.control.contains(x, y)),
                };
                row.control.draw(canvas, fonts, parts.control, look, now);
            }
        }
    }
}
