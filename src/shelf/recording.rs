use crate::record::session::PublicRecordingState;

pub const POPUP_W: u32 = 408;
pub const POPUP_H: u32 = 148;

pub const MARKER_BORDER: u32 = 2;
pub const MARKER_RADIUS: u32 = 6;
pub const MARKER_INFLATE: u32 = MARKER_RADIUS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupButton {
    PauseResume,
    SaveShelf,
    SaveDisk,
    Discard,
}

const BUTTON_Y: f32 = 72.0;
const BUTTON_W: f32 = 88.0;
const BUTTON_H: f32 = 56.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_X: f32 = 16.0;

pub fn pause_resume_rect() -> (f32, f32, f32, f32) {
    (BUTTON_X, BUTTON_Y, BUTTON_W, BUTTON_H)
}

pub fn save_shelf_rect() -> (f32, f32, f32, f32) {
    (
        BUTTON_X + BUTTON_W + BUTTON_GAP,
        BUTTON_Y,
        BUTTON_W,
        BUTTON_H,
    )
}

pub fn save_disk_rect() -> (f32, f32, f32, f32) {
    (
        BUTTON_X + 2.0 * (BUTTON_W + BUTTON_GAP),
        BUTTON_Y,
        BUTTON_W,
        BUTTON_H,
    )
}

pub fn discard_rect() -> (f32, f32, f32, f32) {
    (
        BUTTON_X + 3.0 * (BUTTON_W + BUTTON_GAP),
        BUTTON_Y,
        BUTTON_W,
        BUTTON_H,
    )
}

pub fn popup_hit(
    state: PublicRecordingState,
    enabled: bool,
    x: f64,
    y: f64,
) -> Option<PopupButton> {
    if !enabled
        || matches!(
            state,
            PublicRecordingState::Idle | PublicRecordingState::Finalizing
        )
    {
        return None;
    }
    [
        (PopupButton::PauseResume, pause_resume_rect()),
        (PopupButton::SaveShelf, save_shelf_rect()),
        (PopupButton::SaveDisk, save_disk_rect()),
        (PopupButton::Discard, discard_rect()),
    ]
    .into_iter()
    .find_map(|(button, (bx, by, bw, bh))| {
        (x >= bx as f64 && x < (bx + bw) as f64 && y >= by as f64 && y < (by + bh) as f64)
            .then_some(button)
    })
}

pub fn fmt_elapsed(secs: u64) -> String {
    let secs = secs.min(99 * 60 + 59);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalizing_popup_has_no_live_buttons() {
        for y in 0..POPUP_H {
            for x in 0..POPUP_W {
                assert_eq!(
                    popup_hit(PublicRecordingState::Finalizing, false, x as f64, y as f64),
                    None
                );
            }
        }
    }

    #[test]
    fn recording_and_paused_share_the_pause_resume_cell() {
        let (x, y, w, h) = pause_resume_rect();
        let point = (x as f64 + w as f64 / 2.0, y as f64 + h as f64 / 2.0);
        assert_eq!(
            popup_hit(PublicRecordingState::Recording, true, point.0, point.1),
            Some(PopupButton::PauseResume)
        );
        assert_eq!(
            popup_hit(PublicRecordingState::Paused, true, point.0, point.1),
            Some(PopupButton::PauseResume)
        );
    }

    #[test]
    fn disabled_popup_rejects_every_button() {
        let (x, y, w, h) = save_shelf_rect();
        assert_eq!(
            popup_hit(
                PublicRecordingState::Paused,
                false,
                (x + w / 2.0) as f64,
                (y + h / 2.0) as f64,
            ),
            None
        );
    }

    #[test]
    fn elapsed_formats_mmss() {
        assert_eq!(fmt_elapsed(0), "00:00");
        assert_eq!(fmt_elapsed(75), "01:15");
        assert_eq!(fmt_elapsed(7000), "99:59");
    }
}
