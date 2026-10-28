//! The play badge on a video card, and the arc that turns in it while the
//! clip is still being written (replay plan §13 C1).

use super::layout::ThumbRect;
use super::paint::{BTN_BG, GLYPH_RGB, blend_px, fill_circle};

/// The arc's share of a turn; the rest is the gap that shows it turning.
const ARC_SPAN: f32 = 0.75;
/// Half the arc's stroke, as a share of the badge radius.
const ARC_HALF_STROKE: f32 = 1.0 / 18.0;

/// Draw a ▶ play badge centered on a card, to mark Video cards as distinct from
/// screenshots. The badge is always visible (not hover-gated). `ready` 0 to 1:
/// at 0 the clip is still being written and an arc turns in the badge (`spin`
/// turns, see [`draw_saving_arc`]); towards 1 the arc fades out and the ▶ in.
pub(super) fn draw_play_badge(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    r: &ThumbRect,
    scale: f32,
    ready: f32,
    spin: f32,
) {
    let min_dim = r.w.min(r.h) as f32;
    let radius = (min_dim * 0.14).max(11.0 * scale); // a touch larger; floor for tiny cards
    let cx = r.x as f32 + r.w as f32 / 2.0;
    let cy = r.y as f32 + r.h as f32 / 2.0;

    // Soft dark halo (slightly larger, faint) gives the badge a crisp edge against
    // busy thumbnails; then the translucent disc on top.
    fill_circle(
        canvas,
        cw,
        ch,
        cx,
        cy,
        radius + 1.5 * scale,
        (0, 0, 0),
        0.28,
    );
    fill_circle(canvas, cw, ch, cx, cy, radius, BTN_BG, 0.6);
    let ready = ready.clamp(0.0, 1.0);
    if ready < 1.0 {
        draw_saving_arc(canvas, cw, ch, (cx, cy), radius, spin, 1.0 - ready);
    }
    if ready <= 0.0 {
        return;
    }

    // Right-pointing triangle (▶), optically centred (a play glyph looks centred
    // when its centroid — 1/3 from the flat left edge — sits at the disc centre).
    let tri_h = radius * 1.04; // triangle height
    let tri_w = radius * 0.96; // flat left edge → apex
    let left_x = cx - tri_w / 3.0;
    let apex_x = left_x + tri_w;
    let top_y = cy - tri_h / 2.0;
    let bot_y = cy + tri_h / 2.0;
    let half_h = tri_h / 2.0;

    // Inside test: within the y-band, right of the flat left edge, and left of the
    // boundary that shrinks linearly from the apex (at mid-height) to `left_x` (at
    // the tips). Sampled 4×4 per pixel for smooth anti-aliased edges on all sides.
    let inside = |x: f32, y: f32| -> bool {
        if y < top_y || y > bot_y || x < left_x {
            return false;
        }
        let dy = (y - cy).abs();
        let x_right = apex_x - (apex_x - left_x) * (dy / half_h);
        x <= x_right
    };

    let x0 = left_x.floor() as i32 - 1;
    let x1 = apex_x.ceil() as i32 + 1;
    let y0 = top_y.floor() as i32 - 1;
    let y1 = bot_y.ceil() as i32 + 1;
    const SS: i32 = 4; // 4×4 supersampling
    let inv = 1.0 / (SS * SS) as f32;
    for py in y0..=y1 {
        for px in x0..=x1 {
            let mut hits = 0;
            for sy in 0..SS {
                for sx in 0..SS {
                    let fx = px as f32 + (sx as f32 + 0.5) / SS as f32;
                    let fy = py as f32 + (sy as f32 + 0.5) / SS as f32;
                    if inside(fx, fy) {
                        hits += 1;
                    }
                }
            }
            if hits > 0 {
                let cov = hits as f32 * inv * ready;
                blend_px(
                    canvas,
                    cw,
                    ch,
                    px,
                    py,
                    GLYPH_RGB.0,
                    GLYPH_RGB.1,
                    GLYPH_RGB.2,
                    cov,
                );
            }
        }
    }
}

