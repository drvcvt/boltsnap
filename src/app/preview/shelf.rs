//! V6: the shelf to try out (settings plan, 5g). Two placeholder cards at
//! their real size, painted by the overlay's own code (`shelf_card`), so the
//! look the page sets is the look the shelf has. A card's X dismisses it the
//! way the shelf does, and a new one arrives where the shelf puts new cards.

use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use image::RgbaImage;
use tiny_skia::{FillRule, IntSize, Mask, Pixmap, PixmapPaint, PixmapRef, Transform};

use super::super::{App, Bind, Block, Channel, Target};
use super::Preview;
use crate::shelf_card::layout::{
    Layout, LayoutConfig, animated_card_layout, animated_item_limit, close_cell, hold_size,
    overflow_card_opacity,
};
use crate::shelf_card::paint::{Frame, draw_shelf};
use crate::shelf_card::{self, APPEAR_MS, CardKind, CardLook, DISMISS_MS, ease_out_cubic};
use crate::ui::icons::Icon;
use crate::ui::motion::Spring;
use crate::ui::theme::{self, GAP_XL, GLIDE, ICON, PANEL_RADIUS, SNAP};
use crate::ui::widgets::Value;
use crate::ui::{Canvas, Rect, rounded_rect};

/// A card's shape (`shelf/thumbnail.rs` `CardSize::DEFAULT`).
pub(super) const CARD: (f32, f32) = (190.0, 132.0);
/// From the cards to the block's edges: room for their shadow (16 blur + 3
/// drop), the widest gap.
pub(super) const INSET: f32 = GAP_XL;
/// The most cards the preview shows.
const SHOWN: usize = 2;
/// The placeholder's picture: twice the icon size, alone on a card.
const PICTURE: f32 = 2.0 * ICON;

/// The block's height: two cards of the default size, the gap between.
pub fn height(_width: f32) -> f32 {
    2.0 * INSET + 2.0 * CARD.1 + LayoutConfig::default().gap as f32
}

/// A card's height for `width`, as the shelf makes it
/// (`CardSize::from_width`).
pub fn card_height(width: f32) -> f32 {
    let w = width.round() as i64;
    ((w * CARD.1 as i64 + CARD.0 as i64 / 2) / CARD.0 as i64) as f32
}

/// A card coming or going, since `start`.
#[derive(Clone, Copy)]
struct Anim {
    id: u64,
    leaving: bool,
    start: Instant,
}

impl Anim {
    fn eased(&self, now: Instant) -> f32 {
        let ms = if self.leaving { DISMISS_MS } else { APPEAR_MS };
        let t = now.saturating_duration_since(self.start).as_millis() as f32 / ms as f32;
        ease_out_cubic(t)
    }

    fn done(&self, now: Instant) -> bool {
        self.eased(now) >= 1.0
    }
}

/// The shelf to try out.
pub(super) struct Play {
    /// The cards, newest first.
    ids: Vec<u64>,
    next: u64,
    anims: Vec<Anim>,
    /// The card width, shelf pixels.
    pub(super) width: Spring,
    /// 0 to 1: left to right, top to bottom.
    pub(super) right: Spring,
    pub(super) bottom: Spring,
    /// 0 to 1: the second card shows ("Cards shown" is two or more).
    second: Spring,
    pub(super) shape: Option<ShelfShape>,
    /// The placeholder picture for the last size drawn.
    picture: RefCell<Option<(f32, RgbaImage)>>,
    /// What the painter drew last, and how often it ran.
    painted: RefCell<Option<Painted>>,
    paints: Cell<usize>,
    /// The block's outline in its own size and scale, when the shelf runs
    /// past it, and how often it was built.
    mask: RefCell<Option<(BlockSize, Mask)>>,
    masks: Cell<usize>,
}

impl Play {
    pub(super) fn new(now: Instant) -> Self {
        Self {
            ids: vec![2, 1],
            next: 3,
            anims: Vec::new(),
            width: Spring::settled(CARD.0, now),
            right: Spring::settled(0.0, now),
            bottom: Spring::settled(1.0, now),
            second: Spring::settled(1.0, now),
            shape: None,
            picture: RefCell::new(None),
            painted: RefCell::new(None),
            paints: Cell::new(0),
            mask: RefCell::new(None),
            masks: Cell::new(0),
        }
    }

    pub(super) fn moving(&self, now: Instant) -> bool {
        [self.width, self.right, self.bottom, self.second]
            .iter()
            .any(|spring| !spring.done(now))
            || self.anims.iter().any(|anim| !anim.done(now))
    }

