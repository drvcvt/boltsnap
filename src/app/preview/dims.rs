//! V4: the front card's measure, its width and height in pixels as the shelf
//! draws them, and its inner edge to drag for "Card size" (settings plan, 5e).
//! The card at the corner is the front one; the lines sit beside it, the
//! numbers in one column past the height line.

use std::time::Instant;

use super::super::{AT_REST, App, Bind, Block, Channel, Target};
use super::Preview;
use super::shelf::{CARD, EDGE, PAD, SCREEN, screen};
use crate::ui::theme::{self, GAP_S, GAP_XS, QUICK, TextStyle};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Rect};

/// The end marks across each line.
const TICK: f32 = 5.0;
/// The width number sits beside the card with its capitals on the width
/// line; its descender reaches `DESCENT` below the line, so the line keeps
/// that far from the screen's bottom edge, and `TICK_CLEAR` from the card.
const DESCENT: f32 = 9.0;
const TICK_CLEAR: f32 = 3.0;
/// The least room between the card and the screen's edge for both.
const ROOM: f32 = TICK_CLEAR + DESCENT - 0.5;
/// How wide the edge is to grab.
const GRIP: f32 = 8.0;

/// Where the measure of the front card goes, on whole pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// 1 px tall, under (or over) the card in the corner's margin.
    pub width_line: Rect,
    /// 1 px wide, `GAP_S` beside the card's inner edge.
    pub height_line: Rect,
    /// The numbers' column: their left edge, or their right edge when
    /// `right_aligned`.
    pub label_x: f32,
    pub right_aligned: bool,
    /// The middle of each number's capitals.
    pub width_y: f32,
    pub height_y: f32,
    /// The card's inner edge, to drag.
    pub edge: Rect,
}

/// The measure of `card` on `screen` in `corner` (top left, top right,
/// bottom left, bottom right).
pub fn measure(screen: Rect, card: Rect, corner: usize) -> Measure {
    let (top, right) = (corner < 2, corner % 2 == 1);
    let (x, y) = (card.x.round(), card.y.round());
    let (r, b) = (card.right().round(), card.bottom().round());
    let inner = if right { x } else { r };
    let line_x = if right {
        inner - GAP_S - 1.0
    } else {
        inner + GAP_S
    };
    let line_y = if top {
        ((screen.y + y) / 2.0).floor()
    } else {
        ((b + screen.bottom()) / 2.0)
            .min(screen.bottom() - DESCENT)
            .floor()
    };
    Measure {
        width_line: Rect::new(x, line_y, r - x, 1.0),
        height_line: Rect::new(line_x, y, 1.0, b - y),
        label_x: if right {
            line_x - GAP_XS
        } else {
            line_x + 1.0 + GAP_XS
        },
        right_aligned: right,
        width_y: line_y + 0.5,
        height_y: (y + b) / 2.0,
        edge: Rect::new(inner - GRIP / 2.0, y, GRIP, b - y),
    }
}

/// Dragging the edge: the row it sets, where it started.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    row: usize,
    from_x: f32,
    from_w: i64,
    /// Window pixels per shelf pixel, and +1 when the inner edge is the
    /// right one.
    scale: f32,
    dir: f32,
}

impl App {
    fn shelf_block(&self) -> Option<usize> {
        self.page()
            .blocks
            .iter()
            .position(|block| matches!(block, Block::Preview(Preview::Shelf)))
    }

    fn card_size_row(&self) -> Option<usize> {
        self.page().blocks.iter().position(|block| {
            matches!(block, Block::Row(row) if matches!(row.bind, Bind::Key("shelf.card_width", _)))
        })
    }

    /// The front card's measure in the shelf preview block `rect` now.
    pub fn shelf_measure(&self, rect: Rect, now: Instant) -> Option<Measure> {
        let shape = self.preview.shape?;
        let (card, shown) = *self.card_slots(rect, now).first()?;
        (shown > 0.0).then(|| measure(screen(rect), card, shape.corner))
    }

    /// The numbers: the card's width and height as the shelf makes them
    /// (`CardSize::from_width`).
    pub fn shelf_measure_text(&self) -> Option<(String, String)> {
        let w = self.preview.shape?.width.round() as i64;
        let h = (w * CARD.1 as i64 + CARD.0 as i64 / 2) / CARD.0 as i64;
        Some((format!("{w}\u{a0}px"), format!("{h}\u{a0}px")))
    }

