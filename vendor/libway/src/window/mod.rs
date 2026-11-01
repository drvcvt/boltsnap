//! Caller-owned native windows with bounded commands and retained native leases.
//! No background reader is started. Call [`Host::pump`] on the application thread,
//! drain owned events, and decide whether compositor close requests should close.
//!
//! Native input binds at most eight seats, capped at version nine, and retains at
//! most 64 keyboard/pointer objects including legacy objects without release.
//! The 1024-event cap includes protected focus-cancellation, staged-frame and
//! clipboard terminal slots. Clipboard pipes join this host's existing wait.
//! Variable payload limits and shared clone accounting are documented on [`Keymap`],
//! [`HeldKeys`], [`SeatName`] and [`PointerFrame`]. Missing seats permit output-only use.
#[path = "shell/attention.rs"]
mod attention;
mod clipboard;
#[path = "shell/configure.rs"]
mod configure;
#[path = "shell/decoration.rs"]
mod decoration;
mod input;
#[path = "shell/layer.rs"]
mod layer;
#[path = "shell/layer_protocol.rs"]
mod layer_protocol;
mod lease;
#[path = "shell/manage.rs"]
mod manage;
#[cfg(feature = "owned-native-system")]
mod native;
mod output;
#[path = "shell/protocol.rs"]
mod protocol;
mod proxy;
#[path = "shell/region.rs"]
mod region;
mod scale;
mod state;
#[cfg(test)]
mod tests;
mod text_input;
#[path = "shell/toplevel.rs"]
mod toplevel;
#[path = "shell/types.rs"]
mod types;

use crate::{
    Result,
    runtime::{Coordinator, mailbox::Limits},
};
pub use clipboard::*;
pub use input::{
    AxisDirection, AxisSource, CursorShape, HeldKeys, InputCancelReason, InputDevice, InputEvent,
    InputObservation, InputSeatId, KeyboardObservation, Keymap, PointerAxis, PointerFrame,
    PointerObservation, SeatName,
};
pub use layer::{
    Anchor, Edge, ExclusiveZone, KeyboardInteractivity, Layer, LayerOptions, LayerState,
};
pub use lease::WindowLease;
pub use manage::ResizeEdge;
#[cfg(feature = "owned-native-system")]
pub use native::{NativeFrame, NativeFrameStart, NativePresenter};
pub use output::{
    PixelRect, PresentOutcome, PresentationFlags, PresentationOutcome, PresentationTiming,
};
pub use proxy::{CommandOutcome, CommandSendError, HostWake, WindowProxy};
use proxy::{HostCommand, WakeState};
pub use region::{InputRegion, LogicalRect};
pub use scale::{OutputId, OutputInfo};
use state::WindowState;
use std::{os::unix::net::UnixStream, sync::Arc, time::Instant};
pub use text_input::{
    ImeCause, ImeEvent, ImePreedit, ImePreeditCursor, ImeRectangle, ImeSession, ImeState,
    ImeSurrounding, ImeText, ImeTransaction,
};
pub use toplevel::SizeLimits;
pub use types::*;

const COMMAND_LIMITS: Limits = Limits {
    max_queued: 256,
    max_bytes: 8 * 1024 * 1024,
};
const COMMAND_BATCH: usize = 64;

/// Exclusive connection owner. Native dispatch never invokes application callbacks.
/// This initial window boundary owns roles and configuration; presentation is separate.
pub struct Host {
    coordinator: Coordinator<WindowState, HostCommand>,
    wake: Arc<WakeState>,
}

/// Work completed in one bounded command turn. Protocol dispatch may process more
/// callbacks than the application event capacity, and is not a time budget.
#[derive(Debug)]
pub struct PumpResult {
    /// Applied commands, in admission order, including individual validation errors.
    pub commands: Vec<CommandOutcome>,
    /// Owned events available through [`Host::events`].
    pub events_pending: usize,
    /// Retained work or a cancelled prepare requires another turn without sleeping.
    pub runnable: bool,
}

/// Terminal pump failure with results of commands already applied in this turn.
/// Those commands must not be retried as if they had never been delivered.
#[derive(Debug)]
pub struct PumpError {
    /// The failure that stopped native servicing.
    pub error: crate::Error,
    /// Results retained even when a later command or native operation fails.
    pub commands: Vec<CommandOutcome>,
}

impl std::fmt::Display for PumpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.error.fmt(f)
    }
}

impl std::error::Error for PumpError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

/// Shutdown does not block waiting for application-held leases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    /// Native windows still retained by leases after logical retirement.
    pub outstanding_windows: usize,
}

