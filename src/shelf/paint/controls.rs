#![cfg_attr(windows, allow(dead_code))]
use super::*;
use crate::shelf::recording::PopupButton;

// Linux recording controls, on the mt-ui-style scales: panel padding 16,
// 30 px buttons with a 12 px inset, 8 px gaps, type at 16/14/12 px.
const CTL_PANEL: (u8, u8, u8) = (0x12, 0x12, 0x12);
const CTL_RAISE: (u8, u8, u8) = (0x22, 0x22, 0x22);
const CTL_FG: (u8, u8, u8) = (0xEC, 0xEC, 0xEC);
const CTL_SUB: (u8, u8, u8) = (0x96, 0x96, 0x96);
const CTL_KEY: (u8, u8, u8) = (0x6A, 0x6A, 0x6A);
const CTL_KEY_ON_PRIMARY: (u8, u8, u8) = (0x6E, 0x6E, 0x6E);
const CTL_PAD: f32 = 16.0;
const CTL_HEAD_H: f32 = 18.0;
const CTL_ROW_GAP: f32 = 12.0;
const CTL_H: f32 = 30.0;
const CTL_INSET: f32 = 12.0;
const CTL_GAP: f32 = 8.0;
const CTL_KEY_GAP: f32 = 6.0;
const CTL_TITLE_PX: f32 = 16.0;
const CTL_LABEL_PX: f32 = 14.0;
const CTL_KEY_PX: f32 = 12.0;
const CTL_RADIUS: f32 = 9.0;
/// A button's depth: its drop shadow (down, soft, how dark), how much
/// lighter its top and darker its bottom than its colour, its top light.
const SHADOW: (f32, f32, f32) = (2.0, 6.0, 0.5);
const SHADE: (i16, i16) = (6, -4);
const TOP_LIGHT: f32 = 0.08;

/// Every label a button can show, and its key.
const CONTROL_BUTTONS: [(PopupButton, &[&str], &str); 4] = [
    (PopupButton::PauseResume, &["Pause", "Resume"], "Space"),
    (PopupButton::SaveShelf, &["Save to shelf"], "Enter"),
    (PopupButton::SaveDisk, &["Save to disk"], "D"),
    (PopupButton::Discard, &["Discard", "Confirm"], "Del"),
];

/// Size and button rectangles of the Linux recording controls.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlsLayout {
    pub width: u32,
    pub height: u32,
    pub buttons: [(PopupButton, (f32, f32, f32, f32)); 4],
}

impl ControlsLayout {
    pub fn hit(&self, x: f64, y: f64) -> Option<PopupButton> {
        self.buttons.iter().find_map(|(button, (bx, by, bw, bh))| {
            (x >= *bx as f64 && x < (bx + bw) as f64 && y >= *by as f64 && y < (by + bh) as f64)
                .then_some(*button)
        })
    }
}

/// Each button is as wide as its widest label, so a state change never moves
/// it; the panel is as wide as the row.
pub fn controls_layout(font: &FontVec) -> ControlsLayout {
    let y = CTL_PAD + CTL_HEAD_H + CTL_ROW_GAP;
    let mut x = CTL_PAD;
    let buttons = CONTROL_BUTTONS.map(|(button, labels, key)| {
        let label = labels
            .iter()
            .map(|label| text_width(font, label, CTL_LABEL_PX))
            .fold(0.0, f32::max);
        let width =
            (CTL_INSET * 2.0 + label + CTL_KEY_GAP + text_width(font, key, CTL_KEY_PX)).ceil();
        let rect = (x, y, width, CTL_H);
        x += width + CTL_GAP;
        (button, rect)
    });
    ControlsLayout {
        width: (x - CTL_GAP + CTL_PAD) as u32,
        height: (y + CTL_H + CTL_PAD) as u32,
        buttons,
    }
}

/// Baseline that centres the cap height in a box, on a whole pixel so
/// the letters' flat edges stay sharp.
fn cap_baseline(font: &FontVec, px: f32, top: f32, height: f32) -> f32 {
    let cap = font
        .outline_glyph(font.glyph_id('H').with_scale(px))
        .map(|glyph| glyph.px_bounds().height())
        .unwrap_or(px * 0.7);
    (top + (height + cap) / 2.0).round()
}

fn shade((r, g, b): (u8, u8, u8), by: i16) -> (u8, u8, u8) {
    let step = |c: u8| (i16::from(c) + by).clamp(0, 255) as u8;
    (step(r), step(g), step(b))
}

