//! Painting the navigation and the current page.

use std::time::Instant;

use super::{App, Block, Channel, Target};
use crate::ui::theme::{
    self, GAP_S, GAP_XS, ICON, ICON_ALONE, PANEL_RADIUS, RADIUS, THUMB_W, TextStyle,
};
use crate::ui::widgets::{self, Look};
use crate::ui::{Canvas, Rect};

/// The scroll thumb never gets shorter than this.
const THUMB_MIN: f32 = 24.0;

impl App {
    pub fn draw(&self, canvas: &mut Canvas, now: Instant) {
        canvas.clear(theme::WINDOW);
        self.draw_page(canvas, now);
        // A new page comes in from the window's colour.
        let alpha = self.page_alpha(now);
        if alpha < 1.0 {
            let x = self.nav_w();
            canvas.fill(
                Rect::new(x, 0.0, self.size.0 - x, self.size.1),
                0.0,
                theme::WINDOW,
                1.0 - alpha,
            );
        }
        self.draw_thumb(canvas, now);
        self.draw_nav(canvas, now);
        if let Some((block, list)) = self.visible_overlay(now) {
            let row = self.row(block).expect("overlay belongs to a row");
            row.control.draw_overlay(canvas, &self.fonts, list, now);
        }
    }

    fn draw_page(&self, canvas: &mut Canvas, now: Instant) {
        let layout = self.layout(now);
        self.fonts.draw_in(
            canvas,
            TextStyle::Title,
            &self.page().name,
            layout.title,
            theme::TEXT,
            1.0,
        );
        for panel in &layout.panels {
            canvas.fill(*panel, PANEL_RADIUS, theme::PANEL, 1.0);
        }
        for (index, block) in self.page().blocks.iter().enumerate() {
            let rect = layout.blocks[index];
            if rect.bottom() < 0.0 || rect.y > self.size.1 {
                continue;
            }
            self.draw_block(canvas, index, block, rect, now);
        }
    }

    /// Where the page is in its length, on the window's right edge; only
    /// when it scrolls.
    fn draw_thumb(&self, canvas: &mut Canvas, now: Instant) {
        let max = self.scroll.max();
        if max <= 0.0 {
            return;
        }
        let track = Rect::new(
            self.size.0 - GAP_XS - THUMB_W,
            GAP_XS,
            THUMB_W,
            self.size.1 - 2.0 * GAP_XS,
        );
        let h = (track.h * self.size.1 / (self.size.1 + max)).max(THUMB_MIN);
        let y = track.y + (track.h - h) * self.scroll.offset(now) / max;
        canvas.fill(
            Rect::new(track.x, y, THUMB_W, h),
            THUMB_W / 2.0,
            theme::HOVER,
            1.0,
        );
    }

    fn draw_nav(&self, canvas: &mut Canvas, now: Instant) {
        canvas.fill(
            Rect::new(0.0, 0.0, self.nav_w(), self.size.1),
            0.0,
            theme::SIDEBAR,
            1.0,
        );
        let slot = self.nav_rect(0);
        let pitch = self.nav_rect(1).y - slot.y;
        let selection = Rect {
            y: slot.y + self.nav.value(now) * pitch,
            ..slot
        };
        let focus = self.fade(Target::Nav(self.current), Channel::Focus, now);
        let fill = theme::mix(theme::RAISE, theme::HOVER, focus);
        canvas.fill(selection, RADIUS, fill, 1.0);
        for (index, page) in self.pages.iter().enumerate() {
            let rect = self.nav_rect(index);
            let lit = if index == self.current {
                1.0
            } else {
                self.fade(Target::Nav(index), Channel::Hover, now)
            };
            let color = theme::mix(theme::SUB, theme::TEXT, lit);
            if self.narrow() {
                let x = rect.x + (rect.w - ICON_ALONE) / 2.0;
                page.icon
                    .draw(canvas, widgets::icon_box(rect, x, ICON_ALONE), color, 1.0);
                continue;
            }
            // Text inset like a control's: (height − capitals) / 2 ≈ 12.
            let x = rect.x + theme::GAP_M;
            page.icon
                .draw(canvas, widgets::icon_box(rect, x, ICON), color, 1.0);
            let text_x = x + ICON + GAP_S;
            self.fonts.draw_in(
                canvas,
                TextStyle::Label,
                &page.name,
                Rect::new(text_x, rect.y, rect.right() - theme::GAP_M - text_x, rect.h),
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
                    hover: self.fade(target, Channel::Hover, now),
                    press: self.fade(target, Channel::Press, now),
                    focus: self.fade(target, Channel::Focus, now),
                    focused: self.focus == Some(target),
                    enabled: row.enabled,
                };
                row.control.draw(canvas, fonts, parts.control, look, now);
            }
        }
    }
}
