//! Drag-and-drop over `wl_data_device` v1–v3. Consumers describe intent (targets, payloads,
//! actions); libway owns every protocol object and reports typed results.
//!
//! A cooperating wayland-rs caller passes its existing connection and a surface created on
//! that connection. The reader shares socket reads; the owner still dispatches its own queue.
//! This copy-only consumer rejects Ask/Move rather than promising an operation it did not do.
//!
//! ```no_run
//! use libway::{Display, SurfaceHandle, dnd::*};
//! use wayland_client::{Connection, protocol::wl_surface::WlSurface};
//!
//! fn receive_drops(connection: Connection, surface: &WlSurface) -> libway::Result<()> {
//!     let display = Display::from_connection(connection);
//!     let dnd = Dnd::new(&display, DndOptions::default())?;
//!     let surface = SurfaceHandle::from_surface(surface);
//!     dnd.register_target(&surface, TargetSpec {
//!         rect: LocalRect { x: 0., y: 0., width: 800., height: 600. },
//!         accepts: vec![Accept::Files, Accept::Text],
//!         actions: Actions { copy: true, move_: false, ask: false },
//!         preferred: Action::Copy,
//!         priority: 0,
//!         enabled: true,
//!     })?;
//!     let (tx, rx) = std::sync::mpsc::channel();
//!     let _reader = dnd.spawn_reader(move || { let _ = tx.send(()); })?;
//!     while rx.recv().is_ok() {
//!         for event in dnd.events() {
//!             match event {
//!                 DndEvent::Dropped { transfer, payload, action, .. } => {
//!                     println!("{payload:?}"); // Process/persist the data before accepting.
//!                     let outcome = match action {
//!                         Action::Copy | Action::None => Outcome::Accepted(action),
//!                         _ => Outcome::Rejected,
//!                     };
//!                     dnd.complete(transfer, outcome)?;
//!                 }
//!                 DndEvent::Failed(reason) => return Err(libway::Error::Wayland(reason)),
//!                 _ => {}
//!             }
//!         }
//!     }
//!     Ok(()) // Reader drops before Dnd and Display.
//! }
//! ```

