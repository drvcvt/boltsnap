//! Owner-only DnD participation without changing the legacy session API.
mod requests;
mod wait;

pub use wait::{WaitInterest, WaitView};

use super::{DispatchReport, Dnd};
use crate::{
    Display, Error, Result,
    dnd::{DndEvent, DndOptions},
};
use std::fmt::{self, Write};

const MAX_FAILURE_BYTES: usize = 512;

/// A fresh DnD participant whose mutation is serialized by its mutable owner.
///
/// This type exposes no session references, readers, blocking run method or lazy
/// providers. It does not establish sole read ownership of the shared `Display`;
/// the connection coordinator must enforce that separately. Protocol dispatch
/// and requests must occur after any prepared connection read has been resolved.
///
/// A participant cannot start a legacy background reader:
/// ```compile_fail
/// use libway::dnd::CoordinatedDnd;
/// fn reader(owner: &mut CoordinatedDnd) {
///     owner.spawn_reader(|| {});
/// }
/// ```
/// It cannot independently run a blocking connection-read cycle:
/// ```compile_fail
/// use libway::dnd::CoordinatedDnd;
/// fn independent_read(owner: &mut CoordinatedDnd) {
///     owner.run(std::time::Duration::ZERO);
/// }
/// ```
pub struct CoordinatedDnd {
    dnd: Dnd,
    failure: Option<String>,
}

impl CoordinatedDnd {
    /// Create a private session on a cooperating connection. Guest displays are
    /// rejected before registry requests because their foreign owner reads them.
    /// Readiness is delivered as an event through coordinated dispatch.
    pub fn new(display: &Display, options: DndOptions) -> Result<Self> {
        if display.is_guest() {
            return Err(Error::Unsupported("coordinated DnD on a foreign display"));
        }
        Ok(Self {
            dnd: Dnd::new(display, options)?,
            failure: None,
        })
    }

    /// Service already-read protocol events, transfers and flushes without reading
    /// the connection. Any failure terminalizes this participant. Earlier work
    /// may already have progressed; subsequent work returns `SessionStopped`.
    pub fn dispatch(&mut self) -> Result<DispatchReport> {
        self.ensure_active()?;
        let result = self.dnd.dispatch_report();
        if let Err(error) = &result {
            self.remember(error);
        }
        result
    }

    /// Drain retained consumer events, including those left after a failure.
    /// No application callbacks are invoked.
    pub fn events(&mut self) -> Vec<DndEvent> {
        self.dnd.events()
    }

    /// The first observed terminal cause, retained as at most 512 UTF-8 bytes.
    /// Later failures do not replace it. Admission failures are not terminal.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    fn ensure_active(&mut self) -> Result<()> {
        if self.failure.is_some() {
            return Err(Error::SessionStopped);
        }
        let error = {
            let mut core = self.dnd.lock();
            if let Some(error) = core.engine.fatal.take() {
                Some(error)
            } else if core.engine.overflowed {
                Some(Error::LimitExceeded)
            } else if self.dnd.core.is_poisoned() {
                Some(Error::SessionStopped)
            } else {
                None
            }
        };
        let error = error.or_else(|| {
            self.dnd
                .display
                .connection()
                .backend()
                .last_error()
                .map(|error| Error::Wayland(error.to_string()))
        });
        if let Some(error) = error {
            self.remember(&error);
            return Err(error);
        }
        Ok(())
    }

    fn finish_request<T>(&mut self, result: Result<T>) -> Result<T> {
        if let Err(error @ (Error::Io(_) | Error::Wayland(_))) = &result {
            self.remember(error);
        }
        if self.failure.is_none() {
            self.ensure_active()?;
        }
        result
    }

    fn remember(&mut self, error: &Error) {
        if self.failure.is_none() {
            let mut cause = String::with_capacity(MAX_FAILURE_BYTES);
            let _ = write!(BoundedCause(&mut cause), "{error}");
            self.failure = Some(cause);
        }
    }
}

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
