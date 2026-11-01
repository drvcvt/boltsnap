//! Shared dispatch, transfer and flush ownership under the session mutex.
use super::{engine::Engine, reader, would_block};
use crate::{Error, Result};
use std::{sync::Arc, time::Instant};
use wayland_client::EventQueue;

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
