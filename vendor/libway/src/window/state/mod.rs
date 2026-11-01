//! Caller-owned window records.
mod discovery;
mod lifecycle;
use super::{
    configure::{SuggestedConfiguration, validate_options, validate_title},
    input::{InputCancelReason, InputSeats},
    layer::{LayerOptions, LayerState, validate_namespace, validate_shape, validate_version},
    lease::{Globals, RoleRequest, WindowLease, WindowNative},
    output::{CpuFrame, CpuOutputs, PixelRect, PresentOutcome, Presentation},
    proxy::HostCommand,
    scale::{OutputScales, ScaleGlobals, WindowScale},
    types::{Configuration, ConfigurationRevision, WindowEvent, WindowId, WindowOptions},
};
use crate::{
    Error, Result,
    runtime::{
        QueueState,
        mailbox::{Failure, Notifier},
    },
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
use wayland_client::{
    Connection, Proxy, QueueHandle,
    protocol::{wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_shm::WlShm},
};
use wayland_protocols::xdg::shell::client::xdg_wm_base::XdgWmBase;

#[cfg(any(test, feature = "owned-native-system"))]
use super::lease::PresenterLease;

pub(super) const MAX_EVENTS: usize = 1024;
static CONNECTION_IDS: AtomicU64 = AtomicU64::new(1);

pub(super) struct WindowRecord {
    /// Declared before `native`, so it is destroyed before the surface.
    pub(super) inhibitor: Option<super::attention::Inhibitor>,
    pub(super) native: Arc<WindowNative>,
    pub(super) preferred: (u32, u32),
    pub(super) staged: SuggestedConfiguration,
    pub(super) accepted: Option<Configuration>,
    pub(super) next_revision: u64,
    pub(super) scale: WindowScale,
    /// The last requested layer state; `None` for a toplevel.
    pub(super) layer: Option<LayerState>,
    /// The last requested toplevel size limits.
    pub(super) limits: super::toplevel::SizeLimits,
    /// The parent window of a popup; `None` for toplevels and layers.
    pub(super) parent: Option<WindowId>,
    /// Popup state; `None` for toplevels and layers.
    pub(super) popup: Option<super::popup::PopupRecord>,
    /// A frame was presented (CPU or native), so popups may use it as parent.
    pub(super) mapped: bool,
    #[cfg(feature = "owned-native-system")]
    pub(super) native_pacing: super::native::state::NativePacing,
}

pub(super) struct Bootstrap {
    pub(super) registry: Option<WlRegistry>,
    pub(super) compositor: Option<(u32, WlCompositor)>,
    pub(super) shell: Option<(u32, XdgWmBase)>,
    pub(super) shm: Option<WlShm>,
    pub(super) scaling: ScaleGlobals,
    pub(super) decoration: super::decoration::DecorationGlobals,
    pub(super) layer_shell: super::layer_protocol::LayerShellGlobal,
    pub(super) attention: super::attention::AttentionGlobals,
    connection: Connection,
    notifier: Notifier<HostCommand>,
}

impl Drop for Bootstrap {
    fn drop(&mut self) {
        self.scaling.destroy();
        self.decoration.destroy();
        self.layer_shell.destroy();
        self.attention.destroy();
        if let Some((_, shell)) = &self.shell {
            shell.destroy();
        }
        // wl_registry has no wire destructor. Its retained proxy and server
        // object remain tied to the connection through bootstrap teardown.
        if self.shell.is_some() || self.registry.is_some() {
            let _ = self.connection.flush();
            let _ = self.notifier.notify();
        }
    }
}

pub(crate) struct WindowState {
    pub(super) connection: Connection,
    pub(super) qh: QueueHandle<Self>,
    notifier: Notifier<HostCommand>,
    pub(super) connection_id: u64,
    pub(super) bootstrap: Option<Bootstrap>,
    pub(super) globals: Option<Arc<Globals>>,
    pub(super) windows: BTreeMap<WindowId, WindowRecord>,
    pub(super) outputs: OutputScales,
    pub(super) output: CpuOutputs,
    pub(super) presentation: Presentation,
    pub(super) input: InputSeats,
    pub(super) clipboard: super::clipboard::Clipboard,
    pub(super) text_input: super::text_input::TextInput,
    pub(super) events: VecDeque<WindowEvent>,
    pub(super) tokens: super::attention::PendingTokens,
    #[cfg(feature = "owned-native-system")]
    pub(super) native_attempt: Option<super::native::state::Attempt>,
    next_window: u64,
    pub(super) create_allowed: bool,
    stopped: bool,
    failure: Option<Failure>,
}

impl WindowState {
    /// The toplevel or layer at the base of `id`'s popup chain; `id` itself
    /// for a toplevel, a layer or an unknown window.
    pub(in crate::window) fn tree_root(&self, id: WindowId) -> WindowId {
        let mut root = id;
        // A chain holds at most the window cap; the bound also stops a cycle.
        for _ in 0..super::lease::MAX_WINDOWS {
            match self.windows.get(&root).and_then(|record| record.parent) {
                Some(parent) => root = parent,
                None => break,
            }
        }
        root
    }

    pub(crate) fn new(
        connection: &Connection,
        qh: &QueueHandle<Self>,
        notifier: Notifier<HostCommand>,
    ) -> Self {
        let identity = CONNECTION_IDS
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1));
        let mut state = Self {
            connection: connection.clone(),
            qh: qh.clone(),
            notifier,
            connection_id: identity.unwrap_or(0),
            bootstrap: None,
            globals: None,
            windows: BTreeMap::new(),
            outputs: OutputScales::default(),
            output: CpuOutputs::default(),
            presentation: Presentation::default(),
            input: InputSeats::default(),
            clipboard: super::clipboard::Clipboard::default(),
            text_input: super::text_input::TextInput::default(),
            events: VecDeque::new(),
            tokens: Default::default(),
            #[cfg(feature = "owned-native-system")]
            native_attempt: None,
            next_window: 1,
            create_allowed: true,
            stopped: false,
            failure: None,
        };
        if identity.is_err() {
            state.remember(&Error::InvalidInput("connection identity exhausted"));
            return state;
        }
        if state.events.try_reserve_exact(MAX_EVENTS).is_err() {
            state.remember(&Error::LimitExceeded);
            return state;
        }
        state.begin_discovery();
        state
    }

    pub(crate) fn create_window(&mut self, options: WindowOptions) -> Result<WindowId> {
        self.ensure_active()?;
        validate_options(&options)?;
        let globals = self.creation_globals()?;
        let preferred = (options.preferred_width, options.preferred_height);
        let request = RoleRequest::Toplevel {
            title: options.title,
            app_id: options.app_id,
        };
        self.insert(globals, preferred, None, request)
    }

    /// Validate everything before any request, then create the layer role.
    pub(crate) fn create_layer(&mut self, options: LayerOptions) -> Result<WindowId> {
        self.ensure_active()?;
        validate_namespace(&options.namespace)?;
        validate_shape(&options.state)?;
        let globals = self.creation_globals()?;
        validate_version(&options.state, globals.layer_shell.available()?.version())?;
        let output = options
            .output
            .map(|id| self.outputs.announced(self.connection_id, id).cloned())
            .transpose()?;
        let state = options.state;
        // Layer configures replace 0 by the requested size; these values only
        // satisfy the shared record shape and are never selected.
        let preferred = (state.width.max(1), state.height.max(1));
        let request = RoleRequest::Layer {
            namespace: options.namespace,
            output: output.as_ref(),
            state: &state,
        };
        self.insert(globals, preferred, Some(state), request)
    }

    pub(in crate::window) fn creation_globals(&self) -> Result<Arc<Globals>> {
        if !self.create_allowed {
            return Err(Error::Unsupported("required window global was removed"));
        }
        Ok(self.globals.as_ref().ok_or(Error::NotReady)?.clone())
    }

    pub(in crate::window) fn insert(
        &mut self,
        globals: Arc<Globals>,
        preferred: (u32, u32),
        layer: Option<LayerState>,
        request: RoleRequest<'_>,
    ) -> Result<WindowId> {
        let next = self
            .next_window
            .checked_add(1)
            .ok_or(Error::LimitExceeded)?;
        let id = WindowId {
            connection: self.connection_id,
            sequence: self.next_window,
        };
        let result = WindowNative::create(globals, &self.qh, id, request);
        match result {
            Ok(native) => {
                self.next_window = next;
                self.windows.insert(
                    id,
                    WindowRecord {
                        inhibitor: None,
                        scale: native.scale_state(),
                        native,
                        preferred,
                        staged: SuggestedConfiguration::default(),
                        accepted: None,
                        next_revision: 1,
                        layer,
                        limits: Default::default(),
                        parent: None,
                        popup: None,
                        mapped: false,
                        #[cfg(feature = "owned-native-system")]
                        native_pacing: super::native::state::NativePacing::default(),
                    },
                );
                Ok(id)
            }
            Err(error) => {
                if matches!(error, Error::Wayland(_) | Error::Io(_)) {
                    self.remember(&error);
                }
                Err(error)
            }
        }
    }

    pub(crate) fn lease(&mut self, id: WindowId) -> Result<WindowLease> {
        self.ensure_active()?;
        Ok(self.record(id)?.native.lease())
    }

    pub(crate) fn configuration(&self, id: WindowId) -> Result<Option<Configuration>> {
        self.ensure_active()?;
        Ok(self.record(id)?.accepted)
    }

    pub(crate) fn set_title(&mut self, id: WindowId, title: String) -> Result<()> {
        self.ensure_active()?;
        validate_title(&title)?;
        self.record(id)?.native.toplevel()?.set_title(title);
        if let Some(error) = self.connection.backend().last_error() {
            let error = Error::Wayland(error.to_string());
            self.remember(&error);
            return Err(error);
        }
        Ok(())
    }

    #[cfg(any(test, feature = "owned-native-system"))]
    pub(crate) fn claim_presenter(&mut self, id: WindowId) -> Result<PresenterLease> {
        self.ensure_active()?;
        self.record(id)?.native.claim()
    }

    pub(crate) fn enable_cpu(&mut self, id: WindowId) -> Result<()> {
        self.ensure_active()?;
        self.record(id)?;
        if self
            .globals
            .as_ref()
            .is_none_or(|globals| globals.shm.is_none())
        {
            return Err(Error::Unsupported(
                "wl_shm is required for CPU presentation",
            ));
        }
        if self.output.contains(id) {
            return Err(Error::InvalidInput("CPU presenter is already enabled"));
        }
        let lease = self.record(id)?.native.claim()?;
        self.output.enable(id, lease)
    }

    pub(crate) fn present_cpu(
        &mut self,
        id: WindowId,
        revision: ConfigurationRevision,
        rgba: &[u8],
        damage: Option<&[PixelRect]>,
    ) -> Result<PresentOutcome> {
        self.ensure_active()?;
        let configuration = self.record(id)?.accepted.ok_or(Error::NotReady)?;
        let shm = self
            .globals
            .as_ref()
            .and_then(|globals| globals.shm.as_ref())
            .ok_or(Error::Unsupported(
                "wl_shm is required for CPU presentation",
            ))?;
        let result = self.output.present(
            id,
            CpuFrame {
                configuration,
                revision,
                rgba,
                damage,
            },
            shm,
            &self.qh,
            (&mut self.presentation, &mut self.events),
        );
        if let Err(error @ (Error::Io(_) | Error::Wayland(_))) = &result {
            self.remember(error);
        }
        if let Some(error) = self.connection.backend().last_error() {
            let error = Error::Wayland(error.to_string());
            self.remember(&error);
            return Err(error);
        }
        if matches!(result, Ok(PresentOutcome::Presented))
            && let Some(record) = self.windows.get_mut(&id)
        {
            record.mapped = true;
        }
        result
    }

    pub(crate) fn events(&mut self) -> Vec<WindowEvent> {
        let events = self.events.drain(..).collect();
        self.clipboard.drained();
        events
    }

    pub(crate) fn failure(&self) -> Option<&str> {
        self.failure.as_ref().map(Failure::as_str)
    }

    pub(super) fn active(&self) -> bool {
        !self.stopped && self.failure.is_none()
    }

    pub(super) fn ensure_active(&self) -> Result<()> {
        self.ensure_live()?;
        #[cfg(feature = "owned-native-system")]
        if self.native_attempt.is_some() {
            return Err(Error::InvalidInput("native frame attempt remains open"));
        }
        Ok(())
    }

    pub(super) fn ensure_live(&self) -> Result<()> {
        if !self.active() {
            return Err(Error::SessionStopped);
        }
        if let Some(error) = self.connection.backend().last_error() {
            return Err(Error::Wayland(error.to_string()));
        }
        Ok(())
    }

    fn validate_id(&self, id: WindowId) -> Result<()> {
        if id.connection != self.connection_id {
            return Err(Error::InvalidInput("window belongs to another connection"));
        }
        if id.sequence == 0 || id.sequence >= self.next_window {
            return Err(Error::InvalidInput("unknown window identity"));
        }
        Ok(())
    }

    pub(super) fn record(&self, id: WindowId) -> Result<&WindowRecord> {
        self.validate_id(id)?;
        self.windows
            .get(&id)
            .ok_or(Error::InvalidInput("window is retired"))
    }

    pub(super) fn remember(&mut self, error: &Error) {
        if self.failure.is_none() {
            self.failure = Some(Failure::new("window", error));
        }
    }

    /// Focus and staged frames retain cancellation capacity inside the same cap.
    pub(super) fn event_room(&self, count: usize) -> bool {
        self.events
            .len()
            .checked_add(count)
            .and_then(|used| used.checked_add(self.input.retirement_count(None)))
            .and_then(|used| used.checked_add(self.clipboard.reserved()))
            .and_then(|used| used.checked_add(self.text_input.reserved()))
            .and_then(|used| used.checked_add(self.presentation.reserved()))
            .is_some_and(|used| used <= MAX_EVENTS)
    }

    pub(super) fn push(&mut self, event: WindowEvent) -> bool {
        if !self.event_room(1) {
            self.remember(&Error::LimitExceeded);
            return false;
        }
        self.events.push_back(event);
        true
    }
}

impl QueueState for WindowState {
    fn pending(&self) -> usize {
        self.events.len()
    }
    fn failure(&self) -> Option<&str> {
        WindowState::failure(self)
    }
    fn service_transfers(&mut self, now: std::time::Instant) -> Result<bool> {
        self.clipboard.service(now, &mut self.events)
    }
    fn transfer_interests<'a>(
        &'a self,
        interests: &mut Vec<crate::runtime::Interest<'a>>,
    ) -> Result<()> {
        self.clipboard.transfers.interests(interests)
    }
    fn transfer_deadline(&self) -> Option<std::time::Instant> {
        self.clipboard.transfers.deadline()
    }
    fn stop(&mut self) {
        self.retire_all();
    }
}

impl Drop for WindowState {
    fn drop(&mut self) {
        self.retire_all();
    }
}