/// A raised button at (x, y): a soft shadow under it, its face a little
/// lighter at the top than at the bottom, and a thin light along its top.
#[allow(clippy::too_many_arguments)]
fn raised_button(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    (x, y, w, h): (f32, f32, f32, f32),
    color: (u8, u8, u8),
    alpha: f32,
) {
    let (drop, blur, dark) = SHADOW;
    let r = CTL_RADIUS;
    for py in (y - blur).floor() as i32..=(y + h + drop + blur).ceil() as i32 {
        for px in (x - blur).floor() as i32..=(x + w + blur).ceil() as i32 {
            let (lx, ly) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y - drop);
            // Signed distance to the button's outline, moved down.
            let dx = (lx - w / 2.0).abs() - (w / 2.0 - r);
            let dy = (ly - h / 2.0).abs() - (h / 2.0 - r);
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
            let sdf = outside + dx.max(dy).min(0.0) - r;
            let cov = (1.0 - sdf / blur).clamp(0.0, 1.0);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, 0, 0, 0, dark * alpha * cov * cov);
            }
        }
    }
    let (top, bottom) = (shade(color, SHADE.0), shade(color, SHADE.1));
    for py in y.floor() as i32..(y + h).ceil() as i32 {
        let t = ((py as f32 + 0.5 - y) / h).clamp(0.0, 1.0);
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t).round() as u8;
        let (cr, cg, cb) = (
            mix(top.0, bottom.0),
            mix(top.1, bottom.1),
            mix(top.2, bottom.2),
        );
        for px in x.floor() as i32..(x + w).ceil() as i32 {
            let cov = rr_coverage(px as f32 + 0.5 - x, py as f32 + 0.5 - y, w, h, r);
            if cov > 0.0 {
                blend_px(canvas, cw, ch, px, py, cr, cg, cb, alpha * cov);
            }
        }
    }
    // The light along the top, between the corners.
    let top_row = y.round() as i32;
    for px in (x + r).round() as i32..(x + w - r).round() as i32 {
        blend_px(
            canvas,
            cw,
            ch,
            px,
            top_row,
            255,
            255,
            255,
            TOP_LIGHT * alpha,
        );
    }
}

/// The Linux recording controls. `recovery`: a save failed, Resume is inert.
/// `notice`: why the recording paused itself. `discard_armed`: Discard waits
/// for its confirming second press.
#[allow(clippy::too_many_arguments)]
pub fn draw_recording_controls(
    canvas: &mut [u8],
    layout: &ControlsLayout,
    state: crate::record::session::PublicRecordingState,
    enabled: bool,
    recovery: bool,
    notice: Option<&str>,
    discard_armed: bool,
    elapsed: &str,
    fonts: (&FontVec, &FontVec),
) {
    use crate::record::session::PublicRecordingState;

    let (body, strong) = fonts;
    let (w, h) = (layout.width, layout.height);
    clear(canvas);
    fill_round_rect(
        canvas, w, h, 0.0, 0.0, w as f32, h as f32, 14.0, CTL_PANEL, 1.0,
    );
    let ellipsis = if strong.glyph_id('…').0 == 0 {
        "..."
    } else {
        "…"
    };
    let saving = format!("Saving{ellipsis}");
    // The state is the title and its brightness: running bright, the
    // rest dimmed.
    let (title, tone) = match state {
        PublicRecordingState::Recording => ("Recording", CTL_FG),
        PublicRecordingState::Paused if recovery => ("Save failed", CTL_FG),
        PublicRecordingState::Paused => (notice.unwrap_or("Paused"), CTL_SUB),
        PublicRecordingState::Finalizing => (saving.as_str(), CTL_SUB),
        PublicRecordingState::Idle => ("Idle", CTL_SUB),
    };
    let head = cap_baseline(strong, CTL_TITLE_PX, CTL_PAD, CTL_HEAD_H);
    draw_font_text(
        canvas,
        w,
        h,
        strong,
        CTL_PAD,
        head,
        title,
        CTL_TITLE_PX,
        tone,
    );
    draw_font_text(
        canvas,
        w,
        h,
        body,
        (w as f32 - CTL_PAD - text_width(body, elapsed, CTL_TITLE_PX)).round(),
        cap_baseline(body, CTL_TITLE_PX, CTL_PAD, CTL_HEAD_H),
        elapsed,
        CTL_TITLE_PX,
        CTL_SUB,
    );

    let live = enabled
        && !matches!(
            state,
            PublicRecordingState::Idle | PublicRecordingState::Finalizing
        );
    for (button, (x, y, bw, bh)) in &layout.buttons {
        let label = match button {
            PopupButton::PauseResume if state == PublicRecordingState::Paused => "Resume",
            PopupButton::PauseResume => "Pause",
            PopupButton::SaveShelf => "Save to shelf",
            PopupButton::SaveDisk => "Save to disk",
            PopupButton::Discard if discard_armed => "Confirm",
            PopupButton::Discard => "Discard",
        };
        let key = CONTROL_BUTTONS
            .iter()
            .find(|(candidate, _, _)| candidate == button)
            .map_or("", |(_, _, key)| key);
        let active = live && !(recovery && *button == PopupButton::PauseResume);
        let primary = active && *button == PopupButton::SaveShelf;
        let (fill, text, hint) = match (active, primary) {
            (_, true) => (CTL_FG, CTL_PANEL, CTL_KEY_ON_PRIMARY),
            (true, false) => (CTL_RAISE, CTL_FG, CTL_KEY),
            (false, _) => (CTL_RAISE, CTL_KEY, CTL_KEY),
        };
        raised_button(canvas, w, h, (*x, *y, *bw, *bh), fill, 1.0);
        draw_font_text(
            canvas,
            w,
            h,
            body,
            x + CTL_INSET,
            cap_baseline(body, CTL_LABEL_PX, *y, *bh),
            label,
            CTL_LABEL_PX,
            text,
        );
        draw_font_text(
            canvas,
            w,
            h,
            body,
            (x + bw - CTL_INSET - text_width(body, key, CTL_KEY_PX)).round(),
            cap_baseline(body, CTL_KEY_PX, *y, *bh),
            key,
            CTL_KEY_PX,
            hint,
        );
    }
}
