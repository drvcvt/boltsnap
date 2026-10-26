//! V4: the front card's measure, its width and height in pixels as the shelf
//! draws them, and its inner edge to drag for "Card size" (settings plan, 5e).
//! The height stands beside the front card's inner edge, the width past the
//! last card of the stack where the screen has room; each number sits in a
//! small surface at the middle of its line, on the side away from the cards
//! (V4b).

use std::time::Instant;

use super::super::{AT_REST, App, Bind, Block, Channel, Target};
use super::Preview;
use super::shelf::{CARD, SCREEN, screen, shelf_cards};
use crate::ui::theme::{self, GAP_S, GAP_XS, QUICK, TextStyle};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Rect};

/// The end marks across each line.
const TICK: f32 = 5.0;
/// A number's surface: its capitals with `GAP_XS` around.
const CHIP_H: f32 = TextStyle::Key.cap() + 2.0 * GAP_XS;
/// From a card to its line, the line, to the number, the number, to the
/// screen's edge.
const ROOM: f32 = GAP_S + 1.0 + GAP_XS + CHIP_H + GAP_XS;
/// How wide the edge is to grab.
const GRIP: f32 = 8.0;

/// Where the measure of the front card goes, on whole pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// 1 px tall, `GAP_S` past the stack's last card.
    pub width_line: Rect,
    /// 1 px wide, `GAP_S` beside the front card's inner edge.
    pub height_line: Rect,
    /// The numbers' surfaces, at the middle of their lines.
    pub width_chip: Rect,
    pub height_chip: Rect,
    /// The card's inner edge, to drag.
    pub edge: Rect,
}

/// The measure of `front` on `screen` in `corner` (top left, top right,
/// bottom left, bottom right), the width line past `far`, the y of the
/// stack's free end; `chips` are the numbers' surfaces' widths. A number
/// wider than a small card stays on the screen, at its line's end.
pub fn measure(screen: Rect, front: Rect, far: f32, corner: usize, chips: (f32, f32)) -> Measure {
    let (top, right) = (corner < 2, corner % 2 == 1);
    let (x, y) = (front.x.round(), front.y.round());
    let (r, b) = (front.right().round(), front.bottom().round());
    let inner = if right { x } else { r };
    let line_x = if right {
        inner - GAP_S - 1.0
    } else {
        inner + GAP_S
    };
    let far = far.round();
    let line_y = if top { far + GAP_S } else { far - GAP_S - 1.0 };
    let chip_y = if top {
        line_y + 1.0 + GAP_XS
    } else {
        line_y - GAP_XS - CHIP_H
    };
    let chip_x = if right {
        line_x - GAP_XS - chips.1
    } else {
        line_x + 1.0 + GAP_XS
    };
    Measure {
        width_line: Rect::new(x, line_y, r - x, 1.0),
        height_line: Rect::new(line_x, y, 1.0, b - y),
        width_chip: Rect::new(
            ((x + r - chips.0) / 2.0)
                .round()
                .min(screen.right() - GAP_XS - chips.0)
                .max(screen.x + GAP_XS),
            chip_y,
            chips.0,
            CHIP_H,
        ),
        height_chip: Rect::new(chip_x, ((y + b - CHIP_H) / 2.0).round(), chips.1, CHIP_H),
        edge: Rect::new(inner - GRIP / 2.0, y, GRIP, b - y),
    }
}

/// The free end of `count` cards `width` wide in `corner`, on the reference
/// screen: past the last card, where the width line goes.
pub(super) fn far_end(corner: usize, width: f32, count: usize) -> f32 {
    let cards = shelf_cards(corner, width, count.max(1));
    let last = cards[cards.len() - 1];
    if corner < 2 { last.bottom() } else { last.y }
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
        let screen = screen(rect);
        let far = screen.y + self.preview.far.value(now) * screen.w / SCREEN.0;
        (shown > 0.0).then(|| measure(screen, card, far, shape.corner, self.chip_widths()))
    }

    /// How wide the numbers' surfaces are.
    fn chip_widths(&self) -> (f32, f32) {
        let Some((width, height)) = self.shelf_measure_text() else {
            return (0.0, 0.0);
        };
        let chip = |text: &str| self.fonts.width(TextStyle::Key, text).ceil() + 2.0 * GAP_XS;
        (chip(&width), chip(&height))
    }

    /// The numbers: the card's width and height as the shelf makes them
    /// (`CardSize::from_width`).
    pub fn shelf_measure_text(&self) -> Option<(String, String)> {
        let w = self.preview.shape?.width.round() as i64;
        let h = (w * CARD.1 as i64 + CARD.0 as i64 / 2) / CARD.0 as i64;
        Some((format!("{w}\u{a0}px"), format!("{h}\u{a0}px")))
    }

    /// 0 to 1: how much the measure shows; only where the screen has room
    /// for it past the stack and beside the front card.
    pub fn measure_shown(&self, now: Instant) -> f32 {
        self.preview.dims.value(now).clamp(0.0, 1.0)
    }

    /// Point the measure at whether the screen has room for it at this
    /// width, for the cards as the controls hold them.
    pub(in crate::app) fn sync_dims(&mut self, animate: bool, now: Instant) {
        let chips = self.chip_widths();
        let room = self
            .shelf_block()
            .zip(self.preview.shape)
            .is_some_and(|(block, shape)| {
                let rect = self.layout_at(0.0, now + AT_REST).blocks[block];
                let scale = screen(rect).w / SCREEN.0;
                let front = shelf_cards(shape.corner, shape.width, 1)[0];
                let far = far_end(shape.corner, shape.width, shape.count);
                let past = if shape.corner < 2 {
                    SCREEN.1 - far
                } else {
                    far
                };
                let beside = if shape.corner % 2 == 1 {
                    front.x
                } else {
                    SCREEN.0 - front.right()
                };
                shape.count > 0
                    && past * scale >= ROOM
                    && beside * scale >= GAP_S + 1.0 + 2.0 * GAP_XS + chips.1
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
        let surface = theme::mix(theme::RAISE, theme::HOVER, lift);
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
        for (text, chip) in [(&width, m.width_chip), (&height, m.height_chip)] {
            canvas.fill(chip, GAP_XS, surface, shown);
            // Capitals `GAP_XS` from the top and bottom: the cap height is
            // a whole pixel, so they sit exactly in the middle.
            let baseline = chip.y + GAP_XS + fonts.cap_height(TextStyle::Key);
            let x = chip.x + ((chip.w - fonts.width(TextStyle::Key, text)) / 2.0).round();
            fonts.draw(
                canvas,
                TextStyle::Key,
                text,
                (x, baseline.round()),
                theme::TEXT,
                shown,
            );
        }
    }
}
