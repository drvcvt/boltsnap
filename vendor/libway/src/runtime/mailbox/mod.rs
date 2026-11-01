//! Bounded private command admission and credits retained through application.
//! Charges describe trusted internal command data, not arbitrary T allocations.
#[cfg(test)]
mod tests;
mod wake;

use crate::{Error, Result};
use std::{
    collections::VecDeque,
    fmt::{self, Write},
    os::fd::BorrowedFd,
    sync::{Arc, Mutex, MutexGuard, Weak},
};
use wake::Wake;

const MAX_FAILURE_BYTES: usize = 512;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_queued: usize,
    pub(crate) max_bytes: usize,
}

/// The first terminal cause, formatted without retaining more than 512 UTF-8 bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure(String);

impl Failure {
    pub(crate) fn new(context: &str, error: impl fmt::Display) -> Self {
        let mut text = String::with_capacity(MAX_FAILURE_BYTES);
        let _ = write!(BoundedCause(&mut text), "{context}: {error}");
        Self(text)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Failure {}

struct BoundedCause<'a>(&'a mut String);

impl Write for BoundedCause<'_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let mut end = text.len().min(MAX_FAILURE_BYTES - self.0.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&text[..end]);
        Ok(())
    }
}

/// Full/closed/failed rejection returns the original value without admission.
/// AdmittedWakeFailed returns no value: the failed receiver retains that command
/// until shutdown, so a caller must not retry it as an unaccepted operation.
#[derive(Debug)]
pub(crate) enum SendError<T> {
    Full(T),
    Closed(T),
    Failed(T, Failure),
    AdmittedWakeFailed(Failure),
}

struct Entry<T> {
    value: T,
    charge: usize,
}

struct State<T> {
    queue: VecDeque<Entry<T>>,
    // Checked admission maintains queue charges plus all live extracted credits.
    // Each charge is released once, only after its payload has left host ownership.
    retained_bytes: usize,
    closed: bool,
    failure: Option<Failure>,
}

struct Shared<T> {
    state: Mutex<State<T>>,
    limits: Limits,
    wake: Wake,
}

impl<T> Shared<T> {
    fn lock(&self) -> MutexGuard<'_, State<T>> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                let mut state = poisoned.into_inner();
                if state.failure.is_none() {
                    state.failure = Some(Failure::new("mailbox", "mutex poisoned"));
                }
                state
            }
        }
    }

    fn fail(&self, cause: Failure) -> Failure {
        let mut state = self.lock();
        state.failure.get_or_insert(cause).clone()
    }
}

/// Only this owner can receive commands or consume wake notifications.
pub(crate) struct Mailbox<T> {
    shared: Arc<Shared<T>>,
}

/// A send-only handle, with no descriptor or receiver access.
pub(crate) struct Sender<T> {
    shared: Arc<Shared<T>>,
}

/// Best-effort wake for native lease teardown without retaining the receiver or
/// queued commands. A queued command may own a native lease carrying this handle
/// without a strong Shared -> command -> lease -> Shared ownership cycle.
pub(crate) struct Notifier<T> {
    shared: Weak<Shared<T>>,
}

impl<T> Clone for Notifier<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<T> Notifier<T> {
    /// No command is admitted and no queue or byte credit is consumed. After
    /// receiver shutdown this is a successful best-effort no-op; while active,
    /// wake errors retain the first terminal cause before receiver access.
    pub(crate) fn notify(&self) -> std::result::Result<(), Failure> {
        let Some(shared) = self.shared.upgrade() else {
            return Ok(());
        };
        let mut state = shared.lock();
        if state.closed {
            return Ok(());
        }
        if let Some(cause) = &state.failure {
            return Err(cause.clone());
        }
        if let Err(error) = shared.wake.notify() {
            let cause = Failure::new("mailbox wake notification", error);
            return Err(state.failure.get_or_insert(cause).clone());
        }
        Ok(())
    }
}

impl<T> Clone for Sender<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

/// Queue entries deliberately have no Arc-backed credit, avoiding a cycle from
/// Shared through its queue back to Shared. Only extracted entries own a credit.
struct Credit<T> {
    shared: Arc<Shared<T>>,
    bytes: usize,
}

impl<T> Drop for Credit<T> {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        // This unique credit was included by checked admission and no queue
        // removal releases it; subtraction cannot underflow.
        state.retained_bytes -= self.bytes;
    }
}

/// Value precedes credit so abandoning a received command destroys its payload
/// outside the mutex before releasing the charge, including during unwinding.
pub(crate) struct Received<T> {
    value: T,
    credit: Credit<T>,
}

