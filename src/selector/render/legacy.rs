//! Controls only the frozen Windows backend draws: the REC pill, the audio
//! button and the frame checkbox. Their pixels are pinned by
//! `tests::windows_pixels_stay_pinned`.

use super::*;

// REC pill metrics — shared by `rec_pill_rect` (placement/hit-zone) and
// `draw_rec_pill` (interior), so the clickable area exactly matches the drawn pill.
const REC_PX: f32 = 17.0;
const REC_PAD: f64 = 7.0;
const REC_DOT_R: f64 = 5.0; // red dot radius
const REC_DOT_GAP: f64 = 6.0; // gap between dot and text
const FRAME_CHECK_GAP: f64 = 6.0;
const FRAME_LABEL: &str = "REC Border Indicator";
const FRAME_PX: f32 = 13.0;
const FRAME_PAD: f64 = 7.0;
const FRAME_BOX_SIZE: f64 = 16.0;
const FRAME_BOX_GAP: f64 = 7.0;
const AUDIO_GAP: f64 = 6.0;
const AUDIO_PX: f32 = 13.0;
const AUDIO_PAD: f64 = 7.0;
const AUDIO_DOT_R: f64 = 4.0;
const AUDIO_DOT_GAP: f64 = 6.0;

/// On-screen rect `(bx, by, bw, bh)` of the REC pill for selection `sel`, placed
/// exactly like `draw_rec_pill`. Returned so the record-mode selector can hit-test
/// the pill as a clickable Start button. `None` for a sub-pixel selection.
pub fn rec_pill_rect(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let (x, y, w, h) = sel;
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(REC_PX));
    let text_w: f32 = "REC"
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let text_h = scaled.ascent() - scaled.descent();
    // The pill carries a leading red dot, so widen the content box by the dot
    // diameter plus its gap; badge_rect handles placement + on-screen clamping.
    let content_w = REC_DOT_R * 2.0 + REC_DOT_GAP + text_w as f64;
    let rect = crate::selector::edit::Rect {
        x: x as f64,
        y: y as f64,
        w: w as f64,
        h: h as f64,
    };
    Some(crate::selector::edit::badge_rect(
        rect,
        content_w,
        text_h as f64,
        REC_PAD,
        6.0,
        surf_w as f64,
        surf_h as f64,
    ))
}

/// On-screen hit box for the frame checkbox beside the REC pill.
pub fn record_frame_checkbox_rect(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let (rx, ry, rw, rh) = rec_pill_rect(sel, surf_w, surf_h)?;
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(FRAME_PX));
    let text_w: f32 = FRAME_LABEL
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let width = FRAME_PAD * 2.0 + FRAME_BOX_SIZE + FRAME_BOX_GAP + text_w as f64;
    let right = rx + rw + FRAME_CHECK_GAP;
    if right + width <= surf_w as f64 {
        return Some((right, ry, width, rh));
    }
    let left = rx - FRAME_CHECK_GAP - width;
    (left >= 0.0 && ry >= 0.0 && ry + rh <= surf_h as f64).then_some((left, ry, width, rh))
}

pub fn record_audio_button_rect(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<(f64, f64, f64, f64)> {
    let rec = rec_pill_rect(sel, surf_w, surf_h)?;
    let frame = record_frame_checkbox_rect(sel, surf_w, surf_h)?;
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(AUDIO_PX));
    let text_w: f32 = "AUDIO OFF"
        .chars()
        .map(|c| scaled.h_advance(font.glyph_id(c)))
        .sum();
    let width = AUDIO_PAD * 2.0 + AUDIO_DOT_R * 2.0 + AUDIO_DOT_GAP + text_w as f64;
    let height = rec.3;
    let overlaps = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
    };
    for (x, y) in [
        (frame.0 + frame.2 + AUDIO_GAP, frame.1),
        (frame.0 - AUDIO_GAP - width, frame.1),
    ] {
        let candidate = (x, y, width, height);
        if x >= 0.0
            && x + width <= surf_w as f64
            && !overlaps(candidate, rec)
            && !overlaps(candidate, frame)
        {
            return Some(candidate);
        }
    }
    let x = rec.0.min((surf_w as f64 - width).max(0.0));
    for y in [rec.1 + rec.3 + AUDIO_GAP, rec.1 - AUDIO_GAP - height] {
        if width <= surf_w as f64 && y >= 0.0 && y + height <= surf_h as f64 {
            return Some((x, y, width, height));
        }
    }
    None
}

