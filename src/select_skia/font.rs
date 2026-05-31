//! A tiny dependency-free 5x7 bitmap font: digits 0-9, '×', and space.

#![allow(dead_code)] // filled in by later tasks

/// Glyph cell width in bits.
pub const GLYPH_W: usize = 5;
/// Glyph cell height in rows.
pub const GLYPH_H: usize = 7;

/// Total rendered height of a glyph at `scale` (px).
pub fn glyph_h_px(scale: u32) -> u32 {
    GLYPH_H as u32 * scale
}

/// Horizontal advance per glyph at `scale`: cell width + 1px gap.
pub fn advance_px(scale: u32) -> u32 {
    (GLYPH_W as u32 + 1) * scale
}

/// 5x7 bitmap for `c`, one `u8` per row (low `GLYPH_W` bits, MSB = leftmost
/// column). `None` for unsupported chars. Only digits, the multiplication sign
/// `×`, and space are defined — enough for a `W×H` label.
pub fn glyph(c: char) -> Option<[u8; GLYPH_H]> {
    let g = match c {
        ' ' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '×' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        _ => return None,
    };
    Some(g)
}

/// Pixel width of `text` at `scale`, summing the advance of every supported
/// glyph (unsupported chars are skipped).
pub fn label_width(text: &str, scale: u32) -> u32 {
    text.chars().filter(|c| glyph(*c).is_some()).count() as u32 * advance_px(scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_glyphs_resolve_unknown_dont() {
        assert!(glyph('0').is_some());
        assert!(glyph('9').is_some());
        assert!(glyph('×').is_some());
        assert!(glyph(' ').is_some());
        assert!(glyph('a').is_none());
        assert!(glyph('x').is_none()); // ascii x is NOT the separator; '×' is
    }

    #[test]
    fn glyph_is_5x7() {
        let g = glyph('8').unwrap();
        assert_eq!(g.len(), GLYPH_H);
        // every row uses only the low GLYPH_W bits
        for row in g {
            assert_eq!(row & !((1 << GLYPH_W) - 1), 0);
        }
    }

    #[test]
    fn label_width_sums_advances() {
        // "12" = 2 glyphs, advance = (GLYPH_W + 1) * scale each.
        assert_eq!(label_width("12", 2), 2 * (GLYPH_W as u32 + 1) * 2);
        // unknown chars contribute nothing
        assert_eq!(label_width("1a2", 1), label_width("12", 1));
    }
}
