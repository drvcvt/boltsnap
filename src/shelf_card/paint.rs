//! Painting the cards into premultiplied BGRA, the overlay's `wl_shm`
//! format: rounded thumbnails, their shadow, the play badge and the hover
//! buttons.

use image::{RgbaImage, imageops};

use super::layout::{Layout, LayoutConfig, ThumbRect, close_cell, save_cell};
use super::{CardKind, CardLook};

/// Thumbnail card corner radius, in pixels.
const CARD_RADIUS: f32 = 10.0;

/// Hover-button styling: a small translucent dark circle with an anti-aliased
/// glyph. Minimal and unobtrusive over the screenshot.
const BTN_BG: (u8, u8, u8) = (18, 18, 24);
const BTN_BG_A: f32 = 0.52;
pub const GLYPH_RGB: (u8, u8, u8) = (244, 244, 248);
pub const GLYPH_CLOSE_RGB: (u8, u8, u8) = (255, 124, 124);
const GLYPH_OK_RGB: (u8, u8, u8) = (120, 230, 150);
const GLYPH_HALF_W: f32 = 0.8; // half stroke width -> ~1.6px strokes

/// Set the whole canvas to transparent (0,0,0,0).
pub fn clear(canvas: &mut [u8]) {
    for b in canvas.iter_mut() {
        *b = 0;
    }
}

/// Composite an opaque RGBA thumbnail as a rounded card (rounded corners,
/// transparent outside the radius, no border) in premultiplied BGRA. The
/// full-size slot is `img.dimensions()` at (dx, dy); the card is scaled by
/// `scale` (0..1) and centred in that slot, and `opacity` (0..1) is the card's
/// final alpha — the caller picks the base translucency (`CardLook::opacity`,
/// or `1.0` for the hovered card) and folds in any appear/dismiss fade;
/// `radius` rounds its corners. `scale == 1.0 && opacity == 1.0` is a
/// fully-opaque settled card.
#[allow(clippy::too_many_arguments)]
pub fn blit_thumb_card_anim(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    img: &RgbaImage,
    dx: u32,
    dy: u32,
    scale: f32,
    opacity: f32,
    radius: f32,
) {
    let (iw, ih) = img.dimensions();
    let scale = scale.clamp(0.0, 1.0);
    let sw = ((iw as f32 * scale).round() as u32).max(1);
    let sh = ((ih as f32 * scale).round() as u32).max(1);
    // Resize only when actually scaling; the card is small so this is cheap.
    let scaled;
    let src: &RgbaImage = if sw == iw && sh == ih {
        img
    } else {
        scaled = imageops::resize(img, sw, sh, imageops::FilterType::Triangle);
        &scaled
    };
    // Centre the scaled card in the original iw×ih slot.
    let ox = dx + (iw - sw) / 2;
    let oy = dy + (ih - sh) / 2;
    let w = sw as f32;
    let h = sh as f32;
    let r = radius.min(w / 2.0).min(h / 2.0);
    let global = opacity.clamp(0.0, 1.0);
    for sy in 0..sh {
        let py = oy + sy;
        if py >= ch {
            break;
        }
        for sx in 0..sw {
            let px = ox + sx;
            if px >= cw {
                break;
            }
            let cov = rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w, h, r);
            if cov <= 0.0 {
                continue; // transparent corner -> leave the canvas untouched
            }
            // Premultiplied BGRA: scaling RGB and alpha together by the same
            // factor keeps the pixel valid while making the card translucent.
            let a = cov * global;
            let p = src.get_pixel(sx, sy).0;
            let idx = ((py * cw + px) * 4) as usize;
            canvas[idx] = (p[2] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 1] = (p[1] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 2] = (p[0] as f32 * a).round().clamp(0.0, 255.0) as u8;
            canvas[idx + 3] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Coverage of a rounded rectangle (size w×h, corner radius r) at point (px,py):
/// ~1.0 well inside, ~0.0 well outside, anti-aliased across ~1px at the edge.
/// Uses the standard rounded-box signed distance field.
pub fn rr_coverage(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let cx = w / 2.0;
    let cy = h / 2.0;
    let dx = (px - cx).abs() - (cx - r);
    let dy = (py - cy).abs() - (cy - r);
    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
    let inside = dx.max(dy).min(0.0);
    let sdf = outside + inside - r;
    (0.5 - sdf).clamp(0.0, 1.0)
}

/// Source-over composite a straight-alpha colour onto the premultiplied BGRA
/// canvas at integer (x,y). `a` is fractional coverage in 0..=1.
#[allow(clippy::too_many_arguments)]
pub fn blend_px(canvas: &mut [u8], cw: u32, ch: u32, x: i32, y: i32, r: u8, g: u8, b: u8, a: f32) {
    if x < 0 || y < 0 || x as u32 >= cw || y as u32 >= ch {
        return;
    }
    let a = a.clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let idx = ((y as u32 * cw + x as u32) * 4) as usize;
    let inv = 1.0 - a;
    let nb = b as f32 * a + canvas[idx] as f32 * inv;
    let ng = g as f32 * a + canvas[idx + 1] as f32 * inv;
    let nr = r as f32 * a + canvas[idx + 2] as f32 * inv;
    let na = a * 255.0 + canvas[idx + 3] as f32 * inv;
    canvas[idx] = nb.round().clamp(0.0, 255.0) as u8;
    canvas[idx + 1] = ng.round().clamp(0.0, 255.0) as u8;
    canvas[idx + 2] = nr.round().clamp(0.0, 255.0) as u8;
    canvas[idx + 3] = na.round().clamp(0.0, 255.0) as u8;
}

/// Anti-aliased filled circle.
#[allow(clippy::too_many_arguments)]
pub fn fill_circle(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    cx: f32,
    cy: f32,
    radius: f32,
    c: (u8, u8, u8),
    a: f32,
) {
    let x0 = (cx - radius - 1.0).floor() as i32;
    let x1 = (cx + radius + 1.0).ceil() as i32;
    let y0 = (cy - radius - 1.0).floor() as i32;
    let y1 = (cy + radius + 1.0).ceil() as i32;
    for py in y0..=y1 {
        for px in x0..=x1 {
            let d = (((px as f32 + 0.5) - cx).powi(2) + ((py as f32 + 0.5) - cy).powi(2)).sqrt();
            let cov = (radius + 0.5 - d).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, c.0, c.1, c.2, a * cov);
            }
        }
    }
}

