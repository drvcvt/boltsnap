//! Observations distinguish delivered work, retained work and wait causes.
#[derive(Debug, Default)]
pub(crate) struct Turn {
    pub(crate) callbacks: usize,
    pub(crate) dnd_callbacks: usize,
    pub(crate) events: usize,
    pub(crate) dnd_events: usize,
    pub(crate) transfers_ready: bool,
    pub(crate) commands: bool,
    pub(crate) write_pending: bool,
    pub(crate) wait: WaitReport,
}

impl Turn {
    pub(crate) fn runnable(&self) -> bool {
        self.events > 0
            || self.dnd_events > 0
            || self.transfers_ready
            || self.commands
            || self.wait.retry
    }
}

#[derive(Debug, Default)]
pub(crate) struct WaitReport {
    pub(crate) read: bool,
    #[cfg(test)]
    pub(crate) writable: bool,
    #[cfg(test)]
    pub(crate) pipe: bool,
    #[cfg(test)]
    pub(crate) wake: bool,
    #[cfg(test)]
    pub(crate) timed_out: bool,
    #[cfg(test)]
    pub(crate) interrupted: bool,
    pub(crate) retry: bool,
}
