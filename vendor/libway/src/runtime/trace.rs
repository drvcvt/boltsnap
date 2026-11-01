//! Test-only observation points; production invokes no callback during reservations.
#[cfg(test)]
#[derive(Clone, Copy)]
pub(super) enum PollFault {
    Interrupted,
    Invalid,
    Fatal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    BeforeDrain,
    AfterDrain,
    BeforePrepare,
    AfterPrepare,
    BeforePoll,
}

#[derive(Default)]
pub(super) struct Trace {
    #[cfg(test)]
    pub(super) poll_fault: Option<PollFault>,
    #[cfg(test)]
    pub(super) hook: Option<Box<dyn FnMut(Phase)>>,
    #[cfg(test)]
    pub(super) prepared: usize,
    #[cfg(test)]
    pub(super) reads: usize,
    #[cfg(test)]
    pub(super) cancelled: usize,
    #[cfg(test)]
    pub(super) polls: usize,
    #[cfg(test)]
    pub(super) dispatches: usize,
    #[cfg(test)]
    pub(super) flushes: usize,
    #[cfg(test)]
    pub(super) reserved: bool,
}

impl Trace {
    pub(super) fn phase(&mut self, _phase: Phase) {
        #[cfg(test)]
        if let Some(hook) = &mut self.hook {
            hook(_phase);
        }
    }
    pub(super) fn dispatched(&mut self) {
        #[cfg(test)]
        {
            assert!(!self.reserved);
            self.dispatches += 1;
        }
    }
    pub(super) fn flushed(&mut self) {
        #[cfg(test)]
        {
            assert!(!self.reserved);
            self.flushes += 1;
        }
    }
    pub(super) fn prepare(&mut self) {
        #[cfg(test)]
        {
            assert!(!self.reserved);
            self.reserved = true;
            self.prepared += 1;
        }
    }
    pub(super) fn resolved(&mut self, _read: bool) {
        #[cfg(test)]
        {
            assert!(self.reserved);
            self.reserved = false;
            if _read {
                self.reads += 1;
            } else {
                self.cancelled += 1;
            }
        }
    }
    pub(super) fn poll(&mut self) {
        #[cfg(test)]
        {
            assert!(self.reserved);
            self.polls += 1;
        }
    }
}
