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

/// What the last frame showed that the next may have to paint over.
pub(super) struct Painted {
    hover: Option<Target>,
    /// Something was moving that can change any part of the window.
    anywhere: bool,
    /// Where something stayed in its own rectangle while it moved.
    rects: Vec<Rect>,
}

impl App {
    /// Where the frame at `now` can differ from the last one painted, or
    /// `None` when it has to be painted whole: the first frame, after `full`
    /// (any input but a pointer move, new values, a new size), and while or
    /// right after something moved that can change any part of the window
    /// (`moving_anywhere`, a drag, an open list, blocks leaving). Otherwise
    /// the rectangles of the fades and controls that move now or moved for
    /// the last frame, and of both ends of a hover that changed: the whole
    /// block or navigation entry each, never one box around several, whose
    /// edges would cut through other icons (tiny-skia strokes a curve cut
    /// by the pixmap's edge a little differently; settings paint speed
    /// plan, stage D). Call it once for every frame painted.
    pub fn damage(&mut self, now: Instant, full: bool) -> Option<Vec<Rect>> {
        let last = self.painted.take();
        let layout = self.layout(now);
        let anywhere = self.moving_anywhere(now)
            || self.capture.is_some()
            || self.visible_overlay(now).is_some()
            || !layout.gone.is_empty();
        let mut lost = false;
        let mut rects = Vec::new();
        let mut place = |target: Option<Target>| {
            let rect = match target {
                None => return,
                Some(Target::Nav(index)) if index < self.pages.len() => {
                    Some(self.nav_rect(index, now))
                }
                Some(Target::Nav(_)) => None,
                Some(
                    Target::Control(block)
                    | Target::Reset(block)
                    | Target::Preview(block)
                    | Target::Card(block, _),
                ) => layout.blocks.get(block).copied(),
            };
            match rect {
                Some(rect) => rects.push(rect),
                None => lost = true,
            }
        };
        for target in self.fades.running(now) {
            place(Some(target));
        }
        for (index, block) in self.page().blocks.iter().enumerate() {
            if let Block::Row(row) = block
                && row.control.animating(now)
            {
                place(Some(Target::Control(index)));
            }
        }
        if let Some(last) = &last
            && last.hover != self.hover
        {
            place(last.hover);
            place(self.hover);
        }
        let hover = self.hover;
        self.painted = Some(Painted {
            hover,
            anywhere,
            rects: rects.clone(),
        });
        let last = last?;
        if full || anywhere || last.anywhere || lost {
            return None;
        }
        let mut damage: Vec<Rect> = Vec::new();
        for rect in rects.into_iter().chain(last.rects) {
            if !damage.contains(&rect) {
                damage.push(rect);
            }
        }
        Some(damage)
    }

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
            let (rect, visible) = (layout.blocks[index], layout.visible[index]);
            if rect.y + visible < 0.0 || rect.y > self.size.1 {
                continue;
            }
            if visible >= rect.h {
                self.draw_block(canvas, index, block, rect, now);
            } else {
                self.draw_part(canvas, index, block, rect, visible, now);
            }
        }
        // Leaving blocks are no target any more: drawn as nobody's.
        for &(gone, rect, visible) in &layout.gone {
            self.draw_part(
                canvas,
                usize::MAX,
                &self.gone[gone].block,
                rect,
                visible,
                now,
            );
        }
    }

    /// A block whose room is opening or closing: only its top `visible`
    /// pixels show, and its content only over the last quarter of its room
    /// (`presence::ink`). Drawn whole into its own pixels first, so nothing
    /// of it lands on the blocks below.
    fn draw_part(
        &self,
        canvas: &mut Canvas,
        index: usize,
        block: &Block,
        rect: Rect,
        visible: f32,
        now: Instant,
    ) {
        let ink = super::presence::ink(if rect.h > 0.0 { visible / rect.h } else { 0.0 });
        let scale = canvas.scale;
        let rows = (visible * scale).round() as u32;
        if ink <= 0.0 || rows == 0 {
            return;
        }
        let Some(mut own) = Canvas::new(rect.w.ceil() as u32, rect.h.ceil() as u32, scale as u32)
        else {
            return;
        };
        self.draw_block(
            &mut own,
            index,
            block,
            Rect::new(0.0, 0.0, rect.w, rect.h),
            now,
        );
        let rows = rows.min(own.pixmap.height());
        let Some(part) = tiny_skia::IntRect::from_xywh(0, 0, own.pixmap.width(), rows)
            .and_then(|cut| own.pixmap.clone_rect(cut))
        else {
            return;
        };
        let paint = tiny_skia::PixmapPaint {
            opacity: ink,
            ..Default::default()
        };
        canvas.pixmap.draw_pixmap(
            (rect.x * scale).round() as i32 - canvas.origin.0,
            (rect.y * scale).round() as i32 - canvas.origin.1,
            part.as_ref(),
            &paint,
            tiny_skia::Transform::identity(),
            None,
        );
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
