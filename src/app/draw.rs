//! Painting the navigation and the current page.

use std::time::Instant;

use super::{App, Block, Channel, Target};
use crate::ui::icons::Icon;
use crate::ui::theme::{
    self, GAP_S, GAP_XS, ICON_ALONE, NAV_W, PANEL_RADIUS, RADIUS, THUMB_W, TextStyle,
};
use crate::ui::widgets::{self, Look};
use crate::ui::{Canvas, Rect};

/// The scroll thumb never gets shorter than this.
const THUMB_MIN: f32 = 24.0;

impl App {
    pub fn draw(&self, canvas: &mut Canvas, now: Instant) {
        canvas.clear(theme::WINDOW);
        self.draw_page(canvas, now);
        self.draw_thumb(canvas, now);
        // A new page comes in from the window's colour.
        let alpha = self.page_alpha(now);
        if alpha < 1.0 {
            let x = self.nav_w(now);
            canvas.fade(
                Rect::new(x, 0.0, self.size.0 - x, self.size.1),
                theme::WINDOW,
                alpha,
            );
        }
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

    /// Where the page is in its length, on the window's right edge; it
    /// fades in when the page gets longer than the window and out again.
    fn draw_thumb(&self, canvas: &mut Canvas, now: Instant) {
        let alpha = self.thumb_alpha(now);
        if alpha <= 0.0 {
            return;
        }
        // Fading out, a page that fits shows the whole track.
        let max = self.scroll.max().max(0.0);
        let track = Rect::new(
            self.size.0 - GAP_XS - THUMB_W,
            GAP_XS,
            THUMB_W,
            self.size.1 - 2.0 * GAP_XS,
        );
        let h = (track.h * self.size.1 / (self.size.1 + max)).max(THUMB_MIN);
        let at = if max > 0.0 {
            self.scroll.offset(now) / max
        } else {
            0.0
        };
        let y = track.y + (track.h - h) * at;
        canvas.fill(
            Rect::new(track.x, y, THUMB_W, h),
            THUMB_W / 2.0,
            theme::HOVER,
            alpha,
        );
    }

    fn draw_nav(&self, canvas: &mut Canvas, now: Instant) {
        let nav_w = self.nav_w(now);
        canvas.fill(
            Rect::new(0.0, 0.0, nav_w, self.size.1),
            0.0,
            theme::SIDEBAR,
            1.0,
        );
        // Pressed into the free corner, under everything else in the sidebar.
        self.mark.draw(
            canvas,
            super::brand::Mark::place(self.size.1),
            self.mark_alpha(now),
        );
        let slot = self.nav_rect(0, now);
        let pitch = self.nav_rect(1, now).y - slot.y;
        let selection = Rect {
            y: slot.y + self.nav.value(now) * pitch,
            ..slot
        };
        canvas.fill(selection, RADIUS, theme::RAISE, 1.0);
        let focus = self.fade(Target::Nav(self.current), Channel::Focus, now);
        widgets::focus_ring(canvas, selection, RADIUS, focus * self.keyed(now));
        // Closing, the icon grows to its free-standing size and moves to the
        // middle; the names fade out in the first half and are cut at the
        // edge, never shortened while they go.
        let open = self.nav_open(now);
        let names = (2.0 * open - 1.0).clamp(0.0, 1.0);
        let full = NAV_W - 2.0 * GAP_S;
        let text_x = self.nav_text_x();
        for (index, page) in self.pages.iter().enumerate() {
            let rect = self.nav_rect(index, now);
            let lit = self
                .fade(Target::Nav(index), Channel::Current, now)
                .max(self.fade(Target::Nav(index), Channel::Hover, now));
            let color = theme::mix(theme::SUB, theme::TEXT, lit);
            page.icon
                .draw(canvas, self.nav_icon(index, now), color, 1.0);
            if names <= 0.0 {
                continue;
            }
            let name = self.fonts.ellipsize(
                TextStyle::Label,
                &page.name,
                rect.x + full - theme::GAP_M - text_x,
            );
            let clip = Rect::new(text_x, rect.y, (rect.right() - text_x).max(0.0), rect.h);
            self.fonts.draw_clipped(
                canvas,
                TextStyle::Label,
                &name,
                (
                    text_x,
                    self.fonts.baseline(TextStyle::Label, rect.y, rect.h),
                ),
                color,
                names,
                clip,
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
            Block::Preview(preview) => self.draw_preview(canvas, preview, rect, now),
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
                // Names and hints come broken into lines that fit.
                for (index, name) in parts.names.iter().enumerate() {
                    let y = parts.label.y + index as f32 * parts.name_pitch;
                    let line = Rect { y, ..parts.label };
                    fonts.draw_in(canvas, TextStyle::Label, name, line, text, 1.0);
                }
                if let Some(first) = parts.hint {
                    for (index, hint) in parts.hints.iter().enumerate() {
                        let y = first.y + index as f32 * parts.hint_pitch;
                        fonts.draw_in(canvas, TextStyle::Hint, hint, Rect { y, ..first }, sub, 1.0);
                    }
                }
                let target = Target::Control(index);
                let look = Look {
                    hover: self.fade(target, Channel::Hover, now),
                    press: self.fade(target, Channel::Press, now),
                    focus: self.fade(target, Channel::Focus, now),
                    ring: self.fade(target, Channel::Focus, now) * self.keyed(now),
                    focused: self.focus == Some(target),
                    enabled: row.enabled,
                };
                row.control.draw(canvas, fonts, parts.control, look, now);
                let reset = Target::Reset(index);
                let shown = self.fade(reset, Channel::Shown, now);
                if shown > 0.0 {
                    let place = parts.reset.unwrap_or_else(|| parts.reset_place());
                    let look = Look {
                        hover: self.fade(reset, Channel::Hover, now),
                        press: self.fade(reset, Channel::Press, now),
                        focus: self.fade(reset, Channel::Focus, now),
                        ring: self.fade(reset, Channel::Focus, now) * self.keyed(now),
                        focused: self.focus == Some(reset),
                        enabled: row.enabled,
                    };
                    canvas.fill(place, RADIUS, look.surface(), shown);
                    widgets::focus_ring(canvas, place, RADIUS, look.ring * shown);
                    let x = place.x + (place.w - ICON_ALONE) / 2.0;
                    Icon::RotateCcw.draw(
                        canvas,
                        widgets::icon_box(place, x, ICON_ALONE),
                        theme::SUB,
                        shown,
                    );
                }
            }
        }
    }
}