impl<T> Received<T> {
    /// Invoke only after releasing any prepared read and borrowed wait view.
    /// No mailbox lock is held; credit remains charged throughout the callback
    /// and its unwind. A value retained by the callback becomes caller-owned.
    pub(crate) fn apply<R>(self, f: impl FnOnce(T) -> R) -> R {
        let Self { value, credit } = self;
        let result = f(value);
        drop(credit);
        result
    }
}

impl<T> Mailbox<T> {
    pub(crate) fn new(limits: Limits) -> Result<Self> {
        if limits.max_queued == 0 || limits.max_bytes == 0 {
            return Err(Error::InvalidInput("mailbox limits must be positive"));
        }
        let mut queue = VecDeque::new();
        queue
            .try_reserve_exact(limits.max_queued)
            .map_err(|_| Error::LimitExceeded)?;
        let wake = Wake::new()?;
        Ok(Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    queue,
                    retained_bytes: 0,
                    closed: false,
                    failure: None,
                }),
                limits,
                wake,
            }),
        })
    }

    pub(crate) fn sender(&self) -> Sender<T> {
        Sender {
            shared: self.shared.clone(),
        }
    }

    pub(crate) fn notifier(&self) -> Notifier<T> {
        Notifier {
            shared: Arc::downgrade(&self.shared),
        }
    }

    pub(crate) fn pop(&mut self) -> std::result::Result<Option<Received<T>>, Failure> {
        let mut state = self.shared.lock();
        if let Some(cause) = &state.failure {
            return Err(cause.clone());
        }
        Ok(state.queue.pop_front().map(|entry| Received {
            value: entry.value,
            credit: Credit {
                shared: self.shared.clone(),
                bytes: entry.charge,
            },
        }))
    }

    /// Check after consuming the wake and applying the finite command slice.
    /// Never drain again between the final empty check and poll: a producer in
    /// that gap leaves its durable token readable for the upcoming poll.
    pub(crate) fn has_pending(&self) -> std::result::Result<bool, Failure> {
        let state = self.shared.lock();
        if let Some(cause) = &state.failure {
            return Err(cause.clone());
        }
        Ok(!state.queue.is_empty())
    }

    pub(crate) fn drain_wake(&mut self) -> std::result::Result<(), Failure> {
        if let Some(cause) = self.failure() {
            return Err(cause);
        }
        self.shared
            .wake
            .drain()
            .map_err(|error| self.shared.fail(Failure::new("mailbox wake read", error)))
    }

    pub(crate) fn wake_fd(&self) -> BorrowedFd<'_> {
        self.shared.wake.fd()
    }

    pub(crate) fn failure(&self) -> Option<Failure> {
        self.shared.lock().failure.clone()
    }

    pub(crate) fn close(&mut self) {
        let (pending, bytes) = {
            let mut state = self.shared.lock();
            state.closed = true;
            let pending = std::mem::take(&mut state.queue);
            // Queued charges are a subset of checked retained_bytes, so their
            // sum cannot overflow or exceed the later outstanding balance.
            let bytes = pending.iter().map(|entry| entry.charge).sum::<usize>();
            (pending, bytes)
        };
        // A payload destructor may send or inspect failure; neither can deadlock.
        // Closed admission also prevents reusing bytes until these drops finish.
        drop(pending);
        self.shared.lock().retained_bytes -= bytes;
    }
}

impl<T> Drop for Mailbox<T> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<T> Sender<T> {
    #[cfg(test)]
    pub(crate) fn fail_next_wake(&self, errno: i32) {
        self.shared.wake.fail_next_write(errno);
    }

    /// The charge comes only from the coordinator's trusted command enum. It
    /// accounts variable host-retained bytes, not arbitrary payload allocations.
    /// Admission and its nonblocking wake are serialized before receiver access.
    pub(crate) fn send(&self, value: T, charge: usize) -> std::result::Result<(), SendError<T>> {
        let mut state = self.shared.lock();
        if let Some(cause) = &state.failure {
            return Err(SendError::Failed(value, cause.clone()));
        }
        if state.closed {
            return Err(SendError::Closed(value));
        }
        let Some(bytes) = state.retained_bytes.checked_add(charge) else {
            return Err(SendError::Full(value));
        };
        if state.queue.len() >= self.shared.limits.max_queued
            || bytes > self.shared.limits.max_bytes
        {
            return Err(SendError::Full(value));
        }
        state.queue.push_back(Entry { value, charge });
        state.retained_bytes = bytes;
        if let Err(error) = self.shared.wake.notify() {
            let cause = Failure::new("mailbox wake write", error);
            let cause = state.failure.get_or_insert(cause).clone();
            return Err(SendError::AdmittedWakeFailed(cause));
        }
        Ok(())
    }
}
