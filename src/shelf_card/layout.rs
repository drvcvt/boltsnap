#[derive(Clone, Copy)]
pub struct LayoutConfig {
    pub pad: u32,      // outer padding inside the surface
    pub gap: u32,      // vertical gap between thumbs
    pub icon: u32,     // icon square size
    pub pad_icon: u32, // inset of each corner button from the thumb's edge
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            // pad gives the surface room around cards so the diffuse card
            // drop-shadow (see draw_card_shadow) isn't clipped at the shelf edge.
            pad: 22,
            gap: 10,
            icon: 15,
            pad_icon: 7,
        }
    }
}

/// `[shelf] corner`: the screen corner the shelf sits in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Corner {
    #[default]
    BottomLeft,
    BottomRight,
    TopLeft,
    TopRight,
}

impl Corner {
    pub fn parse(value: &str) -> Option<Corner> {
        Some(match value {
            "bottom-left" => Corner::BottomLeft,
            "bottom-right" => Corner::BottomRight,
            "top-left" => Corner::TopLeft,
            "top-right" => Corner::TopRight,
            _ => return None,
        })
    }

    pub fn is_top(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::TopRight)
    }

    pub fn is_right(self) -> bool {
        matches!(self, Corner::BottomRight | Corner::TopRight)
    }
}

pub struct ThumbRect {
    pub id: u64,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Hit {
    Body(u64),
    Save(u64),
    Close(u64),
}

pub struct Layout {
    pub width: u32,
    pub height: u32,
    pub thumbs: Vec<ThumbRect>,
}

impl Layout {
    /// `sizes` is newest-first: (id, thumb_w, thumb_h). Stacks top-to-bottom.
    pub fn compute(sizes: &[(u64, u32, u32)], cfg: &LayoutConfig) -> Layout {
        let widest = sizes.iter().map(|(_, w, _)| *w).max().unwrap_or(0);
        let mut thumbs = Vec::with_capacity(sizes.len());
        let mut y = cfg.pad;
        for (i, (id, w, h)) in sizes.iter().enumerate() {
            if i > 0 {
                y += cfg.gap;
            }
            thumbs.push(ThumbRect {
                id: *id,
                x: cfg.pad,
                y,
                w: *w,
                h: *h,
            });
            y += *h;
        }
        let width = if sizes.is_empty() {
            1
        } else {
            cfg.pad * 2 + widest
        };
        let height = if sizes.is_empty() { 1 } else { y + cfg.pad };
        Layout {
            width,
            height,
            thumbs,
        }
    }