/// A 270° arc with round ends in a badge of `radius` at `centre`, turned by
/// `spin` turns (clockwise), at `alpha`. Its ring spans about the ▶'s height
/// (outer edge at half the radius), its stroke a ninth of the radius: as
/// heavy as the ▶ reads next to it.
fn draw_saving_arc(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    (cx, cy): (f32, f32),
    radius: f32,
    spin: f32,
    alpha: f32,
) {
    use std::f32::consts::TAU;
    let half = radius * ARC_HALF_STROKE;
    let ring = radius * 0.5 - half;
    let start = spin.rem_euclid(1.0) * TAU;
    let ends = [start, start + ARC_SPAN * TAU].map(|a| (cx + ring * a.cos(), cy + ring * a.sin()));
    let reach = (ring + half + 1.0).ceil() as i32;
    for py in cy as i32 - reach..=cy as i32 + reach {
        for px in cx as i32 - reach..=cx as i32 + reach {
            let (x, y) = (px as f32 + 0.5, py as f32 + 0.5);
            let (dx, dy) = (x - cx, y - cy);
            // Screen y points down, so a growing angle turns clockwise.
            let along = (dy.atan2(dx) - start).rem_euclid(TAU);
            let distance = if along <= ARC_SPAN * TAU {
                ((dx * dx + dy * dy).sqrt() - ring).abs()
            } else {
                ends.iter()
                    .map(|(ex, ey)| ((x - ex).powi(2) + (y - ey).powi(2)).sqrt())
                    .fold(f32::MAX, f32::min)
            };
            let coverage = (half + 0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                blend_px(
                    canvas,
                    cw,
                    ch,
                    px,
                    py,
                    GLYPH_RGB.0,
                    GLYPH_RGB.1,
                    GLYPH_RGB.2,
                    coverage * alpha,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use image::RgbaImage;

    use super::super::layout::{Layout, LayoutConfig};
    use super::super::paint::{Frame, GLYPH_RGB, draw_shelf};
    use super::super::{CardKind, CardLook};

    /// A settled video card, hovered or not, with `saving` and `spin`.
    fn video(
        hovered: bool,
        saving: &[(u64, f32)],
        spin: f32,
        save_button: bool,
    ) -> (Vec<u8>, Layout) {
        let thumb = RgbaImage::from_pixel(190, 132, image::Rgba([90, 120, 160, 255]));
        let cfg = LayoutConfig::default();
        let layout = Layout::compute(&[(1, thumb.width(), thumb.height())], &cfg);
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        let frame = Frame {
            layout: &layout,
            cfg: &cfg,
            hovered: if hovered { &[(1, 1.0)] } else { &[] },
            anims: &[],
            save_flash: None,
            save_button,
            saving,
            spin,
            look: CardLook::default(),
            scale: 1.0,
        };
        draw_shelf(&mut canvas, layout.width, layout.height, &frame, |_| {
            Some((&thumb, CardKind::Video))
        });
        (canvas, layout)
    }

    fn brightest(canvas: &[u8]) -> u8 {
        canvas.chunks(4).map(|px| px[1]).max().unwrap()
    }

    #[test]
    fn a_ready_card_is_the_video_card_and_a_saving_one_has_no_play_glyph() {
        let (plain, layout) = video(false, &[], 0.0, true);
        assert_eq!(video(false, &[(1, 1.0)], 0.3, true).0, plain);
        let (saving, _) = video(false, &[(1, 0.0)], 0.0, true);
        assert_ne!(saving, plain);
        // The ▶ covers the badge centre; the arc's ring stays off it.
        let r = &layout.thumbs[0];
        let at = |canvas: &[u8], x: u32, y: u32| {
            let i = ((y * layout.width + x) * 4) as usize;
            canvas[i + 1]
        };
        let (cx, cy) = (r.x + r.w / 2, r.y + r.h / 2);
        assert!(
            at(&plain, cx, cy) > 200,
            "the play glyph sits at the centre"
        );
        assert!(
            at(&saving, cx, cy) < 120,
            "no glyph at the centre while saving"
        );
        // The arc is drawn in the glyph's colour, so it has the ▶'s contrast.
        assert!(brightest(&saving) >= GLYPH_RGB.1 - 8);
    }

    #[test]
    fn the_arc_turns_and_a_whole_turn_looks_the_same() {
        let at = |spin| video(false, &[(1, 0.0)], spin, true).0;
        assert_ne!(at(0.0), at(0.25));
        assert_eq!(at(0.0), at(1.0));
        assert_eq!(at(0.25), at(-0.75));
    }

    #[test]
    fn a_saving_card_has_no_save_button_until_it_is_ready() {
        let saving = [(1, 0.5)];
        assert_eq!(
            video(true, &saving, 0.0, true).0,
            video(true, &saving, 0.0, false).0
        );
        assert_ne!(
            video(true, &[(1, 1.0)], 0.0, true).0,
            video(true, &[(1, 1.0)], 0.0, false).0
        );
    }

    /// `SAVING_OUT=file cargo test --lib saving_shot -- --ignored`: a video
    /// card while saving (arc at 0, 1/8 and 1/4 turn), halfway to ready, and
    /// ready, over the dark desktop (0x12), three times enlarged.
    #[test]
    #[ignore]
    fn saving_shot() {
        let out = std::env::var("SAVING_OUT").expect("SAVING_OUT");
        let states = [
            (0.0, 0.0),
            (0.0, 0.125),
            (0.0, 0.25),
            (0.5, 0.0),
            (1.0, 0.0),
        ];
        let shots: Vec<_> = states
            .iter()
            .map(|&(ready, spin)| video(false, &[(1, ready)], spin, true))
            .collect();
        let (w, h) = (shots[0].1.width, shots[0].1.height);
        let mut strip = RgbaImage::new(w * states.len() as u32, h);
        for (n, (canvas, _)) in shots.iter().enumerate() {
            for (i, bgra) in canvas.as_chunks::<4>().0.iter().enumerate() {
                // Premultiplied BGRA over 0x12.
                let under = |c: u8| (u32::from(c) + 18 * (255 - u32::from(bgra[3])) / 255) as u8;
                let (x, y) = (i as u32 % w, i as u32 / w);
                strip.put_pixel(
                    n as u32 * w + x,
                    y,
                    image::Rgba([under(bgra[2]), under(bgra[1]), under(bgra[0]), 255]),
                );
            }
        }
        let big = image::imageops::resize(
            &strip,
            strip.width() * 3,
            strip.height() * 3,
            image::imageops::FilterType::Nearest,
        );
        big.save(out).unwrap();
    }
}