mod drag;
mod engine;
mod interop;
mod reader;
mod transfer;
mod uri;
use crate::{Display, Error, Result, SurfaceHandle, TransferError};
use engine::Engine;
pub use reader::Reader;
use std::{
    os::fd::AsRawFd,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use wayland_client::EventQueue;

static DND_IDS: AtomicU64 = AtomicU64::new(1);

/// Per-session resource bounds. Numeric zero limits disable the corresponding capacity;
/// transfer timeouts must be positive and representable by the monotonic clock.
#[derive(Debug, Clone, Copy)]
pub struct DndLimits {
    /// Maximum bytes per incoming representation (default 1 MiB). Outgoing payloads are uncapped.
    pub max_payload: usize,
    /// Maximum parsed file URI entries (default 1024); no partial file list is returned.
    pub max_entries: usize,
    /// Maximum MIME announcements retained per incoming offer (default 64); later ones are
    /// ignored. Outgoing drags exceeding this count fail with `LimitExceeded`.
    pub max_mime_types: usize,
    /// Concurrent unanswered incoming drops and, separately, concurrent outgoing pipes
    /// (default 4). Delivered drops occupy their slot until `complete` is called.
    pub max_transfers: usize,
    /// Maximum interval without positive-byte pipe progress (default five seconds).
    pub inactivity: Duration,
    /// Maximum pipe-transfer duration (default 15 seconds), shared across MIME fallbacks.
    /// Does not time out an already delivered drop awaiting `complete`.
    pub total: Duration,
    /// Maximum queued consumer events (default 256); overflow fails the session.
    pub max_events: usize,
    /// Maximum registered targets (default 256).
    pub max_targets: usize,
}
impl Default for DndLimits {
    fn default() -> Self {
        Self {
            max_payload: 1024 * 1024,
            max_entries: 1024,
            max_mime_types: 64,
            max_transfers: 4,
            inactivity: Duration::from_secs(5),
            total: Duration::from_secs(15),
            max_events: 256,
            max_targets: 256,
        }
    }
}
/// Session limits and optional input tracking. Default is suitable for drop-only consumers.
#[derive(Debug, Clone, Default)]
pub struct DndOptions {
    /// Resource and transfer-time bounds validated at session creation.
    pub limits: DndLimits,
    /// Bind `wl_pointer`/`wl_touch` to learn implicit-grab serials for `start_drag`.
    /// Costs one reader wakeup per input event; leave off for drop-only consumers.
    pub track_input: bool,
}
/// Registry name of a `wl_seat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SeatId(pub(crate) u32);
/// Opaque target identity belonging to one DnD session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TargetId(pub(crate) u64, pub(crate) u64);
/// Opaque incoming drop identity; answer it exactly once after delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferId(pub(crate) u64, pub(crate) u64);
/// Opaque outgoing drag identity belonging to one DnD session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DragId(pub(crate) u64, pub(crate) u64);

/// Set of `wl_data_device_manager.dnd_action` values. XDND `Link`/`Private` have no Wayland
/// equivalent and are never claimed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Actions {
    /// Allow copying data without removing it at the source.
    pub copy: bool,
    /// Allow moving data; successful finish can authorize deletion at the source.
    pub move_: bool,
    /// Allow deferred consumer choice between copy and move.
    pub ask: bool,
}
impl Actions {
    pub(crate) fn to_wire(self) -> u32 {
        u32::from(self.copy) | (u32::from(self.move_) << 1) | (u32::from(self.ask) << 2)
    }
    pub(crate) fn from_wire(bits: u32) -> Self {
        Self {
            copy: bits & 1 != 0,
            move_: bits & 2 != 0,
            ask: bits & 4 != 0,
        }
    }
}
/// Negotiated operation. Only Copy and Move can finish a v3 drop successfully.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Action {
    /// No negotiated action; also the only reported action on v1/v2.
    #[default]
    None,
    /// Preserve source data.
    Copy,
    /// Transfer ownership; the source may remove originals after successful finish.
    Move,
    /// Consumer must choose an offered Copy or Move before completion.
    Ask,
}
impl Action {
    pub(crate) fn to_wire(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Copy => 1,
            Self::Move => 2,
            Self::Ask => 4,
        }
    }
    pub(crate) fn from_wire(bits: u32) -> Self {
        match bits {
            1 => Self::Copy,
            2 => Self::Move,
            4 => Self::Ask,
            _ => Self::None,
        }
    }
}
/// Surface-local logical coordinates, the same space as `wl_data_device.enter`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalRect {
    /// Left edge in surface-local logical units.
    pub x: f64,
    /// Top edge in surface-local logical units.
    pub y: f64,
    /// Horizontal extent in logical units; nonpositive extents never hit.
    pub width: f64,
    /// Vertical extent in logical units; nonpositive extents never hit.
    pub height: f64,
}
impl LocalRect {
    /// Hit-test with inclusive left/top and exclusive right/bottom edges.
    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}
