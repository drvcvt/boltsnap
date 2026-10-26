//! P1/V4: the shelf on a screen, to scale, following the page's controls;
//! the newest screenshot of a whole screen behind it, darkened so the cards
//! stand in front, and the newest others on the cards (settings plan, 5e).

use std::time::Instant;

use super::super::{App, Bind, Block};
use super::Preview;
use super::shots::cover;
use crate::ui::motion::Spring;
use crate::ui::theme::{self, GAP_M, GLIDE, PANEL_RADIUS, QUICK, SNAP};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Rect};

/// The screen the shelf preview scales from.
pub(super) const SCREEN: (f32, f32) = (1920.0, 1080.0);
/// The shelf's own geometry: from both edges of its corner
/// (`linux/shelf/cards.rs` `EDGE_MARGIN`), its padding and the gap between
/// cards (`shelf/layout.rs` `LayoutConfig`), a card's shape and corners
/// (`shelf/thumbnail.rs` `CardSize::DEFAULT`, `shelf/paint.rs` `CARD_RADIUS`).
pub(super) const EDGE: f32 = 24.0;
pub(super) const PAD: f32 = 22.0;
const GAP: f32 = 10.0;
pub(super) const CARD: (f32, f32) = (190.0, 132.0);
const CARD_RADIUS: f32 = 10.0;
/// The most cards the shelf shows (`shelf.visible_cards`, schema).
pub const MOST: usize = 8;
/// How far the screenshot behind the cards goes towards `WINDOW`: enough
/// that `TEXT` on a white screen keeps 4.5:1 (it needs 62.5 %).
pub const BACKDROP_DARK: f32 = 0.65;

/// The shelf preview's height for a column `width` wide.
pub fn height(width: f32) -> f32 {
    screen_w(width) * SCREEN.1 / SCREEN.0 + 2.0 * GAP_M
}

fn screen_w(width: f32) -> f32 {
    (width - 2.0 * GAP_M).max(0.0)
}

/// The preview's screen: the block's whole width, `GAP_M` in on every side
/// like any group's content, on whole pixels.
pub fn screen(rect: Rect) -> Rect {
    let w = screen_w(rect.w).round();
    let h = (w * SCREEN.1 / SCREEN.0).round();
    Rect::new(rect.x + GAP_M, rect.y + GAP_M, w, h)
}

/// Where the shelf puts `count` cards `width` wide in a corner (as the
/// segment orders them: top left, top right, bottom left, bottom right),
/// on the reference screen.
pub fn shelf_cards(corner: usize, width: f32, count: usize) -> Vec<Rect> {
    let h = (width * CARD.1 / CARD.0).round();
    let (top, right) = (corner < 2, corner % 2 == 1);
    let x = if right {
        SCREEN.0 - EDGE - PAD - width
    } else {
        EDGE + PAD
    };
    (0..count)
        .map(|index| {
            let from_edge = EDGE + PAD + index as f32 * (h + GAP);
            let y = if top {
                from_edge
            } else {
                SCREEN.1 - from_edge - h
            };
            Rect::new(x, y, width, h)
        })
        .collect()
}

/// One card of the shelf preview, on the reference screen.
#[derive(Clone, Copy)]
pub(super) struct Card {
    x: Spring,
    y: Spring,
    w: Spring,
    h: Spring,
    shown: Spring,
}

impl Card {
    fn at(rect: Rect, shown: bool, now: Instant) -> Self {
        Card {
            x: Spring::settled(rect.x, now),
            y: Spring::settled(rect.y, now),
            w: Spring::settled(rect.w, now),
            h: Spring::settled(rect.h, now),
            shown: Spring::settled(f32::from(u8::from(shown)), now),
        }
    }

    fn moving(&self, now: Instant) -> bool {
        [self.x, self.y, self.w, self.h, self.shown]
            .iter()
            .any(|spring| !spring.done(now))
    }
}

/// What the shelf preview follows: corner, card width and count as the
/// page's controls hold them now, before they are saved.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShelfShape {
    pub(super) corner: usize,
    pub(super) width: f32,
    pub(super) count: usize,
}

