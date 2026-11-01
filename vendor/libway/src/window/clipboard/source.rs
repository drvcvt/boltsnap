//! Eager bounded source installation for both selections and source retirement
//! independent of window life.
use super::*;
use std::{os::fd::OwnedFd, time::Duration};

fn validate(entries: &[ClipboardRepresentation]) -> Result<()> {
    if entries.is_empty() || entries.len() > 8 {
        return Err(Error::InvalidInput(
            "clipboard source requires 1..8 representations",
        ));
    }
    let mut bytes = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        if entry.mime.is_empty()
            || entry.mime.len() > 256
            || entry.mime.contains('\0')
            || entries[..index]
                .iter()
                .any(|prior| prior.mime == entry.mime)
        {
            return Err(Error::InvalidInput("invalid or duplicate clipboard MIME"));
        }
        bytes = bytes
            .checked_add(entry.bytes.len())
            .ok_or(Error::LimitExceeded)?;
    }
    if bytes > 2 * 1024 * 1024 {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}
impl WindowState {
    pub(crate) fn set_clipboard(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        entries: &[ClipboardRepresentation],
    ) -> Result<ClipboardSourceId> {
        self.set_selection(Kind::Clipboard, window, seat, entries)
    }
    pub(crate) fn set_primary_selection(
        &mut self,
        window: WindowId,
        seat: InputSeatId,
        entries: &[ClipboardRepresentation],
    ) -> Result<ClipboardSourceId> {
        self.set_selection(Kind::Primary, window, seat, entries)
    }
    fn set_selection(
        &mut self,
        kind: Kind,
        window: WindowId,
        seat: InputSeatId,
        entries: &[ClipboardRepresentation],
    ) -> Result<ClipboardSourceId> {
        self.ensure_active()?;
        self.record(window)?;
        let target = self.clipboard.target(window, seat, kind)?;
        let serial = target.press.ok_or(Error::NotReady)?;
        let device = target.device(kind).ok_or(Error::NotReady)?;
        validate(entries)?;
        if !self.event_room(1) {
            return Err(Error::LimitExceeded);
        }
        let retained = self.clipboard.retain_source(entries)?;
        self.clipboard
            .sources
            .try_reserve(1)
            .map_err(|_| Error::LimitExceeded)?;
        let (host, next) =
            Clipboard::identity(&mut self.clipboard.next_source, self.connection_id)?;
        let id = ClipboardSourceId(host, next);
        let native = self
            .clipboard
            .create_source(device, id, &retained, serial, &self.qh)?;
        self.clipboard.installed(
            Source {
                id,
                native,
                entries: retained,
            },
            seat,
            kind,
            &mut self.events,
        );
        Ok(id)
    }
}
impl Clipboard {
    fn retain_source(
        &mut self,
        entries: &[ClipboardRepresentation],
    ) -> Result<Vec<(String, Payload)>> {
        let mut retained = Vec::new();
        retained
            .try_reserve_exact(entries.len())
            .map_err(|_| Error::LimitExceeded)?;
        for entry in entries {
            retained.push((entry.mime.clone(), self.budget.payload(&entry.bytes)?));
        }
        Ok(retained)
    }
    // Offers every MIME and claims the selection before the source is recorded.
    fn create_source(
        &self,
        device: Device,
        id: ClipboardSourceId,
        retained: &[(String, Payload)],
        serial: u32,
        qh: &QueueHandle<WindowState>,
    ) -> Result<SourceNative> {
        Ok(match device {
            Device::Clipboard(device) => {
                let manager = &self.manager.as_ref().ok_or(Error::NotReady)?.1;
                let native = manager.create_data_source(qh, id);
                for (mime, _) in retained {
                    native.offer(mime.clone());
                }
                device.set_selection(Some(&native), serial);
                SourceNative::Clipboard(native)
            }
            Device::Primary(device) => {
                let manager = &self.primary_manager.as_ref().ok_or(Error::NotReady)?.1;
                let native = manager.create_source(qh, id);
                for (mime, _) in retained {
                    native.offer(mime.clone());
                }
                device.set_selection(Some(&native), serial);
                SourceNative::Primary(native)
            }
        })
    }
    fn installed(
        &mut self,
        source: Source,
        seat: InputSeatId,
        kind: Kind,
        events: &mut VecDeque<WindowEvent>,
    ) {
        if let Some(old) = self
            .seats
            .iter()
            .find(|entry| entry.id == seat)
            .and_then(|entry| entry.slot(kind).source)
        {
            self.retire_source(old, events);
        }
        if let Some(target) = self.seats.iter_mut().find(|target| target.id == seat) {
            target.slot_mut(kind).source = Some(source.id);
        }
        self.sources.push(source);
    }
    pub(super) fn retire_source(
        &mut self,
        id: ClipboardSourceId,
        events: &mut VecDeque<WindowEvent>,
    ) {
        let Some(index) = self.sources.iter().position(|source| source.id == id) else {
            return;
        };
        self.sources.remove(index).native.destroy();
        for slot in self.seats.iter_mut().flat_map(|seat| &mut seat.slots) {
            if slot.source == Some(id) {
                slot.source = None;
            }
        }
        events.push_back(WindowEvent::Clipboard(ClipboardEvent::SourceCancelled {
            source: id,
        }));
    }
    pub(super) fn send(&mut self, id: ClipboardSourceId, mime: &str, fd: OwnedFd) -> Result<()> {
        let Some(source) = self.sources.iter().find(|source| source.id == id) else {
            return Ok(());
        };
        let Some((_, payload)) = source.entries.iter().find(|(name, _)| name == mime) else {
            return Ok(());
        };
        let Some(seat) = self
            .seats
            .iter()
            .find(|seat| seat.slots.iter().any(|slot| slot.source == Some(id)))
            .map(|seat| seat.id)
        else {
            return Ok(());
        };
        let deadline = Instant::now()
            .checked_add(Duration::from_secs(5))
            .ok_or(Error::LimitExceeded)?;
        self.transfers.send(fd, payload.clone(), deadline, seat)
    }
}
