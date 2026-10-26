//! The Boltsnap mark pressed into the sidebar's free corner (settings plan,
//! 4k): a shade darker than the sidebar, a darker rim along its top edge and
//! a lighter one along its bottom, as if light from above fell into it.

use std::cell::RefCell;

use tiny_skia::{FillRule, Pixmap, PixmapPaint, Transform};

use crate::ui::theme;
use crate::ui::{Canvas, Rect, icons, paint};

const SVG: &str = include_str!("../../assets/logo/boltsnap-mark.svg");
/// The mark's view box; Potrace wrote it ten times larger and upside down.
const VIEW: (f32, f32) = (1952.0, 2304.0);
/// How tall the mark is, and how far it reaches past the window's corner.
pub const HEIGHT: f32 = 168.0;
const BLEED: f32 = 28.0;
/// Room around the drawing for the 1 px rims.
const PAD: u32 = 2;

/// The mark, drawn once per scale.
#[derive(Default)]
pub struct Mark {
    drawn: RefCell<Option<(u32, Pixmap)>>,
}

impl Mark {
    /// Where the mark sits in a window `height` tall: its corner cut off by
    /// the window's left and bottom edges.
    pub fn place(height: f32) -> Rect {
        let width = HEIGHT * VIEW.0 / VIEW.1;
        Rect::new(-BLEED, height - HEIGHT + BLEED, width, HEIGHT)
    }

    pub fn draw(&self, canvas: &mut Canvas, at: Rect, alpha: f32) {
        if alpha <= 0.0 {
            return;
        }
        let scale = canvas.scale as u32;
        let mut drawn = self.drawn.borrow_mut();
        if drawn.as_ref().is_none_or(|(s, _)| *s != scale) {
            *drawn = render(canvas.scale).map(|pixmap| (scale, pixmap));
        }
        let Some((_, pixmap)) = drawn.as_ref() else {
            return;
        };
        let x = (at.x * canvas.scale).round() as i32 - PAD as i32;
        let y = (at.y * canvas.scale).round() as i32 - PAD as i32;
        let paint = PixmapPaint {
            opacity: alpha.clamp(0.0, 1.0),
            ..PixmapPaint::default()
        };
        canvas
            .pixmap
            .draw_pixmap(x, y, pixmap.as_ref(), &paint, Transform::identity(), None);
    }
}

/// The mark at `HEIGHT` and `scale`, with its rims, on transparent ground.
fn render(scale: f32) -> Option<Pixmap> {
    // Each path on its own, as SVG fills them: one path for all would cut
    // holes where the dot and the body overlap.
    let paths: Vec<_> = SVG
        .split("<path")
        .skip(1)
        .filter_map(|element| icons::parse(&format!("<path{element}")))
        .collect();
    let unit = HEIGHT * scale / VIEW.1;
    let (w, h) = (
        (VIEW.0 * unit).ceil() as u32 + 2 * PAD,
        (VIEW.1 * unit).ceil() as u32 + 2 * PAD,
    );
    let mut pixmap = Pixmap::new(w, h)?;
    // Potrace's units: x / 10, and y / 10 counted up from the bottom.
    let place = Transform::from_row(
        unit / 10.0,
        0.0,
        0.0,
        -unit / 10.0,
        PAD as f32,
        VIEW.1 * unit + PAD as f32,
    );
    for (color, dy) in [
        (theme::SHADE, -1.0),
        (theme::LIP, 1.0),
        (theme::PRESSED, 0.0),
    ] {
        for path in &paths {
            pixmap.fill_path(
                path,
                &paint(color, 1.0),
                FillRule::EvenOdd,
                place.post_translate(0.0, dy),
                None,
            );
        }
    }
    Some(pixmap)
}

#[cfg(test)]
mod tests {
    #[test]
    fn both_paths_of_the_mark_parse() {
        let parsed: Vec<_> = super::SVG
            .split("<path")
            .skip(1)
            .map(|element| crate::ui::icons::parse(&format!("<path{element}")).map(|p| p.bounds()))
            .collect();
        println!("{parsed:?}");
        assert_eq!(parsed.len(), 2);
        assert!(parsed.iter().all(Option::is_some), "{parsed:?}");
    }
}
