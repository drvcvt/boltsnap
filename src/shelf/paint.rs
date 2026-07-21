use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
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
/// `scale` (0..1) and centred in that slot, and `opacity` (0..1) is the card's
/// final alpha — the caller picks the base translucency (e.g. `CARD_OPACITY`, or
/// `1.0` for the hovered card) and folds in any appear/dismiss fade. `scale == 1.0
/// && opacity == 1.0` is a fully-opaque settled card.
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
    let radius = (min_dim * 0.14).max(11.0); // a touch larger; floor for tiny cards
    let cx = r.x as f32 + r.w as f32 / 2.0;
    let cy = r.y as f32 + r.h as f32 / 2.0;

    // Soft dark halo (slightly larger, faint) gives the badge a crisp edge against
    // busy thumbnails; then the translucent disc on top.
    fill_circle(canvas, cw, ch, cx, cy, radius + 1.5, (0, 0, 0), 0.28);
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

/// Recording-overlay accent (red ●, button glyphs).
const REC_RGB: (u8, u8, u8) = (235, 64, 64);
/// Region marker frame: black, for a clean, unobtrusive recording outline.
const MARKER_RGB: (u8, u8, u8) = (0, 0, 0);
const MARKER_A: f32 = 0.95;
/// Indicator pill background (matches the shelf/quickshell dark).
const IND_BG: (u8, u8, u8) = (0x12, 0x12, 0x12); // #121212, matching the badge/pill
/// Indicator button chip: a neutral lift of the #121212 surface (no blue tint),
/// so Stop/Confirm/Cancel read as buttons on the dark pill.
const IND_BTN_BG: (u8, u8, u8) = (0x26, 0x26, 0x26); // #262626

/// Draw the click-through region marker: a rounded, anti-aliased `border`-px black
/// frame on a transparent `w`×`h` surface. The surface is inflated past the
/// recorded rect by `radius` so the rounded corners sit fully OUTSIDE the recording
/// and are never captured. The frame is the ring between an outer rounded rect
/// (the whole surface, radius `radius`) and an inner one inset by `border`.
pub fn draw_marker_border(canvas: &mut [u8], w: u32, h: u32, border: u32, radius: u32) {
    clear(canvas);
    if w == 0 || h == 0 {
        return;
    }
    let half = (w.min(h) as f32) / 2.0;
    let b = (border as f32).clamp(1.0, half);
    let r_out = (radius as f32).clamp(b, half);
    let r_in = (r_out - b).max(0.0);
    let (cr, cg, cb) = MARKER_RGB;
    let iw = w as f32 - 2.0 * b;
    let ih = h as f32 - 2.0 * b;
    for y in 0..h {
        for x in 0..w {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let outer = rr_coverage(px, py, w as f32, h as f32, r_out);
            let inner = rr_coverage(px - b, py - b, iw, ih, r_in);
            let cov = (outer - inner).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, w, h, x as i32, y as i32, cr, cg, cb, cov * MARKER_A);
            }
        }
    }
}

