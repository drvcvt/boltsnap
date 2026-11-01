//! The soft drop shadow behind each settled shelf card.

use super::layout::ThumbRect;
use super::paint::{CARD_RADIUS, blend_px, rr_coverage};

const DX: f32 = 0.0;
/// Downward offset: reads as "lifted".
const DY: f32 = 3.0;
/// A wider falloff is more diffuse.
const BLUR: f32 = 16.0;
/// Lighter, less heavy.
const ALPHA: f32 = 0.24;
/// Masks kept per painting thread before the cache starts over.
const MOST_MASKS: usize = 32;

/// One pixel of a shadow: its alpha before dithering, 0 where it shows
/// nothing, and whether it lies in the falloff, which is dithered.
#[derive(Clone, Copy)]
struct Cell {
    shade: f32,
    dither: bool,
}

/// A shadow worked out once per card size and scale, for a card at (0, 0):
/// rows of cells from the box's corner (`x0`, `y0`).
struct Mask {
    x0: i32,
    y0: i32,
    width: usize,
    cells: Vec<Cell>,
}

thread_local! {
    static MASKS: std::cell::RefCell<std::collections::HashMap<(u32, u32, u32, bool), std::rc::Rc<Mask>>> =
        std::cell::RefCell::default();
}

/// Soft drop shadow behind a shelf card, for a subtle "lift" (drawn for every
/// settled card, independent of hover/opacity). Rounded-box SDF feathered over
/// `BLUR` px, slightly offset downward; drawn before the cards so each card sits
/// on top. `covered`: the card is drawn at `r` at its full size, and the
/// pixels it paints over are skipped, since none of the shadow shows there.
/// Only the dithering and the blend are per frame; the rest is per size.
pub(super) fn draw_card_shadow(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    r: &ThumbRect,
    scale: f32,
    covered: bool,
) {
    let mask = MASKS.with(|masks| {
        let mut masks = masks.borrow_mut();
        if masks.len() >= MOST_MASKS {
            masks.clear();
        }
        masks
            .entry((r.w, r.h, scale.to_bits(), covered))
            .or_insert_with(|| std::rc::Rc::new(mask(r.w, r.h, scale, covered)))
            .clone()
    });
    for (row, cells) in mask.cells.chunks(mask.width).enumerate() {
        let py = r.y as i32 + mask.y0 + row as i32;
        if py < 0 || py >= ch as i32 {
            continue;
        }
        for (column, cell) in cells.iter().enumerate() {
            if cell.shade == 0.0 {
                continue;
            }
            let px = r.x as i32 + mask.x0 + column as i32;
            let alpha = if cell.dither {
                dithered(cell.shade, px, py)
            } else {
                cell.shade
            };
            blend_px(canvas, cw, ch, px, py, 0, 0, 0, alpha);
        }
    }
}

/// The shadow of a `w` × `h` card at (0, 0).
fn mask(w: u32, h: u32, scale: f32, covered: bool) -> Mask {
    let blur = BLUR * scale;
    let (sx, sy) = (DX * scale, DY * scale);
    let (w, h) = (w as f32, h as f32);
    let rad = CARD_RADIUS * scale;
    // The card's own corners, as `blit_thumb_card_anim` rounds them.
    let card_rad = rad.min(w / 2.0).min(h / 2.0);
    let (cxw, cyh) = (w / 2.0, h / 2.0);
    let x0 = (sx - blur - 1.0).floor() as i32;
    let x1 = (sx + w + blur + 1.0).ceil() as i32;
    let y0 = (sy - blur - 1.0).floor() as i32;
    let y1 = (sy + h + blur + 1.0).ceil() as i32;
    let mut cells = Vec::with_capacity(((x1 - x0 + 1) * (y1 - y0 + 1)) as usize);
    for py in y0..=y1 {
        for px in x0..=x1 {
            let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
            let under_card = covered
                && fx > 0.0
                && fy > 0.0
                && fx < w
                && fy < h
                && rr_coverage(fx, fy, w, h, card_rad) > 0.0;
            let (lx, ly) = (fx - sx, fy - sy);
            // Signed distance to the rounded card rect (<=0 inside).
            let dx = (lx - cxw).abs() - (cxw - rad);
            let dy = (ly - cyh).abs() - (cyh - rad);
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
            let inside = dx.max(dy).min(0.0);
            let sdf = outside + inside - rad;
            let ramp = (1.0 - sdf / blur).clamp(0.0, 1.0);
            let shows = ramp > 0.0 && !under_card;
            // Only the falloff is a gradient; under the card the alpha is
            // flat and needs no grain.
            cells.push(Cell {
                shade: if shows { ALPHA * smoothstep(ramp) } else { 0.0 },
                dither: ramp < 1.0,
            });
        }
    }
    Mask {
        x0,
        y0,
        width: (x1 - x0 + 1) as usize,
        cells,
    }
}

/// 0 to 1 with no corner at either end: a linear ramp's ends read as edges
/// (Mach bands) in a soft shadow.
fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// How far the shadow's alpha is spread around its value, in 8-bit steps:
/// over a dark desktop (≈ 0x12) a faint black gradient has only a few
/// output levels, 18 → 14 across the blur, each a band 3–4 px wide. ±3.5
/// steps is ±0.25 of such a level there: enough to break up every band's
/// edge, which is what reads as a stripe, while over white it is a grain of
/// ±3.5 levels, about what Hyprland's own blur adds so it does not band.
/// Twice that would also fill the bands but shows as grain on light ground
/// (compared 27.10.).
const SHADOW_SPREAD: f32 = 7.0;

