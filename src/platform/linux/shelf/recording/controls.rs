//! The recording controls: the marker around what is recorded and the popup
//! with its buttons, keys and discard confirmation.

use smithay_client_toolkit::compositor::Region;
use smithay_client_toolkit::seat::keyboard::Keysym;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, KeyboardInteractivity, Layer};
use smithay_client_toolkit::shm::slot::SlotPool;
use wayland_client::QueueHandle;
use wayland_client::protocol::wl_surface::WlSurface;

use crate::record::session::{CaptureScope, PublicRecordingState, RecordingAction, SessionPhase};
use crate::shelf::recording::PopupButton;

use super::placement::monitor_for_geometry;
use super::{Daemon, font, notify};

pub(super) fn recording_action_label(action: RecordingAction) -> &'static str {
    match action {
        RecordingAction::Pause => "pause",
        RecordingAction::Resume => "resume",
        RecordingAction::SaveShelf => "save to shelf",
        RecordingAction::SaveDisk => "save to disk",
        RecordingAction::Discard => "discard",
    }
}

pub(super) fn recording_controls_visible(phase: SessionPhase) -> bool {
    phase != SessionPhase::Discarding
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ControlsOpen {
    /// Recording: pause, show the controls when the recorders have stopped.
    PauseFirst,
    /// A pause is already stopping the recorders.
    AfterPause,
    /// Nothing is being recorded.
    Now,
}

pub(super) fn controls_open_plan(phase: SessionPhase) -> ControlsOpen {
    match phase {
        SessionPhase::Recording => ControlsOpen::PauseFirst,
        SessionPhase::Pausing => ControlsOpen::AfterPause,
        _ => ControlsOpen::Now,
    }
}

/// Esc continues a recording only when the controls paused it themselves.
pub(super) fn esc_resumes(auto_paused: bool, phase: SessionPhase, resume_blocked: bool) -> bool {
    auto_paused && phase == SessionPhase::Paused && !resume_blocked
}

pub(super) const DISCARD_CONFIRM: std::time::Duration = std::time::Duration::from_secs(3);

pub(super) fn discard_confirmed(
    armed: Option<std::time::Instant>,
    now: std::time::Instant,
) -> bool {
    armed.is_some_and(|at| now.saturating_duration_since(at) < DISCARD_CONFIRM)
}

/// Popup keys: Space pauses or resumes, Enter saves to the shelf, D to disk,
/// Delete discards (with confirmation).
pub(super) fn popup_key_button(keysym: Keysym) -> Option<PopupButton> {
    match keysym {
        Keysym::space => Some(PopupButton::PauseResume),
        Keysym::Return | Keysym::KP_Enter => Some(PopupButton::SaveShelf),
        Keysym::d | Keysym::D => Some(PopupButton::SaveDisk),
        Keysym::Delete | Keysym::KP_Delete => Some(PopupButton::Discard),
        _ => None,
    }
}

impl Daemon {
    /// Build the click-through marker layer surface (a `Top` layer with an EMPTY
    /// input region so clicks pass through to the apps being recorded). Returns
    /// the surface, its draw pool, and the region (retained so it isn't destroyed).
    ///
    /// The surface is created on the focused output (margins are per-output, so a
    /// null output would mis-place the marker on a non-origin monitor). The rect
    /// is in compositor-GLOBAL coords; we convert it to OUTPUT-LOCAL margins by
    /// subtracting that output's layout origin — the same origin `record_flow`
    /// added to globalize the selection, so the subtraction is exact.
    pub(super) fn create_marker(&mut self, geo: &crate::record::Geometry, qh: &QueueHandle<Self>) {
        use crate::shelf::recording::MARKER_INFLATE;
        let inflate = MARKER_INFLATE as i32;
        let mw = geo.w + 2 * MARKER_INFLATE;
        let mh = geo.h + 2 * MARKER_INFLATE;

        let monitor = self
            .cached_monitors()
            .into_iter()
            .find(|monitor| monitor_for_geometry(std::slice::from_ref(monitor), geo).is_some());
        let (output_name, ox, oy) = monitor
            .map(|monitor| (Some(monitor.name), monitor.x, monitor.y))
            .unwrap_or((None, 0, 0));
        let surface = self.compositor.create_surface(qh);
        let (_name, output) = self.target_output(output_name.as_deref());
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some("boltsnap-recording"),
            output.as_ref(),
        );
        layer.set_anchor(Anchor::TOP | Anchor::LEFT);
        // Convert the GLOBAL rect to OUTPUT-LOCAL margins by subtracting the
        // focused monitor's layout origin (the inverse of record_flow's globalize),
        // then anchor top-left and push to the rect via margins (inflated by border).
        let local_x = geo.x - ox;
        let local_y = geo.y - oy;
        layer.set_margin((local_y - inflate).max(0), 0, 0, (local_x - inflate).max(0));
        layer.set_size(mw, mh);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(-1);

        // EMPTY input region == clicks pass through. `Region::new` with no `add`
        // produces an empty region; retain it (it self-destroys on drop).
        let region = Region::new(&self.compositor).ok();
        if let Some(reg) = region.as_ref() {
            layer.set_input_region(Some(reg.wl_region()));
        } else {
            eprintln!("boltsnap daemon: could not create empty input region for marker");
        }
        layer.commit();

        self.marker = Some(layer);
        self.marker_pool = SlotPool::new((mw * mh * 4) as usize, &self.shm).ok();
        self.marker_region = region;
        self.marker_configured = false;
    }

    fn create_popup(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        // fontconfig and the font file cost ~10 ms; reload only when `ui_font` changes.
        let family = crate::config::Config::load().ui_font;
        if self.popup_font_family.as_ref() != Some(&family) {
            self.popup_fonts = font::load_ui_font_weights(family.as_deref(), (500.0, 600.0));
            self.popup_layout = crate::shelf::paint::controls::controls_layout(&self.popup_fonts.0);
            self.popup_font_family = Some(family);
        }
        let (width, height) = (self.popup_layout.width, self.popup_layout.height);
        let pool = SlotPool::new((width * height * 4) as usize, &self.shm)
            .map_err(|error| format!("allocate recording controls: {error}"))?;
        let surface = self.compositor.create_surface(qh);
        let (_name, output) = self.target_output(self.focused_output.as_deref());
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Overlay,
            Some("boltsnap-recording-controls"),
            output.as_ref(),
        );
        layer.set_size(width, height);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        layer.set_exclusive_zone(-1);
        layer.commit();
        self.popup = Some(layer);
        self.popup_pool = Some(pool);
        self.popup_configured = false;
        Ok(())
    }

    /// Draw the click-through marker border into its surface.
    pub(super) fn draw_marker(&mut self) {
        if !self.marker_configured {
            return;
        }
        let geo = match self.recording.as_ref().map(|session| &session.scope) {
            Some(CaptureScope::Area(geo)) => *geo,
            _ => return,
        };
        let (layer, pool) = match (self.marker.as_ref(), self.marker_pool.as_mut()) {
            (Some(l), Some(p)) => (l, p),
            _ => return,
        };
        use crate::shelf::recording::MARKER_INFLATE;
        let mw = geo.w + 2 * MARKER_INFLATE;
        let mh = geo.h + 2 * MARKER_INFLATE;
        let stride = (mw * 4) as i32;
        let (buffer, canvas) = match pool.create_buffer(
            mw as i32,
            mh as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        crate::shelf::paint::draw_marker_border(
            canvas,
            mw,
            mh,
            crate::shelf::recording::MARKER_BORDER,
            crate::shelf::recording::MARKER_RADIUS,
        );
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, mw as i32, mh as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
    }

    pub(super) fn draw_popup(&mut self) {
        if !self.popup_configured {
            return;
        }
        let notice = self.pause_title;
        let (state, enabled, recovery, elapsed) = match self.recording.as_ref() {
            Some(session) => (
                session.public_state(),
                session.actions_enabled(),
                session.resume_blocked,
                crate::shelf::recording::fmt_elapsed(
                    session.elapsed_at(std::time::Instant::now()).as_secs(),
                ),
            ),
            None => return,
        };
        let (layer, pool) = match (self.popup.as_ref(), self.popup_pool.as_mut()) {
            (Some(l), Some(p)) => (l, p),
            _ => return,
        };
        let (width, height) = (self.popup_layout.width, self.popup_layout.height);
        let stride = (width * 4) as i32;
        let (buffer, canvas) = match pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wayland_client::protocol::wl_shm::Format::Argb8888,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        crate::shelf::paint::controls::draw_recording_controls(
            canvas,
            &self.popup_layout,
            state,
            enabled,
            recovery,
            notice,
            self.discard_armed.is_some(),
            &elapsed,
            (&self.popup_fonts.0, &self.popup_fonts.1),
        );
        let surface = layer.wl_surface();
        surface.damage_buffer(0, 0, width as i32, height as i32);
        let _ = buffer.attach_to(surface);
        layer.commit();
    }

    pub(super) fn show_recording_controls(&mut self, qh: &QueueHandle<Self>) -> Result<(), String> {
        if !self
            .recording
            .as_ref()
            .is_some_and(|session| recording_controls_visible(session.phase))
        {
            return Err("recording controls are unavailable".into());
        }
        let phase = self.recording.as_ref().map(|session| session.phase);
        match phase.map(controls_open_plan) {
            // Pause first: gsr reads the composed scanout, so the controls would
            // be recorded. They appear once the recorders have stopped.
            Some(ControlsOpen::PauseFirst) => {
                self.handle_recording_action(RecordingAction::Pause, qh)?;
                self.auto_paused = true;
                self.controls_pending = true;
            }
            Some(ControlsOpen::AfterPause) => self.controls_pending = true,
            Some(ControlsOpen::Now) | None => {
                if self.popup.is_none() {
                    self.create_popup(qh)?;
                } else {
                    self.draw_popup();
                }
            }
        }
        Ok(())
    }

    pub(super) fn remove_marker(&mut self) {
        self.marker = None;
        self.marker_pool = None;
        self.marker_region = None;
        self.marker_configured = false;
    }

    pub(super) fn remove_popup(&mut self) {
        self.discard_armed = None;
        self.popup = None;
        self.popup_pool = None;
        self.popup_configured = false;
    }

    pub(super) fn is_popup_surface(&self, surface: &WlSurface) -> bool {
        self.popup
            .as_ref()
            .map(|popup| popup.wl_surface() == surface)
            .unwrap_or(false)
    }

    /// True if `surface` is the active recording's click-through marker surface.
    pub(super) fn is_marker_surface(&self, surface: &WlSurface) -> bool {
        self.marker
            .as_ref()
            .map(|marker| marker.wl_surface() == surface)
            .unwrap_or(false)
    }

    pub(super) fn on_popup_click(&mut self, pos: (f64, f64)) {
        if let Some(button) = self.popup_layout.hit(pos.0, pos.1) {
            self.press_popup_button(button);
        }
    }

    pub(super) fn discard_confirm_remaining(&self) -> Option<std::time::Duration> {
        self.discard_armed
            .map(|at| DISCARD_CONFIRM.saturating_sub(at.elapsed()))
    }

    /// Discard falls back from Confirm once its window has passed.
    pub(super) fn expire_discard_confirm(&mut self) {
        if self
            .discard_armed
            .is_some_and(|at| !discard_confirmed(Some(at), std::time::Instant::now()))
        {
            self.discard_armed = None;
            self.draw_popup();
        }
    }

    /// A popup button pressed by click or key. Discard only acts on a second
    /// press within `DISCARD_CONFIRM`; any other button disarms it.
    pub(super) fn press_popup_button(&mut self, button: PopupButton) {
        let Some(session) = self.recording.as_ref() else {
            return;
        };
        let state = session.public_state();
        if !session.actions_enabled()
            || matches!(
                state,
                PublicRecordingState::Idle | PublicRecordingState::Finalizing
            )
            || (button == PopupButton::PauseResume && session.resume_blocked)
        {
            return;
        }
        let now = std::time::Instant::now();
        let confirmed = discard_confirmed(self.discard_armed.take(), now);
        let action = match button {
            PopupButton::PauseResume if state == PublicRecordingState::Paused => {
                RecordingAction::Resume
            }
            PopupButton::PauseResume => RecordingAction::Pause,
            PopupButton::SaveShelf => RecordingAction::SaveShelf,
            PopupButton::SaveDisk => RecordingAction::SaveDisk,
            PopupButton::Discard if confirmed => RecordingAction::Discard,
            PopupButton::Discard => {
                self.discard_armed = Some(now);
                self.draw_popup();
                return;
            }
        };
        if let Some(qh) = self.qh.clone()
            && let Err(error) = self.handle_recording_action(action, &qh)
        {
            notify(&format!("Recording: {error}"));
        }
        self.draw_popup();
    }
}