/// Accepted representations, in consumer preference order within [`TargetSpec::accepts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Accept {
    /// `text/uri-list` decoded to local absolute paths.
    Files,
    /// UTF-8 text via `text/plain;charset=utf-8` and X11 aliases.
    Text,
    /// Raw bytes for the first offered MIME type in this list.
    Mime(Vec<String>),
}
/// Drop target geometry, accepted payloads and action policy. Use finite coordinates and
/// positive extents for meaningful hit testing; geometry is stored as supplied. Actions
/// must be nonempty and include the preferred action (None is allowed as a preference).
#[derive(Debug, Clone)]
pub struct TargetSpec {
    /// Active region in surface-local logical units, tested by [`LocalRect::contains`].
    pub rect: LocalRect,
    /// Consumer preference order; the first entry the offer satisfies wins.
    pub accepts: Vec<Accept>,
    /// Allowed operations advertised to the source.
    pub actions: Actions,
    /// Preferred operation within the allowed set, or None for no preference.
    pub preferred: Action,
    /// Higher wins when targets overlap; ties go to the most recently registered.
    pub priority: i32,
    /// False keeps registration but excludes it from hit testing.
    pub enabled: bool,
}
/// Completed incoming representation, owned by the consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Payload {
    /// Local absolute paths, losslessly decoded from file URIs.
    Files(Vec<PathBuf>),
    /// Decoded text; legacy STRING is converted from Latin-1.
    Text(String),
    /// Raw bytes for an explicitly requested MIME type.
    Bytes {
        /// Selected MIME announcement.
        mime: String,
        /// Complete payload, bounded by `DndLimits::max_payload`.
        data: Vec<u8>,
    },
}
/// Local paths of a `text/uri-list` by the rules drops use: RFC 8089 `file:` URIs with an
/// empty, `localhost` or local-hostname authority, lossless bytes, all entries or an error. For file URIs met
/// outside a drop, such as a terminal's OSC 8 links.
///
/// ```
/// let paths = libway::dnd::paths_from_uri_list(b"file:///tmp/a%20b\r\n", 16).unwrap();
/// assert_eq!(paths, [std::path::PathBuf::from("/tmp/a b")]);
/// assert!(libway::dnd::paths_from_uri_list(b"https://example.com/", 16).is_err());
/// ```
/// Errors support both libway's result type and ordinary Rust error propagation:
/// ```
/// # use libway::dnd::paths_from_uri_list;
/// fn paths() -> libway::Result<Vec<std::path::PathBuf>> {
///     Ok(paths_from_uri_list(b"file:///tmp/a\n", 16)?)
/// }
/// fn main() -> Result<(), Box<dyn std::error::Error>> {
///     let _ = paths_from_uri_list(b"file:///tmp/a\n", 16)?;
///     assert_eq!(paths()?.len(), 1);
///     Ok(())
/// }
/// ```
pub fn paths_from_uri_list(
    bytes: &[u8],
    max_entries: usize,
) -> std::result::Result<Vec<PathBuf>, TransferError> {
    uri::parse_uri_list(bytes, max_entries)
}
/// Produces the bytes for one MIME type of a [`DragData::Lazy`] payload.
pub type Provider = Box<dyn FnMut(&str) -> Option<Vec<u8>> + Send>;
/// Representations offered by an outgoing drag. Names must be nonempty, NUL-free and at
/// most 1024 UTF-8 bytes; count is bounded by `max_mime_types`. X11 aliases remain valid.
/// Payload bytes are not bounded by the incoming `max_payload` limit.
pub enum DragData {
    /// Offered as `text/uri-list` and, for text-only targets, as plain text.
    Files(Vec<PathBuf>),
    /// Offered as UTF-8 text plus the X11 aliases `UTF8_STRING`, `TEXT`, `STRING`.
    Text(String),
    /// Files for targets that take `text/uri-list`, and `text` verbatim for text targets:
    /// a terminal selection that names files drops as files on a file manager and as the
    /// same characters on an editor.
    FilesAndText {
        /// Local absolute file paths; relative paths are rejected before sending requests.
        paths: Vec<PathBuf>,
        /// Text served verbatim to text targets (STRING uses lossy Latin-1 conversion).
        text: String,
    },
    /// Explicit MIME/payload pairs, materialized when requested.
    Static(Vec<(String, Vec<u8>)>),
    /// Called once per requested announced type on the dispatching thread (the reader thread
    /// with [`Dnd::spawn_reader`]) while libway holds its session lock: it must not wait on
    /// the consumer thread or call into the `Dnd`. `None` sends nothing (the reader sees EOF).
    Lazy {
        /// Announced names; validation never calls the provider.
        mimes: Vec<String>,
        /// Invoked per requested announced type under the session lock.
        provide: Provider,
    },
}
impl std::fmt::Debug for DragData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Files(p) => f.debug_tuple("Files").field(p).finish(),
            Self::Text(t) => f.debug_tuple("Text").field(&t.len()).finish(),
            Self::FilesAndText { paths, text } => (f.debug_struct("FilesAndText"))
                .field("paths", paths)
                .field("text", &text.len())
                .finish(),
            Self::Static(v) => {
                let mimes: Vec<_> = v.iter().map(|(m, _)| m).collect();
                f.debug_tuple("Static").field(&mimes).finish()
            }
            Self::Lazy { mimes, .. } => f.debug_tuple("Lazy").field(mimes).finish(),
        }
    }
}
/// Optional premultiplied RGBA drag icon, copied into owned SHM storage at drag start.
#[derive(Debug, Clone)]
pub struct DragIcon {
    /// Width in pixels, from 1 through 1024.
    pub width: u32,
    /// Height in pixels, from 1 through 1024.
    pub height: u32,
    /// Pointer position in icon pixels: `0 <= x < width`, `0 <= y < height`.
    pub hotspot: (i32, i32),
    /// Tightly packed premultiplied RGBA8, `width * height * 4` bytes.
    pub rgba_premultiplied: Vec<u8>,
}
/// Outgoing drag intent; invalid arguments are rejected before replacing the seat's drag.
#[derive(Debug)]
pub struct DragRequest {
    /// `None` picks a seat that has a serial.
    pub seat: Option<SeatId>,
    /// Surface the drag starts from; must belong to this connection.
    pub origin: SurfaceHandle,
    /// Serial of the button press or touch down that started the implicit grab. Required
    /// unless `DndOptions::track_input` recorded one.
    pub serial: Option<u32>,
    /// Representations to offer; file paths must be absolute.
    pub data: DragData,
    /// Allowed operations; ignored by protocol versions before v3.
    pub actions: Actions,
    /// Optional icon; None creates no icon surface.
    pub icon: Option<DragIcon>,
}
/// Final compositor outcome for an outgoing drag, independent of pipe-write completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragOutcome {
    /// Compositor reported completion; v1/v2 infer this from a send followed by cancellation
    /// and report None because those versions do not negotiate actions.
    Dropped(Action),
    /// Source cancelled, explicitly stopped or replaced by another drag on the same seat.
    Cancelled,
}
/// Consumer observations. Drain via [`Dnd::events`] after every reader wake.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum DndEvent {
    /// Initial registry and seat discovery completed.
    Ready {
        /// Bound data-device-manager version (1 through 3).
        version: u32,
        /// Currently bound seats.
        seats: Vec<SeatId>,
    },
    /// A seat was added after initial discovery.
    SeatAdded(SeatId),
    /// A seat was removed; associated transfers are aborted.
    SeatRemoved(SeatId),
    /// Drag entered one of this connection's surfaces.
    Enter {
        /// Seat controlling this drag.
        seat: SeatId,
        /// Surface entered, on the session's connection.
        surface: SurfaceHandle,
        /// Retained prefix of MIME announcements, bounded by `max_mime_types`.
        offered: Vec<String>,
        /// Actions offered by the source; empty on protocol versions before v3.
        source_actions: Actions,
    },
    /// Coalesced: only the newest position per seat is queued. `target` is `None` when no
    /// enabled target under the pointer accepts any offered type.
    Hover {
        /// Seat controlling this drag.
        seat: SeatId,
        /// Selected accepting target, or None when no eligible target accepts the offer.
        target: Option<TargetId>,
        /// Horizontal position in surface-local logical units.
        x: f64,
        /// Vertical position in surface-local logical units.
        y: f64,
        /// Latest negotiated action; None before negotiation and on v1/v2.
        action: Action,
    },
    /// The drag left, or was dropped. A drop onto a target is followed by `Dropped` or
    /// `TransferFailed`.
    Leave {
        /// Seat whose hover ended.
        seat: SeatId,
    },
    /// The payload arrived; answer with [`Dnd::complete`].
    Dropped {
        /// Seat that initiated the drop.
        seat: SeatId,
        /// Pending answer identity; call `complete` after processing the payload.
        transfer: TransferId,
        /// Target selected at drop time.
        target: TargetId,
        /// Action to honor when completing; Ask requires a final choice.
        action: Action,
        /// Owned, fully read and decoded payload.
        payload: Payload,
    },
    /// Incoming drop could not deliver data; no `complete` call is needed.
    TransferFailed {
        /// Seat that initiated the drop.
        seat: SeatId,
        /// Target selected at drop time, possibly since unregistered.
        target: TargetId,
        /// Typed reason for rejecting the transfer.
        reason: TransferError,
    },
    /// The target under an outgoing drag changed its mind.
    DragFeedback {
        /// Outgoing drag receiving feedback.
        drag: DragId,
        /// Whether the remote target accepted a MIME type.
        accepted: bool,
        /// Latest negotiated action, possibly None.
        action: Action,
    },
    /// An outgoing drag ended. Already opened send pipes may still be draining.
    DragEnded {
        /// Outgoing drag that ended.
        drag: DragId,
        /// Compositor result or local cancellation.
        outcome: DragOutcome,
    },
    /// The session is unusable; drop it and create a new one.
    Failed(String),
}
impl DndEvent {
    pub(crate) fn seat(&self) -> Option<SeatId> {
        match self {
            Self::Enter { seat, .. }
            | Self::Hover { seat, .. }
            | Self::Leave { seat }
            | Self::Dropped { seat, .. }
            | Self::TransferFailed { seat, .. } => Some(*seat),
            _ => None,
        }
    }
}

