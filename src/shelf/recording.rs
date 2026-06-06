//! Area-recording lifecycle for the shelf daemon: the state owned while a
//! `wf-recorder` child is running, the two overlay surfaces (a click-through
//! region marker + a control indicator), and the pure layout/draw helpers for
//! the indicator (●+MM:SS+Stop, then Confirm/Cancel).
//!
//! The live lifecycle (spawning the child, SIGINT, off-thread `wait()`/ffmpeg,
//! finalizing the finished mp4 into a card) is driven from `shelf/mod.rs`; this module keeps
//! the deterministic, testable pieces (indicator geometry, hit-testing, drawing
//! into a premultiplied-BGRA canvas) separate.

use smithay_client_toolkit::{
    compositor::Region, shell::wlr_layer::LayerSurface, shm::slot::SlotPool,
};

use crate::record::Geometry;

/// Indicator surface size (logical px). Fixed; both phases draw inside it.
pub const IND_W: u32 = 124;
pub const IND_H: u32 = 34;

/// Thickness of the click-through region marker frame, its corner radius, and how
/// far the marker surface is inflated past the recorded rect on each side. The
/// inflate is the radius (not the border) so the rounded corners sit fully OUTSIDE
/// the recording and are never captured; on the straight edges the frame floats a
/// small `radius - border` gap off the rect.
pub const MARKER_BORDER: u32 = 2;
pub const MARKER_RADIUS: u32 = 6;
pub const MARKER_INFLATE: u32 = MARKER_RADIUS;

/// Which controls the indicator currently shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecPhase {
    /// Recording in progress: ● + MM:SS + Stop (■).
    Recording,
    /// Stopped, awaiting the user: Confirm (✓) / Cancel (✕).
    Stopped,
}

/// A button the user can click on the indicator surface.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum IndButton {
    Stop,
    Confirm,
    Cancel,
}

/// All in-flight recording state. Dropping it unmaps both overlay surfaces and
/// (because `Region` destroys on drop) the marker's input region.
pub struct Recording {
    /// The `wf-recorder` child. `Some` while we still own it; on Confirm/Cancel
    /// it is `take()`n and reaped on a detached thread (the daemon never blocks
    /// on `wait()`). After Stop, SIGINT has been sent but the child is left here
    /// until Confirm/Cancel so the consumer can `wait()` for the finalized mp4.
    pub child: Option<std::process::Child>,
    /// Temp `.mp4` wf-recorder writes to.
    pub path: std::path::PathBuf,
    /// When recording began (drives the MM:SS readout).
    pub started: std::time::Instant,
    /// Region in compositor-global coords (for the marker surface geometry).
    pub geo: Geometry,

    /// Click-through frame surface, just outside the recorded rect. Dropped when
    /// the user hits Stop (no point framing a finished recording).
    pub marker: Option<LayerSurface>,
    pub marker_pool: Option<SlotPool>,
    /// Empty `wl_region` set as the marker's input region (= clicks pass through).
    /// MUST be retained: `Region` destroys the underlying region on drop.
    pub marker_region: Option<Region>,
    pub marker_configured: bool,

    /// Control indicator surface. `None` for fullscreen recordings, which show no
    /// overlay (it would be captured into the full-screen video).
    pub indicator: Option<LayerSurface>,
    pub indicator_pool: Option<SlotPool>,
    pub indicator_configured: bool,

    pub phase: RecPhase,
    /// Last whole-second value drawn, so the 1s tick only repaints on change.
    pub last_drawn_secs: Option<u64>,
    /// True for fullscreen recordings: there is no Confirm/Cancel step, so a Stop
    /// (keyboard) finalizes straight into a card.
    pub auto_confirm: bool,
}

