//! The overlay's own surfaces: the shelf from its model, the drag icon, the
//! recording marker and controls. The cards themselves are painted by
//! `boltsnap::shelf_card::paint`, which the settings preview shares.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont, point};
use boltsnap::shelf_card::CardLook;
pub use boltsnap::shelf_card::paint::clear;
use boltsnap::shelf_card::paint::{
    Frame, GLYPH_CLOSE_RGB, GLYPH_RGB, blend_px, fill_circle, rr_coverage,
};
use image::RgbaImage;

use crate::shelf::layout::{Layout, LayoutConfig};
use crate::shelf::model::ShelfModel;

/// Render the whole shelf from the model: each thumbnail, plus hover icons
/// on the hovered thumb.
#[allow(clippy::too_many_arguments)]
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
    look: CardLook,
) {
    let frame = Frame {
        layout,
        cfg,
        hovered,
        anims,
        save_flash,
        save_button: true,
        look,
        scale: 1.0,
    };
    boltsnap::shelf_card::paint::draw_shelf(canvas, cw, ch, &frame, |id| {
        model.get(id).map(|card| (&card.thumb, card.kind))
    });
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
    let look = CardLook {
        opacity: 1.0,
        ..CardLook::default()
    };
    draw_shelf(
        canvas, cw, ch, layout, model, hovered, cfg, anims, save_flash, look,
    );
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

// The frozen Windows shelf keeps calling the version without recovery.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn draw_recording_popup(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    state: crate::record::session::PublicRecordingState,
    enabled: bool,
    elapsed: &str,
    font: &FontVec,
) {
    draw_recording_popup_with(canvas, w, h, state, enabled, false, elapsed, font);
}

/// `recovery`: saving failed; the title says so and Resume is dimmed.
#[allow(clippy::too_many_arguments)]
pub fn draw_recording_popup_with(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    state: crate::record::session::PublicRecordingState,
    enabled: bool,
    recovery: bool,
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
        PublicRecordingState::Paused if recovery => "SAVE FAILED",
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
    for ((x, y, bw, bh), label, destructive, inert) in [
        (
            pause_resume_rect(),
            if state == PublicRecordingState::Paused {
                "RESUME"
            } else {
                "PAUSE"
            },
            false,
            recovery,
        ),
        (save_shelf_rect(), "SHELF SAVE", false, false),
        (save_disk_rect(), "DISK SAVE", false, false),
        (discard_rect(), "DISCARD", true, false),
    ] {
        let alpha = if inert { 0.45 } else { button_alpha };
        fill_round_rect(canvas, w, h, x, y, bw, bh, 10.0, IND_BTN_BG, alpha);
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

/// Linux recording controls; the frozen Windows shell keeps `draw_recording_popup`.
pub mod controls;

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
#[allow(clippy::too_many_arguments)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
    fn controls_layout_keeps_one_padding_and_one_row() {
        use crate::shelf::recording::PopupButton;
        let font = crate::shelf::font::fallback_popup_font();
        let layout = controls::controls_layout(&font);
        let rects: Vec<_> = layout.buttons.iter().map(|(_, rect)| *rect).collect();
        let (first, last) = (rects[0], rects[3]);
        assert_eq!(first.0, 16.0, "left padding");
        assert_eq!(last.0 + last.2, layout.width as f32 - 16.0, "right padding");
        assert_eq!(
            first.1 + first.3,
            layout.height as f32 - 16.0,
            "bottom padding"
        );
        for pair in rects.windows(2) {
            assert_eq!(pair[1].0 - (pair[0].0 + pair[0].2), 8.0, "gap");
        }
        assert!(rects.iter().all(|r| r.1 == first.1 && r.3 == 30.0));
        for (button, (x, y, w, h)) in &layout.buttons {
            let centre = ((x + w / 2.0) as f64, (y + h / 2.0) as f64);
            assert_eq!(layout.hit(centre.0, centre.1), Some(*button));
        }
        assert_eq!(layout.hit(8.0, 8.0), None);
        assert_eq!(layout.buttons[1].0, PopupButton::SaveShelf);
    }

    #[test]
    fn controls_paint_states_primary_and_confirm() {
        use crate::record::session::PublicRecordingState;
        let font = crate::shelf::font::fallback_popup_font();
        let layout = controls::controls_layout(&font);
        let draw = |state, enabled, armed| {
            let mut buf = vec![0u8; (layout.width * layout.height * 4) as usize];
            controls::draw_recording_controls(
                &mut buf,
                &layout,
                state,
                enabled,
                false,
                None,
                armed,
                "01:23",
                (&font, &font),
            );
            buf
        };
        // Left of the label, at mid height: clear of the corners and the text.
        let pixel = |buf: &[u8], (x, y, _, h): (f32, f32, f32, f32)| {
            let at = ((((y + h / 2.0) as u32) * layout.width + x as u32 + 4) * 4) as usize;
            buf[at..at + 4].to_vec()
        };
        let cell = |buf: &[u8], (x, y, w, h): (f32, f32, f32, f32)| {
            (y as u32..(y + h) as u32)
                .flat_map(|py| {
                    let start = ((py * layout.width + x as u32) * 4) as usize;
                    buf[start..start + w as usize * 4].to_vec()
                })
                .collect::<Vec<_>>()
        };
        let paused = draw(PublicRecordingState::Paused, true, false);
        let shelf = layout.buttons[1].1;
        let disk = layout.buttons[2].1;
        let discard = layout.buttons[3].1;
        // The one primary action is the bright fill; the rest stay a raise.
        assert!(pixel(&paused, shelf)[0] > 0xE0);
        assert!(pixel(&paused, disk)[0] < 0x30);
        assert_ne!(
            cell(&paused, discard),
            cell(&draw(PublicRecordingState::Paused, true, true), discard)
        );
        // While saving nothing is primary.
        assert!(pixel(&draw(PublicRecordingState::Finalizing, false, false), shelf)[0] < 0x30);
        // Panel is opaque everywhere inside its rounded corners.
        let centre = ((layout.height / 2 * layout.width + 4) * 4 + 3) as usize;
        assert_eq!(paused[centre], 255);
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
}
