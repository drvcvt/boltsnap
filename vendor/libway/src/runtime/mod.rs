//! Private connection owner shared by native capability queues and admitted commands.
pub(crate) mod mailbox;
#[cfg(test)]
mod tests;
mod trace;
mod turn;
mod wait;

use crate::{Display, Error, Result};
use mailbox::{Failure, Limits, Mailbox, Received, Sender};
use std::{os::fd::AsFd, os::unix::net::UnixStream, time::Instant};
use trace::Trace;
pub(crate) use turn::{Turn, WaitReport};
pub(crate) use wait::Interest;
use wayland_client::{Connection, EventQueue, QueueHandle, backend::WaylandError};

/// Implemented only by library-owned protocol states, never application callbacks.
pub(crate) trait QueueState: 'static {
    fn pending(&self) -> usize;
    /// Service library-owned pipes outside a prepared connection read. Return
    /// true after byte progress or a yielded slice, preventing a second transfer
    /// slice after waiting in this turn. A subsequent EAGAIN-only turn may sleep.
    fn service_transfers(&mut self, _now: Instant) -> Result<bool> {
        Ok(false)
    }
    /// Borrows prevent descriptor retirement or reuse while the wait is armed.
    fn transfer_interests<'a>(&'a self, _output: &mut Vec<Interest<'a>>) -> Result<()> {
        Ok(())
    }
    fn transfer_deadline(&self) -> Option<Instant> {
        None
    }
    fn failure(&self) -> Option<&str> {
        None
    }
    fn stop(&mut self) {}
}

/// The connection and every reading/dispatching capability stay behind this owner.
/// Initialization is crate-private: internal states may keep proxies, but may not
/// publish the connection, foreign ObjectData, queue handles or queue freezes.
pub(crate) struct Coordinator<S: QueueState, C> {
    pub(crate) state: S,
    queue: EventQueue<S>,
    #[cfg(feature = "dnd")]
    dnd: Option<crate::dnd::CoordinatedDnd>,
    #[cfg(feature = "dnd")]
    stopped_dnd_events: Vec<crate::dnd::DndEvent>,
    display: Display,
    mailbox: Mailbox<C>,
    failure: Option<Failure>,
    write_pending: bool,
    trace: Trace,
}

impl<S: QueueState, C> Coordinator<S, C> {
    pub(crate) fn from_socket(
        socket: UnixStream,
        limits: Limits,
        initialize: impl FnOnce(&Connection, &QueueHandle<S>, mailbox::Notifier<C>) -> S,
    ) -> Result<Self> {
        Self::new(Display::from_socket(socket)?, limits, initialize)
    }

    pub(crate) fn connect(
        limits: Limits,
        initialize: impl FnOnce(&Connection, &QueueHandle<S>, mailbox::Notifier<C>) -> S,
    ) -> Result<Self> {
        Self::new(Display::connect()?, limits, initialize)
    }

    fn new(
        display: Display,
        limits: Limits,
        initialize: impl FnOnce(&Connection, &QueueHandle<S>, mailbox::Notifier<C>) -> S,
    ) -> Result<Self> {
        let mailbox = Mailbox::new(limits)?;
        let queue = display.connection().new_event_queue();
        let state = initialize(display.connection(), &queue.handle(), mailbox.notifier());
        Ok(Self {
            state,
            queue,
            #[cfg(feature = "dnd")]
            dnd: None,
            #[cfg(feature = "dnd")]
            stopped_dnd_events: Vec::new(),
            display,
            mailbox,
            failure: None,
            write_pending: false,
            trace: Trace::default(),
        })
    }

    pub(crate) fn sender(&self) -> Sender<C> {
        self.mailbox.sender()
    }

    #[cfg(all(test, feature = "owned-native-system"))]
    pub(crate) fn read_dispatch_counts(&self) -> (usize, usize) {
        (self.trace.prepared, self.trace.dispatches)
    }