/// Consumer's final answer after processing a delivered drop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The consumer took the data. On v3 this must match the negotiated Copy/Move action;
    /// for Ask choose Copy or Move offered by the source. None cannot be accepted on v3.
    /// On v1/v2 None, Copy and Move acknowledge locally without sending `finish`.
    /// Ask is never a final answer. Invalid answers leave the transfer available to retry.
    Accepted(Action),
    /// Refused; the offer is destroyed without `finish`.
    Rejected,
}

fn would_block(e: &wayland_client::backend::WaylandError) -> bool {
    matches!(e, wayland_client::backend::WaylandError::Io(e) if e.kind() == std::io::ErrorKind::WouldBlock)
}
pub(crate) fn checked_deadline(now: Instant, timeout: Duration) -> Result<Instant> {
    now.checked_add(timeout)
        .ok_or(Error::InvalidInput("duration exceeds the clock range"))
}
/// Round positive fractional milliseconds up so poll does not spin before the deadline.
pub(crate) fn poll_timeout(deadline: Instant) -> i32 {
    let nanos = deadline
        .saturating_duration_since(Instant::now())
        .as_nanos();
    nanos.div_ceil(1_000_000).min(i32::MAX as u128) as i32
}
/// Read into the queues. A readable socket without a complete message is not an error.
pub(crate) fn read_events(guard: wayland_client::backend::ReadEventsGuard) -> Result<()> {
    match guard.read() {
        Err(wayland_client::backend::WaylandError::Io(e))
            if e.kind() == std::io::ErrorKind::WouldBlock =>
        {
            Ok(())
        }
        other => other.map(drop).map_err(|e| Error::Wayland(e.to_string())),
    }
}