    /// The cards there now: a leaving one until it has gone, a new one
    /// from when it starts to arrive.
    fn live(&self, now: Instant) -> impl Iterator<Item = u64> + '_ {
        self.ids.iter().copied().filter(move |id| {
            !self.anims.iter().any(|anim| {
                anim.id == *id
                    && (if anim.leaving {
                        anim.done(now)
                    } else {
                        anim.start > now
                    })
            })
        })
    }

    fn anim(&self, id: u64) -> Option<&Anim> {
        self.anims.iter().find(|anim| anim.id == id)
    }
}

/// What the preview follows: the controls as they are now, before saving.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ShelfShape {
    /// As the segment orders them: top left, top right, bottom left, bottom
    /// right.
    pub(super) corner: usize,
    pub(super) width: f32,
    pub(super) count: usize,
    pub(super) look: CardLook,
    pub(super) animations: bool,
}

/// One frame of the preview in device pixels: where the shelf's surface
/// sits in the block, its layout, the cards mid-animation, the scale.
pub(super) struct Placed {
    pub(super) origin: (i32, i32),
    pub(super) layout: Layout,
    pub(super) cfg: LayoutConfig,
    anims: Vec<(u64, f32, f32)>,
    scale: f32,
    /// A card's full size, device pixels: the placeholder's, whatever the
    /// slots are doing.
    card: (u32, u32),
}

impl Placed {
    /// A card's rect in the window, logical pixels.
    fn rect(&self, (x, y, w, h): (u32, u32, u32, u32)) -> Rect {
        let s = self.scale;
        Rect::new(
            (self.origin.0 + x as i32) as f32 / s,
            (self.origin.1 + y as i32) as f32 / s,
            w as f32 / s,
            h as f32 / s,
        )
    }
}

fn mix(from: f32, to: f32, t: f32) -> f32 {
    from + (to - from) * t
}

impl App {
    pub(super) fn shelf_block(&self) -> Option<usize> {
        self.page()
            .blocks
            .iter()
            .position(|block| matches!(block, Block::Preview(Preview::Shelf)))
    }

    fn shelf_shape(&self) -> Option<ShelfShape> {
        let page = self.page();
        self.shelf_block()?;
        let value = |name: &str| {
            page.blocks.iter().find_map(|block| match block {
                Block::Row(row) => match &row.bind {
                    Bind::Key(key, _) if *key == name => Some(row.control.value()),
                    _ => None,
                },
                _ => None,
            })
        };
        let on = |name: &str, default: bool| match value(name) {
            Some(Value::Bool(on)) => on,
            _ => default,
        };
        let (Some(Value::Choice(corner)), Some(Value::Int(width)), Some(Value::Int(count))) = (
            value("shelf.corner"),
            value("shelf.card_width"),
            value("shelf.visible_cards"),
        ) else {
            return None;
        };
        let default = CardLook::default();
        let opacity = match value("shelf.card_opacity") {
            Some(Value::Int(percent)) => percent as f32 / 100.0,
            _ => default.opacity,
        };
        Some(ShelfShape {
            corner,
            width: width as f32,
            count: count.max(1) as usize,
            look: CardLook {
                opacity,
                solid_on_hover: on("shelf.solid_on_hover", default.solid_on_hover),
                shadow: on("shelf.card_shadow", default.shadow),
            },
            animations: on("shelf.animations", true),
        })
    }

    /// Point the preview at what the controls hold: a new corner glides the
    /// cards there, a new width resizes them, a second card comes and goes
    /// with "Cards shown". `animate` false sets them (a page just shown).
    pub(super) fn sync_shelf(&mut self, animate: bool, now: Instant) {
        let Some(shape) = self.shelf_shape() else {
            self.preview.play.shape = None;
            return;
        };
        let animate = animate && !self.reduced_motion && self.preview.play.shape.is_some();
        let play = &mut self.preview.play;
        let right = f32::from(u8::from(shape.corner % 2 == 1));
        let bottom = f32::from(u8::from(shape.corner >= 2));
        let second = f32::from(u8::from(shape.count >= SHOWN));
        if animate {
            play.width.retarget(shape.width, SNAP, now);
            play.right.retarget(right, GLIDE, now);
            play.bottom.retarget(bottom, GLIDE, now);
            play.second.retarget(second, SNAP, now);
        } else {
            play.width.set(shape.width, now);
            play.right.set(right, now);
            play.bottom.set(bottom, now);
            play.second.set(second, now);
        }
        play.shape = Some(shape);
        self.sync_dims(animate, now);
    }

