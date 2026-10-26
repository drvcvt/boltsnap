//! The recording selector's toolbar: layout, drawing, its own icons.

use super::*;

/// Toolbar icons drawn locally, with thick strokes and rounded contours.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Icon {
    Record,
    Cut,
    Crop,
    Volume,
    VolumeOff,
}

const ICON_STROKE: f32 = 2.4;

fn fill_icon(pm: &mut Pixmap, path: &tiny_skia::Path, rgb: (u8, u8, u8)) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb.0, rgb.1, rgb.2, 255);
    paint.anti_alias = true;
    pm.fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
}

fn stroke_icon(pm: &mut Pixmap, path: &tiny_skia::Path, rgb: (u8, u8, u8)) {
    let mut paint = Paint::default();
    paint.set_color_rgba8(rgb.0, rgb.1, rgb.2, 255);
    paint.anti_alias = true;
    let stroke = Stroke {
        width: ICON_STROKE,
        line_cap: tiny_skia::LineCap::Round,
        line_join: tiny_skia::LineJoin::Round,
        ..Default::default()
    };
    pm.stroke_path(path, &paint, &stroke, Transform::identity(), None);
}

/// Draw `icon` in `rgb` with the top-left of its 18 px box at (`x`, `y`).
fn draw_icon(pm: &mut Pixmap, icon: Icon, x: f64, y: f64, rgb: (u8, u8, u8)) {
    let (ox, oy) = (x as f32, y as f32);
    match icon {
        Icon::Record => {
            if let Some(path) = circle_path(ox + 9.0, oy + 9.0, 4.5) {
                fill_icon(pm, &path, rgb);
            }
        }
        // Two blades crossing above two finger rings.
        Icon::Cut => {
            let mut pb = PathBuilder::new();
            pb.move_to(ox + 5.6, oy + 12.0);
            pb.line_to(ox + 13.5, oy + 2.5);
            pb.move_to(ox + 12.4, oy + 12.0);
            pb.line_to(ox + 4.5, oy + 2.5);
            if let Some(path) = pb.finish() {
                stroke_icon(pm, &path, rgb);
            }
            for cx in [4.5f32, 13.5] {
                if let Some(path) = circle_path(ox + cx, oy + 13.5, 2.4) {
                    stroke_icon(pm, &path, rgb);
                }
            }
        }
        // The four corners of a recording frame.
        Icon::Crop => {
            let mut pb = PathBuilder::new();
            for (cx, cy, sx, sy) in [
                (2.5f32, 2.5f32, 1.0f32, 1.0f32),
                (15.5, 2.5, -1.0, 1.0),
                (15.5, 15.5, -1.0, -1.0),
                (2.5, 15.5, 1.0, -1.0),
            ] {
                pb.move_to(ox + cx, oy + cy + 4.0 * sy);
                pb.line_to(ox + cx, oy + cy + 2.5 * sy);
                pb.quad_to(ox + cx, oy + cy, ox + cx + 2.5 * sx, oy + cy);
                pb.line_to(ox + cx + 4.0 * sx, oy + cy);
            }
            if let Some(path) = pb.finish() {
                stroke_icon(pm, &path, rgb);
            }
        }
        // A speaker, with sound waves or a cross.
        Icon::Volume | Icon::VolumeOff => {
            let mut body = PathBuilder::new();
            body.move_to(ox + 2.5, oy + 6.0);
            body.line_to(ox + 4.0, oy + 6.0);
            body.quad_to(ox + 4.5, oy + 6.0, ox + 5.0, oy + 5.5);
            body.line_to(ox + 8.0, oy + 3.0);
            body.quad_to(ox + 9.5, oy + 1.8, ox + 9.5, oy + 3.8);
            body.line_to(ox + 9.5, oy + 14.2);
            body.quad_to(ox + 9.5, oy + 16.2, ox + 8.0, oy + 15.0);
            body.line_to(ox + 5.0, oy + 12.5);
            body.quad_to(ox + 4.5, oy + 12.0, ox + 4.0, oy + 12.0);
            body.line_to(ox + 2.5, oy + 12.0);
            body.quad_to(ox + 1.0, oy + 12.0, ox + 1.0, oy + 10.5);
            body.line_to(ox + 1.0, oy + 7.5);
            body.quad_to(ox + 1.0, oy + 6.0, ox + 2.5, oy + 6.0);
            body.close();
            if let Some(path) = body.finish() {
                fill_icon(pm, &path, rgb);
            }
            let mut pb = PathBuilder::new();
            if icon == Icon::Volume {
                pb.move_to(ox + 12.0, oy + 6.8);
                pb.quad_to(ox + 14.0, oy + 9.0, ox + 12.0, oy + 11.2);
                pb.move_to(ox + 14.5, oy + 4.5);
                pb.quad_to(ox + 18.5, oy + 9.0, ox + 14.5, oy + 13.5);
            } else {
                pb.move_to(ox + 12.5, oy + 7.0);
                pb.line_to(ox + 16.0, oy + 10.5);
                pb.move_to(ox + 16.0, oy + 7.0);
                pb.line_to(ox + 12.5, oy + 10.5);
            }
            if let Some(path) = pb.finish() {
                stroke_icon(pm, &path, rgb);
            }
        }
    }
}

