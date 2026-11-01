//! Incoming pipe admission, typed completion and MIME fallback lifecycle.
use super::{Engine, abort};
use crate::{
    TransferError,
    dnd::{
        DndEvent, Payload, TransferId,
        interop::PayloadKind,
        transfer::{self, Done},
        uri,
    },
};
use std::{os::fd::AsFd, time::Instant};
use wayland_client::protocol::wl_data_offer;

impl Engine {
    pub(super) fn receive(
        &mut self,
        id: TransferId,
        wl: &wl_data_offer::WlDataOffer,
        mime: &str,
        started: Instant,
    ) -> std::result::Result<(), TransferError> {
        let (read, write) = transfer::open_pipe().map_err(|_| TransferError::Io)?;
        if started.elapsed() >= self.limits.total {
            return Err(TransferError::Timeout);
        }
        wl.receive(mime.into(), write.as_fd());
        drop(write);
        self.reactor
            .push_incoming(id, read, started, Instant::now());
        Ok(())
    }
    pub(super) fn fail_drop(&mut self, id: TransferId, reason: TransferError) {
        let Some(d) = self.drops.remove(&id) else {
            return;
        };
        self.reactor.cancel_incoming(id);
        abort(&d);
        d.wl.destroy();
        let (seat, target) = (d.seat, d.target);
        self.push(DndEvent::TransferFailed {
            seat,
            target,
            reason,
        });
    }
    /// Service pipes and turn finished reads into typed payloads.
    pub fn service_transfers(&mut self, now: Instant) {
        for done in self.reactor.service(now, &self.limits) {
            let Done::Incoming { id, result } = done else {
                continue;
            };
            self.deliver(id, result);
        }
    }
    fn deliver(&mut self, id: TransferId, result: std::result::Result<Vec<u8>, TransferError>) {
        let Some(d) = self.drops.get(&id) else {
            return;
        };
        let payload = result.and_then(|bytes| match d.kind {
            PayloadKind::Files => {
                uri::parse_uri_list(&bytes, self.limits.max_entries).map(Payload::Files)
            }
            PayloadKind::Utf8Text | PayloadKind::Latin1Text => {
                uri::decode_text(&bytes, d.kind).map(Payload::Text)
            }
            PayloadKind::Bytes => Ok(Payload::Bytes {
                mime: d.mime.clone(),
                data: bytes,
            }),
        });
        let payload = if d.started.elapsed() >= self.limits.total {
            Err(TransferError::Timeout)
        } else {
            payload
        };
        match payload {
            Ok(payload) => {
                let d = self.drops.get_mut(&id).unwrap();
                d.received = true;
                let (seat, target, action) = (d.seat, d.target, d.action);
                self.push(DndEvent::Dropped {
                    seat,
                    transfer: id,
                    target,
                    action,
                    payload,
                });
            }
            // A browser's uri-list holds https links: take the offer's text instead.
            Err(TransferError::Malformed) if !self.drops[&id].fallbacks.is_empty() => {
                if self.drops[&id].started.elapsed() >= self.limits.total {
                    self.fail_drop(id, TransferError::Timeout);
                    return;
                }
                let d = self.drops.get_mut(&id).unwrap();
                let (mime, kind) = d.fallbacks.pop_front().unwrap();
                (d.mime, d.kind) = (mime.clone(), kind);
                let (wl, started) = (d.wl.clone(), d.started);
                if let Err(reason) = self.receive(id, &wl, &mime, started) {
                    self.fail_drop(id, reason);
                }
            }
            Err(reason) => self.fail_drop(id, reason),
        }
    }
}