/// Anti-aliased line segment with round caps, half-width `hw`.
#[allow(clippy::too_many_arguments)]
fn stroke_line(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    hw: f32,
    c: (u8, u8, u8),
    a: f32,
) {
    let minx = (x0.min(x1) - hw - 1.0).floor() as i32;
    let maxx = (x0.max(x1) + hw + 1.0).ceil() as i32;
    let miny = (y0.min(y1) - hw - 1.0).floor() as i32;
    let maxy = (y0.max(y1) + hw + 1.0).ceil() as i32;
    let bax = x1 - x0;
    let bay = y1 - y0;
    let blen2 = (bax * bax + bay * bay).max(1e-6);
    for py in miny..=maxy {
        for px in minx..=maxx {
            let pax = (px as f32 + 0.5) - x0;
            let pay = (py as f32 + 0.5) - y0;
            let t = ((pax * bax + pay * bay) / blen2).clamp(0.0, 1.0);
            let dx = pax - bax * t;
            let dy = pay - bay * t;
            let dist = (dx * dx + dy * dy).sqrt();
            let cov = (hw + 0.5 - dist).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, c.0, c.1, c.2, a * cov);
            }
        }
    }
}

/// Draw a ▶ play badge centered on a card, to mark Video cards as distinct from
/// screenshots. The badge is always visible (not hover-gated).
fn draw_play_badge(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, scale: f32) {
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
                let cov = hits as f32 * inv;
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

/// One frame of the shelf, besides the cards' pictures.
pub struct Frame<'a> {
    pub layout: &'a Layout,
    /// Button geometry, in the canvas's pixels.
    pub cfg: &'a LayoutConfig,
    pub hovered: Option<u64>,
    /// (id, scale, opacity) of the cards mid-animation.
    pub anims: &'a [(u64, f32, f32)],
    /// The card whose save button shows the ✓.
    pub save_flash: Option<u64>,
    /// The hovered card shows its save button; the settings preview has no
    /// file to save and leaves it out.
    pub save_button: bool,
    pub look: CardLook,
    /// Canvas pixels per shelf pixel: 1 for the overlay, the window's scale
    /// for the preview. Radii, the shadow and strokes follow it.
    pub scale: f32,
}