impl Host {
    /// Connect to the selected Wayland display. Discovery completes asynchronously.
    pub fn connect() -> Result<Self> {
        let coordinator = Coordinator::connect(COMMAND_LIMITS, WindowState::new)?;
        Ok(Self {
            wake: WakeState::new(coordinator.sender()),
            coordinator,
        })
    }

    /// Own a supplied Wayland socket, useful for explicitly selected displays.
    pub fn from_socket(socket: UnixStream) -> Result<Self> {
        let coordinator = Coordinator::from_socket(socket, COMMAND_LIMITS, WindowState::new)?;
        Ok(Self {
            wake: WakeState::new(coordinator.sender()),
            coordinator,
        })
    }

    /// Obtain a bounded send-only handle suitable for worker threads.
    pub fn proxy(&self) -> WindowProxy {
        WindowProxy::new(self.coordinator.sender())
    }

    /// Obtain a weak worker wake value. It retains no display or event owner.
    pub fn wake(&self) -> HostWake {
        self.wake.handle()
    }

    /// Apply at most 64 admitted commands, then service native work. Wait at most
    /// until the absolute deadline only when no command result or event is pending.
    pub fn pump(
        &mut self,
        deadline: Option<Instant>,
    ) -> std::result::Result<PumpResult, PumpError> {
        #[cfg(feature = "owned-native-system")]
        if self.coordinator.state.native_attempt.is_some() {
            return Err(PumpError {
                error: crate::Error::InvalidInput("native frame attempt remains open"),
                commands: Vec::new(),
            });
        }
        let mut commands = Vec::new();
        let mut applied = false;
        for _ in 0..COMMAND_BATCH {
            let command = match self.coordinator.command() {
                Ok(Some(command)) => command,
                Ok(None) => break,
                Err(error) => return Err(PumpError { error, commands }),
            };
            applied = true;
            if let Some(outcome) = command.apply(|command| self.apply(command)) {
                commands.push(outcome);
            }
        }
        let result = if applied {
            self.coordinator.dispatch()
        } else {
            self.coordinator.step(deadline)
        };
        let turn = match result {
            Ok(turn) => turn,
            Err(error) => return Err(PumpError { error, commands }),
        };
        Ok(PumpResult {
            commands,
            events_pending: turn.events,
            runnable: turn.runnable(),
        })
    }

    /// Drain owned notifications. Retained events remain accessible after failure.
    pub fn events(&mut self) -> Vec<WindowEvent> {
        let events = self.coordinator.state.events();
        if events
            .iter()
            .any(|event| matches!(event, WindowEvent::Wake))
            && let Err(error) = self.wake.collected()
        {
            self.coordinator.state.remember(&error);
        }
        events
    }

    /// Add a fresh coordinated DnD participant. Registry readiness is asynchronous;
    /// no standalone reader or mutable participant escapes from this host.
    #[cfg(feature = "dnd")]
    pub fn enable_dnd(&mut self, options: crate::dnd::DndOptions) -> Result<()> {
        self.coordinator.enable_dnd(options)
    }

    /// Drain owned DnD events, including outcomes retained before a terminal failure.
    #[cfg(feature = "dnd")]
    pub fn dnd_events(&mut self) -> Vec<crate::dnd::DndEvent> {
        self.coordinator.dnd_events()
    }

    /// Create after [`WindowEvent::Ready`]. Invalid options issue no native requests.
    pub fn create_window(&mut self, options: WindowOptions) -> Result<WindowId> {
        self.coordinator
            .mutate(|state| state.create_window(options))
    }

    /// Retain every native dependency of a live window without exposing proxies.
    pub fn lease(&mut self, window: WindowId) -> Result<WindowLease> {
        self.coordinator.mutate(|state| state.lease(window))
    }

    /// Read the last complete acknowledged configuration, if one has arrived.
    pub fn configuration(&mut self, window: WindowId) -> Result<Option<Configuration>> {
        self.coordinator.mutate(|state| state.configuration(window))
    }

    /// Retire an identity. Repeated close of an already retired own identity succeeds.
    pub fn close(&mut self, window: WindowId) -> Result<()> {
        self.coordinator.mutate(|state| state.close(window))
    }

    /// Validate and change a live window title on the owner thread.
    pub fn set_title(&mut self, window: WindowId, title: String) -> Result<()> {
        self.coordinator
            .mutate(|state| state.set_title(window, title))
    }