    /// 0 to 1: how much the measure shows; only where the corner's margin
    /// has room for it.
    pub fn measure_shown(&self, now: Instant) -> f32 {
        self.preview.dims.value(now).clamp(0.0, 1.0)
    }

    /// Point the measure at whether the margin has room at this width.
    pub(in crate::app) fn sync_dims(&mut self, animate: bool, now: Instant) {
        let room = self.shelf_block().is_some_and(|block| {
            let rect = self.layout_at(0.0, now + AT_REST).blocks[block];
            screen(rect).w * (EDGE + PAD) / SCREEN.0 >= ROOM
        });
        let to = f32::from(u8::from(room));
        if animate {
            self.preview.dims.retarget(to, QUICK, now);
        } else {
            self.preview.dims.set(to, now);
        }
    }

    /// The edge under `at`, if the pointer is on it.
    pub(in crate::app) fn edge_at(
        &self,
        (x, y): (f32, f32),
        rects: &[Rect],
        now: Instant,
    ) -> Option<Target> {
        let block = self.shelf_block()?;
        self.card_size_row()?;
        let measure = self.shelf_measure(rects[block], now)?;
        measure
            .edge
            .contains(x, y)
            .then_some(Target::Preview(block))
    }

    /// The pointer took the edge of the preview in `block` at `x`.
    pub(in crate::app) fn grab_edge(&mut self, block: usize, x: f32, now: Instant) {
        let (Some(row), Some(shape)) = (self.card_size_row(), self.preview.shape) else {
            return;
        };
        let Some(Value::Int(from_w)) = self.value(row) else {
            return;
        };
        let rect = self.layout(now).blocks[block];
        self.preview.drag = Some(Drag {
            row,
            from_x: x,
            from_w,
            scale: screen(rect).w / SCREEN.0,
            dir: if shape.corner % 2 == 1 { -1.0 } else { 1.0 },
        });
    }

    /// The pointer moved while holding the edge: "Card size" follows, the
    /// slider snapping it to its steps.
    pub(in crate::app) fn drag_edge(&mut self, x: f32, now: Instant) {
        let Some(drag) = self.preview.drag else {
            return;
        };
        let width = drag.from_w as f32 + drag.dir * (x - drag.from_x) / drag.scale.max(0.01);
        if let Some(control) = self.control(drag.row) {
            control.adopt(&Value::Int(width.round() as i64), now);
        }
    }

    pub(in crate::app) fn drop_edge(&mut self) {
        self.preview.drag = None;
    }

    /// The row the edge is setting, while it is dragged.
    pub(in crate::app) fn dragged_row(&self) -> Option<usize> {
        self.preview.drag.map(|drag| drag.row)
    }

    pub(super) fn draw_dims(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        let shown = self.measure_shown(now);
        let (Some(m), Some((width, height))) =
            (self.shelf_measure(rect, now), self.shelf_measure_text())
        else {
            return;
        };
        if shown <= 0.0 {
            return;
        }
        let lift = self.shelf_block().map_or(0.0, |block| {
            let target = Target::Preview(block);
            self.fade(target, Channel::Hover, now)
                .max(self.fade(target, Channel::Press, now))
        });
        let line = theme::mix(theme::SUB, theme::TEXT, lift);
        let (w, h) = (m.width_line, m.height_line);
        let half = (TICK / 2.0).floor();
        for part in [
            w,
            Rect::new(w.x, w.y - half, 1.0, TICK),
            Rect::new(w.right() - 1.0, w.y - half, 1.0, TICK),
            h,
            Rect::new(h.x - half, h.y, TICK, 1.0),
            Rect::new(h.x - half, h.bottom() - 1.0, TICK, 1.0),
        ] {
            canvas.fill(part, 0.0, line, shown);
        }
        let fonts = &self.fonts;
        let cap = fonts.cap_height(TextStyle::Key);
        for (text, middle) in [(&width, m.width_y), (&height, m.height_y)] {
            let x = if m.right_aligned {
                m.label_x - fonts.width(TextStyle::Key, text)
            } else {
                m.label_x
            };
            let baseline = (middle + cap / 2.0).round();
            fonts.draw(
                canvas,
                TextStyle::Key,
                text,
                (x.round(), baseline),
                theme::TEXT,
                shown,
            );
        }
    }
}