/// Render the whole shelf: each thumbnail, plus hover icons on the hovered
/// thumb. `picture` gives a card's thumbnail and kind by id.
pub fn draw_shelf<'p>(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    frame: &Frame,
    picture: impl Fn(u64) -> Option<(&'p RgbaImage, CardKind)>,
) {
    let (layout, anims, hovered) = (frame.layout, frame.anims, frame.hovered);
    clear(canvas);
    // Pass 1: a soft drop shadow behind every settled card — independent of hover
    // and opacity, so all cards read as lifted. Drawn first so each card blits on
    // top of ALL shadows (no card edge is darkened by a neighbour's shadow).
    if frame.look.shadow {
        for r in &layout.thumbs {
            if !anims.iter().any(|(id, _, _)| *id == r.id) && picture(r.id).is_some() {
                draw_card_shadow(canvas, cw, ch, r, frame.scale);
            }
        }
    }
    // Pass 2: cards + overlays on top.
    for r in &layout.thumbs {
        // Hide overlays on a card that is mid-animation (scaling/fading).
        let animating = anims.iter().any(|(id, _, _)| *id == r.id);
        let hovered_now = hovered == Some(r.id) && !animating;
        let card = picture(r.id);
        if let Some((thumb, _)) = card {
            let (scale, anim_opacity) = anims
                .iter()
                .find(|(id, _, _)| *id == r.id)
                .map(|(_, s, o)| (*s, *o))
                .unwrap_or((1.0, 1.0));
            // Hovered card is fully opaque so it's easy to read, unless the
            // look keeps it translucent. The appear/dismiss fade still
            // multiplies in.
            let base = if hovered == Some(r.id) && frame.look.solid_on_hover {
                1.0
            } else {
                frame.look.opacity
            };
            blit_thumb_card_anim(
                canvas,
                cw,
                ch,
                thumb,
                r.x,
                r.y,
                scale,
                base * anim_opacity,
                CARD_RADIUS * frame.scale,
            );
        }
        // ▶ play badge on Video cards (settled only, so it doesn't sit at full size
        // over a scaling card during the appear/dismiss animation).
        if !animating && card.map(|(_, kind)| kind) == Some(CardKind::Video) {
            draw_play_badge(canvas, cw, ch, r, frame.scale);
        }
        if hovered_now {
            draw_hover_icons(canvas, cw, ch, r, frame);
        }
    }
}

/// Soft drop shadow behind a shelf card, for a subtle "lift" (drawn for every
/// settled card, independent of hover/opacity). Rounded-box SDF feathered over
/// `BLUR` px, slightly offset downward; drawn before the cards so each card sits
/// on top. Only the offset/blur fringe shows (the part under the card is
/// overwritten by the blit).
fn draw_card_shadow(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, scale: f32) {
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
            let cov = (1.0 - sdf / blur).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, 0, 0, 0, ALPHA * cov);
            }
        }
    }
}