impl App {
    fn shelf_shape(&self) -> Option<ShelfShape> {
        let page = self.page();
        if !page
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Preview(Preview::Shelf)))
        {
            return None;
        }
        let value = |name: &str| {
            page.blocks.iter().find_map(|block| match block {
                Block::Row(row) => match &row.bind {
                    Bind::Key(key, _) if *key == name => Some(row.control.value()),
                    _ => None,
                },
                _ => None,
            })
        };
        let (Some(Value::Choice(corner)), Some(Value::Int(width)), Some(Value::Int(count))) = (
            value("shelf.corner"),
            value("shelf.card_width"),
            value("shelf.visible_cards"),
        ) else {
            return None;
        };
        Some(ShelfShape {
            corner,
            width: width as f32,
            count: (count.max(0) as usize).min(MOST),
        })
    }

    /// Point the preview's cards at what the controls hold: a new corner
    /// glides them there, a new width resizes them, a new count fades cards
    /// in or out where they belong. `animate` false sets them (a page just
    /// shown).
    pub(super) fn sync_shelf(&mut self, animate: bool, now: Instant) {
        let Some(shape) = self.shelf_shape() else {
            self.preview.shape = None;
            return;
        };
        let animate = animate && !self.reduced_motion;
        let slots = shelf_cards(shape.corner, shape.width, MOST);
        let moved = self.preview.shape.map(|old| old.corner != shape.corner);
        let far = super::dims::far_end(shape.corner, shape.width, shape.count);
        if self.preview.cards.len() != MOST || moved.is_none() || !animate {
            self.preview.far.set(far, now);
            self.preview.cards = slots
                .iter()
                .enumerate()
                .map(|(index, slot)| Card::at(*slot, index < shape.count, now))
                .collect();
        } else if self.preview.shape != Some(shape) {
            let token = if moved == Some(true) { GLIDE } else { SNAP };
            self.preview.far.retarget(far, token, now);
            for (index, (card, slot)) in self.preview.cards.iter_mut().zip(&slots).enumerate() {
                let shown = index < shape.count;
                if shown && card.shown.target() > 0.0 {
                    card.x.retarget(slot.x, token, now);
                    card.y.retarget(slot.y, token, now);
                    card.w.retarget(slot.w, token, now);
                    card.h.retarget(slot.h, token, now);
                } else {
                    // A card that comes or goes does so where it belongs.
                    card.x.set(slot.x, now);
                    card.y.set(slot.y, now);
                    card.w.set(slot.w, now);
                    card.h.set(slot.h, now);
                }
                card.shown.retarget(f32::from(u8::from(shown)), QUICK, now);
            }
        }
        self.preview.shape = Some(shape);
        self.sync_dims(animate, now);
    }

    pub(super) fn shelf_moving(&self, now: Instant) -> bool {
        self.preview.cards.iter().any(|card| card.moving(now)) || !self.preview.far.done(now)
    }

    /// Every slot's card right now, in window coordinates inside the
    /// preview block `rect`, with how much it shows (0 for a hidden one).
    pub(super) fn card_slots(&self, rect: Rect, now: Instant) -> Vec<(Rect, f32)> {
        let screen = screen(rect);
        let scale = screen.w / SCREEN.0;
        self.preview
            .cards
            .iter()
            .map(|card| {
                let r = Rect::new(
                    screen.x + card.x.value(now) * scale,
                    screen.y + card.y.value(now) * scale,
                    card.w.value(now) * scale,
                    card.h.value(now) * scale,
                );
                (r, card.shown.value(now).clamp(0.0, 1.0))
            })
            .collect()
    }

    /// The shelf preview's cards right now, in window coordinates inside the
    /// preview block `rect`, with how much each shows.
    pub fn preview_cards(&self, rect: Rect, now: Instant) -> Vec<(Rect, f32)> {
        self.card_slots(rect, now)
            .into_iter()
            .filter(|(_, shown)| *shown > 0.0)
            .collect()
    }

    pub(super) fn draw_shelf(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        let screen = screen(rect);
        // Inside a group whose padding equals its radius: the next step down.
        canvas.fill(screen, theme::RADIUS, theme::WINDOW, 1.0);
        let pictures = self.preview.shots_in.value(now).clamp(0.0, 1.0);
        if let Some(backdrop) = &self.preview.shots.screen {
            let place = cover(backdrop.size(), screen);
            canvas.picture(backdrop, place, screen, theme::RADIUS, pictures);
            canvas.fill(
                screen,
                theme::RADIUS,
                theme::WINDOW,
                BACKDROP_DARK * pictures,
            );
        }
        let radius = CARD_RADIUS * screen.w / SCREEN.0;
        let slots = self.card_slots(rect, now);
        for (index, (card, shown)) in slots.into_iter().enumerate() {
            // What runs past the screen's edge is cut there.
            let x = card.x.max(screen.x);
            let y = card.y.max(screen.y);
            let right = card.right().min(screen.right());
            let bottom = card.bottom().min(screen.bottom());
            if shown <= 0.0 || right <= x || bottom <= y {
                continue;
            }
            let visible = Rect::new(x, y, right - x, bottom - y);
            canvas.fill(visible, radius, theme::SELECTED, shown);
            // Newest first from the corner out: a card keeps its picture
            // when the count changes.
            if let Some(picture) = self.preview.shots.cards.get(index) {
                canvas.picture(picture, card, visible, radius, shown * pictures);
            }
        }
        self.draw_dims(canvas, rect, now);
    }
}
