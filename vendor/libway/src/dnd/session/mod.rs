//! Consumer API and owned socket wait cycles; protocol state belongs to Engine.
mod coordinated;
use super::{
    DndEvent, DndOptions, DragId, DragRequest, LocalRect, Outcome, OutgoingLimits, Reader,
    TargetId, TargetSpec, TransferId, checked_deadline,
    core::Core,
    engine::{self, Engine},
    poll_timeout, read_events, reader,
};
use crate::{Display, Error, Result, SurfaceHandle};
pub use coordinated::{CoordinatedDnd, WaitInterest, WaitView};
use std::{
    os::fd::AsRawFd,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static DND_IDS: AtomicU64 = AtomicU64::new(1);

/// Observations from one successful nonblocking dispatch, sampled under the session lock.
/// This is not a wait token: another caller/reader can change the state after return.
/// Counts do not bound dispatch time, wire reads, callback work or retained queue memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchReport {
    /// Protocol callbacks dispatched on this session's queue during this call only.
    pub callbacks: usize,
    /// Consumer events still queued, including events produced before this call.
    /// They remain available through [`Dnd::events`]; reporting does not drain them.
    pub pending_events: usize,
    /// The transfer slice yielded after progress with unfinished pipes. Service again
    /// before sleeping; this is a conservative hint, not a descriptor readiness result.
    /// False does not mean there are no transfers or deadlines to monitor.
    pub transfers_ready: bool,
    /// The latest flush returned WouldBlock. Monitor connection writability until
    /// successfully flushed; false does not establish readiness of any other queue.
    pub write_pending: bool,
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
        self.dispatch_report().map(drop)
    }
    /// Dispatch already-read events, service pipes and flush, with the same effects and
    /// errors as [`Dnd::dispatch`]. Does not read the connection socket or drain app events.
    /// Existing reader wakers and lazy data providers may run under the session lock.
    /// On error no report is returned; earlier work may already have progressed.
    ///
    /// A zero callback count is not a sleep decision. Pending consumer events and yielded
    /// transfers need service; even an all-zero report still needs a coordinated wait view
    /// covering other queues, pipe descriptors, deadlines, app wake and concurrent changes.
    pub fn dispatch_report(&self) -> Result<DispatchReport> {
        let mut core = self.lock();
        let generation = core.engine.reactor.generation;
        let result = core.service(Instant::now());
        core.kick(generation);
        core.wake();
        Ok(DispatchReport {
            callbacks: result?,
            pending_events: core.engine.events.len(),
            transfers_ready: core.engine.reactor.ready(),
            write_pending: core.write_pending,
        })
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
        self.prepare_drag(request, None)
    }
    /// Start with explicit outgoing payload bounds, including unique buffer capacity and
    /// preparation overlap. Oversize input returns [`Error::LimitExceeded`] whole, before
    /// replacing an active drag. Eager preparation happens outside the session lock;
    /// this initiating call is synchronous. Lazy callbacks retain their original contract
    /// and can allocate/block before their returned buffer is checked.
    pub fn start_drag_bounded(
        &self,
        request: DragRequest,
        limits: OutgoingLimits,
    ) -> Result<DragId> {
        self.prepare_drag(request, Some(limits))
    }
    fn prepare_drag(&self, request: DragRequest, limits: Option<OutgoingLimits>) -> Result<DragId> {
        let max_mimes = {
            let core = self.lock();
            if !core.engine.ready {
                return Err(Error::NotReady);
            }
            request.origin.validate(&core.engine.conn)?;
            core.engine.limits.max_mime_types
        };
        let request = super::representations::Request::new(request, max_mimes, limits)?;
        self.request(|engine| engine.start_drag(request))
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