pub(crate) struct Core {
    pub queue: EventQueue<Engine>,
    pub engine: Engine,
    /// Installed by the reader; runs under the lock at most once per `events()` drain.
    pub waker: Option<Box<dyn Fn() + Send>>,
    /// The reader's wake handle while a reader runs.
    pub kick: Option<Arc<reader::Control>>,
    /// The last flush left requests queued; poll for writable space until drained.
    pub write_pending: bool,
}
impl Core {
    /// Dispatch protocol events, service pipes, flush. Returns the dispatched message count.
    pub fn service(&mut self, now: Instant) -> Result<usize> {
        let dispatched = self
            .queue
            .dispatch_pending(&mut self.engine)
            .map_err(|e| Error::Wayland(e.to_string()))?;
        self.engine.service_transfers(now);
        self.flush()?;
        if let Some(e) = self.engine.fatal.take() {
            return Err(e);
        }
        if self.engine.overflowed {
            return Err(Error::LimitExceeded);
        }
        Ok(dispatched)
    }
    pub fn flush(&mut self) -> Result<()> {
        let pending = match self.queue.flush() {
            Ok(()) => false,
            Err(e) if would_block(&e) => true,
            Err(e) => return Err(Error::Wayland(e.to_string())),
        };
        if pending != self.write_pending {
            self.write_pending = pending;
            if let Some(control) = &self.kick {
                control.wake();
            }
        }
        Ok(())
    }
    /// Tell a sleeping reader that the transfer fds changed under it (consumer-side dispatch
    /// or request), so it polls the new set instead of sleeping on the old one.
    pub fn kick(&self, generation: u64) {
        if let Some(control) = self
            .kick
            .as_ref()
            .filter(|_| self.engine.reactor.generation != generation)
        {
            control.wake();
        }
    }
    /// Wake the consumer if events wait and it was not woken since the last drain.
    pub fn wake(&mut self) {
        if let Some(waker) = &self.waker
            && !self.engine.events.is_empty()
            && !self.engine.wake_armed
        {
            self.engine.wake_armed = true;
            waker();
        }
    }
}