/// `alpha` with noise that changes from pixel to pixel ("interleaved
/// gradient noise", Jimenez 2014), then rounded to 8 bits: the same alpha on
/// average, but the few output levels of a faint gradient over a dark
/// desktop become a fine grain instead of bands. Near zero the spread
/// narrows so nothing is cut at 0 and the average holds.
fn dithered(alpha: f32, x: i32, y: i32) -> f32 {
    let noise = (52.982_918 * (0.067_110_56 * x as f32 + 0.005_837_15 * y as f32).fract()).fract();
    let steps = alpha * 255.0;
    let spread = SHADOW_SPREAD.min(2.0 * steps);
    ((steps + (noise - 0.5) * spread).round() / 255.0).clamp(0.0, 1.0)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The shadow as it was drawn before the masks, pixel by pixel.
    pub(in crate::shelf_card) fn reference_shadow(
        canvas: &mut [u8],
        cw: u32,
        ch: u32,
        r: &ThumbRect,
        scale: f32,
    ) {
        const DX: f32 = 0.0;
        const DY: f32 = 3.0; // downward offset reads as "lifted"
        const BLUR: f32 = 16.0; // wider falloff = more diffuse
        const ALPHA: f32 = 0.24; // lighter, less heavy
        let blur = BLUR * scale;
        let (sx, sy) = (r.x as f32 + DX * scale, r.y as f32 + DY * scale);
        let (w, h) = (r.w as f32, r.h as f32);
        let rad = CARD_RADIUS * scale;
        let (cxw, cyh) = (w / 2.0, h / 2.0);
        let x0 = (sx - blur - 1.0).floor() as i32;
        let x1 = (sx + w + blur + 1.0).ceil() as i32;
        let y0 = (sy - blur - 1.0).floor() as i32;
        let y1 = (sy + h + blur + 1.0).ceil() as i32;
        for py in y0..=y1 {
            for px in x0..=x1 {
                let lx = (px as f32 + 0.5) - sx;
                let ly = (py as f32 + 0.5) - sy;
                // Signed distance to the rounded card rect (<=0 inside).
                let dx = (lx - cxw).abs() - (cxw - rad);
                let dy = (ly - cyh).abs() - (cyh - rad);
                let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
                let inside = dx.max(dy).min(0.0);
                let sdf = outside + inside - rad;
                let ramp = (1.0 - sdf / blur).clamp(0.0, 1.0);
                if ramp > 0.0 {
                    // Only the falloff is a gradient; under the card the alpha is
                    // flat and needs no grain.
                    let shade = ALPHA * smoothstep(ramp);
                    let alpha = if ramp < 1.0 {
                        dithered(shade, px, py)
                    } else {
                        shade
                    };
                    blend_px(canvas, cw, ch, px, py, 0, 0, 0, alpha);
                }
            }
        }
    }

    #[test]
    fn the_mask_paints_what_the_shadow_painted_before() {
        for (scale, rect) in [
            (1.0, (22, 22, 190, 132)),
            (1.25, (30, 41, 237, 165)),
            (1.0, (0, 0, 40, 25)),
        ] {
            let r = ThumbRect {
                id: 1,
                x: rect.0,
                y: rect.1,
                w: rect.2,
                h: rect.3,
            };
            let (cw, ch) = (rect.0 + rect.2 + 60, rect.1 + rect.3 + 60);
            let background: Vec<u8> = (0..cw * ch * 4).map(|i| (i * 37 % 251) as u8).collect();
            let mut before = background.clone();
            reference_shadow(&mut before, cw, ch, &r, scale);
            let mut now = background.clone();
            draw_card_shadow(&mut now, cw, ch, &r, scale, false);
            assert!(before == now, "scale {scale}, {rect:?}");
            // Drawn twice from the cache, the same again.
            let mut again = background.clone();
            draw_card_shadow(&mut again, cw, ch, &r, scale, false);
            assert!(again == now);
        }
    }

    #[test]
    fn the_shadow_is_dithered_its_average_kept_and_free_of_bands() {
        // Over a dark desktop (18) the composited shadow has a few levels;
        // beside a straight card edge its alpha depends on x only, so each
        // column is one level, a stripe. Dithered, every column between two
        // levels mixes both, and on average it is the smooth shadow.
        let bg = 18.0_f32;
        let rows = 0..64;
        let mut striped = 0;
        for x in 0..64 {
            let want = 0.24 * smoothstep(x as f32 / 64.0);
            let out = |alpha: f32| (bg * (1.0 - alpha)).round();
            let column: Vec<f32> = rows.clone().map(|y| out(dithered(want, x, y))).collect();
            let mean = rows.clone().map(|y| dithered(want, x, y)).sum::<f32>() / 64.0;
            assert!(
                (mean - want).abs() < 0.6 / 255.0,
                "average at {x}: {mean} vs {want}"
            );
            // Near the edge between two output levels: there a stripe shows.
            let level = bg * (1.0 - want);
            let between = (level - level.floor() - 0.5).abs() < 0.2;
            if between && column.iter().all(|v| *v == column[0]) {
                striped += 1;
            }
        }
        assert_eq!(
            striped, 0,
            "no band edge is a straight line over a dark desktop"
        );
        assert_eq!(dithered(0.0, 3, 4), 0.0, "nothing where there is no shadow");
        assert_eq!(
            (smoothstep(0.0), smoothstep(0.5), smoothstep(1.0)),
            (0.0, 0.5, 1.0)
        );
    }
}
