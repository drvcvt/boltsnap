//! The front card's measure, its width and height in pixels as the shelf
//! makes them, and its inner edge to drag for "Card size" (settings plan,
//! 5e V4b, 5g V6): a line along the inner edge of the card in the corner,
//! the numbers in a small surface at its middle, on the side away from the
//! card. It marks where that card rests, so a card coming or going does not
//! move it.

use std::time::Instant;

use super::super::{AT_REST, App, Bind, Block, Channel, Target};
use super::shelf::{INSET, card_height};
use crate::ui::theme::{self, GAP_S, GAP_XS, QUICK, TextStyle};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Rect};

/// The end marks across the line.
const TICK: f32 = 5.0;
/// The numbers' surface: their capitals with `GAP_XS` around.
const CHIP_H: f32 = TextStyle::Key.cap() + 2.0 * GAP_XS;
/// How wide the edge is to grab.
const GRIP: f32 = 8.0;

/// Where the measure of the front card goes, on whole pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Measure {
    /// 1 px wide, `GAP_S` beside the card's inner edge.
    pub height_line: Rect,
    /// The numbers' surface, at the middle of the line.
    pub chip: Rect,
    /// What drags the card's inner edge: from just inside the card over
    /// the line and the numbers.
    pub edge: Rect,
}

/// The measure of `front` in a left (`right` false) or right corner; `chip`
/// is the numbers' surface's width. `shown` false: only the strip along
/// the edge drags, the measure is not there.
pub fn measure(front: Rect, right: bool, chip: f32, shown: bool) -> Measure {
    let (x, y) = (front.x.round(), front.y.round());
    let (r, b) = (front.right().round(), front.bottom().round());
    let inner = if right { x } else { r };
    let line_x = if right {
        inner - GAP_S - 1.0
    } else {
        inner + GAP_S
    };
    let chip_x = if right {
        line_x - GAP_XS - chip
    } else {
        line_x + 1.0 + GAP_XS
    };
    // From `GRIP / 2` inside the card to `GAP_XS` past the numbers.
    let reach = if shown {
        GAP_S + 1.0 + GAP_XS + chip + GAP_XS
    } else {
        GRIP / 2.0
    };
    let (from, to) = if right {
        (inner - reach, inner + GRIP / 2.0)
    } else {
        (inner - GRIP / 2.0, inner + reach)
    };
    Measure {
        height_line: Rect::new(line_x, y, 1.0, b - y),
        chip: Rect::new(chip_x, ((y + b - CHIP_H) / 2.0).round(), chip, CHIP_H),
        edge: Rect::new(from, y, to - from, b - y),
    }
}

/// Dragging the edge: the row it sets, where it started.
#[derive(Clone, Copy, Debug)]
pub(super) struct Drag {
    row: usize,
    from_x: f32,
    from_w: i64,
    /// +1 when the inner edge is the right one.
    dir: f32,
}

impl App {
    fn card_size_row(&self) -> Option<usize> {
        self.page().blocks.iter().position(|block| {
            matches!(block, Block::Row(row) if matches!(row.bind, Bind::Key("shelf.card_width", _)))
        })
    }

    /// Where the card in the corner rests in the preview block `rect` now.
    pub fn front_card(&self, rect: Rect, now: Instant) -> Option<Rect> {
        let play = &self.preview.play;
        play.shape?;
        let w = play.width.value(now);
        let h = card_height(w);
        let mix = |from: f32, to: f32, t: f32| from + (to - from) * t;
        Some(Rect::new(
            mix(
                rect.x + INSET,
                rect.right() - INSET - w,
                play.right.value(now),
            ),
            mix(
                rect.y + INSET,
                rect.bottom() - INSET - h,
                play.bottom.value(now),
            ),
            w,
            h,
        ))
    }

    /// The front card's measure in the shelf preview block `rect` now.
    pub fn shelf_measure(&self, rect: Rect, now: Instant) -> Option<Measure> {
        let shape = self.preview.play.shape?;
        let front = self.front_card(rect, now)?;
        Some(measure(
            front,
            shape.corner % 2 == 1,
            self.chip_width(),
            self.preview.dims.target() > 0.0,
        ))
    }

    fn chip_width(&self) -> f32 {
        self.shelf_measure_text().map_or(0.0, |text| {
            self.fonts.width(TextStyle::Key, &text).ceil() + 2.0 * GAP_XS
        })
    }

    /// The numbers: the card's width and height as the shelf makes them.
    pub fn shelf_measure_text(&self) -> Option<String> {
        let w = self.preview.play.shape?.width;
        Some(format!(
            "{}\u{a0}×\u{a0}{}\u{a0}px",
            w.round(),
            card_height(w)
        ))
    }

    /// 0 to 1: how much the measure shows; only where the block has room
    /// for it beside the card.
    pub fn measure_shown(&self, now: Instant) -> f32 {
        self.preview.dims.value(now).clamp(0.0, 1.0)
    }

    /// Point the measure at whether the block has room for it at this
    /// width, for the card as the controls hold it.
    pub(in crate::app) fn sync_dims(&mut self, animate: bool, now: Instant) {
        let chip = self.chip_width();
        let room = self
            .shelf_block()
            .zip(self.preview.play.shape)
            .is_some_and(|(block, shape)| {
                let rect = self.layout_at(0.0, now + AT_REST).blocks[block];
                rect.w - INSET - shape.width >= GAP_S + 1.0 + 2.0 * GAP_XS + chip
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

    /// The pointer took the edge at `x`.
    pub(in crate::app) fn grab_edge(&mut self, x: f32) {
        let (Some(row), Some(shape)) = (self.card_size_row(), self.preview.play.shape) else {
            return;
        };
        let Some(Value::Int(from_w)) = self.value(row) else {
            return;
        };
        self.preview.drag = Some(Drag {
            row,
            from_x: x,
            from_w,
            dir: if shape.corner % 2 == 1 { -1.0 } else { 1.0 },
        });
    }

    /// The pointer moved while holding the edge: "Card size" follows, the
    /// slider snapping it to its steps.
    pub(in crate::app) fn drag_edge(&mut self, x: f32, now: Instant) {
        let Some(drag) = self.preview.drag else {
            return;
        };
        let width = drag.from_w as f32 + drag.dir * (x - drag.from_x);
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
        let (Some(m), Some(text)) = (self.shelf_measure(rect, now), self.shelf_measure_text())
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
        let h = m.height_line;
        let half = (TICK / 2.0).floor();
        for part in [
            h,
            Rect::new(h.x - half, h.y, TICK, 1.0),
            Rect::new(h.x - half, h.bottom() - 1.0, TICK, 1.0),
        ] {
            canvas.fill(part, 0.0, line, shown);
        }
        let fonts = &self.fonts;
        let chip = m.chip;
        canvas.fill(chip, GAP_XS, surface, shown);
        // Capitals `GAP_XS` from the top and bottom: the cap height is a
        // whole pixel, so they sit exactly in the middle.
        let baseline = chip.y + GAP_XS + fonts.cap_height(TextStyle::Key);
        let x = chip.x + ((chip.w - fonts.width(TextStyle::Key, &text)) / 2.0).round();
        fonts.draw(
            canvas,
            TextStyle::Key,
            &text,
            (x, baseline.round()),
            theme::TEXT,
            shown,
        );
    }
}
