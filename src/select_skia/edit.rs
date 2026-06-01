//! Pure geometry + interaction math for the editable selector. No Wayland here.

#![allow(dead_code)] // wired into the driver in Task 8+; removed there.

/// Axis-aligned rectangle in surface pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    /// Build a normalized rect (non-negative w/h) from two opposite corners.
    pub fn from_corners(a: (f64, f64), b: (f64, f64)) -> Rect {
        Rect {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (a.0 - b.0).abs(),
            h: (a.1 - b.1).abs(),
        }
    }
    pub fn right(&self) -> f64 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }
    pub fn contains(&self, p: (f64, f64)) -> bool {
        p.0 >= self.x && p.0 <= self.right() && p.1 >= self.y && p.1 <= self.bottom()
    }
}

/// The eight resize handles: four corners and four edge midpoints.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Handle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl Handle {
    /// All eight handles with their centers for a given rect.
    pub fn centers(rect: Rect) -> [(Handle, (f64, f64)); 8] {
        let (l, t, r, b) = (rect.x, rect.y, rect.right(), rect.bottom());
        let (cx, cy) = (l + rect.w / 2.0, t + rect.h / 2.0);
        [
            (Handle::TopLeft, (l, t)),
            (Handle::Top, (cx, t)),
            (Handle::TopRight, (r, t)),
            (Handle::Right, (r, cy)),
            (Handle::BottomRight, (r, b)),
            (Handle::Bottom, (cx, b)),
            (Handle::BottomLeft, (l, b)),
            (Handle::Left, (l, cy)),
        ]
    }
}

/// What the cursor is over, for the editable state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Region {
    Handle(Handle),
    Inside,
    Outside,
}

/// Classify the cursor against the selection. Handles (within `handle_r` of a
/// handle center) win over Inside; corners are listed before edges so a corner
/// wins when both are within range.
pub fn hit_region(rect: Rect, cursor: (f64, f64), handle_r: f64) -> Region {
    for (h, (hx, hy)) in Handle::centers(rect) {
        if (cursor.0 - hx).abs() <= handle_r && (cursor.1 - hy).abs() <= handle_r {
            return Region::Handle(h);
        }
    }
    if rect.contains(cursor) {
        Region::Inside
    } else {
        Region::Outside
    }
}

/// New rect when dragging `handle` to `cursor`. The opposite edge(s) stay fixed.
/// Cursor is clamped to the surface; the result is normalized, at least `min` in
/// each dimension, and fully inside the surface.
pub fn resize_rect(
    rect: Rect,
    handle: Handle,
    cursor: (f64, f64),
    min: f64,
    surf_w: f64,
    surf_h: f64,
) -> Rect {
    let cx = cursor.0.clamp(0.0, surf_w);
    let cy = cursor.1.clamp(0.0, surf_h);
    let (mut l, mut t, mut r, mut b) = (rect.x, rect.y, rect.right(), rect.bottom());
    match handle {
        Handle::TopLeft => {
            l = cx;
            t = cy;
        }
        Handle::Top => t = cy,
        Handle::TopRight => {
            r = cx;
            t = cy;
        }
        Handle::Right => r = cx,
        Handle::BottomRight => {
            r = cx;
            b = cy;
        }
        Handle::Bottom => b = cy,
        Handle::BottomLeft => {
            l = cx;
            b = cy;
        }
        Handle::Left => l = cx,
    }
    let w = (l - r).abs().max(min).min(surf_w);
    let h = (t - b).abs().max(min).min(surf_h);
    let x = l.min(r).clamp(0.0, (surf_w - w).max(0.0));
    let y = t.min(b).clamp(0.0, (surf_h - h).max(0.0));
    Rect { x, y, w, h }
}

