//! Send-only admission with an explicit distinction between rejection and failed wake.
mod wake;

use super::{WindowCommand, WindowId};
use crate::{
    Result,
    runtime::mailbox::{SendError, Sender},
};
pub use wake::HostWake;
pub(super) use wake::{HostCommand, WakeState};

/// An applied command result. Admission itself never implies successful application.
#[derive(Debug)]
pub struct CommandOutcome {
    /// Identity targeted by the command, in the proxy's admission order.
    pub window: WindowId,
    /// Application or validation result for that command.
    pub result: Result<()>,
}

/// Bounded send-only host proxy. It cannot read or dispatch the Wayland connection.
#[derive(Clone)]
pub struct WindowProxy {
    sender: Sender<HostCommand>,
}

/// Admission failure. Only the first three variants return an unaccepted command.
#[derive(Debug)]
pub enum CommandSendError {
    /// Queue or retained-byte capacity is full; the original command is returned.
    Full(WindowCommand),
    /// The host was shut down; the original command is returned.
    Closed(WindowCommand),
    /// The host wake mechanism failed before admission.
    Failed(WindowCommand, String),
    /// Admission succeeded but its wake failed. Do not retry as an unsent command.
    AdmittedWakeFailed(String),
}

impl WindowProxy {
    pub(super) fn new(sender: Sender<HostCommand>) -> Self {
        Self { sender }
    }

    /// Admit without waiting for the application thread. At most 256 commands and
    /// 8 MiB of retained string capacity are admitted by a host at once.
    pub fn try_send(&self, command: WindowCommand) -> std::result::Result<(), CommandSendError> {
        let charge = command.retained_bytes();
        self.sender
            .send(HostCommand::Window(command), charge)
            .map_err(|error| match error {
                SendError::Full(command) => CommandSendError::Full(command.into_window()),
                SendError::Closed(command) => CommandSendError::Closed(command.into_window()),
                SendError::Failed(command, cause) => {
                    CommandSendError::Failed(command.into_window(), cause.to_string())
                }
                SendError::AdmittedWakeFailed(cause) => {
                    CommandSendError::AdmittedWakeFailed(cause.to_string())
                }
            })
    }
}