    pub(super) fn shelf_moving(&self, now: Instant) -> bool {
        self.preview.play.moving(now)
    }

    /// The X on card `id`: it leaves as on the shelf, then a new card
    /// arrives where the shelf puts new ones. One after the other: a card
    /// arriving while one leaves makes the shelf scroll its last card out
    /// and back. Without animations, or with reduced motion, both at once.
    pub(in crate::app) fn dismiss_card(&mut self, id: u64, now: Instant) {
        let animate = !self.reduced_motion
            && self
                .preview
                .play
                .shape
                .is_some_and(|shape| shape.animations);
        let play = &mut self.preview.play;
        // Gone for good: the ones that finished leaving.
        let gone: Vec<u64> = play
            .anims
            .iter()
            .filter(|anim| anim.leaving && anim.done(now))
            .map(|anim| anim.id)
            .collect();
        play.ids.retain(|id| !gone.contains(id));
        play.anims.retain(|anim| !anim.done(now));
        if !play.ids.contains(&id) || play.anim(id).is_some_and(|anim| anim.leaving) {
            return;
        }
        let new = play.next;
        play.next += 1;
        play.ids.insert(0, new);
        // A card still arriving leaves from where it is, as on the shelf
        // (`linux/shelf/anim.rs` replaces a card's animation).
        play.anims.retain(|anim| anim.id != id);
        if animate {
            play.anims.push(Anim {
                id,
                leaving: true,
                start: now,
            });
            play.anims.push(Anim {
                id: new,
                leaving: false,
                start: now + Duration::from_millis(DISMISS_MS as u64),
            });
        } else {
            play.ids.retain(|card| *card != id);
        }
    }

    /// The shelf in the block `rect` now, at `scale` device pixels per
    /// logical one: the overlay's own layout and choreography
    /// (`linux/shelf/anim.rs`), its surface hung in the chosen corner.
    pub(super) fn place_shelf(&self, rect: Rect, scale: f32, now: Instant) -> Option<Placed> {
        let shape = self.preview.play.shape?;
        let play = &self.preview.play;
        let s = scale;
        let base = LayoutConfig::default();
        let cfg = LayoutConfig {
            pad: (base.pad as f32 * s).round() as u32,
            gap: (base.gap as f32 * s).round() as u32,
            icon: (base.icon as f32 * s).round() as u32,
            pad_icon: (base.pad_icon as f32 * s).round() as u32,
        };
        let width = play.width.value(now);
        let (w, h) = (
            (width * s).round() as u32,
            (card_height(width) * s).round() as u32,
        );
        let second = play.second.value(now).clamp(0.0, 1.0);
        let visible = if second > 0.0 { SHOWN } else { 1 };
        let top = shape.corner < 2;
        let live: Vec<u64> = play.live(now).collect();
        let appear = live.first().and_then(|id| {
            play.anim(*id)
                .filter(|anim| !anim.leaving && !anim.done(now))
                .map(|anim| anim.eased(now))
        });
        let mut leaving: Vec<f32> = live
            .iter()
            .filter_map(|id| play.anim(*id))
            .filter(|anim| anim.leaving)
            .map(|anim| anim.eased(now))
            .collect();
        leaving.sort_by(|left, right| right.total_cmp(left));
        let limit = animated_item_limit(live.len(), visible, appear.is_some(), leaving.len());
        let items: Vec<(u64, u32, u32, f32)> = live
            .iter()
            .take(limit)
            .enumerate()
            .map(|(index, id)| {
                let leave = play
                    .anim(*id)
                    .filter(|anim| anim.leaving)
                    .map_or(0.0, |anim| anim.eased(now));
                // The second card folds away with "Cards shown" at one.
                let fold = if index == 1 { 1.0 - second } else { 0.0 };
                (*id, w, h, leave.max(fold))
            })
            .collect();
        let mut layout = animated_card_layout(&items, &cfg, visible, top, appear);
        if !leaving.is_empty() {
            let rest = (
                2 * cfg.pad + w,
                2 * cfg.pad + visible as u32 * h + (visible as u32 - 1) * cfg.gap,
            );
            layout = hold_size(layout, rest, top);
        }
        let anims = items
            .iter()
            .enumerate()
            .filter_map(|(position, (id, ..))| {
                let (scale, opacity) = match play.anim(*id) {
                    Some(anim) if anim.leaving => shelf_card::dismiss(anim.eased(now)),
                    Some(anim) if !anim.done(now) => shelf_card::appear(anim.eased(now)),
                    _ => (
                        1.0,
                        overflow_card_opacity(position, visible, appear, &leaving).unwrap_or(1.0),
                    ),
                };
                let opacity = if position == 1 {
                    opacity * second
                } else {
                    opacity
                };
                ((scale - 1.0).abs() > f32::EPSILON || (opacity - 1.0).abs() > f32::EPSILON)
                    .then_some((*id, scale, opacity))
            })
            .collect();
        // The surface's corner sits `INSET − pad` in from the block's.
        let edge = (INSET * s).round() as i32 - cfg.pad as i32;
        let near = (rect.x * s).round() as i32 + edge;
        let far = ((rect.right() * s).round() as i32 - edge - layout.width as i32) as f32;
        let x = mix(near as f32, far, play.right.value(now)).round() as i32;
        let near = (rect.y * s).round() as i32 + edge;
        let far = ((rect.bottom() * s).round() as i32 - edge - layout.height as i32) as f32;
        let y = mix(near as f32, far, play.bottom.value(now)).round() as i32;
        Some(Placed {
            origin: (x, y),
            layout,
            cfg,
            anims,
            scale,
            card: (w, h),
        })
    }