    /// Only internal host operations may run here, outside every read reservation.
    pub(crate) fn mutate<T>(&mut self, operation: impl FnOnce(&mut S) -> Result<T>) -> Result<T> {
        self.ensure_active()?;
        let result = operation(&mut self.state);
        let active = self.ensure_active();
        match result {
            Ok(value) => active.map(|()| value),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn failure(&self) -> Option<&str> {
        self.failure
            .as_ref()
            .map(Failure::as_str)
            .or_else(|| self.state.failure())
    }

    /// Service requests made by a host command batch without sleeping on its result.
    pub(crate) fn dispatch(&mut self) -> Result<Turn> {
        self.ensure_active()?;
        let result = self.service();
        self.finish(result)
    }

    /// Hosts apply at most their configured command batch before calling step.
    /// Received keeps its charge through apply/drop, outside the queue mutex.
    pub(crate) fn command(&mut self) -> Result<Option<Received<C>>> {
        self.ensure_active()?;
        let result = self.mailbox.pop().map_err(mailbox_error);
        self.finish(result)
    }

    #[cfg(feature = "dnd")]
    pub(crate) fn enable_dnd(&mut self, options: crate::dnd::DndOptions) -> Result<()> {
        self.ensure_active()?;
        if self.dnd.is_some() {
            return Err(Error::Unsupported("DnD already enabled"));
        }
        match crate::dnd::CoordinatedDnd::new(&self.display, options) {
            Ok(dnd) => {
                self.dnd = Some(dnd);
                Ok(())
            }
            Err(error @ (Error::Wayland(_) | Error::Io(_))) => self.finish(Err(error)),
            Err(error) => Err(error),
        }
    }

    #[cfg(feature = "dnd")]
    pub(crate) fn dnd_events(&mut self) -> Vec<crate::dnd::DndEvent> {
        match &mut self.dnd {
            Some(dnd) => dnd.events(),
            None => std::mem::take(&mut self.stopped_dnd_events),
        }
    }

    /// One dispatch/wait turn. No unconditional retry loop or polling interval.
    /// Consumer events and queued commands must be drained by the host before sleep.
    pub(crate) fn step(&mut self, deadline: Option<Instant>) -> Result<Turn> {
        self.ensure_active()?;
        let result = self.step_inner(deadline);
        self.finish(result)
    }

    fn step_inner(&mut self, deadline: Option<Instant>) -> Result<Turn> {
        self.trace.phase(trace::Phase::BeforeDrain);
        self.mailbox.drain_wake().map_err(mailbox_error)?;
        self.trace.phase(trace::Phase::AfterDrain);
        let first = self.service()?;
        if first.runnable() || first.callbacks > 0 || first.dnd_callbacks > 0 {
            return Ok(first);
        }
        let waited = self.wait(deadline)?;
        self.mailbox.drain_wake().map_err(mailbox_error)?;
        let mut turn = self.service()?;
        turn.wait = waited;
        Ok(turn)
    }

    fn service(&mut self) -> Result<Turn> {
        self.ensure_active()?;
        self.trace.dispatched();
        let callbacks = self
            .queue
            .dispatch_pending(&mut self.state)
            .map_err(|error| Error::Wayland(error.to_string()))?;
        self.ensure_active()?;
        let transfers_ready = self.state.service_transfers(Instant::now())?;
        let mut turn = Turn {
            callbacks,
            events: self.state.pending(),
            transfers_ready,
            ..Turn::default()
        };
        #[cfg(feature = "dnd")]
        if let Some(dnd) = &mut self.dnd {
            let report = dnd.dispatch()?;
            turn.dnd_callbacks = report.callbacks;
            turn.dnd_events = report.pending_events;
            turn.transfers_ready |= report.transfers_ready;
        }
        self.trace.flushed();
        self.write_pending = match self.display.connection().flush() {
            Ok(()) => false,
            Err(error) if would_block(&error) => true,
            Err(error) => return Err(Error::Wayland(error.to_string())),
        };
        self.ensure_active()?;
        turn.write_pending = self.write_pending;
        turn.commands = self.mailbox.has_pending().map_err(mailbox_error)?;
        Ok(turn)
    }

    fn wait(&mut self, deadline: Option<Instant>) -> Result<WaitReport> {
        let deadline = wait::earliest(deadline, self.state.transfer_deadline());
        #[cfg(feature = "dnd")]
        let view = self.dnd.as_mut().map(|dnd| dnd.wait_view()).transpose()?;
        let connection = self.display.connection();
        let mut interests = vec![wait::Interest {
            fd: connection.as_fd(),
            readable: true,
            writable: self.write_pending,
        }];
        #[cfg(feature = "dnd")]
        let deadline = if let Some(view) = &view {
            if view.immediate() {
                return Ok(WaitReport {
                    retry: true,
                    ..WaitReport::default()
                });
            }
            interests.extend(view.interests().skip(1).map(|interest| wait::Interest {
                fd: interest.fd,
                readable: interest.readable,
                writable: interest.writable,
            }));
            wait::earliest(deadline, view.deadline())
        } else {
            deadline
        };
        self.state.transfer_interests(&mut interests)?;
        interests.push(wait::Interest {
            fd: self.mailbox.wake_fd(),
            readable: true,
            writable: false,
        });
        let set = wait::WaitSet {
            interests,
            deadline,
        };
        wait::run(connection, &self.mailbox, set, &mut self.trace)
    }

    fn ensure_active(&mut self) -> Result<()> {
        if self.failure.is_some() {
            return Err(Error::SessionStopped);
        }
        if let Some(cause) = self.state.failure() {
            let error = Error::Wayland(cause.to_owned());
            return self.finish(Err(error));
        }
        if let Some(cause) = self.mailbox.failure() {
            return self.finish(Err(mailbox_error(cause)));
        }
        if let Some(error) = self.display.connection().backend().last_error() {
            return self.finish(Err(Error::Wayland(error.to_string())));
        }
        Ok(())
    }

    fn finish<T>(&mut self, result: Result<T>) -> Result<T> {
        if let Err(error) = &result {
            if self.failure.is_none() {
                self.failure = Some(Failure::new("coordinator", error));
                #[cfg(feature = "dnd")]
                if let Some(mut dnd) = self.dnd.take() {
                    // Retain delivered outcomes, but release transfers and target
                    // protocol objects before native window roles are retired.
                    self.stopped_dnd_events = dnd.events();
                    drop(dnd);
                }
                self.state.stop();
            }
            self.mailbox.close();
        }
        result
    }
}

fn mailbox_error(error: Failure) -> Error {
    Error::Wayland(error.to_string())
}

fn would_block(error: &WaylandError) -> bool {
    matches!(error, WaylandError::Io(error) if error.kind() == std::io::ErrorKind::WouldBlock)
}