fn draw_hover_icons(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, frame: &Frame) {
    let cfg = frame.cfg;
    let hw = GLYPH_HALF_W * frame.scale;
    let s = cfg.icon as f32;
    let (clx, cly, _, _) = close_cell(r, cfg);
    fill_circle(
        canvas,
        cw,
        ch,
        clx as f32 + s / 2.0,
        cly as f32 + s / 2.0,
        s / 2.0 - 0.5,
        BTN_BG,
        BTN_BG_A,
    );
    draw_glyph(
        canvas,
        cw,
        ch,
        Glyph::Close,
        clx as f32,
        cly as f32,
        s,
        hw,
        GLYPH_CLOSE_RGB,
    );
    if !frame.save_button {
        return;
    }
    let (sx, sy, _, _) = save_cell(r, cfg);
    fill_circle(
        canvas,
        cw,
        ch,
        sx as f32 + s / 2.0,
        sy as f32 + s / 2.0,
        s / 2.0 - 0.5,
        BTN_BG,
        BTN_BG_A,
    );
    if frame.save_flash == Some(r.id) {
        draw_glyph(
            canvas,
            cw,
            ch,
            Glyph::Check,
            sx as f32,
            sy as f32,
            s,
            hw,
            GLYPH_OK_RGB,
        );
    } else {
        draw_glyph(
            canvas,
            cw,
            ch,
            Glyph::Save,
            sx as f32,
            sy as f32,
            s,
            hw,
            GLYPH_RGB,
        );
    }
}

/// Which glyph to stamp in a button cell.
enum Glyph {
    Close,
    Save,
    Check,
}