    /// The cards in the block `rect` now, in the window's logical pixels,
    /// with their close buttons; not the ones on their way out.
    pub fn play_cards(&self, rect: Rect, now: Instant) -> Vec<(u64, Rect, Rect)> {
        let Some(placed) = self.place_shelf(rect, 1.0, now) else {
            return Vec::new();
        };
        let play = &self.preview.play;
        placed
            .layout
            .thumbs
            .iter()
            .filter(|thumb| !play.anim(thumb.id).is_some_and(|anim| anim.leaving))
            .map(|thumb| {
                (
                    thumb.id,
                    placed.rect((thumb.x, thumb.y, thumb.w, thumb.h)),
                    placed.rect(close_cell(thumb, &placed.cfg)),
                )
            })
            .collect()
    }

    /// The card under `at`, as a hover target, if any.
    pub(in crate::app) fn card_at(
        &self,
        at: (f32, f32),
        rects: &[Rect],
        now: Instant,
    ) -> Option<Target> {
        let block = self.shelf_block()?;
        // What runs past the block is cut there, so it is not there to hit
        // either: the rows below keep their pointer.
        if !rects[block].contains(at.0, at.1) {
            return None;
        }
        self.play_cards(rects[block], now)
            .into_iter()
            .find(|(_, card, _)| card.contains(at.0, at.1))
            .map(|(id, ..)| Target::Card(block, id))
    }

    /// A press on a card: on its X, it goes.
    pub(in crate::app) fn press_card(
        &mut self,
        block: usize,
        id: u64,
        at: (f32, f32),
        now: Instant,
    ) {
        let rect = self.layout(now).blocks[block];
        let on_close = self
            .play_cards(rect, now)
            .iter()
            .any(|(card, _, close)| *card == id && close.contains(at.0, at.1));
        if on_close {
            self.dismiss_card(id, now);
            // As on the shelf: a closed card is no longer hovered, so it
            // leaves at its own opacity.
            let target = Target::Card(block, id);
            self.fades.set(target, Channel::Hover, 0.0, now);
            if self.hover == Some(target) {
                self.hover = None;
            }
        }
    }

