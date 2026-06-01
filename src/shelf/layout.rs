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
            // pad gives the surface room around cards so the hover drop-shadow
            // (see draw_card_shadow) isn't clipped at the shelf edge.
            pad: 18,
            gap: 10,
            icon: 15,
            pad_icon: 7,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> LayoutConfig {
        LayoutConfig::default()
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
}
