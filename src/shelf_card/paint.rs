//! Painting the cards into premultiplied BGRA, the overlay's `wl_shm`
//! format: rounded thumbnails, their shadow, the play badge and the hover
//! buttons.

use image::{RgbaImage, imageops};

use super::badge::draw_play_badge;
use super::layout::{Layout, LayoutConfig, ThumbRect, close_cell, save_cell};
use super::shadow::draw_card_shadow;
use super::{CardKind, CardLook};

/// Thumbnail card corner radius, in pixels.
pub(super) const CARD_RADIUS: f32 = 10.0;

/// Hover-button styling: a small translucent dark circle with an anti-aliased
/// glyph. Minimal and unobtrusive over the screenshot.
pub(super) const BTN_BG: (u8, u8, u8) = (18, 18, 24);
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
    let coverage = corner_mask(sw, sh, r);
    // Premultiplied BGRA: scaling RGB and alpha together by the same factor
    // keeps the pixel valid while making the card translucent. Inside the
    // corners the factor is `global` for every pixel, worked out per value.
    let inside: [u8; 256] = std::array::from_fn(|v| premultiplied(v as u8, global));
    let inside_alpha = (global * 255.0).round().clamp(0.0, 255.0) as u8;
    // Clipped at the canvas's right and bottom edges.
    let columns = sw.min(cw.saturating_sub(ox)) as usize;
    let rows = sh.min(ch.saturating_sub(oy));
    if columns == 0 {
        return;
    }
    for sy in 0..rows {
        let start = ((oy + sy) * cw + ox) as usize * 4;
        let row = &mut canvas[start..start + columns * 4];
        let from = &src.as_raw()[(sy * sw) as usize * 4..][..columns * 4];
        let covers = &coverage[(sy * sw) as usize..][..columns];
        for ((out, p), &cov) in row
            .chunks_exact_mut(4)
            .zip(from.chunks_exact(4))
            .zip(covers)
        {
            if cov <= 0.0 {
                continue; // transparent corner -> leave the canvas untouched
            }
            if cov == 1.0 {
                out.copy_from_slice(&[
                    inside[usize::from(p[2])],
                    inside[usize::from(p[1])],
                    inside[usize::from(p[0])],
                    inside_alpha,
                ]);
                continue;
            }
            let a = cov * global;
            out.copy_from_slice(&[
                premultiplied(p[2], a),
                premultiplied(p[1], a),
                premultiplied(p[0], a),
                (a * 255.0).round().clamp(0.0, 255.0) as u8,
            ]);
        }
    }
}

fn premultiplied(value: u8, alpha: f32) -> u8 {
    (f32::from(value) * alpha).round().clamp(0.0, 255.0) as u8
}

thread_local! {
    static CORNERS: std::cell::RefCell<std::collections::HashMap<(u32, u32, u32), std::rc::Rc<Vec<f32>>>> =
        std::cell::RefCell::default();
}

/// `rr_coverage` of every pixel of a `w` × `h` card with corner radius `r`,
/// worked out once per size and kept per painting thread.
fn corner_mask(w: u32, h: u32, r: f32) -> std::rc::Rc<Vec<f32>> {
    CORNERS.with(|corners| {
        let mut corners = corners.borrow_mut();
        if corners.len() >= 32 {
            corners.clear();
        }
        corners
            .entry((w, h, r.to_bits()))
            .or_insert_with(|| {
                let (fw, fh) = (w as f32, h as f32);
                let cells = (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| rr_coverage(x as f32 + 0.5, y as f32 + 0.5, fw, fh, r))
                    .collect();
                std::rc::Rc::new(cells)
            })
            .clone()
    })
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

/// One frame of the shelf, besides the cards' pictures.
pub struct Frame<'a> {
    pub layout: &'a Layout,
    /// Button geometry, in the canvas's pixels.
    pub cfg: &'a LayoutConfig,
    /// (id, how far its hover has come, 0 to 1) of the hovered cards: 1 at
    /// once, or a fade in and out with `[shelf] hover_fade`.
    pub hovered: &'a [(u64, f32)],
    /// (id, scale, opacity) of the cards mid-animation.
    pub anims: &'a [(u64, f32, f32)],
    /// The card whose save button shows the ✓.
    pub save_flash: Option<u64>,
    /// The hovered card shows its save button; the settings preview has no
    /// file to save and leaves it out.
    pub save_button: bool,
    /// (id, how far it has come to ready, 0 to 1) of the video cards whose
    /// clip is still being written or has just been: an arc turns in their
    /// badge, and they have no save button until 1.
    pub saving: &'a [(u64, f32)],
    /// How far the saving arc has turned, in turns.
    pub spin: f32,
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
            if anims.iter().any(|(id, _, _)| *id == r.id) {
                continue;
            }
            if let Some((thumb, _)) = picture(r.id) {
                // A settled card covers its whole rectangle at full size.
                let covered = thumb.dimensions() == (r.w, r.h);
                draw_card_shadow(canvas, cw, ch, r, frame.scale, covered);
            }
        }
    }
    // Pass 2: cards + overlays on top.
    for r in &layout.thumbs {
        // Hide overlays on a card that is mid-animation (scaling/fading).
        let animating = anims.iter().any(|(id, _, _)| *id == r.id);
        let lift = hovered
            .iter()
            .find(|(id, _)| *id == r.id)
            .map_or(0.0, |(_, lift)| lift.clamp(0.0, 1.0));
        let hovered_now = lift > 0.0 && !animating;
        let card = picture(r.id);
        if let Some((thumb, _)) = card {
            let (scale, anim_opacity) = anims
                .iter()
                .find(|(id, _, _)| *id == r.id)
                .map(|(_, s, o)| (*s, *o))
                .unwrap_or((1.0, 1.0));
            // Hovered card is fully opaque so it's easy to read, unless the
            // look keeps it translucent; on its way there with a hover fade.
            // The appear/dismiss fade still multiplies in.
            let base = if frame.look.solid_on_hover {
                frame.look.opacity + (1.0 - frame.look.opacity) * lift
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
            draw_play_badge(
                canvas,
                cw,
                ch,
                r,
                frame.scale,
                ready(frame, r.id),
                frame.spin,
            );
        }
        if hovered_now {
            draw_hover_icons(canvas, cw, ch, r, frame, lift);
        }
    }
}

/// How far card `id` has come to ready: 1 unless it is saving.
fn ready(frame: &Frame, id: u64) -> f32 {
    frame
        .saving
        .iter()
        .find(|(card, _)| *card == id)
        .map_or(1.0, |(_, ready)| *ready)
}

/// The card's buttons, at `lift` of their strength (a hover fade).
fn draw_hover_icons(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect, frame: &Frame, lift: f32) {
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
        s / 2.0 - 0.5 * frame.scale,
        BTN_BG,
        BTN_BG_A * lift,
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
        lift,
    );
    if !frame.save_button || ready(frame, r.id) < 1.0 {
        return;
    }
    let (sx, sy, _, _) = save_cell(r, cfg);
    fill_circle(
        canvas,
        cw,
        ch,
        sx as f32 + s / 2.0,
        sy as f32 + s / 2.0,
        s / 2.0 - 0.5 * frame.scale,
        BTN_BG,
        BTN_BG_A * lift,
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
            lift,
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
            lift,
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
    a: f32,
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
            stroke_line(canvas, cw, ch, mid, y + inset, mid, y + s * 0.58, hw, c, a);
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
                a,
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
                a,
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
                a,
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
                a,
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
                a,
            );
        }
    }
}
#[cfg(test)]
mod tests;