    /// The placeholder picture `w`×`h` device pixels: a card's surface with
    /// the image icon in the middle; none of the user's pictures.
    fn picture(&self, (w, h): (u32, u32), scale: f32) -> Option<std::cell::Ref<'_, RgbaImage>> {
        let cache = &self.preview.play.picture;
        // Keyed by size and scale: the icon is drawn at the scale.
        if cache
            .borrow()
            .as_ref()
            .is_none_or(|(at, picture)| picture.dimensions() != (w, h) || *at != scale)
        {
            let mut canvas = Canvas {
                pixmap: Pixmap::new(w, h)?,
                scale,
            };
            canvas.clear(theme::SELECTED);
            let size = Rect::new(0.0, 0.0, w as f32 / scale, h as f32 / scale);
            Icon::Image.draw(
                &mut canvas,
                size.centered(PICTURE, PICTURE),
                theme::SUB,
                1.0,
            );
            // Opaque, so premultiplied and straight are the same.
            *cache.borrow_mut() =
                RgbaImage::from_raw(w, h, canvas.pixmap.take()).map(|picture| (scale, picture));
        }
        std::cell::Ref::filter_map(cache.borrow(), |kept| {
            kept.as_ref().map(|(_, picture)| picture)
        })
        .ok()
    }

    pub(super) fn draw_shelf(&self, canvas: &mut Canvas, rect: Rect, now: Instant) {
        canvas.fill(rect, PANEL_RADIUS, theme::PANEL, 1.0);
        if let (Some(shape), Some(placed)) = (
            self.preview.play.shape,
            self.place_shelf(rect, canvas.scale, now),
        ) {
            self.paint_cards(canvas, rect, shape.look, &placed, now);
        }
        self.draw_dims(canvas, rect, now);
    }

    /// The cards as the overlay paints them, the hovered ones blended in by
    /// how far their hover has come. The painter runs only when what it
    /// shows changes; a hover blends two kept pictures inside its card.
    fn paint_cards(
        &self,
        canvas: &mut Canvas,
        rect: Rect,
        look: CardLook,
        placed: &Placed,
        now: Instant,
    ) {
        let layout = &placed.layout;
        let block = self.shelf_block();
        let lifts: Vec<(usize, f32)> = layout
            .thumbs
            .iter()
            .enumerate()
            .map(|(index, thumb)| {
                let lift = block.map_or(0.0, |block| {
                    self.fade(Target::Card(block, thumb.id), Channel::Hover, now)
                });
                (index, lift.clamp(0.0, 1.0))
            })
            .filter(|(_, lift)| *lift > 0.0)
            .collect();
        let key = PaintKey {
            size: (layout.width.max(1), layout.height.max(1)),
            thumbs: layout
                .thumbs
                .iter()
                .map(|t| (t.id, t.x, t.y, t.w, t.h))
                .collect(),
            anims: placed.anims.clone(),
            look,
            scale: placed.scale,
        };
        // One card fully hovered: only its picture is needed; otherwise the
        // picture without hover too, to blend from.
        let solo = match lifts.as_slice() {
            [(index, lift)] if *lift >= 1.0 => Some(layout.thumbs[*index].id),
            _ => None,
        };
        let mut wanted = if solo.is_some() {
            Vec::new()
        } else {
            vec![None]
        };
        wanted.extend(
            lifts
                .iter()
                .map(|(index, _)| Some(layout.thumbs[*index].id)),
        );
        self.keep_painted(&key, &wanted, placed);
        let painted = self.preview.play.painted.borrow();
        let Some(painted) = painted.as_ref() else {
            return;
        };
        let state = |hovered: Option<u64>| {
            painted
                .states
                .iter()
                .find(|(id, _)| *id == hovered)
                .map(|(_, pixmap)| pixmap)
        };
        let blended;
        let cards = if let Some(id) = solo {
            let Some(hovered) = state(Some(id)) else {
                return;
            };
            hovered
        } else {
            let Some(base) = state(None) else {
                return;
            };
            if lifts.is_empty() {
                base
            } else {
                let mut out = base.clone();
                for (index, lift) in &lifts {
                    let thumb = &layout.thumbs[*index];
                    if let Some(hovered) = state(Some(thumb.id)) {
                        blend_rect(
                            &mut out,
                            hovered,
                            (thumb.x, thumb.y, thumb.w, thumb.h),
                            *lift,
                        );
                    }
                }
                blended = out;
                &blended
            }
        };
        self.draw_clipped(canvas, rect, placed, cards.as_ref());
    }

    /// Have the painter's picture for every state in `wanted` (no hover,
    /// hover on a card) at `key`, painting only what is missing.
    fn keep_painted(&self, key: &PaintKey, wanted: &[Option<u64>], placed: &Placed) {
        let play = &self.preview.play;
        let mut painted = play.painted.borrow_mut();
        if painted.as_ref().is_none_or(|painted| painted.key != *key) {
            *painted = Some(Painted {
                key: key.clone(),
                states: Vec::new(),
            });
        }
        let Some(painted) = painted.as_mut() else {
            return;
        };
        let (lw, lh) = key.size;
        for hovered in wanted {
            if painted.states.iter().any(|(id, _)| id == hovered) {
                continue;
            }
            let Some(picture) = self.picture(placed.card, placed.scale) else {
                return;
            };
            let mut pixels = vec![0u8; (lw * lh * 4) as usize];
            let frame = Frame {
                layout: &placed.layout,
                cfg: &placed.cfg,
                hovered: *hovered,
                anims: &placed.anims,
                save_flash: None,
                save_button: false,
                look: key.look,
                scale: placed.scale,
            };
            draw_shelf(&mut pixels, lw, lh, &frame, |_| {
                Some((&*picture, CardKind::Image))
            });
            play.paints.set(play.paints.get() + 1);
            // BGRA as the overlay's buffers hold it, to tiny-skia's RGBA.
            for pixel in pixels.chunks_exact_mut(4) {
                pixel.swap(0, 2);
            }
            if let Some(pixmap) =
                IntSize::from_wh(lw, lh).and_then(|size| Pixmap::from_vec(pixels, size))
            {
                painted.states.push((*hovered, pixmap));
            }
        }
    }

    /// The cards onto the canvas, cut at the block where they run past it:
    /// drawn first into a block-sized picture through a block-sized mask of
    /// its rounded outline, kept while the block keeps its size, so
    /// scrolling the page (which only moves the block) builds nothing anew.
    fn draw_clipped(&self, canvas: &mut Canvas, rect: Rect, placed: &Placed, cards: PixmapRef) {
        let s = canvas.scale;
        let (x, y) = placed.origin;
        let (w, h) = (placed.layout.width as i32, placed.layout.height as i32);
        let (bx, by) = ((rect.x * s).round() as i32, (rect.y * s).round() as i32);
        let (bw, bh) = ((rect.w * s).round() as u32, (rect.h * s).round() as u32);
        let paint = PixmapPaint::default();
        let inside = x >= bx && y >= by && x + w <= bx + bw as i32 && y + h <= by + bh as i32;
        if inside {
            canvas
                .pixmap
                .draw_pixmap(x, y, cards, &paint, Transform::identity(), None);
            return;
        }
        let key = (bw, bh, s);
        let cache = &self.preview.play.mask;
        if cache.borrow().as_ref().is_none_or(|(kept, _)| *kept != key) {
            let (Some(mut mask), Some(outline)) = (
                Mask::new(bw, bh),
                rounded_rect(Rect::new(0.0, 0.0, rect.w, rect.h), PANEL_RADIUS),
            ) else {
                return;
            };
            mask.fill_path(&outline, FillRule::Winding, true, canvas.transform());
            self.preview
                .play
                .masks
                .set(self.preview.play.masks.get() + 1);
            *cache.borrow_mut() = Some((key, mask));
        }
        let kept = cache.borrow();
        let (Some((_, mask)), Some(mut block)) = (kept.as_ref(), Pixmap::new(bw, bh)) else {
            return;
        };
        block.draw_pixmap(
            x - bx,
            y - by,
            cards,
            &paint,
            Transform::identity(),
            Some(mask),
        );
        canvas
            .pixmap
            .draw_pixmap(bx, by, block.as_ref(), &paint, Transform::identity(), None);
    }

    /// How often the block's mask was built, for the tests.
    #[cfg(test)]
    pub fn shelf_masks(&self) -> usize {
        self.preview.play.masks.get()
    }

    /// How often the painter has run, for the tests.
    #[cfg(test)]
    pub fn shelf_paints(&self) -> usize {
        self.preview.play.paints.get()
    }
}

