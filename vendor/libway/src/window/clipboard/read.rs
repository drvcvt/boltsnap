//! Atomic receive admission, reserved terminal events and queued payload credits.
use super::*;
use std::{os::fd::AsFd, time::Duration};

impl WindowState {
    pub(crate) fn receive_clipboard(&mut self, read: ClipboardRead) -> Result<ClipboardTransferId> {
        let native = self.read_offer(&read)?;
        if !self.event_room(1) {
            return Err(Error::LimitExceeded);
        }
        self.clipboard
            .reads
            .try_reserve(1)
            .map_err(|_| Error::LimitExceeded)?;
        self.clipboard
            .queued
            .try_reserve(self.clipboard.reads.len() + 1)
            .map_err(|_| Error::LimitExceeded)?;
        let charge = self.clipboard.budget.reserve(read.max_bytes + 1)?;
        let (input, output) = transfer::open_pipe()?;
        let next = self
            .clipboard
            .next_transfer
            .checked_add(1)
            .ok_or(Error::LimitExceeded)?;
        let id = ClipboardTransferId(self.connection_id, next);
        let replace = self
            .clipboard
            .reads
            .iter()
            .find(|prior| prior.seat == read.seat)
            .map(|prior| prior.id);
        let replaced = self.clipboard.transfers.read(transfer::ReadAdmission {
            id,
            fd: input,
            max_bytes: read.max_bytes,
            deadline: read.deadline,
            replace,
        })?;
        self.clipboard.next_transfer = next;
        native.receive(read.mime, output.as_fd());
        drop(output);
        if let Some(completion) = replaced {
            self.clipboard.complete(completion, &mut self.events);
        }
        self.clipboard.reads.push(Read {
            id,
            window: read.window,
            seat: read.seat,
            _charge: charge,
        });
        Ok(id)
    }
    fn read_offer(&self, read: &ClipboardRead) -> Result<OfferNative> {
        self.ensure_active()?;
        self.record(read.window)?;
        // The offer id names its selection; an unknown one is checked against the
        // ordinary manager unless only the primary manager is live.
        let kind = match self
            .clipboard
            .offers
            .iter()
            .find(|offer| offer.id == read.offer)
        {
            Some(offer) => offer.native.kind(),
            None if !self.clipboard.manager_live && self.clipboard.primary_live => Kind::Primary,
            None => Kind::Clipboard,
        };
        let target = self.clipboard.target(read.window, read.seat, kind)?;
        if target.slot(kind).offer != Some(read.offer) {
            return Err(Error::InvalidInput("clipboard offer is stale"));
        }
        let offer = self
            .clipboard
            .offers
            .iter()
            .find(|offer| offer.id == read.offer && offer.seat == read.seat)
            .ok_or(Error::InvalidInput("clipboard offer is stale"))?;
        if !offer.mimes.contains(&read.mime) {
            return Err(Error::InvalidInput("clipboard MIME was not offered"));
        }
        let now = Instant::now();
        if !(1..=1024 * 1024).contains(&read.max_bytes)
            || read.deadline <= now
            || read.deadline.saturating_duration_since(now) > Duration::from_secs(30)
        {
            return Err(Error::InvalidInput(
                "invalid clipboard receive limit/deadline",
            ));
        }
        Ok(offer.native.clone())
    }
    pub(crate) fn cancel_clipboard(&mut self, id: ClipboardTransferId) -> Result<bool> {
        self.ensure_active()?;
        if id.0 != self.connection_id || id.1 == 0 || id.1 > self.clipboard.next_transfer {
            return Err(Error::InvalidInput("unknown clipboard transfer"));
        }
        Ok(self.clipboard.cancel(id, &mut self.events))
    }
}
impl Clipboard {
    pub(super) fn cancel(
        &mut self,
        id: ClipboardTransferId,
        events: &mut VecDeque<WindowEvent>,
    ) -> bool {
        let Some(completion) = self.transfers.cancel(id) else {
            return false;
        };
        self.complete(completion, events);
        true
    }
    pub(super) fn complete(
        &mut self,
        completion: transfer::Completion,
        events: &mut VecDeque<WindowEvent>,
    ) {
        let Some(index) = self.reads.iter().position(|read| read.id == completion.id) else {
            return;
        };
        let read = self.reads.remove(index);
        self.queued.push(read._charge);
        events.push_back(WindowEvent::Clipboard(ClipboardEvent::Received {
            transfer: read.id,
            window: read.window,
            seat: read.seat,
            outcome: completion.outcome,
        }));
    }
    pub(crate) fn service(
        &mut self,
        now: Instant,
        events: &mut VecDeque<WindowEvent>,
    ) -> Result<bool> {
        let service = self.transfers.service(now);
        for completion in service.completions.into_iter().flatten() {
            self.complete(completion, events);
        }
        if let Some(error) = service.failure {
            return Err(error);
        }
        Ok(service.runnable || service.bytes > 0)
    }
}