    /// Install eager MIME bytes using this seat's latest actual press in its
    /// current keyboard-focus epoch. Sources survive focus loss and window close.
    ///
    /// Retains immutable Arc payloads before replacing the prior source. Admits
    /// 1..=8 representations with distinct MIME names of at most 256 bytes and 2 MiB
    /// aggregate advertised bytes. Aliases share one storage charge. Sources,
    /// active reads and queued bytes share 32 MiB of host credit. At most 16
    /// nonblocking sends run at once, each with five seconds from its Send event.
    /// Protocol dispatch invokes no application callback.
    ///
    /// Invalid targets, focus or MIME data return InvalidInput; no eligible real
    /// press in this focus epoch returns NotReady. Absent or removed manager
    /// returns Unsupported, remaining unavailable until a new Host. LimitExceeded
    /// admission preserves the prior source. Replacement or compositor retirement
    /// publishes SourceCancelled; already accepted sends retain their exact bytes.
    pub fn set_clipboard(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        representations: &[ClipboardRepresentation],
    ) -> Result<ClipboardSourceId> {
        self.coordinator
            .mutate(|state| state.set_clipboard(window, seat, representations))
    }

    /// Validate clipboard capability and current keyboard focus, then return the
    /// seat's bounded current offer. Metadata may precede keyboard Enter.
    ///
    /// Owns at most 32 MIME names of 256 bytes each. No selection returns None;
    /// existing metadata survives same-client focus switches. A retired/wrong
    /// target, seat or focus returns InvalidInput. An absent or removed manager
    /// returns Unsupported; removal remains unavailable until a new Host.
    /// This accessor issues no protocol request and starts no receive.
    pub fn clipboard_offer(
        &self,
        window: WindowId,
        seat: InputSeatId,
    ) -> Result<Option<ClipboardOffer>> {
        if self.coordinator.failure().is_some() {
            return Err(crate::Error::SessionStopped);
        }
        self.coordinator.state.clipboard_offer(window, seat)
    }

    /// Install an eager primary selection source (middle-click paste) for the
    /// current keyboard-focus epoch, with the same admission, limits, serial rule,
    /// budget and errors as [`Host::set_clipboard`]. The seat's clipboard source
    /// is independent: replacing one never cancels the other.
    ///
    /// An absent or removed `zwp_primary_selection_device_manager_v1` returns
    /// Unsupported, remaining unavailable until a new Host. Replacement or
    /// compositor retirement publishes SourceCancelled.
    pub fn set_primary_selection(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        representations: &[ClipboardRepresentation],
    ) -> Result<ClipboardSourceId> {
        self.coordinator
            .mutate(|state| state.set_primary_selection(window, seat, representations))
    }

    /// Return the seat's current primary offer under the focus and capability
    /// rules of [`Host::clipboard_offer`]. Read it with [`Host::receive_clipboard`].
    /// This accessor issues no protocol request and starts no receive.
    pub fn primary_selection_offer(
        &self,
        window: WindowId,
        seat: InputSeatId,
    ) -> Result<Option<ClipboardOffer>> {
        if self.coordinator.failure().is_some() {
            return Err(crate::Error::SessionStopped);
        }
        self.coordinator.state.primary_selection_offer(window, seat)
    }

    /// Admit a bounded asynchronous read of one exact current offer and MIME,
    /// either a clipboard or a primary offer. Successful admission supersedes the
    /// seat's prior read of either kind with Cancelled.
    ///
    /// Validates the exact current offer, advertised MIME and live keyboard focus.
    /// See ClipboardRead for 1 MiB payload and future 30-second deadline bounds.
    /// Reserves the maximum bytes plus one overflow byte and terminal event slot
    /// before admission, within eight active reads and the shared 32 MiB credit.
    /// LimitExceeded leaves a prior read unchanged, including at eight active reads.
    ///
    /// Bytes arrive only after EOF; no partial payload is delivered. The existing
    /// pump/wait services bounded nonblocking pipes. Selection changes cannot
    /// switch an admitted payload. Focus/window/seat retirement cancels it. An
    /// accepted read publishes one Received outcome unless the host terminates;
    /// queued payload credit retires on events collection or terminal teardown.
    /// Stale offers, unadvertised MIME or invalid targets/bounds return InvalidInput;
    /// a missing/removed manager returns Unsupported.
    pub fn receive_clipboard(&mut self, read: ClipboardRead) -> Result<ClipboardTransferId> {
        self.coordinator
            .mutate(|state| state.receive_clipboard(read))
    }

