//! Closing one window and retiring all: both release owned protocols before
//! the final flush.
use super::*;

impl WindowState {
    /// Close `id` and every popup above it, topmost first: children before
    /// their parent, newer siblings before older ones. One `Closed` each.
    pub(crate) fn close(&mut self, id: WindowId) -> Result<()> {
        self.ensure_live()?;
        self.validate_id(id)?;
        if !self.windows.contains_key(&id) {
            #[cfg(feature = "owned-native-system")]
            if self
                .native_attempt
                .is_some_and(|attempt| attempt.window != id)
            {
                return Err(Error::InvalidInput(
                    "another native frame attempt remains open",
                ));
            }
            return Ok(());
        }
        let order = self.close_order(id);
        #[cfg(feature = "owned-native-system")]
        if self
            .native_attempt
            .is_some_and(|attempt| !order.contains(&attempt.window))
        {
            return Err(Error::InvalidInput(
                "another native frame attempt remains open",
            ));
        }
        // Reserve every discrete notification before touching ownership.
        if !self.event_room(order.len()) {
            let error = Error::LimitExceeded;
            self.remember(&error);
            return Err(error);
        }
        for window in order {
            self.close_one(window)?;
        }
        Ok(())
    }

    /// `id` last, after every popup above it: deeper first, then newer first.
    pub(in crate::window) fn close_order(&self, id: WindowId) -> Vec<WindowId> {
        let depth_below = |window: WindowId| {
            let mut current = window;
            for depth in 1..=super::super::lease::MAX_WINDOWS {
                match self.windows.get(&current).and_then(|record| record.parent) {
                    Some(parent) if parent == id => return Some(depth),
                    Some(parent) => current = parent,
                    None => return None,
                }
            }
            None
        };
        let mut above: Vec<(usize, WindowId)> = self
            .windows
            .keys()
            .filter_map(|window| depth_below(*window).map(|depth| (depth, *window)))
            .collect();
        above.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let mut order: Vec<WindowId> = above.into_iter().map(|(_, window)| window).collect();
        order.push(id);
        order
    }

    /// Retire one window whose event slot is already reserved.
    fn close_one(&mut self, id: WindowId) -> Result<()> {
        if let Err(error) = self.text_input.close_window(id, &mut self.events) {
            self.remember(&error);
            return Err(error);
        }
        self.clipboard.close_window(id, &mut self.events);
        self.input
            .cancel(&mut self.events, Some(id), InputCancelReason::WindowClosed);
        if let Some(record) = self.windows.remove(&id) {
            #[cfg(feature = "owned-native-system")]
            if self
                .native_attempt
                .is_some_and(|attempt| attempt.window == id)
            {
                self.native_attempt = None;
            }
            record.native.retire();
            self.output.retire(id);
            self.presentation.close_window(id);
            self.events.push_back(WindowEvent::Closed { window: id });
            drop(record);
        }
        Ok(())
    }

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