pub fn draw_recording_popup(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    state: crate::record::session::PublicRecordingState,
    enabled: bool,
    elapsed: &str,
    font: &FontVec,
) {
    use crate::record::session::PublicRecordingState;
    use crate::shelf::recording::{
        discard_rect, pause_resume_rect, save_disk_rect, save_shelf_rect,
    };

    clear(canvas);
    fill_round_rect(
        canvas, w, h, 0.0, 0.0, w as f32, h as f32, 16.0, IND_BG, 0.96,
    );
    let title = match state {
        PublicRecordingState::Idle => "IDLE",
        PublicRecordingState::Recording => "RECORDING",
        PublicRecordingState::Paused => "PAUSED",
        PublicRecordingState::Finalizing => "SAVING...",
    };
    fill_circle(canvas, w, h, 22.0, 28.0, 5.0, REC_RGB, 1.0);
    let header_px = 18.0;
    let scaled = font.as_scaled(PxScale::from(header_px));
    let header_baseline = 28.0 + (scaled.ascent() + scaled.descent()) / 2.0;
    draw_font_text(
        canvas,
        w,
        h,
        font,
        36.0,
        header_baseline,
        title,
        header_px,
        GLYPH_RGB,
    );
    draw_font_text(
        canvas,
        w,
        h,
        font,
        382.0 - text_width(font, elapsed, header_px),
        header_baseline,
        elapsed,
        header_px,
        GLYPH_RGB,
    );

    let button_alpha = if enabled && state != PublicRecordingState::Finalizing {
        0.95
    } else {
        0.45
    };
    for ((x, y, bw, bh), label, destructive) in [
        (
            pause_resume_rect(),
            if state == PublicRecordingState::Paused {
                "RESUME"
            } else {
                "PAUSE"
            },
            false,
        ),
        (save_shelf_rect(), "SHELF SAVE", false),
        (save_disk_rect(), "DISK SAVE", false),
        (discard_rect(), "DISCARD", true),
    ] {
        fill_round_rect(canvas, w, h, x, y, bw, bh, 10.0, IND_BTN_BG, button_alpha);
        let color = if destructive {
            GLYPH_CLOSE_RGB
        } else {
            GLYPH_RGB
        };
        let text_px = 14.0;
        let scaled = font.as_scaled(PxScale::from(text_px));
        draw_font_text(
            canvas,
            w,
            h,
            font,
            x + (bw - text_width(font, label, text_px)) / 2.0,
            y + bh / 2.0 + (scaled.ascent() + scaled.descent()) / 2.0,
            label,
            text_px,
            color,
        );
    }
}

fn text_width(font: &FontVec, text: &str, px: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut width = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(previous) = previous {
            width += scaled.kern(previous, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}

#[allow(clippy::too_many_arguments)]
fn draw_font_text(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    font: &FontVec,
    x: f32,
    baseline: f32,
    text: &str,
    px: f32,
    color: (u8, u8, u8),
) {
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut caret = x;
    let mut previous = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(previous) = previous {
            caret += scaled.kern(previous, id);
        }
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                blend_px(
                    canvas,
                    w,
                    h,
                    bounds.min.x as i32 + gx as i32,
                    bounds.min.y as i32 + gy as i32,
                    color.0,
                    color.1,
                    color.2,
                    coverage,
                );
            });
        }
        caret += scaled.h_advance(id);
        previous = Some(id);
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
    draw_shelf_with_opacity(
        canvas,
        cw,
        ch,
        layout,
        model,
        hovered,
        cfg,
        anims,
        save_flash,
        CARD_OPACITY,
    );
}

#[cfg(target_os = "windows")]
pub fn draw_shelf_opaque(
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
    draw_shelf_with_opacity(
        canvas, cw, ch, layout, model, hovered, cfg, anims, save_flash, 1.0,
    );
}

fn draw_shelf_with_opacity(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    layout: &Layout,
    model: &ShelfModel,
    hovered: Option<u64>,
    cfg: &LayoutConfig,
    anims: &[(u64, f32, f32)],
    save_flash: Option<u64>,
    card_opacity: f32,
) {
    clear(canvas);
    // Pass 1: a soft drop shadow behind every settled card — independent of hover
    // and opacity, so all cards read as lifted. Drawn first so each card blits on
    // top of ALL shadows (no card edge is darkened by a neighbour's shadow).
    for r in &layout.thumbs {
        if !anims.iter().any(|(id, _, _)| *id == r.id) && model.get(r.id).is_some() {
            draw_card_shadow(canvas, cw, ch, r);
        }
    }
    // Pass 2: cards + overlays on top.
    for r in &layout.thumbs {
        // Hide overlays on a card that is mid-animation (scaling/fading).
        let animating = anims.iter().any(|(id, _, _)| *id == r.id);
        let hovered_now = hovered == Some(r.id) && !animating;
        if let Some(thumb) = model.get(r.id) {
            let (scale, anim_opacity) = anims
                .iter()
                .find(|(id, _, _)| *id == r.id)
                .map(|(_, s, o)| (*s, *o))
                .unwrap_or((1.0, 1.0));
            // Hovered card is fully opaque so it's easy to read; others use the
            // translucent base. The appear/dismiss fade still multiplies in.
            let base = if hovered == Some(r.id) {
                1.0
            } else {
                card_opacity
            };
            blit_thumb_card_anim(
                canvas,
                cw,
                ch,
                &thumb.thumb,
                r.x,
                r.y,
                scale,
                base * anim_opacity,
            );
        }
        // ▶ play badge on Video cards (settled only, so it doesn't sit at full size
        // over a scaling card during the appear/dismiss animation).
        if !animating
            && model.get(r.id).map(|t| t.kind) == Some(crate::shelf::model::CardKind::Video)
        {
            draw_play_badge(canvas, cw, ch, r);
        }
        if hovered_now {
            draw_hover_icons(canvas, cw, ch, r, cfg, save_flash == Some(r.id));
        }
    }
}