/// Translate `rect` by (dx, dy), clamped so it stays fully within the surface.
pub fn move_rect(rect: Rect, dx: f64, dy: f64, surf_w: f64, surf_h: f64) -> Rect {
    let x = (rect.x + dx).clamp(0.0, (surf_w - rect.w).max(0.0));
    let y = (rect.y + dy).clamp(0.0, (surf_h - rect.h).max(0.0));
    Rect {
        x,
        y,
        w: rect.w,
        h: rect.h,
    }
}

/// Pixel rect (x, y, w, h) of the dimension badge pill. `text_w`/`text_h` are the
/// rendered label size; `pad` is interior padding, `gap` the offset from the
/// selection edge. Sits just above the selection's top-left, flipping just inside
/// the top when there is no room above, and shifting left to stay on-screen.
pub fn badge_rect(
    sel: Rect,
    text_w: f64,
    text_h: f64,
    pad: f64,
    gap: f64,
    surf_w: f64,
    surf_h: f64,
) -> (f64, f64, f64, f64) {
    let w = text_w + 2.0 * pad;
    let h = text_h + 2.0 * pad;
    let mut x = sel.x;
    let mut y = sel.y - gap - h;
    if y < 0.0 {
        y = (sel.y + gap).min((surf_h - h).max(0.0));
    }
    if x + w > surf_w {
        x = surf_w - w;
    }
    if x < 0.0 {
        x = 0.0;
    }
    (x, y, w, h)
}

/// Top-left + size of the source window (in image pixels) to sample for the
/// magnifier, an `sample`×`sample` box centered on `cursor`, clamped to the image.
pub fn magnifier_source(
    cursor: (f64, f64),
    sample: u32,
    img_w: u32,
    img_h: u32,
) -> (u32, u32, u32, u32) {
    let s = sample.min(img_w).min(img_h);
    let half = s as f64 / 2.0;
    let max_x = (img_w - s) as i64;
    let max_y = (img_h - s) as i64;
    let x = ((cursor.0 - half).round() as i64).clamp(0, max_x.max(0)) as u32;
    let y = ((cursor.1 - half).round() as i64).clamp(0, max_y.max(0)) as u32;
    (x, y, s, s)
}

