#[derive(Clone, Copy)]
pub struct LayoutConfig {
    pub pad: u32,      // outer padding inside the surface
    pub gap: u32,      // vertical gap between thumbs
    pub icon: u32,     // icon square size
    pub icon_gap: u32, // gap between icons
    pub pad_icon: u32, // inset of the icon strip from the thumb's top-right corner
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self { pad: 12, gap: 10, icon: 15, icon_gap: 5, pad_icon: 7 }
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
    Edit(u64),
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
            thumbs.push(ThumbRect { id: *id, x: cfg.pad, y, w: *w, h: *h });
            y += *h;
        }
        let width = if sizes.is_empty() { 1 } else { cfg.pad * 2 + widest };
        let height = if sizes.is_empty() { 1 } else { y + cfg.pad };
        Layout { width, height, thumbs }
    }

    /// Icon strip lives at the thumb's top-right: [edit][close], close rightmost.
    fn icon_rect(&self, r: &ThumbRect, slot_from_right: u32, cfg: &LayoutConfig) -> (u32, u32, u32, u32) {
        let right = (r.x + r.w).saturating_sub(cfg.pad_icon);
        let x = right
            .saturating_sub((slot_from_right + 1) * cfg.icon)
            .saturating_sub(slot_from_right * cfg.icon_gap);
        let y = r.y + cfg.pad_icon;
        (x, y, cfg.icon, cfg.icon)
    }

    pub fn hit(&self, x: f64, y: f64, cfg: &LayoutConfig) -> Option<Hit> {
        for r in &self.thumbs {
            let inside = x >= r.x as f64
                && x < (r.x + r.w) as f64
                && y >= r.y as f64
                && y < (r.y + r.h) as f64;
            if !inside {
                continue;
            }
            // icons: slot 0 = close (rightmost), slot 1 = edit
            for (slot, make) in [(0u32, Hit::Close(r.id)), (1, Hit::Edit(r.id))] {
                let (ix, iy, iw, ih) = self.icon_rect(r, slot, cfg);
                if x >= ix as f64 && x < (ix + iw) as f64 && y >= iy as f64 && y < (iy + ih) as f64 {
                    return Some(make);
                }
            }
            return Some(Hit::Body(r.id));
        }
        None
    }
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
    fn hit_body_vs_two_icons_vs_outside() {
        let c = cfg();
        let lay = Layout::compute(&[(7, 260, 180)], &c);
        let r = &lay.thumbs[0];
        // center of the thumb -> body
        let cx = (r.x + r.w / 2) as f64;
        let cy = (r.y + r.h / 2) as f64;
        assert_eq!(lay.hit(cx, cy, &c), Some(Hit::Body(7)));
        // close icon = rightmost slot
        let close_cx = (r.x + r.w - c.pad_icon - c.icon / 2) as f64;
        let icon_cy = (r.y + c.pad_icon + c.icon / 2) as f64;
        assert_eq!(lay.hit(close_cx, icon_cy, &c), Some(Hit::Close(7)));
        // edit icon = next slot to the left of close
        let edit_cx = (r.x + r.w - c.pad_icon - c.icon - c.icon_gap - c.icon / 2) as f64;
        assert_eq!(lay.hit(edit_cx, icon_cy, &c), Some(Hit::Edit(7)));
        // far outside
        assert_eq!(lay.hit(10_000.0, 10_000.0, &c), None);
    }
}