pub struct RecordToolbar {
    pub bounds: (f64, f64, f64, f64),
    pub controls: [(f64, f64, f64, f64); 4],
}

// Keep 24 px click targets, but let the 21 px labels fill them. Only a
// single pixel separates the controls from the surrounding toolbar surface.
const TB_PAD: f64 = 1.0;
const TB_GAP: f64 = 4.0;
pub(super) const TB_CARD_RADIUS: f32 = 9.0;
// Concentric outer/inner corners keep active fills inside the toolbar contour.
const TB_CONTROL_RADIUS: f32 = TB_CARD_RADIUS - TB_PAD as f32;
const TB_TEXT_PAD: f64 = 6.0;
const TB_LABELS: [&str; 4] = ["REC", "Clip", "Frame", "Audio"];

/// Left inset of a control's label, from the control's own left edge. Every
/// control carries an icon, as every module on the bar does.
fn tb_label_x() -> f64 {
    UI_ICON_PAD + UI_ICON + UI_CONTENT_GAP
}

/// The mark a control shows; the audio toggle swaps glyph the way the bar's
/// own microphone and volume modules do.
fn tb_icon(control: usize, on: bool) -> Icon {
    match control {
        0 => Icon::Record,
        1 => Icon::Cut,
        2 => Icon::Crop,
        _ if on => Icon::Volume,
        _ => Icon::VolumeOff,
    }
}

