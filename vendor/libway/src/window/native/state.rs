//! One native attempt per host and one real frame callback per retained window.
use super::NativePresenter;
use crate::{
    Error, Result,
    window::{Configuration, ConfigurationRevision, WindowId, state::WindowState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::window) struct NativeFrameToken {
    pub(in crate::window) window: WindowId,
    pub(in crate::window) sequence: u64,
}

pub(in crate::window) struct NativePacing {
    pub(in crate::window) pending: Option<NativeFrameToken>,
    pub(in crate::window) next_sequence: u64,
}

impl Default for NativePacing {
    fn default() -> Self {
        Self {
            pending: None,
            next_sequence: 1,
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::window) struct Attempt {
    pub(in crate::window) window: WindowId,
    staged: bool,
}

pub(super) enum Admission {
    Ready(Configuration),
    NeedsFrame,
    Suspended,
}

impl WindowState {
    pub(super) fn begin_native(
        &mut self,
        presenter: &NativePresenter,
        revision: ConfigurationRevision,
    ) -> Result<Admission> {
        self.ensure_active()?;
        let record = self.record(presenter.id())?;
        if presenter.is_retired() {
            return Err(Error::InvalidInput("native presenter is retired"));
        }
        let configuration = record.accepted.ok_or(Error::NotReady)?;
        if configuration.revision != revision {
            return Err(Error::InvalidInput(
                "native configuration revision is stale",
            ));
        }
        if configuration.suspended {
            return Ok(Admission::Suspended);
        }
        if record.native_pacing.pending.is_some() {
            return Ok(Admission::NeedsFrame);
        }
        record
            .native_pacing
            .next_sequence
            .checked_add(1)
            .ok_or(Error::LimitExceeded)?;
        // Reserve input cancellations and Closed within the same event cap.
        // While this owner phase is open no dispatch or other mutation may
        // increase that cancellation count. Ordinary
        // borrowing enforces that; the retained marker also covers forgotten guards.
        if !self.event_room(1) {
            return Err(Error::LimitExceeded);
        }
        self.native_attempt = Some(Attempt {
            window: presenter.id(),
            staged: false,
        });
        Ok(Admission::Ready(configuration))
    }

    pub(super) fn stage_native(
        &mut self,
        presenter: &NativePresenter,
        configuration: Configuration,
        extent: (u32, u32),
    ) -> Result<()> {
        self.ensure_live()?;
        let id = presenter.id();
        let attempt = self
            .native_attempt
            .ok_or(Error::InvalidInput("no native frame attempt"))?;
        if attempt.window != id || attempt.staged {
            return Err(Error::InvalidInput(
                "native frame already staged or foreign",
            ));
        }
        if presenter.is_retired() || self.record(id)?.accepted != Some(configuration) {
            return Err(Error::InvalidInput(
                "native frame configuration is no longer current",
            ));
        }
        if extent != (configuration.pixel_width, configuration.pixel_height) {
            return Err(Error::InvalidDimensions);
        }
        let record = self
            .windows
            .get_mut(&id)
            .ok_or(Error::InvalidInput("native window is retired"))?;
        let sequence = record.native_pacing.next_sequence;
        let next = sequence.checked_add(1).ok_or(Error::LimitExceeded)?;
        let token = NativeFrameToken {
            window: id,
            sequence,
        };
        // Mark before the first request: an error or unwind after staging cannot
        // make this surface reusable by pretending an uncancellable callback fired.
        self.native_attempt = Some(Attempt {
            window: id,
            staged: true,
        });
        record.native_pacing.next_sequence = next;
        record.native_pacing.pending = Some(token);
        presenter.lease.configure_surface(configuration)?;
        presenter.lease.surface().frame(&self.qh, token);
        // Feedback attaches to the driver's commit, the next one on this surface.
        self.presentation
            .consume(id, presenter.lease.surface(), &self.qh, &mut self.events);
        Ok(())
    }

    pub(super) fn finish_native(&mut self, id: WindowId, scheduled: bool) -> Result<()> {
        self.ensure_live()?;
        let attempt = self
            .native_attempt
            .ok_or(Error::InvalidInput("no native frame attempt"))?;
        if attempt.window != id {
            return Err(Error::InvalidInput(
                "native attempt belongs to another window",
            ));
        }
        if scheduled && !attempt.staged {
            return Err(Error::InvalidInput("native frame was not staged"));
        }
        if !scheduled && attempt.staged {
            // close reserves Closed before changing ownership. Admission already
            // checked a slot and no native event dispatch ran during this borrow.
            self.close(id)?;
        }
        self.native_attempt = None;
        Ok(())
    }

    pub(in crate::window) fn native_frame_done(
        &mut self,
        token: NativeFrameToken,
    ) -> Option<WindowId> {
        let record = self.windows.get_mut(&token.window)?;
        if record.native_pacing.pending != Some(token) {
            return None;
        }
        record.native_pacing.pending = None;
        Some(token.window)
    }
}