/// `over` blended into `out` inside `(x, y, w, h)` by `t`, premultiplied
/// channel by channel.
fn blend_rect(out: &mut Pixmap, over: &Pixmap, (x, y, w, h): (u32, u32, u32, u32), t: f32) {
    let width = out.width();
    let (right, bottom) = ((x + w).min(width), (y + h).min(out.height()));
    let t = (t * 256.0).round() as u32;
    let from = over.data();
    let data = out.data_mut();
    for row in y..bottom {
        let span = (row * width + x) as usize * 4..(row * width + right) as usize * 4;
        for (to, on) in data[span.clone()].iter_mut().zip(&from[span]) {
            let (to32, on32) = (u32::from(*to), u32::from(*on));
            *to = ((to32 * (256 - t) + on32 * t) >> 8) as u8;
        }
    }
}

/// A kept mask's block: width and height in device pixels, and the scale.
type BlockSize = (u32, u32, f32);

/// What a kept picture of the painter shows.
#[derive(Clone, PartialEq)]
pub(super) struct PaintKey {
    size: (u32, u32),
    thumbs: Vec<(u64, u32, u32, u32, u32)>,
    anims: Vec<(u64, f32, f32)>,
    look: CardLook,
    scale: f32,
}

/// The painter's pictures at one key, by hovered card.
pub(super) struct Painted {
    key: PaintKey,
    states: Vec<(Option<u64>, Pixmap)>,
}