pub fn record_toolbar(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
) -> Option<RecordToolbar> {
    let (sx, sy, sw, sh) = sel;
    if ![sx, sy, sw, sh].iter().all(|v| v.is_finite()) || sw < 1.0 || sh < 1.0 {
        return None;
    }
    // Measured in DemiBold, the widest a label ever gets, so a module keeps its
    // width when its state changes and the strip never shifts under the cursor.
    let widths: [f64; 4] = std::array::from_fn(|i| {
        (tb_label_x() + text_width(TB_LABELS[i], UI_TEXT_PX, true)).ceil() + TB_TEXT_PAD
    });
    // One strip if it fits, else a 2×2 grid, else a single column.
    let (columns, column_widths, column_x, row_y, width, height) =
        [4usize, 2, 1].into_iter().find_map(|columns| {
            let rows = 4 / columns;
            let column_widths: [f64; 4] = std::array::from_fn(|c| {
                if c < columns {
                    widths
                        .iter()
                        .skip(c)
                        .step_by(columns)
                        .copied()
                        .fold(0.0, f64::max)
                } else {
                    0.0
                }
            });
            let mut column_x = [0.0; 4];
            let mut inner_w = 0.0;
            for c in 0..columns {
                if c > 0 {
                    inner_w += TB_GAP;
                }
                column_x[c] = inner_w;
                inner_w += column_widths[c];
            }
            let mut row_y = [0.0; 4];
            let mut inner_h = 0.0;
            for (r, y) in row_y.iter_mut().enumerate().take(rows) {
                if r > 0 {
                    inner_h += TB_GAP;
                }
                *y = inner_h;
                inner_h += UI_BLOCK_H;
            }
            let (width, height) = (inner_w + TB_PAD * 2.0, inner_h + TB_PAD * 2.0);
            (width + 16.0 <= surf_w as f64 && height + 16.0 <= surf_h as f64).then_some((
                columns,
                column_widths,
                column_x,
                row_y,
                width,
                height,
            ))
        })?;
    let mut x = (sx as f64).clamp(8.0, surf_w as f64 - width - 8.0);
    let above = sy as f64 - height - 12.0;
    let below = (sy + sh) as f64 + 12.0;
    let y = if above >= 8.0 {
        above
    } else if below + height <= surf_h as f64 - 8.0 {
        below
    } else {
        x = (sx as f64 + 12.0).clamp(8.0, surf_w as f64 - width - 8.0);
        sy as f64 + 12.0
    }
    .clamp(8.0, surf_h as f64 - height - 8.0);
    let controls = std::array::from_fn(|i| {
        (
            x + TB_PAD + column_x[i % columns],
            y + TB_PAD + row_y[i / columns],
            column_widths[i % columns],
            UI_BLOCK_H,
        )
    });
    Some(RecordToolbar {
        bounds: (x, y, width, height),
        controls,
    })
}

pub fn draw_record_toolbar(
    pm: &mut Pixmap,
    toolbar: &RecordToolbar,
    states: (bool, bool, bool),
    hovered: Option<usize>,
) {
    let (clip_enabled, frame_enabled, audio_enabled) = states;
    fill_rounded(pm, toolbar.bounds, TB_CARD_RADIUS, UI_SURFACE);
    for (i, &(x, y, w, h)) in toolbar.controls.iter().enumerate() {
        let enabled = i != 1 || clip_enabled;
        let on = (i == 2 && frame_enabled) || (i == 3 && audio_enabled);
        // Record keeps a resting fill as the primary action; every module takes
        // the bar's white hover fill.
        let fill = match (hovered == Some(i) && enabled, i == 0) {
            (true, true) => Some(BAR_ACTIVE),
            (true, false) => Some(BAR_HOVER),
            (false, true) => Some(BAR_HOVER_SOFT),
            (false, false) => None,
        };
        if let Some(rgba) = fill {
            fill_rounded(pm, (x, y, w, h), TB_CONTROL_RADIUS, rgba);
        }
        // Record is the one place with hue. Elsewhere the icon says on or off
        // the way the bar's own modules do, by going bright or dim, and an
        // active module sets its label in DemiBold.
        let (icon_rgb, strong) = match i {
            0 => (UI_RECORD, true),
            1 if enabled => (UI_FOREGROUND, false),
            1 => (UI_FAINT, false),
            _ if on => (UI_FOREGROUND, true),
            _ => (UI_DIM, false),
        };
        draw_icon(
            pm,
            tb_icon(i, on),
            x + UI_ICON_PAD,
            y + (h - UI_ICON) / 2.0,
            icon_rgb,
        );
        draw_text_aa(
            pm,
            (x + tb_label_x()) as f32,
            centered_baseline(y, h, UI_TEXT_PX),
            TB_LABELS[i],
            UI_TEXT_PX,
            // A label is always the bar's foreground; only an unavailable
            // action dims, the way a disabled menu entry does.
            if enabled { UI_FOREGROUND } else { UI_DIM },
            strong,
        );
    }
}
