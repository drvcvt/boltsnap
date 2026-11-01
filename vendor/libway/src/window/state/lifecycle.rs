//! Retirement releases owned protocols before the final flush.
use super::*;

impl WindowState {
    /// Retire identities without waiting for leases or dispatching. The returned
    /// count names native owners still retained outside the host's live records.
    pub(crate) fn retire_all(&mut self) -> usize {
        self.text_input.clear(&mut self.events);
        self.clipboard.clear(&mut self.events);
        self.input
            .cancel(&mut self.events, None, InputCancelReason::Shutdown);
        self.input.clear();
        self.stopped = true;
        #[cfg(feature = "owned-native-system")]
        {
            self.native_attempt = None;
        }
        for record in self.windows.values() {
            record.native.retire();
        }
        self.output.clear();
        self.presentation.clear();
        self.tokens.clear();
        self.windows.clear();
        self.outputs.clear();
        // Busy CPU output can detach content even while external leases retain
        // the native role chain. Output releases also precede this flush instead
        // of being queued by later field Drop; the lease may survive indefinitely.
        let _ = self.connection.flush();
        self.globals
            .as_ref()
            .map_or(0, |globals| globals.native_windows.load(Ordering::Acquire))
    }
}
