#![cfg_attr(windows, allow(dead_code))]
use super::*;
use crate::shelf::recording::PopupButton;

// Linux recording controls, on the mt-ui-style scales: panel padding 16,
// 30 px buttons with a 12 px inset, 8 px gaps, capitals 12/10/8 px tall.
const CTL_PANEL: (u8, u8, u8) = (0x12, 0x12, 0x12);
/// A button's face, pressed in: a shade below the panel, as the mark's
/// `theme::PRESSED` below the sidebar.
const CTL_FACE: (u8, u8, u8) = (0x0A, 0x0A, 0x0A);
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
/// Type by the height of its capitals, as the settings window sets it
/// (`ui::text::Fonts::size`): whole pixels, so their flat edges fall on
/// pixel rows. Labels as the window's rows, keys as its `Key`.
const CTL_TITLE_CAP: f32 = 12.0;
const CTL_LABEL_CAP: f32 = 10.0;
const CTL_KEY_CAP: f32 = 8.0;
const CTL_RADIUS: f32 = 9.0;
/// The rims of a pressed button, as the mark's against the sidebar
/// (`theme::SHADE` 13 darker, `theme::LIP` 11 lighter than its ground):
/// above, the shadowed wall, below, the lit one.
const RIM: ((u8, u8, u8), (u8, u8, u8)) = ((0x05, 0x05, 0x05), (0x1D, 0x1D, 0x1D));

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
            .map(|label| text_width(font, label, px(font, CTL_LABEL_CAP)))
            .fold(0.0, f32::max);
        let width =
            (CTL_INSET * 2.0 + label + CTL_KEY_GAP + text_width(font, key, px(font, CTL_KEY_CAP)))
                .ceil();
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

/// The size that makes `font`'s capitals `cap` pixels tall.
pub(super) fn px(font: &FontVec, cap: f32) -> f32 {
    let per_px = font
        .outline(font.glyph_id('H'))
        .map(|outline| outline.bounds.height().abs() / font.height_unscaled())
        .filter(|per_px| *per_px > 0.0)
        .unwrap_or(0.7);
    cap / per_px
}

/// Baseline that centres capitals `cap` tall in a box: a whole pixel, as
/// the box and the capitals are whole pixels with an even difference.
fn cap_baseline(cap: f32, top: f32, height: f32) -> f32 {
    (top + (height + cap) / 2.0).round()
}

/// A button pressed into the panel like the Boltsnap mark into the
/// settings sidebar (`app/brand.rs`): its outline one pixel up in a
/// darker shade of the panel and one pixel down in a lighter one, as if
/// light from above fell into it, its face on top.
fn pressed_button(
    canvas: &mut [u8],
    cw: u32,
    ch: u32,
    (x, y, w, h): (f32, f32, f32, f32),
    face: (u8, u8, u8),
) {
    for (color, dy) in [(RIM.0, -1.0), (RIM.1, 1.0), (face, 0.0)] {
        fill_round_rect(canvas, cw, ch, x, y + dy, w, h, CTL_RADIUS, color, 1.0);
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
    let head = cap_baseline(CTL_TITLE_CAP, CTL_PAD, CTL_HEAD_H);
    draw_font_text(
        canvas,
        w,
        h,
        strong,
        CTL_PAD,
        head,
        title,
        px(strong, CTL_TITLE_CAP),
        tone,
    );
    draw_font_text(
        canvas,
        w,
        h,
        body,
        (w as f32 - CTL_PAD - text_width(body, elapsed, px(body, CTL_TITLE_CAP))).round(),
        head,
        elapsed,
        px(body, CTL_TITLE_CAP),
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
            (true, false) => (CTL_FACE, CTL_FG, CTL_KEY),
            (false, _) => (CTL_FACE, CTL_KEY, CTL_KEY),
        };
        pressed_button(canvas, w, h, (*x, *y, *bw, *bh), fill);
        draw_font_text(
            canvas,
            w,
            h,
            body,
            x + CTL_INSET,
            cap_baseline(CTL_LABEL_CAP, *y, *bh),
            label,
            px(body, CTL_LABEL_CAP),
            text,
        );
        draw_font_text(
            canvas,
            w,
            h,
            body,
            (x + bw - CTL_INSET - text_width(body, key, px(body, CTL_KEY_CAP))).round(),
            cap_baseline(CTL_KEY_CAP, *y, *bh),
            key,
            px(body, CTL_KEY_CAP),
            hint,
        );
    }
}