#[allow(dead_code)]
pub fn draw_record_audio_button(
    pm: &mut Pixmap,
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    enabled: bool,
) {
    let Some((x, y, w, h)) = record_audio_button_rect(sel, surf_w, surf_h) else {
        return;
    };
    let mut background = Paint::default();
    background.set_color_rgba8(0x12, 0x12, 0x12, 230);
    background.anti_alias = true;
    if let Some(path) = rounded_rect(x as f32, y as f32, w as f32, h as f32, 7.0) {
        pm.fill_path(
            &path,
            &background,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }
    let mut dot = Paint::default();
    let (dot_rgb, text_rgb) = if enabled {
        ((0xff, 0x3b, 0x30), (0xf0, 0xf0, 0xf0))
    } else {
        ((0x70, 0x70, 0x70), (0xa0, 0xa0, 0xa0))
    };
    dot.set_color_rgba8(dot_rgb.0, dot_rgb.1, dot_rgb.2, 255);
    dot.anti_alias = true;
    if let Some(path) = circle_path(
        (x + AUDIO_PAD + AUDIO_DOT_R) as f32,
        (y + h / 2.0) as f32,
        AUDIO_DOT_R as f32,
    ) {
        pm.fill_path(&path, &dot, FillRule::Winding, Transform::identity(), None);
    }
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(AUDIO_PX));
    let text_h = scaled.ascent() - scaled.descent();
    let baseline = y as f32 + (h as f32 - text_h) / 2.0 + scaled.ascent();
    draw_text_aa(
        pm,
        (x + AUDIO_PAD + AUDIO_DOT_R * 2.0 + AUDIO_DOT_GAP) as f32,
        baseline,
        if enabled { "AUDIO ON" } else { "AUDIO OFF" },
        AUDIO_PX,
        text_rgb,
        false,
    );
}

#[allow(dead_code)]
pub fn draw_record_frame_checkbox(
    pm: &mut Pixmap,
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    checked: bool,
) {
    let Some((x, y, w, h)) = record_frame_checkbox_rect(sel, surf_w, surf_h) else {
        return;
    };
    let mut background = Paint::default();
    background.set_color_rgba8(0x12, 0x12, 0x12, 230);
    background.anti_alias = true;
    if let Some(path) = rounded_rect(x as f32, y as f32, w as f32, h as f32, 7.0) {
        pm.fill_path(
            &path,
            &background,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
    }

    let box_x = x + FRAME_PAD;
    let box_y = y + (h - FRAME_BOX_SIZE) / 2.0;
    let mut box_paint = Paint::default();
    box_paint.set_color_rgba8(0xd0, 0xd0, 0xd0, 255);
    box_paint.anti_alias = true;
    if let Some(path) = rounded_rect(
        box_x as f32,
        box_y as f32,
        FRAME_BOX_SIZE as f32,
        FRAME_BOX_SIZE as f32,
        3.0,
    ) {
        pm.stroke_path(
            &path,
            &box_paint,
            &Stroke {
                width: 1.5,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    if checked {
        let mut check = PathBuilder::new();
        check.move_to((box_x + 3.0) as f32, (box_y + 8.0) as f32);
        check.line_to((box_x + 6.5) as f32, (box_y + 11.5) as f32);
        check.line_to((box_x + 13.5) as f32, (box_y + 4.5) as f32);
        let Some(check) = check.finish() else { return };
        let mut check_paint = Paint::default();
        check_paint.set_color_rgba8(0xff, 0x3b, 0x30, 255);
        check_paint.anti_alias = true;
        pm.stroke_path(
            &check,
            &check_paint,
            &Stroke {
                width: 2.2,
                ..Default::default()
            },
            Transform::identity(),
            None,
        );
    }

    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(FRAME_PX));
    let text_h = scaled.ascent() - scaled.descent();
    let baseline = y as f32 + ((h as f32 - text_h) / 2.0) + scaled.ascent();
    draw_text_aa(
        pm,
        (box_x + FRAME_BOX_SIZE + FRAME_BOX_GAP) as f32,
        baseline,
        FRAME_LABEL,
        FRAME_PX,
        (0xf0, 0xf0, 0xf0),
        false,
    );
}

/// Draw the recording affordance: a small pill near the selection with a red
/// filled dot and the text "REC". Placed like the dimension badge (above-left,
/// flipping at edges) and reuses the badge font/text rendering. Red accent so it
/// reads as record. Used by the record-mode selector instead of the W×H badge.
/// The clickable hit-zone is `rec_pill_rect` (same box).
#[allow(dead_code)]
pub fn draw_rec_pill(pm: &mut Pixmap, sel: (f32, f32, f32, f32), surf_w: u32, surf_h: u32) {
    let Some((bx, by, bw, bh)) = rec_pill_rect(sel, surf_w, surf_h) else {
        return;
    };
    let label = "REC";
    let font = ui_font(false);
    let scaled = font.as_scaled(PxScale::from(REC_PX));
    let mut pill = Paint::default();
    pill.set_color_rgba8(0x12, 0x12, 0x12, 230); // #121212, matching the W×H badge
    pill.anti_alias = true;
    if let Some(path) = rounded_rect(bx as f32, by as f32, bw as f32, bh as f32, 7.0) {
        pm.fill_path(&path, &pill, FillRule::Winding, Transform::identity(), None);
    }
    // Red record dot, vertically centered in the pill.
    let mut dot = Paint::default();
    dot.set_color_rgba8(0xff, 0x3b, 0x30, 255); // record red
    dot.anti_alias = true;
    let dot_cx = bx + REC_PAD + REC_DOT_R;
    let dot_cy = by + bh / 2.0;
    if let Some(path) = circle_path(dot_cx as f32, dot_cy as f32, REC_DOT_R as f32) {
        pm.fill_path(&path, &dot, FillRule::Winding, Transform::identity(), None);
    }
    let baseline = by as f32 + REC_PAD as f32 + scaled.ascent();
    draw_text_aa(
        pm,
        (bx + REC_PAD + REC_DOT_R * 2.0 + REC_DOT_GAP) as f32,
        baseline,
        label,
        REC_PX,
        (0xf0, 0xf0, 0xf0),
        false,
    );
}