    /// Retire an active own read. Completed own identities return false; foreign
    /// or never-issued identities reject without affecting another transfer.
    /// Active cancellation publishes one Received(Cancelled) using the reserved
    /// slot. Queued outcomes remain available for collection and are not rewritten.
    /// Invalid identities return InvalidInput; stopped hosts return SessionStopped.
    pub fn cancel_clipboard(&mut self, transfer: ClipboardTransferId) -> Result<bool> {
        self.coordinator
            .mutate(|state| state.cancel_clipboard(transfer))
    }

    /// Activate text-input-v3 after both exact keyboard and text-input focus.
    /// State validates before replacing a producer; no surface commit is issued.
    pub fn enable_ime(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        state: ImeState<'_>,
    ) -> Result<ImeSession> {
        self.coordinator
            .mutate(|owner| owner.enable_ime(window, seat, state))
    }

    /// Update a complete state for the exact activation. Unsynchronized Done
    /// blocks retain only the newest bounded update until a matching count arrives.
    pub fn update_ime(&mut self, session: ImeSession, state: ImeState<'_>) -> Result<()> {
        self.coordinator
            .mutate(|owner| owner.update_ime(session, state))
    }

    /// Disable an active own association, publishing its reserved Ended event.
    /// Repeated disable of a retired own identity returns false.
    pub fn disable_ime(&mut self, session: ImeSession) -> Result<bool> {
        self.coordinator.mutate(|owner| owner.disable_ime(session))
    }

    /// Claim this window's single presenter for owned shared-memory CPU output.
    pub fn enable_cpu(&mut self, window: WindowId) -> Result<()> {
        self.coordinator.mutate(|state| state.enable_cpu(window))
    }

    /// Present tightly packed premultiplied RGBA8 for the latest configuration.
    /// Busy buffers and frame pacing return a retry reason without changing pixels.
    pub fn present_cpu(
        &mut self,
        window: WindowId,
        revision: ConfigurationRevision,
        rgba: &[u8],
    ) -> Result<PresentOutcome> {
        self.coordinator
            .mutate(|state| state.present_cpu(window, revision, rgba, None))
    }

    /// Like [`Host::present_cpu`], with `rgba` still the complete frame, but only
    /// `damage` (buffer pixels) changed since the last `Presented` frame. Rects are
    /// clamped; empty ones are ignored; none left is `InvalidInput`. More than 16
    /// collapse to their bounding box. Wrong damage shows stale pixels.
    pub fn present_cpu_damaged(
        &mut self,
        window: WindowId,
        revision: ConfigurationRevision,
        rgba: &[u8],
        damage: &[PixelRect],
    ) -> Result<PresentOutcome> {
        self.coordinator
            .mutate(|state| state.present_cpu(window, revision, rgba, Some(damage)))
    }

    /// Claim native interop exclusively. Create and use the cooperating GPU surface
    /// only between pump calls, never concurrently with connection read preparation.
    #[cfg(feature = "owned-native-system")]
    pub fn claim_native(&mut self, window: WindowId) -> Result<NativePresenter> {
        self.coordinator
            .mutate(|state| state.claim_presenter(window))
            .map(NativePresenter::new)
    }

    /// Begin a cooperating native frame for this claim's current revision.
    /// Readiness gates configure/acquire before any native requests; the returned
    /// guard exclusively borrows both host and claim until scheduling or cancellation.
    /// Keep the claim in an Arc owned by the driver surface when that surface needs
    /// static ownership. Never configure or acquire while suspended or NeedsFrame.
    /// A forgotten guard blocks pump until explicit close of its window or shutdown.
    #[cfg(feature = "owned-native-system")]
    pub fn begin_native_frame<'a>(
        &'a mut self,
        presenter: &'a NativePresenter,
        revision: ConfigurationRevision,
    ) -> Result<NativeFrameStart<'a>> {
        native::begin(self, presenter, revision)
    }

    /// Bounded first terminal cause, retained after subsequent stopped operations.
    pub fn failure(&self) -> Option<&str> {
        self.coordinator.failure()
    }

    /// Retire all identities and release the host without waiting for native leases.
    pub fn shutdown(mut self) -> ShutdownReport {
        ShutdownReport {
            outstanding_windows: self.coordinator.state.retire_all(),
        }
    }

    fn apply(&mut self, command: HostCommand) -> Option<CommandOutcome> {
        let command = match command {
            HostCommand::Window(command) => command,
            HostCommand::Wake => {
                self.coordinator.state.push(WindowEvent::Wake);
                return None;
            }
        };
        let window = command.window();
        let result = match command {
            WindowCommand::Close(id) => self.coordinator.state.close(id),
            WindowCommand::SetTitle(id, title) => self.coordinator.state.set_title(id, title),
        };
        Some(CommandOutcome { window, result })
    }
}