/// Soft drop shadow behind a shelf card, for a subtle "lift" (drawn for every
/// settled card, independent of hover/opacity). Rounded-box SDF feathered over
/// `BLUR` px, slightly offset downward; drawn before the cards so each card sits
/// on top. Only the offset/blur fringe shows (the part under the card is
/// overwritten by the blit).
fn draw_card_shadow(canvas: &mut [u8], cw: u32, ch: u32, r: &ThumbRect) {
    const DX: f32 = 0.0;
    const DY: f32 = 3.0; // downward offset reads as "lifted"
    const BLUR: f32 = 16.0; // wider falloff = more diffuse
    const ALPHA: f32 = 0.24; // lighter, less heavy
    let (sx, sy) = (r.x as f32 + DX, r.y as f32 + DY);
    let (w, h) = (r.w as f32, r.h as f32);
    let rad = CARD_RADIUS;
    let (cxw, cyh) = (w / 2.0, h / 2.0);
    let x0 = (sx - BLUR - 1.0).floor() as i32;
    let x1 = (sx + w + BLUR + 1.0).ceil() as i32;
    let y0 = (sy - BLUR - 1.0).floor() as i32;
    let y1 = (sy + h + BLUR + 1.0).ceil() as i32;
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
            let cov = (1.0 - sdf / BLUR).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, 0, 0, 0, ALPHA * cov);
            }
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
        blit_thumb_card_anim(&mut buf, 20, 20, &img, 0, 0, 1.0, CARD_OPACITY);
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
        blit_thumb_card_anim(&mut buf, 40, 40, &img, 0, 0, 0.5, CARD_OPACITY * 0.5);
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
        draw_marker_border(&mut buf, w, h, 2, 0);
        // Top-left edge pixel is painted (alpha > 0).
        let edge = 0usize;
        assert!(buf[edge + 3] > 0, "border edge should be painted");
        // Centre is transparent (interior not captured into the video).
        let mid = (((h / 2) * w + w / 2) * 4) as usize;
        assert_eq!(buf[mid + 3], 0, "marker interior must stay transparent");
    }

    #[test]
    fn popup_paints_background_and_all_control_cells() {
        use crate::record::session::PublicRecordingState;
        use crate::shelf::recording::{
            POPUP_H, POPUP_W, discard_rect, pause_resume_rect, save_disk_rect, save_shelf_rect,
        };

        let mut buf = vec![0u8; (POPUP_W * POPUP_H * 4) as usize];
        let font = crate::shelf::font::fallback_popup_font();
        draw_recording_popup(
            &mut buf,
            POPUP_W,
            POPUP_H,
            PublicRecordingState::Recording,
            true,
            "01:23",
            &font,
        );
        assert!(buf[((POPUP_H / 2 * POPUP_W + POPUP_W / 2) * 4 + 3) as usize] > 0);
        for (x, y, w, h) in [
            pause_resume_rect(),
            save_shelf_rect(),
            save_disk_rect(),
            discard_rect(),
        ] {
            let px = (x + w / 2.0) as u32;
            let py = (y + h / 2.0) as u32;
            assert!(buf[((py * POPUP_W + px) * 4 + 3) as usize] > 0);
        }
    }

    #[test]
    fn popup_font_renderer_paints_text_pixels() {
        let (w, h) = (180, 48);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let font = crate::shelf::font::fallback_popup_font();
        draw_font_text(
            &mut buf,
            w,
            h,
            &font,
            8.0,
            30.0,
            "RECORDING 01:23",
            18.0,
            GLYPH_RGB,
        );
        assert!(buf.chunks_exact(4).any(|pixel| pixel[3] != 0));
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