/// One DnD session per display connection. Register targets for any number of surfaces.
pub struct Dnd {
    display: Display,
    core: Arc<Mutex<Core>>,
}
impl Dnd {
    /// Binds registry, seats and data devices on `display`. Sends requests only; readiness
    /// arrives as [`DndEvent::Ready`].
    /// Zero or unrepresentable transfer timeouts return [`Error::InvalidInput`] before
    /// any objects are created. Initial connection write failures are returned directly.
    pub fn new(display: &Display, options: DndOptions) -> Result<Self> {
        for duration in [options.limits.inactivity, options.limits.total] {
            if duration.is_zero() {
                return Err(Error::InvalidInput("transfer deadlines must be positive"));
            }
            checked_deadline(Instant::now(), duration)?;
        }
        let conn = display.connection();
        let queue = conn.new_event_queue::<Engine>();
        let id = DND_IDS.fetch_add(1, Ordering::Relaxed);
        let engine = Engine::new(conn, queue.handle(), id, options);
        let mut core = Core {
            queue,
            engine,
            waker: None,
            kick: None,
            write_pending: false,
        };
        core.flush()?;
        Ok(Self {
            display: display.clone(),
            core: Arc::new(Mutex::new(core)),
        })
    }
    fn lock(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap_or_else(|p| p.into_inner())
    }
    /// Run a consumer request, send it, and wake the consumer for events it produced.
    fn request<T>(&self, f: impl FnOnce(&mut engine::Engine) -> Result<T>) -> Result<T> {
        let mut core = self.lock();
        let generation = core.engine.reactor.generation;
        let value = f(&mut core.engine);
        core.kick(generation);
        let value = value?;
        core.flush()?;
        core.wake();
        Ok(value)
    }
    /// Non-blocking: dispatch already-read events and service transfers. Safe next to a
    /// reader thread, for example to take in a button press right before [`Dnd::start_drag`].
    /// Errors from dispatch, connection I/O or event-queue overflow make the session unusable.
    pub fn dispatch(&self) -> Result<()> {
        let mut core = self.lock();
        let generation = core.engine.reactor.generation;
        let result = core.service(Instant::now());
        core.kick(generation);
        core.wake();
        result.map(drop)
    }
    /// Read and dispatch on a background thread, the way winit's own loop coexists with other
    /// readers. `waker` runs at most once per `events()` drain (and once on failure), also for
    /// events that the consumer's own calls produce. It runs under the session lock and must
    /// not block: `EventLoopProxy::send_event` or a channel send. Do not call `run` meanwhile.
    /// Also do not call `wait_ready` while it runs; wait for the Ready event instead.
    /// All other consumer methods may run beside the reader. Starting a second reader
    /// returns [`Error::Unsupported`]. Drop the reader before the DnD session/display owner.
    pub fn spawn_reader(&self, waker: impl Fn() + Send + 'static) -> Result<Reader> {
        let conn = self.display.connection().clone();
        reader::spawn(conn, self.core.clone(), waker)
    }
    /// One bounded wait-and-dispatch cycle that reads the socket itself. Returns as soon as
    /// events are queued. Refused on guest displays, whose owner loop reads; use a reader
    /// thread there.
    /// Zero performs at most one nonblocking read cycle. Unrepresentable timeouts return
    /// [`Error::InvalidInput`]. Do not use concurrently with a reader or another `run` call.
    pub fn run(&self, timeout: Duration) -> Result<()> {
        if self.display.is_guest() {
            return Err(Error::Unsupported("Dnd::run on a foreign display"));
        }
        let conn = self.display.connection();
        let deadline = checked_deadline(Instant::now(), timeout)?;
        loop {
            self.dispatch()?;
            if !self.lock().engine.events.is_empty() {
                return Ok(());
            }
            let Some(guard) = conn.prepare_read() else {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                continue;
            };
            // Another reader may have queued ours before we prepared (Rust backend).
            if self.lock().service(Instant::now())? > 0 {
                if Instant::now() >= deadline {
                    return Ok(());
                }
                continue;
            }
            let (mut fds, next) = {
                let mut core = self.lock();
                core.flush()?;
                let fd = conn.backend().poll_fd().as_raw_fd();
                core.engine
                    .reactor
                    .poll_set(fd, core.write_pending, &core.engine.limits)?
            };
            let wait = next.map_or(deadline, |d| d.min(deadline));
            let ms = poll_timeout(wait);
            let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, ms) };
            if ret < 0 {
                let err = std::io::Error::last_os_error();
                if err.kind() != std::io::ErrorKind::Interrupted {
                    return Err(err.into());
                }
            }
            if fds.iter().any(|fd| fd.revents & libc::POLLNVAL != 0) {
                return Err(std::io::Error::from_raw_os_error(libc::EBADF).into());
            }
            if ret > 0 && fds[0].revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0 {
                read_events(guard)?;
            } else {
                drop(guard);
            }
            self.dispatch()?;
            if ret > 0 || Instant::now() >= deadline {
                return Ok(());
            }
        }
    }
    /// Block until the initial registry sync finished. Not available on guest displays.
    /// Zero only checks current readiness, otherwise returning [`Error::Timeout`].
    /// Unrepresentable timeouts return [`Error::InvalidInput`], even if already ready.
    pub fn wait_ready(&self, timeout: Duration) -> Result<()> {
        if self.display.is_guest() {
            return Err(Error::Unsupported("Dnd::wait_ready on a foreign display"));
        }
        let end = checked_deadline(Instant::now(), timeout)?;
        while !self.lock().engine.ready {
            if Instant::now() >= end {
                return Err(Error::Timeout);
            }
            self.run(end.saturating_duration_since(Instant::now()))?;
        }
        Ok(())
    }
    /// Drain queued events; re-arms the reader wakeup.
    pub fn events(&self) -> Vec<DndEvent> {
        let mut core = self.lock();
        core.engine.wake_armed = false;
        core.engine.events.drain(..).collect()
    }
    /// Register a target on a live surface of this display. Invalid action policies,
    /// foreign safe handles and destroyed safe handles return [`Error::InvalidInput`].
    /// At capacity returns [`Error::LimitExceeded`]. Unregister before destroying the surface.
    pub fn register_target(&self, surface: &SurfaceHandle, spec: TargetSpec) -> Result<TargetId> {
        surface.validate(self.display.connection())?;
        self.request(|e| e.register_target(surface.id().clone(), spec))
    }
    /// Move, resize, enable or disable a target; the current hover is renegotiated.
    /// None keeps the respective value; geometry is stored as supplied.
    /// Stale or foreign ids return [`Error::UnknownTarget`].
    pub fn update_target(
        &self,
        id: TargetId,
        rect: Option<LocalRect>,
        enabled: Option<bool>,
    ) -> Result<()> {
        self.request(|e| e.update_target(id, rect, enabled))
    }
    /// Answer a [`DndEvent::Dropped`]; every delivered drop needs exactly one answer and
    /// counts against `DndLimits::max_transfers` until then.
    /// See [`Outcome::Accepted`] for valid actions. A mismatch returns [`Error::InvalidInput`]
    /// without completing the transfer, so the caller can correct or reject it.
    pub fn complete(&self, transfer: TransferId, outcome: Outcome) -> Result<()> {
        self.request(|e| e.complete(transfer, outcome))
    }
    /// Start a drag from `request.origin`. The previous drag of that seat is cancelled.
    /// Requires readiness and a valid explicit/tracked serial. Invalid origins, MIME names,
    /// paths or hotspots are rejected before replacing the previous drag. Size/count bounds
    /// return `LimitExceeded`; bad icon dimensions return `InvalidDimensions`.
    pub fn start_drag(&self, request: DragRequest) -> Result<DragId> {
        self.request(|e| e.start_drag(request))
    }
    /// Cancel an active outgoing drag and queue `DragEnded(Cancelled)`. Its already started
    /// sends may finish. An unknown or ended id returns [`Error::UnknownTransfer`].
    pub fn cancel_drag(&self, id: DragId) -> Result<()> {
        self.request(|e| e.cancel_drag(id))
    }
    /// Remove a target, abort its still-reading drops and renegotiate hover.
    /// Already delivered drops still require a `complete` answer. Unknown or foreign
    /// ids return [`Error::UnknownTarget`]. Call before destroying the owning surface.
    pub fn unregister_target(&self, id: TargetId) -> Result<()> {
        self.request(|e| e.unregister_target(id))
    }
}
impl Drop for Dnd {
    fn drop(&mut self) {
        let mut core = self.lock();
        core.engine.shutdown();
        let _ = core.flush();
    }
}