/// `(secs) -> "MM:SS"`, clamped sensibly (caps at 99:59 for layout sanity).
pub fn fmt_elapsed(secs: u64) -> String {
    let secs = secs.min(99 * 60 + 59);
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

/// Hit-test a click at indicator-local (x, y) for the current phase. Returns the
/// button under the cursor, if any.
pub fn ind_hit(phase: RecPhase, x: f64, y: f64) -> Option<IndButton> {
    let inside = |bx: f32, by: f32, bw: f32, bh: f32| {
        x >= bx as f64 && x < (bx + bw) as f64 && y >= by as f64 && y < (by + bh) as f64
    };
    match phase {
        RecPhase::Recording => {
            let (bx, by, bw, bh) = stop_btn_rect();
            inside(bx, by, bw, bh).then_some(IndButton::Stop)
        }
        RecPhase::Stopped => {
            let (cx, cy, cw, ch) = confirm_btn_rect();
            if inside(cx, cy, cw, ch) {
                return Some(IndButton::Confirm);
            }
            let (xx, xy, xw, xh) = cancel_btn_rect();
            inside(xx, xy, xw, xh).then_some(IndButton::Cancel)
        }
    }
}

/// Stop (■) button cell on the Recording-phase indicator: a square on the right.
pub fn stop_btn_rect() -> (f32, f32, f32, f32) {
    let s = 22.0;
    let x = IND_W as f32 - s - 8.0;
    let y = (IND_H as f32 - s) / 2.0;
    (x, y, s, s)
}

/// Confirm (✓) button cell: the left half of the Stopped-phase indicator.
pub fn confirm_btn_rect() -> (f32, f32, f32, f32) {
    let h = IND_H as f32 - 12.0;
    (8.0, 6.0, IND_W as f32 / 2.0 - 12.0, h)
}

/// Cancel (✕) button cell: the right half of the Stopped-phase indicator.
pub fn cancel_btn_rect() -> (f32, f32, f32, f32) {
    let h = IND_H as f32 - 12.0;
    (IND_W as f32 / 2.0 + 4.0, 6.0, IND_W as f32 / 2.0 - 12.0, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elapsed_formats_mmss() {
        assert_eq!(fmt_elapsed(0), "00:00");
        assert_eq!(fmt_elapsed(9), "00:09");
        assert_eq!(fmt_elapsed(75), "01:15");
        assert_eq!(fmt_elapsed(3599), "59:59");
        assert_eq!(fmt_elapsed(7000), "99:59"); // capped
    }

    #[test]
    fn recording_phase_hits_only_stop() {
        let (bx, by, bw, bh) = stop_btn_rect();
        let (cx, cy) = ((bx + bw / 2.0) as f64, (by + bh / 2.0) as f64);
        assert_eq!(ind_hit(RecPhase::Recording, cx, cy), Some(IndButton::Stop));
        // The dot/time area on the far left is not a button.
        assert_eq!(ind_hit(RecPhase::Recording, 6.0, 22.0), None);
        // Confirm/Cancel are not present in the Recording phase.
        let (fx, fy, fw, fh) = confirm_btn_rect();
        assert_eq!(
            ind_hit(
                RecPhase::Recording,
                (fx + fw / 2.0) as f64,
                (fy + fh / 2.0) as f64
            ),
            // left-half overlaps neither the dot logic nor stop; ensure it's not
            // mistaken for a Stop hit.
            None
        );
    }

    #[test]
    fn stopped_phase_splits_confirm_cancel() {
        let (cx, cy, cw, ch) = confirm_btn_rect();
        assert_eq!(
            ind_hit(
                RecPhase::Stopped,
                (cx + cw / 2.0) as f64,
                (cy + ch / 2.0) as f64
            ),
            Some(IndButton::Confirm)
        );
        let (xx, xy, xw, xh) = cancel_btn_rect();
        assert_eq!(
            ind_hit(
                RecPhase::Stopped,
                (xx + xw / 2.0) as f64,
                (xy + xh / 2.0) as f64
            ),
            Some(IndButton::Cancel)
        );
        // Outside both -> None.
        assert_eq!(ind_hit(RecPhase::Stopped, 0.0, 0.0), None);
    }
}
