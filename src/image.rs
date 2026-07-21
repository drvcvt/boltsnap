/// Axis-aligned rectangle in surface pixels.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    /// Build a normalized rectangle from two opposite corners.
    pub fn from_corners(first: (f64, f64), second: (f64, f64)) -> Self {
        Self {
            x: first.0.min(second.0),
            y: first.1.min(second.1),
            w: (first.0 - second.0).abs(),
            h: (first.1 - second.1).abs(),
        }
    }

    pub fn right(&self) -> f64 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.h
    }

    pub fn contains(&self, point: (f64, f64)) -> bool {
        point.0 >= self.x
            && point.0 <= self.right()
            && point.1 >= self.y
            && point.1 <= self.bottom()
    }
}