/// Minimal anti-aliased glyphs centred in a cell at (x,y) of size s.
#[allow(clippy::too_many_arguments)]
fn draw_glyph(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    glyph: Glyph,
    x: f32,
    y: f32,
    s: f32,
    hw: f32,
    c: (u8, u8, u8),
) {
    let inset = s * 0.30;
    let lo = inset;
    let hi = s - inset;
    match glyph {
        Glyph::Close => {
            stroke_line(canvas, cw, ch, x + lo, y + lo, x + hi, y + hi, hw, c, 1.0);
            stroke_line(canvas, cw, ch, x + hi, y + lo, x + lo, y + hi, hw, c, 1.0);
        }
        Glyph::Save => {
            // down-arrow into a tray (the modern "save / download" idiom)
            let mid = x + s / 2.0;
            let head = s * 0.18;
            stroke_line(
                canvas,
                cw,
                ch,
                mid,
                y + inset,
                mid,
                y + s * 0.58,
                hw,
                c,
                1.0,
            );
            stroke_line(
                canvas,
                cw,
                ch,
                mid - head,
                y + s * 0.40,
                mid,
                y + s * 0.58,
                hw,
                c,
                1.0,
            );
            stroke_line(
                canvas,
                cw,
                ch,
                mid + head,
                y + s * 0.40,
                mid,
                y + s * 0.58,
                hw,
                c,
                1.0,
            );
            stroke_line(
                canvas,
                cw,
                ch,
                x + inset,
                y + s * 0.74,
                x + s - inset,
                y + s * 0.74,
                hw,
                c,
                1.0,
            );
        }
        Glyph::Check => {
            stroke_line(
                canvas,
                cw,
                ch,
                x + s * 0.26,
                y + s * 0.52,
                x + s * 0.44,
                y + s * 0.70,
                hw,
                c,
                1.0,
            );
            stroke_line(
                canvas,
                cw,
                ch,
                x + s * 0.44,
                y + s * 0.70,
                x + s * 0.76,
                y + s * 0.30,
                hw,
                c,
                1.0,
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn plain(w: u32, h: u32, rgba: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, image::Rgba(rgba))
    }

    /// One card of `thumb` alone on a canvas, `hovered` or not.
    fn shelf_of(
        thumb: &RgbaImage,
        kind: CardKind,
        hovered: bool,
        look: CardLook,
    ) -> (Vec<u8>, Layout) {
        let cfg = LayoutConfig::default();
        let layout = Layout::compute(&[(1, thumb.width(), thumb.height())], &cfg);
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        let frame = Frame {
            layout: &layout,
            cfg: &cfg,
            hovered: hovered.then_some(1),
            anims: &[],
            save_flash: None,
            save_button: true,
            look,
            scale: 1.0,
        };
        draw_shelf(&mut canvas, layout.width, layout.height, &frame, |id| {
            (id == 1).then_some((thumb, kind))
        });
        (canvas, layout)
    }

    fn alpha_at(canvas: &[u8], layout: &Layout, x: u32, y: u32) -> u8 {
        canvas[((y * layout.width + x) * 4 + 3) as usize]
    }

    #[test]
    fn card_rounds_corners_no_border() {
        let img = plain(20, 20, [10, 120, 240, 255]); // low R so we can tell it from white
        let mut buf = vec![0u8; 20 * 20 * 4];
        blit_thumb_card_anim(&mut buf, 20, 20, &img, 0, 0, 1.0, 0.8, CARD_RADIUS);
        // far corner is outside the radius -> transparent
        assert_eq!(buf[3], 0, "corner should be transparent");
        // centre carries the card alpha (~0.8*255), the thumbnail colour (low R)
        let c = ((10 * 20 + 10) * 4) as usize;
        assert!(
            buf[c + 3] > 180 && buf[c + 3] < 220,
            "centre alpha ~0.8, got {}",
            buf[c + 3]
        );
        assert!(buf[c + 2] < 60, "centre R should be the thumbnail's");
        // left-edge midpoint is the IMAGE colour now, NOT a white border
        let e = ((10 * 20) * 4) as usize;
        assert!(
            buf[e + 3] > 150,
            "left edge should carry card alpha, got {}",
            buf[e + 3]
        );
        assert!(
            buf[e + 2] < 60,
            "left edge R should be the image's, not white"
        );
    }

    #[test]
    fn anim_scale_and_opacity_shrink_and_fade() {
        let img = plain(40, 40, [10, 120, 240, 255]);
        // Half scale, half opacity: card occupies the centre, corners of the slot
        // are untouched (transparent), centre alpha ~ 0.5 * 0.8.
        let mut buf = vec![0u8; 40 * 40 * 4];
        blit_thumb_card_anim(&mut buf, 40, 40, &img, 0, 0, 0.5, 0.8 * 0.5, CARD_RADIUS);
        // slot corner (well outside the centred 20x20 card) stays transparent
        let corner = ((2 * 40 + 2) * 4) as usize;
        assert_eq!(buf[corner + 3], 0, "slot corner should be untouched");
        let c = ((20 * 40 + 20) * 4) as usize;
        assert!(
            buf[c + 3] > 70 && buf[c + 3] < 120,
            "centre alpha ~0.4, got {}",
            buf[c + 3]
        );
    }

    #[test]
    fn clear_zeros_buffer() {
        let mut buf = vec![9u8; 16];
        clear(&mut buf);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn fill_circle_blends_center_opaqueish() {
        let mut buf = vec![0u8; 24 * 24 * 4];
        fill_circle(&mut buf, 24, 24, 12.0, 12.0, 8.0, (10, 20, 30), 0.5);
        let idx = ((12 * 24 + 12) * 4) as usize;
        // center alpha ~ 0.5*255
        assert!(
            buf[idx + 3] > 100 && buf[idx + 3] < 160,
            "got a={}",
            buf[idx + 3]
        );
        // corner stays transparent
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn video_card_draws_play_badge() {
        let thumb = plain(260, 180, [10, 20, 200, 255]);
        let (canvas, layout) = shelf_of(&thumb, CardKind::Video, false, CardLook::default());
        let r = &layout.thumbs[0];
        // plain thumb color sampled away from center
        let plain = canvas[(((r.y + 8) * layout.width + r.x + 8) * 4 + 2) as usize];
        // badge at the card center
        let cidx = (((r.y + r.h / 2) * layout.width + r.x + r.w / 2) * 4) as usize;
        assert_ne!(
            canvas[cidx + 2],
            plain,
            "video card center should carry the play badge"
        );
    }

    #[test]
    fn hovered_thumb_draws_icon_pixels() {
        let thumb = plain(260, 180, [10, 20, 200, 255]);
        let (canvas, layout) = shelf_of(&thumb, CardKind::Image, true, CardLook::default());
        let cfg = LayoutConfig::default();
        // thumb body opaque
        let r = &layout.thumbs[0];
        let (px, py) = (r.x + r.w / 2, r.y + r.h / 2);
        let idx = ((py * layout.width + px) * 4) as usize;
        assert_eq!(canvas[idx + 3], 255, "the hovered card is solid");
        // a glyph pixel near the close-button centre should differ from the
        // plain thumbnail colour (button bg darkened or glyph stroke).
        let bx = r.x + r.w - cfg.pad_icon - cfg.icon / 2;
        let by = r.y + cfg.pad_icon + cfg.icon / 2;
        let bidx = ((by * layout.width + bx) * 4) as usize;
        let plain = canvas[idx + 2]; // R of plain thumb region
        assert_ne!(
            canvas[bidx + 2],
            plain,
            "button area should not be plain thumb colour"
        );
        // a glyph pixel near the save-button (top-left) centre should differ from
        // the plain thumbnail colour.
        let sx = r.x + cfg.pad_icon + cfg.icon / 2;
        let sy = r.y + cfg.pad_icon + cfg.icon / 2;
        let sidx = ((sy * layout.width + sx) * 4) as usize;
        assert_ne!(
            canvas[sidx + 2],
            plain,
            "save button area should not be plain thumb colour"
        );
    }

    #[test]
    fn the_look_sets_opacity_hover_and_shadow() {
        let thumb = plain(190, 132, [200, 200, 200, 255]);
        let look = CardLook {
            opacity: 0.5,
            solid_on_hover: false,
            shadow: false,
        };
        let centre = |(canvas, layout): &(Vec<u8>, Layout)| {
            let r = &layout.thumbs[0];
            alpha_at(canvas, layout, r.x + r.w / 2, r.y + r.h / 2)
        };
        // Beside the card, inside its shadow's reach.
        let beside = |(canvas, layout): &(Vec<u8>, Layout)| {
            let r = &layout.thumbs[0];
            alpha_at(canvas, layout, r.x + r.w / 2, r.y + r.h + 4)
        };
        let rest = shelf_of(&thumb, CardKind::Image, false, look);
        assert_eq!(centre(&rest), 128);
        assert_eq!(centre(&shelf_of(&thumb, CardKind::Image, true, look)), 128);
        assert_eq!(beside(&rest), 0, "no shadow");
        let default = CardLook::default();
        assert_eq!(
            centre(&shelf_of(&thumb, CardKind::Image, false, default)),
            204
        );
        assert_eq!(
            centre(&shelf_of(&thumb, CardKind::Image, true, default)),
            255
        );
        assert!(beside(&shelf_of(&thumb, CardKind::Image, false, default)) > 0);
    }

    #[test]
    fn the_preview_leaves_the_save_button_out() {
        let thumb = plain(190, 132, [10, 20, 200, 255]);
        let cfg = LayoutConfig::default();
        let layout = Layout::compute(&[(1, 190, 132)], &cfg);
        let draw = |save_button| {
            let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
            let frame = Frame {
                layout: &layout,
                cfg: &cfg,
                hovered: Some(1),
                anims: &[],
                save_flash: None,
                save_button,
                look: CardLook::default(),
                scale: 1.0,
            };
            draw_shelf(&mut canvas, layout.width, layout.height, &frame, |_| {
                Some((&thumb, CardKind::Image))
            });
            canvas
        };
        let (with, without) = (draw(true), draw(false));
        let (sx, sy, _, _) = save_cell(&layout.thumbs[0], &cfg);
        let (cx, cy, _, _) = close_cell(&layout.thumbs[0], &cfg);
        let at = |x: u32, y: u32| ((y * layout.width + x) * 4) as usize;
        let centre = |x: u32, y: u32| at(x + cfg.icon / 2, y + cfg.icon / 2);
        assert_ne!(with[centre(sx, sy)..][..4], without[centre(sx, sy)..][..4]);
        assert_eq!(with[centre(cx, cy)..][..4], without[centre(cx, cy)..][..4]);
    }
}