    pub fn hit(&self, x: f64, y: f64, cfg: &LayoutConfig) -> Option<Hit> {
        let inside = |cx: u32, cy: u32, cw: u32, chh: u32| {
            x >= cx as f64 && x < (cx + cw) as f64 && y >= cy as f64 && y < (cy + chh) as f64
        };
        for r in &self.thumbs {
            if !inside(r.x, r.y, r.w, r.h) {
                continue;
            }
            let (clx, cly, clw, clh) = close_cell(r, cfg);
            if inside(clx, cly, clw, clh) {
                return Some(Hit::Close(r.id));
            }
            let (sx, sy, sw, sh) = save_cell(r, cfg);
            if inside(sx, sy, sw, sh) {
                return Some(Hit::Save(r.id));
            }
            return Some(Hit::Body(r.id));
        }
        None
    }
}

/// Top-right close-button cell `(x, y, w, h)` for a card.
pub fn close_cell(r: &ThumbRect, cfg: &LayoutConfig) -> (u32, u32, u32, u32) {
    let x = (r.x + r.w)
        .saturating_sub(cfg.pad_icon)
        .saturating_sub(cfg.icon);
    (x, r.y + cfg.pad_icon, cfg.icon, cfg.icon)
}

/// Top-left save-button cell `(x, y, w, h)` for a card.
pub fn save_cell(r: &ThumbRect, cfg: &LayoutConfig) -> (u32, u32, u32, u32) {
    (r.x + cfg.pad_icon, r.y + cfg.pad_icon, cfg.icon, cfg.icon)
}

pub fn overflow_card_opacity(
    position: usize,
    visible: usize,
    appear_progress: Option<f32>,
    disappear_progresses: &[f32],
) -> Option<f32> {
    let mut overflow = position.checked_sub(visible)?;
    if overflow == 0
        && let Some(progress) = appear_progress
    {
        return Some(1.0 - progress.clamp(0.0, 1.0));
    }
    overflow = overflow.saturating_sub(usize::from(appear_progress.is_some()));
    disappear_progresses
        .get(overflow)
        .map(|progress| progress.clamp(0.0, 1.0))
}

pub fn animated_item_limit(
    model_len: usize,
    visible: usize,
    appearing: bool,
    disappearing: usize,
) -> usize {
    model_len.min(visible + usize::from(appearing) + disappearing)
}

/// `top`: the shelf hangs from a top corner and grows downward, so the older
/// cards slide down to make room for a new one even while the shelf is not
/// full. Hanging from the bottom, it grows upward and they stay put.
pub fn animated_card_layout(
    items: &[(u64, u32, u32, f32)],
    cfg: &LayoutConfig,
    visible: usize,
    top: bool,
    appear_progress: Option<f32>,
) -> Layout {
    if items.is_empty() {
        return Layout::compute(&[], cfg);
    }
    let scroll = appear_progress
        .filter(|_| top || items.len() > visible)
        .map(|progress| {
            ((items[0].2 + cfg.gap) as f32 * (1.0 - progress.clamp(0.0, 1.0))).round() as u32
        });
    let widest = items
        .iter()
        .take(visible)
        .map(|(_, w, ..)| *w)
        .max()
        .unwrap_or(0);
    let mut thumbs = Vec::with_capacity(items.len());
    let mut y = cfg.pad;
    // Cards above that have all collapsed take the gap below them along.
    let mut above_collapsed = 1.0;
    // `visible` cards' worth of slots, a leaving card holding what is left
    // of its own and an arriving one what it has scrolled in: the shelf ends
    // where they are used up, so the card at the bottom slides in or out by
    // just as much as the other one takes or gives up.
    let mut slots = visible as f32;
    let mut bottom = None;
    for (i, (id, w, h, collapse)) in items.iter().enumerate() {
        let before = y;
        if i > 0 {
            y += (cfg.gap as f32 * (1.0 - collapse) * (1.0 - above_collapsed)).round() as u32;
        }
        above_collapsed *= collapse;
        let height = (*h as f32 * (1.0 - collapse)).round() as u32;
        thumbs.push(ThumbRect {
            id: *id,
            x: cfg.pad,
            y: if i > 0 {
                y.saturating_sub(scroll.unwrap_or(0))
            } else {
                y
            },
            w: *w,
            h: height,
        });
        y += height;
        let share = match (i, appear_progress, scroll) {
            (0, Some(progress), Some(_)) => progress.clamp(0.0, 1.0),
            _ => 1.0 - collapse,
        };
        if bottom.is_none() && share > 0.0 && share >= slots {
            let used = (y - before) as f32 * slots / share;
            let end = before + used.round() as u32;
            // Below the first card, the scroll moves it with the cards.
            bottom = Some(if i > 0 {
                end.saturating_sub(scroll.unwrap_or(0))
            } else {
                end
            });
        }
        slots -= share.min(slots);
    }
    Layout {
        width: cfg.pad * 2 + widest,
        height: bottom.unwrap_or(y) + cfg.pad,
        thumbs,
    }
}

/// Keeps the layer at `size` while a card leaves: resizing it every frame made
/// Hyprland rearrange the layer and send a configure that drew again, off the
/// display's beat. Hanging from the bottom, the stack sits on the buffer's
/// bottom edge; the rest is transparent.
pub fn hold_size(mut layout: Layout, (width, height): (u32, u32), top: bool) -> Layout {
    let shift = height.saturating_sub(layout.height);
    if !top {
        for card in &mut layout.thumbs {
            card.y += shift;
        }
    }
    layout.width = layout.width.max(width);
    layout.height = layout.height.max(height);
    layout
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LayoutConfig {
        LayoutConfig::default()
    }

    #[test]
    fn corners_parse_from_the_config_names() {
        assert_eq!(Corner::parse("bottom-left"), Some(Corner::BottomLeft));
        assert_eq!(Corner::parse("top-right"), Some(Corner::TopRight));
        assert_eq!(Corner::parse("top"), None);
        assert!(Corner::TopLeft.is_top() && !Corner::TopLeft.is_right());
        assert!(!Corner::BottomRight.is_top() && Corner::BottomRight.is_right());
        assert_eq!(Corner::default(), Corner::BottomLeft);
    }

    #[test]
    fn stacks_newest_on_top_and_sizes_surface() {
        let c = cfg();
        // newest-first: id 2 (170x100) on top, id 1 (160x90) below
        let lay = Layout::compute(&[(2, 170, 100), (1, 160, 90)], &c);
        assert_eq!(lay.thumbs.len(), 2);
        assert_eq!(lay.thumbs[0].id, 2);
        assert_eq!(lay.thumbs[0].y, c.pad);
        assert_eq!(lay.thumbs[1].id, 1);
        assert_eq!(lay.thumbs[1].y, c.pad + 100 + c.gap);
        assert_eq!(lay.width, c.pad * 2 + 170);
        assert_eq!(lay.height, c.pad * 2 + 100 + c.gap + 90);
    }

    #[test]
    fn close_cell_is_top_right_inset() {
        let c = cfg();
        let lay = Layout::compute(&[(1, 200, 120)], &c);
        let r = &lay.thumbs[0];
        assert_eq!(
            close_cell(r, &c),
            (
                r.x + r.w - c.pad_icon - c.icon,
                r.y + c.pad_icon,
                c.icon,
                c.icon
            )
        );
    }

    #[test]
    fn save_cell_is_top_left_inset() {
        let c = cfg();
        let lay = Layout::compute(&[(1, 200, 120)], &c);
        let r = &lay.thumbs[0];
        assert_eq!(
            save_cell(r, &c),
            (r.x + c.pad_icon, r.y + c.pad_icon, c.icon, c.icon)
        );
    }

    #[test]
    fn hit_save_topleft_close_topright_else_body() {
        let c = cfg();
        let lay = Layout::compute(&[(7, 260, 180)], &c);
        let r = &lay.thumbs[0];
        let cx = (r.x + r.w / 2) as f64;
        let cy = (r.y + r.h / 2) as f64;
        assert_eq!(lay.hit(cx, cy, &c), Some(Hit::Body(7)));
        let (clx, cly, clw, clh) = close_cell(r, &c);
        assert_eq!(
            lay.hit((clx + clw / 2) as f64, (cly + clh / 2) as f64, &c),
            Some(Hit::Close(7))
        );
        let (sx, sy, sw, sh) = save_cell(r, &c);
        assert_eq!(
            lay.hit((sx + sw / 2) as f64, (sy + sh / 2) as f64, &c),
            Some(Hit::Save(7))
        );
        assert_eq!(lay.hit(10_000.0, 10_000.0, &c), None);
    }

    /// Each card's top edge from the edge the shelf hangs from, which the
    /// compositor keeps in place.
    fn from_bottom(layout: &Layout) -> Vec<(u64, i64)> {
        let height = i64::from(layout.height);
        let mut cards: Vec<_> = layout
            .thumbs
            .iter()
            .filter(|card| card.h > 0)
            .map(|card| (card.id, i64::from(card.y) - height))
            .collect();
        cards.sort();
        cards
    }

    /// Cards newest first as (id, height), `collapse` per id.
    fn leaving(cards: &[(u64, u32)], collapse: &[(u64, f32)], visible: usize) -> Layout {
        let items: Vec<_> = cards
            .iter()
            .take(visible + collapse.len())
            .map(|&(id, h)| {
                let c = collapse
                    .iter()
                    .find(|(of, _)| *of == id)
                    .map_or(0.0, |c| c.1);
                (id, 200, h, c)
            })
            .collect();
        animated_card_layout(&items, &cfg(), visible, false, None)
    }

    fn settled(cards: &[(u64, u32)], visible: usize) -> Layout {
        let sizes: Vec<_> = cards
            .iter()
            .take(visible)
            .map(|&(id, h)| (id, 200, h))
            .collect();
        Layout::compute(&sizes, &cfg())
    }

    const MIXED: [(u64, u32); 8] = [
        (0, 100),
        (1, 120),
        (2, 90),
        (3, 150),
        (4, 100),
        (5, 110),
        (6, 200),
        (7, 80),
    ];

    #[test]
    fn a_card_leaving_a_full_shelf_ends_where_the_shelf_settles() {
        let start = leaving(&MIXED, &[(0, 0.0)], 6);
        assert_eq!(start.height, settled(&MIXED, 6).height);
        let end = leaving(&MIXED, &[(0, 1.0)], 6);
        let after = settled(&MIXED[1..], 6);
        assert_eq!(end.height, after.height);
        assert_eq!(from_bottom(&end), from_bottom(&after));
    }

    #[test]
    fn a_card_arriving_at_a_full_shelf_starts_where_the_shelf_stood() {
        let items: Vec<_> = MIXED[..7]
            .iter()
            .map(|&(id, h)| (id, 200, h, 0.0))
            .collect();
        let arriving = |p| animated_card_layout(&items, &cfg(), 6, false, Some(p));
        // Every card the settled shelf shows stands where it shows it.
        let matches = |frame: &Layout, settled: &Layout| {
            let drawn = from_bottom(frame);
            frame.height == settled.height
                && from_bottom(settled).iter().all(|card| drawn.contains(card))
        };
        assert!(matches(&arriving(0.0), &settled(&MIXED[1..], 6)));
        assert!(matches(&arriving(1.0), &settled(&MIXED, 6)));
    }

    #[test]
    fn overlapping_dismissals_hand_over_without_a_jump() {
        let end = leaving(&MIXED, &[(0, 1.0), (1, 0.5)], 6);
        let after = leaving(&MIXED[1..], &[(1, 0.5)], 6);
        assert_eq!(end.height, after.height);
        assert_eq!(from_bottom(&end), from_bottom(&after));
    }

    #[test]
    fn the_top_two_leaving_take_their_gaps_along() {
        let cards = &MIXED[..3];
        let end = leaving(cards, &[(0, 1.0), (1, 1.0)], 6);
        let after = settled(&cards[2..], 6);
        assert_eq!(end.height, after.height);
        assert_eq!(from_bottom(&end), from_bottom(&after));
    }
}
