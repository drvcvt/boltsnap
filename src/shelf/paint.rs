use image::{RgbaImage, imageops};

use crate::shelf::layout::{Layout, LayoutConfig, ThumbRect};
use crate::shelf::model::ShelfModel;

/// Thumbnail card corner radius, in pixels.
const CARD_RADIUS: f32 = 10.0;
/// Card opacity: slightly translucent so text/windows behind the shelf stay
/// readable. 1.0 = fully opaque.
const CARD_OPACITY: f32 = 0.8;

/// Hover-button styling: a small translucent dark circle with an anti-aliased
/// glyph. Minimal and unobtrusive over the screenshot.
const BTN_BG: (u8, u8, u8) = (18, 18, 24);
const BTN_BG_A: f32 = 0.52;
const GLYPH_RGB: (u8, u8, u8) = (244, 244, 248);
const GLYPH_CLOSE_RGB: (u8, u8, u8) = (255, 124, 124);
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
/// `scale` (0..1) and centred in that slot, and `opacity` (0..1) multiplies the
/// global card opacity. `scale == 1.0 && opacity == 1.0` is the settled look;
/// other values drive the appear/dismiss animation.
pub fn blit_thumb_card_anim(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    img: &RgbaImage,
    dx: u32,
    dy: u32,
    scale: f32,
    opacity: f32,
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
    let r = CARD_RADIUS.min(w / 2.0).min(h / 2.0);
    let global = CARD_OPACITY * opacity.clamp(0.0, 1.0);
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

/// Build a premultiplied-BGRA drag icon: scale `src` to (w,h) with Lanczos3,
/// round the corners, and apply a global `opacity` (0..=1). Returns w*h*4 bytes,
/// ready to copy into a wl_shm Argb8888 buffer.
pub fn build_drag_icon(src: &RgbaImage, w: u32, h: u32, radius: f32, opacity: f32) -> Vec<u8> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    if w == 0 || h == 0 {
        return buf;
    }
    let scaled = image::imageops::resize(src, w, h, image::imageops::FilterType::Lanczos3);
    let r = radius.min(w as f32 / 2.0).min(h as f32 / 2.0);
    for sy in 0..h {
        for sx in 0..w {
            let cov =
                rr_coverage(sx as f32 + 0.5, sy as f32 + 0.5, w as f32, h as f32, r) * opacity;
            if cov <= 0.0 {
                continue;
            }
            let p = scaled.get_pixel(sx, sy).0;
            let idx = ((sy * w + sx) * 4) as usize;
            buf[idx] = (p[2] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 1] = (p[1] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 2] = (p[0] as f32 * cov).round().clamp(0.0, 255.0) as u8;
            buf[idx + 3] = (cov * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    buf
}

/// Coverage of a rounded rectangle (size w×h, corner radius r) at point (px,py):
/// ~1.0 well inside, ~0.0 well outside, anti-aliased across ~1px at the edge.
/// Uses the standard rounded-box signed distance field.
fn rr_coverage(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
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
fn blend_px(canvas: &mut [u8], cw: u32, ch: u32, x: i32, y: i32, r: u8, g: u8, b: u8, a: f32) {
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
fn fill_circle(
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
fn draw_play_badge(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect) {
    let min_dim = r.w.min(r.h) as f32;
    let radius = min_dim * 0.12;
    let cx = r.x as f32 + r.w as f32 / 2.0;
    let cy = r.y as f32 + r.h as f32 / 2.0;

    // Translucent dark circle background (slightly more opaque than hover buttons).
    fill_circle(canvas, cw, ch, cx, cy, radius, BTN_BG, 0.55);

    // Filled right-pointing triangle (▶) inside the circle.
    // Triangle dimensions: ~0.9× the circle radius, vertically centred.
    let tri_half_h = radius * 0.9 * 0.5; // half the triangle height
    let tri_w = radius * 0.9; // horizontal span (left edge to apex)
    // Left edge x is slightly left of centre so the triangle is visually centred.
    // Optically, a play glyph looks centred when the centroid (1/3 from left) is at cx.
    let left_x = cx - tri_w / 3.0;
    let top_y = cy - tri_half_h;
    let bot_y = cy + tri_half_h;
    let apex_x = left_x + tri_w;

    // Scanline fill: for each integer row y inside [top_y, bot_y], compute the
    // x-span between the two left diagonal edges and the converging right apex.
    let y0 = (top_y - 1.0).floor() as i32;
    let y1 = (bot_y + 1.0).ceil() as i32;
    for py in y0..=y1 {
        let yf = py as f32 + 0.5;
        // How far along the triangle (0 at top, 1 at bottom)?
        let span = bot_y - top_y;
        if span <= 0.0 {
            break;
        }
        let t = ((yf - top_y) / span).clamp(0.0, 1.0);
        // Left edge of the triangle runs from (left_x, top_y) to (left_x, bot_y).
        // Right edge converges from both corners to the apex.
        let x_left = left_x;
        let x_right = if t <= 0.5 {
            // upper half: from (left_x, top_y) → (apex_x, mid_y)
            left_x + (apex_x - left_x) * (t * 2.0)
        } else {
            // lower half: from (apex_x, mid_y) → (left_x, bot_y)
            left_x + (apex_x - left_x) * ((1.0 - t) * 2.0)
        };
        let xl = x_left.floor() as i32;
        let xr = x_right.ceil() as i32;
        for px in xl..=xr {
            let xf = px as f32 + 0.5;
            // Sub-pixel coverage on left edge
            let cov_l = (xf - x_left + 0.5).clamp(0.0, 1.0);
            // Sub-pixel coverage on right edge
            let cov_r = (x_right - xf + 0.5).clamp(0.0, 1.0);
            let cov = cov_l.min(cov_r);
            if cov > 0.0 {
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

/// Recording-overlay accent (red ●, marker border, button glyphs).
const REC_RGB: (u8, u8, u8) = (235, 64, 64);
/// Indicator pill background (matches the shelf/quickshell dark).
const IND_BG: (u8, u8, u8) = (18, 18, 24);
const IND_BG_A: f32 = 0.92;

/// Draw the click-through region marker: a `border`-px red frame on the INNER
/// edge of a transparent `w`×`h` surface. The surface is inflated past the
/// recorded rect so this border sits just OUTSIDE the recording.
pub fn draw_marker_border(canvas: &mut [u8], w: u32, h: u32, border: u32) {
    clear(canvas);
    if w == 0 || h == 0 {
        return;
    }
    let b = border.min(w / 2).min(h / 2).max(1);
    let (r, g, bl) = REC_RGB;
    for y in 0..h {
        for x in 0..w {
            let on_edge = x < b || x >= w - b || y < b || y >= h - b;
            if on_edge {
                blend_px(canvas, w, h, x as i32, y as i32, r, g, bl, 0.95);
            }
        }
    }
}

/// Draw the recording control indicator into a premultiplied-BGRA `w`×`h` canvas.
/// In the `Recording` phase: a red ● + the `MM:SS` `elapsed` text + a Stop (■)
/// button. In the `Stopped` phase: Confirm (✓) / Cancel (✕) buttons.
pub fn draw_indicator(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    phase: crate::shelf::recording::RecPhase,
    elapsed: &str,
) {
    use crate::shelf::recording::RecPhase;
    clear(canvas);
    // Rounded translucent pill background spanning the whole surface.
    fill_round_rect(
        canvas, w, h, 0.0, 0.0, w as f32, h as f32, 12.0, IND_BG, IND_BG_A,
    );

    match phase {
        RecPhase::Recording => {
            // Red ● on the left, vertically centred.
            let cy = h as f32 / 2.0;
            fill_circle(canvas, w, h, 18.0, cy, 6.0, REC_RGB, 1.0);
            // MM:SS to the right of the dot.
            draw_time(canvas, w, h, 34.0, cy, 18.0, GLYPH_RGB, elapsed);
            // Stop (■): a filled red square button.
            let (bx, by, bw, bh) = crate::shelf::recording::stop_btn_rect();
            fill_round_rect(canvas, w, h, bx, by, bw, bh, 6.0, BTN_BG, 0.85);
            let inset = bw * 0.28;
            fill_round_rect(
                canvas,
                w,
                h,
                bx + inset,
                by + inset,
                bw - 2.0 * inset,
                bh - 2.0 * inset,
                2.0,
                REC_RGB,
                1.0,
            );
        }
        RecPhase::Stopped => {
            // Confirm (✓) on the left.
            let (cx, cy, cw, ch) = crate::shelf::recording::confirm_btn_rect();
            fill_round_rect(canvas, w, h, cx, cy, cw, ch, 8.0, BTN_BG, 0.85);
            let s = ch.min(cw);
            let gx = cx + (cw - s) / 2.0;
            let gy = cy + (ch - s) / 2.0;
            draw_glyph(canvas, w, h, Glyph::Check, gx, gy, s, GLYPH_OK_RGB);
            // Cancel (✕) on the right.
            let (xx, xy, xw, xh) = crate::shelf::recording::cancel_btn_rect();
            fill_round_rect(canvas, w, h, xx, xy, xw, xh, 8.0, BTN_BG, 0.85);
            let s = xh.min(xw);
            let gx = xx + (xw - s) / 2.0;
            let gy = xy + (xh - s) / 2.0;
            draw_glyph(canvas, w, h, Glyph::Close, gx, gy, s, GLYPH_CLOSE_RGB);
        }
    }
}

/// Anti-aliased filled rounded rectangle at (x,y) size (w,h), corner radius `r`,
/// colour `c`, coverage `a`. Reuses the same SDF as the card corners.
fn fill_round_rect(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    r: f32,
    c: (u8, u8, u8),
    a: f32,
) {
    let x0 = (x - 1.0).floor() as i32;
    let y0 = (y - 1.0).floor() as i32;
    let x1 = (x + w + 1.0).ceil() as i32;
    let y1 = (y + h + 1.0).ceil() as i32;
    for py in y0..=y1 {
        for px in x0..=x1 {
            let lx = px as f32 + 0.5 - x;
            let ly = py as f32 + 0.5 - y;
            let cov = rr_coverage(lx, ly, w, h, r);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, c.0, c.1, c.2, a * cov);
            }
        }
    }
}

/// Draw a `MM:SS`-style string left-aligned with its vertical centre at `cy`,
/// using a 7-segment-ish stroked rendering (the embedded badge font has no colon,
/// and the daemon draws with these BGRA primitives, not tiny-skia). `digit_h` is
/// the cell height; digits are `digit_h * 0.55` wide.
fn draw_time(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x: f32,
    cy: f32,
    digit_h: f32,
    c: (u8, u8, u8),
    s: &str,
) {
    let dw = digit_h * 0.55;
    let gap = digit_h * 0.18;
    let top = cy - digit_h / 2.0;
    let mut caret = x;
    for ch_ in s.chars() {
        if ch_ == ':' {
            // Colon: two small dots at 1/3 and 2/3 height.
            let colon_w = digit_h * 0.28;
            let r = digit_h * 0.07;
            let dx = caret + colon_w / 2.0;
            fill_circle(canvas, cw, ch, dx, top + digit_h * 0.34, r, c, 1.0);
            fill_circle(canvas, cw, ch, dx, top + digit_h * 0.66, r, c, 1.0);
            caret += colon_w + gap;
        } else if let Some(d) = ch_.to_digit(10) {
            draw_seg_digit(canvas, cw, ch, caret, top, dw, digit_h, c, d as u8);
            caret += dw + gap;
        } else {
            caret += dw + gap; // unknown char -> advance, draw nothing
        }
    }
}

/// Seven-segment digit at (x, top), cell (w, h). Segments:
/// ```text
///  _a_
/// f   b
///  _g_
/// e   c
///  _d_
/// ```
fn draw_seg_digit(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    x: f32,
    top: f32,
    w: f32,
    h: f32,
    c: (u8, u8, u8),
    d: u8,
) {
    // Segment presence per digit, order [a,b,c,d,e,f,g].
    const SEGS: [[bool; 7]; 10] = [
        [true, true, true, true, true, true, false],     // 0
        [false, true, true, false, false, false, false], // 1
        [true, true, false, true, true, false, true],    // 2
        [true, true, true, true, false, false, true],    // 3
        [false, true, true, false, false, true, true],   // 4
        [true, false, true, true, false, true, true],    // 5
        [true, false, true, true, true, true, true],     // 6
        [true, true, true, false, false, false, false],  // 7
        [true, true, true, true, true, true, true],      // 8
        [true, true, true, true, false, true, true],     // 9
    ];
    let seg = match SEGS.get(d as usize) {
        Some(s) => s,
        None => return,
    };
    let hw = (h * 0.045).max(0.9); // half stroke width
    let l = x + hw;
    let rr = x + w - hw;
    let midy = top + h / 2.0;
    let t = top + hw;
    let b = top + h - hw;
    let line = |canvas: &mut [u8], x0: f32, y0: f32, x1: f32, y1: f32| {
        stroke_line(canvas, cw, ch, x0, y0, x1, y1, hw, c, 1.0);
    };
    if seg[0] {
        line(canvas, l, t, rr, t);
    } // a (top)
    if seg[1] {
        line(canvas, rr, t, rr, midy);
    } // b (top-right)
    if seg[2] {
        line(canvas, rr, midy, rr, b);
    } // c (bottom-right)
    if seg[3] {
        line(canvas, l, b, rr, b);
    } // d (bottom)
    if seg[4] {
        line(canvas, l, midy, l, b);
    } // e (bottom-left)
    if seg[5] {
        line(canvas, l, t, l, midy);
    } // f (top-left)
    if seg[6] {
        line(canvas, l, midy, rr, midy);
    } // g (middle)
}

/// Render the whole shelf: each thumbnail, plus hover icons on the hovered thumb.
pub fn draw_shelf(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    layout: &Layout,
    model: &ShelfModel,
    hovered: Option<u64>,
    cfg: &LayoutConfig,
    anims: &[(u64, f32, f32)],
    save_flash: Option<u64>,
) {
    clear(canvas);
    for r in &layout.thumbs {
        if let Some(thumb) = model.get(r.id) {
            let (scale, opacity) = anims
                .iter()
                .find(|(id, _, _)| *id == r.id)
                .map(|(_, s, o)| (*s, *o))
                .unwrap_or((1.0, 1.0));
            blit_thumb_card_anim(canvas, cw, ch, &thumb.thumb, r.x, r.y, scale, opacity);
        }
        // Draw the ▶ play badge on Video cards (always visible, not hover-gated).
        if model.get(r.id).map(|t| t.kind) == Some(crate::shelf::model::CardKind::Video) {
            draw_play_badge(canvas, cw, ch, r);
        }
        // Hide hover icons on a card that is mid-animation (scaling/fading).
        let animating = anims.iter().any(|(id, _, _)| *id == r.id);
        if hovered == Some(r.id) && !animating {
            draw_hover_icons(canvas, cw, ch, r, cfg, save_flash == Some(r.id));
        }
    }
}

fn draw_hover_icons(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    r: &ThumbRect,
    cfg: &LayoutConfig,
    save_flashing: bool,
) {
    let s = cfg.icon as f32;
    let (clx, cly, _, _) = crate::shelf::layout::close_cell(r, cfg);
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
        GLYPH_CLOSE_RGB,
    );
    let (sx, sy, _, _) = crate::shelf::layout::save_cell(r, cfg);
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
    if save_flashing {
        draw_glyph(
            canvas,
            cw,
            ch,
            Glyph::Check,
            sx as f32,
            sy as f32,
            s,
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
fn draw_glyph(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    glyph: Glyph,
    x: f32,
    y: f32,
    s: f32,
    c: (u8, u8, u8),
) {
    let hw = GLYPH_HALF_W;
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
    use std::path::PathBuf;

    #[test]
    fn card_rounds_corners_no_border() {
        let mut img = RgbaImage::new(20, 20);
        for p in img.pixels_mut() {
            *p = image::Rgba([10, 120, 240, 255]); // low R so we can tell it from white
        }
        let mut buf = vec![0u8; 20 * 20 * 4];
        blit_thumb_card_anim(&mut buf, 20, 20, &img, 0, 0, 1.0, 1.0);
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
        let e = ((10 * 20 + 0) * 4) as usize;
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
    fn drag_icon_is_premultiplied_and_rounded() {
        let mut img = RgbaImage::new(40, 40);
        for p in img.pixels_mut() {
            *p = image::Rgba([200, 100, 50, 255]);
        }
        let buf = build_drag_icon(&img, 40, 40, 8.0, 0.85);
        assert_eq!(buf.len(), 40 * 40 * 4);
        // corner transparent
        assert_eq!(buf[3], 0, "corner alpha should be 0");
        // centre alpha ~ 0.85*255
        let c = ((20 * 40 + 20) * 4) as usize;
        assert!(buf[c + 3] > 200 && buf[c + 3] < 230, "got a={}", buf[c + 3]);
        // premultiplied: every colour channel <= alpha
        assert!(buf[c] <= buf[c + 3] && buf[c + 1] <= buf[c + 3] && buf[c + 2] <= buf[c + 3]);
    }

    #[test]
    fn anim_scale_and_opacity_shrink_and_fade() {
        let mut img = RgbaImage::new(40, 40);
        for p in img.pixels_mut() {
            *p = image::Rgba([10, 120, 240, 255]);
        }
        // Half scale, half opacity: card occupies the centre, corners of the slot
        // are untouched (transparent), centre alpha ~ 0.5 * CARD_OPACITY.
        let mut buf = vec![0u8; 40 * 40 * 4];
        blit_thumb_card_anim(&mut buf, 40, 40, &img, 0, 0, 0.5, 0.5);
        // slot corner (well outside the centred 20x20 card) stays transparent
        let corner = ((2 * 40 + 2) * 4) as usize;
        assert_eq!(buf[corner + 3], 0, "slot corner should be untouched");
        // centre carries ~0.5*0.75*255 ≈ 95
        let c = ((20 * 40 + 20) * 4) as usize;
        assert!(
            buf[c + 3] > 70 && buf[c + 3] < 120,
            "centre alpha ~0.375, got {}",
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
        let mut model = ShelfModel::new();
        let mut t = RgbaImage::new(260, 180);
        for p in t.pixels_mut() {
            *p = image::Rgba([10, 20, 200, 255]);
        }
        let _id = model.add_kind(
            PathBuf::from("/tmp/v.mp4"),
            t,
            "record".into(),
            crate::shelf::model::CardKind::Video,
        );
        let cfg = LayoutConfig::default();
        let sizes: Vec<(u64, u32, u32)> = model
            .newest_first()
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        let layout = Layout::compute(&sizes, &cfg);
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        draw_shelf(
            &mut canvas,
            layout.width,
            layout.height,
            &layout,
            &model,
            None,
            &cfg,
            &[],
            None,
        );
        let r = &layout.thumbs[0];
        // plain thumb color sampled away from center
        let px = r.x + 8;
        let py = r.y + 8;
        let plain = canvas[(((py * layout.width + px) * 4) + 2) as usize];
        // badge at the card center
        let cx = r.x + r.w / 2;
        let cy = r.y + r.h / 2;
        let cidx = ((cy * layout.width + cx) * 4) as usize;
        assert_ne!(
            canvas[cidx + 2],
            plain,
            "video card center should carry the play badge"
        );
    }

    #[test]
    fn marker_border_only_paints_the_edge() {
        let (w, h) = (40u32, 30u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        draw_marker_border(&mut buf, w, h, 2);
        // Top-left edge pixel is painted (alpha > 0).
        let edge = 0usize;
        assert!(buf[edge + 3] > 0, "border edge should be painted");
        // Centre is transparent (interior not captured into the video).
        let mid = (((h / 2) * w + w / 2) * 4) as usize;
        assert_eq!(buf[mid + 3], 0, "marker interior must stay transparent");
    }

    #[test]
    fn indicator_recording_and_stopped_paint_something() {
        use crate::shelf::recording::RecPhase;
        let (w, h) = (
            crate::shelf::recording::IND_W,
            crate::shelf::recording::IND_H,
        );
        let mut buf = vec![0u8; (w * h * 4) as usize];
        draw_indicator(&mut buf, w, h, RecPhase::Recording, "01:23");
        // The pill background makes the whole surface non-transparent.
        let mid = (((h / 2) * w + w / 2) * 4) as usize;
        assert!(buf[mid + 3] > 0, "indicator pill should fill the surface");
        // The Stop button region carries the red accent (R channel dominant).
        let (bx, by, bw, bh) = crate::shelf::recording::stop_btn_rect();
        let sx = (bx + bw / 2.0) as u32;
        let sy = (by + bh / 2.0) as u32;
        let sidx = ((sy * w + sx) * 4) as usize;
        assert!(buf[sidx + 3] > 0, "stop button should be drawn");

        // Stopped phase paints the two control buttons.
        let mut buf2 = vec![0u8; (w * h * 4) as usize];
        draw_indicator(&mut buf2, w, h, RecPhase::Stopped, "");
        let (cx, cy, cw, chh) = crate::shelf::recording::confirm_btn_rect();
        let px = (cx + cw / 2.0) as u32;
        let py = (cy + chh / 2.0) as u32;
        let cidx = ((py * w + px) * 4) as usize;
        assert!(buf2[cidx + 3] > 0, "confirm button should be drawn");
    }

    #[test]
    fn hovered_thumb_draws_icon_pixels() {
        let mut model = ShelfModel::new();
        let mut t = RgbaImage::new(260, 180);
        for p in t.pixels_mut() {
            *p = image::Rgba([10, 20, 200, 255]);
        }
        let id = model.add(PathBuf::from("/tmp/x.png"), t, "area".into());
        let cfg = LayoutConfig::default();
        let sizes: Vec<(u64, u32, u32)> = model
            .newest_first()
            .map(|t| (t.id, t.thumb.width(), t.thumb.height()))
            .collect();
        let layout = Layout::compute(&sizes, &cfg);
        let mut canvas = vec![0u8; (layout.width * layout.height * 4) as usize];
        draw_shelf(
            &mut canvas,
            layout.width,
            layout.height,
            &layout,
            &model,
            Some(id),
            &cfg,
            &[],
            None,
        );
        // thumb body opaque
        let r = &layout.thumbs[0];
        let px = r.x + r.w / 2;
        let py = r.y + r.h / 2;
        let idx = ((py * layout.width + px) * 4) as usize;
        assert!(canvas[idx + 3] > 0, "thumb body should be opaque");
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
}
