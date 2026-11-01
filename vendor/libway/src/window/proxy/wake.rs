//! Coalesced retained host work, independent of window identities and native owners.
use super::WindowCommand;
use crate::{
    Error, Result,
    runtime::mailbox::{SendError, Sender},
};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

pub(in crate::window) enum HostCommand {
    Window(WindowCommand),
    Wake,
}

impl HostCommand {
    pub(in crate::window) fn into_window(self) -> WindowCommand {
        // WindowProxy submits only Window. Mailbox rejection returns the exact
        // submitted value without changing its variant or applying the command.
        let value = match self {
            Self::Window(value) => Some(value),
            Self::Wake => None,
        };
        value.expect("invariant: window admission rejects the submitted window command")
    }
}

pub(in crate::window) struct WakeState {
    pending: Mutex<bool>,
    sender: Sender<HostCommand>,
}

impl WakeState {
    pub(in crate::window) fn new(sender: Sender<HostCommand>) -> Arc<Self> {
        Arc::new(Self {
            pending: Mutex::new(false),
            sender,
        })
    }

    pub(in crate::window) fn handle(self: &Arc<Self>) -> HostWake {
        HostWake {
            state: Arc::downgrade(self),
        }
    }

    fn lock(&self) -> Result<MutexGuard<'_, bool>> {
        self.pending
            .lock()
            .map_err(|_| Error::Wayland("host wake: coalescing mutex poisoned".into()))
    }

    pub(in crate::window) fn collected(&self) -> Result<()> {
        *self.lock()? = false;
        Ok(())
    }
}

/// Weak, clonable worker notification without retaining a display or event owner.
/// Publish owned worker results before waking. After every returned host pump,
/// including ordinary input and command turns, collect host events to reset
/// coalescing, then service worker results before waiting again. Reversing that
/// order can lose a publication coalesced between result service and event collection.
#[derive(Clone)]
pub struct HostWake {
    state: Weak<WakeState>,
}

impl HostWake {
    /// Retain one coalesced [`crate::window::WindowEvent::Wake`] until event collection.
    /// Coalesced `Ok(())` confirms retained work, not host health: it does not probe
    /// a failure latched after the original admission. The pump reports that cause.
    /// Gone or closed hosts are harmless. Queue-full returns `LimitExceeded` and
    /// resets coalescing: already queued commands guarantee a returned pump turn,
    /// whose consumer must collect events, then service worker results. A mailbox
    /// wake error is terminal; admission may already have retained the token, so
    /// do not retry it as unsent. Gate poisoning reports a bounded `Wayland` error.
    /// Only short mutex sections and a nonblocking socket notification run here;
    /// no native call, application callback or connection dispatch occurs.
    pub fn wake(&self) -> Result<()> {
        let Some(state) = self.state.upgrade() else {
            return Ok(());
        };
        // Lock order is wake gate -> mailbox. Collection never holds a mailbox
        // lock, and no receiver path takes this gate while holding that lock.
        // Thus coalesced success follows durable admission, never an enqueue intent.
        let mut pending = state.lock()?;
        if *pending {
            return Ok(());
        }
        match state.sender.send(HostCommand::Wake, 0) {
            Ok(()) => {
                *pending = true;
                Ok(())
            }
            Err(SendError::Full(_)) => Err(Error::LimitExceeded),
            Err(SendError::Closed(_)) => Ok(()),
            Err(SendError::Failed(_, cause) | SendError::AdmittedWakeFailed(cause)) => {
                Err(Error::Wayland(cause.to_string()))
            }
        }
    }

    #[cfg(test)]
    pub(in crate::window) fn fail_next_wake(&self, errno: i32) {
        if let Some(state) = self.state.upgrade() {
            state.sender.fail_next_wake(errno);
        }
    }

    #[cfg(test)]
    pub(in crate::window) fn retain_state(&self) -> Option<Arc<WakeState>> {
        self.state.upgrade()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::mailbox::{Limits, Mailbox};

    #[test]
    fn poisoned_coalescing_is_an_explicit_bounded_error() {
        let mailbox = Mailbox::new(Limits {
            max_queued: 2,
            max_bytes: 64,
        })
        .unwrap();
        let state = WakeState::new(mailbox.sender());
        let wake = state.handle();
        let poisoned = state.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = poisoned.pending.lock().unwrap();
                panic!("test poison");
            })
            .join()
            .is_err()
        );
        let error = wake.wake().unwrap_err().to_string();
        assert!(error.contains("host wake: coalescing mutex poisoned"));
        assert!(error.len() <= 512);
        assert!(state.collected().is_err());
        assert!(!mailbox.has_pending().unwrap());
    }
}