/// Top-left of the loupe square (`loupe` px) on the surface: offset up-and-right
/// of `cursor` by `offset`, flipping toward screen-center near each edge so the
/// loupe never clips.
pub fn magnifier_placement(
    cursor: (f64, f64),
    loupe: f64,
    offset: f64,
    surf_w: f64,
    surf_h: f64,
) -> (f64, f64) {
    let mut x = cursor.0 + offset;
    let mut y = cursor.1 - offset - loupe;
    if x + loupe > surf_w {
        x = cursor.0 - offset - loupe;
    }
    if y < 0.0 {
        y = cursor.1 + offset;
    }
    x = x.clamp(0.0, (surf_w - loupe).max(0.0));
    y = y.clamp(0.0, (surf_h - loupe).max(0.0));
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_region_classifies_corner_edge_inside_outside() {
        let r = Rect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        }; // corners (100,100)-(300,200)
        assert_eq!(
            hit_region(r, (100.0, 100.0), 10.0),
            Region::Handle(Handle::TopLeft)
        );
        assert_eq!(
            hit_region(r, (300.0, 200.0), 10.0),
            Region::Handle(Handle::BottomRight)
        );
        assert_eq!(
            hit_region(r, (200.0, 100.0), 10.0),
            Region::Handle(Handle::Top)
        );
        assert_eq!(hit_region(r, (200.0, 150.0), 10.0), Region::Inside);
        assert_eq!(hit_region(r, (10.0, 10.0), 10.0), Region::Outside);
    }

    #[test]
    fn resize_rect_moves_dragged_edges_and_enforces_min() {
        let r = Rect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        // Drag BottomRight to (260,260): left/top fixed, right/bottom follow.
        let g = resize_rect(r, Handle::BottomRight, (260.0, 260.0), 10.0, 1920.0, 1080.0);
        assert_eq!(
            g,
            Rect {
                x: 100.0,
                y: 100.0,
                w: 160.0,
                h: 160.0
            }
        );
        // Drag Right handle: only width changes.
        let g2 = resize_rect(r, Handle::Right, (150.0, 999.0), 10.0, 1920.0, 1080.0);
        assert_eq!(g2.x, 100.0);
        assert_eq!(g2.w, 50.0);
        assert_eq!(g2.h, 100.0);
        // Dragging Right back past Left enforces the minimum width.
        let g3 = resize_rect(r, Handle::Right, (100.0, 150.0), 10.0, 1920.0, 1080.0);
        assert!(g3.w >= 10.0, "min width enforced, got {}", g3.w);
    }

    #[test]
    fn move_rect_translates_and_clamps_to_surface() {
        let r = Rect {
            x: 100.0,
            y: 100.0,
            w: 200.0,
            h: 100.0,
        };
        assert_eq!(
            move_rect(r, 50.0, 25.0, 1920.0, 1080.0),
            Rect {
                x: 150.0,
                y: 125.0,
                w: 200.0,
                h: 100.0
            }
        );
        // Clamp at the right/bottom: can't push the rect off-surface.
        let c = move_rect(r, 1e9, 1e9, 1920.0, 1080.0);
        assert_eq!(c.right(), 1920.0);
        assert_eq!(c.bottom(), 1080.0);
        // Clamp at the top/left.
        let c2 = move_rect(r, -1e9, -1e9, 1920.0, 1080.0);
        assert_eq!((c2.x, c2.y), (0.0, 0.0));
    }

    #[test]
    fn badge_rect_sits_above_left_then_flips_at_edges() {
        // Roomy: badge sits just above the selection's top-left.
        let sel = Rect {
            x: 400.0,
            y: 400.0,
            w: 200.0,
            h: 100.0,
        };
        let (x, y, w, h) = badge_rect(sel, 60.0, 14.0, 6.0, 6.0, 1920.0, 1080.0);
        assert_eq!(w, 72.0); // 60 + 2*6
        assert_eq!(h, 26.0); // 14 + 2*6
        assert_eq!(x, 400.0);
        assert_eq!(y, 400.0 - 6.0 - 26.0); // gap + height above
        // Selection hugging the top: badge flips just inside the top.
        let top = Rect {
            x: 10.0,
            y: 0.0,
            w: 200.0,
            h: 100.0,
        };
        let (_, y2, _, _) = badge_rect(top, 60.0, 14.0, 6.0, 6.0, 1920.0, 1080.0);
        assert!(y2 >= 0.0, "badge stays on-screen, got {y2}");
    }

    #[test]
    fn magnifier_source_centers_and_clamps() {
        // Centered window deep inside the image.
        assert_eq!(
            magnifier_source((500.0, 500.0), 30, 1920, 1080),
            (485, 485, 30, 30)
        );
        // Clamped at the top-left corner.
        assert_eq!(magnifier_source((2.0, 2.0), 30, 1920, 1080), (0, 0, 30, 30));
        // Clamped at the bottom-right corner.
        assert_eq!(
            magnifier_source((1919.0, 1079.0), 30, 1920, 1080),
            (1890, 1050, 30, 30)
        );
    }

    #[test]
    fn magnifier_placement_offsets_then_flips_at_edges() {
        // Default: up-and-right of the cursor.
        assert_eq!(
            magnifier_placement((500.0, 500.0), 120.0, 24.0, 1920.0, 1080.0),
            (524.0, 356.0)
        );
        // Near the right edge: flips to the left of the cursor.
        let (x, _) = magnifier_placement((1900.0, 500.0), 120.0, 24.0, 1920.0, 1080.0);
        assert!(x + 120.0 <= 1920.0, "loupe stays on-screen, got x={x}");
        // Near the top edge: flips below the cursor.
        let (_, y) = magnifier_placement((500.0, 10.0), 120.0, 24.0, 1920.0, 1080.0);
        assert!(y >= 0.0, "loupe stays on-screen, got y={y}");
    }
}
