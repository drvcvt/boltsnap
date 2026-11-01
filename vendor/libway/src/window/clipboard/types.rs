//! Owned clipboard metadata and outcomes, without protocol or descriptor access.
use super::super::{InputSeatId, WindowId};
use crate::Error;
use std::{sync::Arc, time::Instant};

/// Host-provenanced selection offer identity, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipboardOfferId(pub(crate) u64, pub(crate) u64);
/// Host-provenanced installed source identity, never reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipboardSourceId(pub(crate) u64, pub(crate) u64);
/// Host-provenanced receive identity, never reused, retaining no host resources.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ClipboardTransferId(pub(crate) u64, pub(crate) u64);

/// One immutable eager source representation. Identical Arc aliases share storage.
#[derive(Clone, Debug)]
pub struct ClipboardRepresentation {
    /// Exact MIME name, at most 256 bytes, without NUL.
    pub mime: String,
    /// Exact bytes, without encoding or newline policy.
    pub bytes: Arc<[u8]>,
}
/// Immutable selection metadata. Permission to receive also requires live focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardOffer {
    /// Exact offer identity on this host.
    pub id: ClipboardOfferId,
    /// At most 32 advertised MIME names of at most 256 bytes each.
    pub mimes: Vec<String>,
}
/// An asynchronous receive bound to one exact offer and focused native target.
#[derive(Clone, Debug)]
pub struct ClipboardRead {
    /// Live native target on this host.
    pub window: WindowId,
    /// Live seat with keyboard focus on the target.
    pub seat: InputSeatId,
    /// Exact current selection offer.
    pub offer: ClipboardOfferId,
    /// One exact advertised MIME name.
    pub mime: String,
    /// Payload bound, in 1..=1 MiB, with no partial delivery.
    pub max_bytes: usize,
    /// Future absolute deadline, no more than 30 seconds from admission.
    pub deadline: Instant,
}
/// Exactly one terminal receive result. Failed equality retains cause identity.
#[derive(Clone, Debug)]
pub enum ClipboardReceiveOutcome {
    /// Complete bytes after EOF, including an offered empty payload.
    Bytes(Vec<u8>),
    /// Explicit cancellation or target/focus retirement.
    Cancelled,
    /// The complete representation exceeded the request's bound.
    TooLarge,
    /// The absolute deadline expired before completion.
    TimedOut,
    /// Exact retained operating-system or resource failure.
    Failed(Arc<Error>),
}
impl PartialEq for ClipboardReceiveOutcome {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Bytes(a), Self::Bytes(b)) => a == b,
            (Self::Cancelled, Self::Cancelled)
            | (Self::TooLarge, Self::TooLarge)
            | (Self::TimedOut, Self::TimedOut) => true,
            (Self::Failed(a), Self::Failed(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}
/// Owned clipboard and primary selection notifications. No UTF-8 or editor
/// policy is applied.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ClipboardEvent {
    /// Per-seat current offer. It may arrive before keyboard Enter.
    Selection {
        /// Seat whose current selection changed.
        seat: InputSeatId,
        /// Immutable current offer, or no selection.
        offer: Option<ClipboardOffer>,
    },
    /// Per-seat current primary offer, independent of `Selection`. The protocol
    /// does not promise it before keyboard Enter.
    PrimarySelection {
        /// Seat whose current primary selection changed.
        seat: InputSeatId,
        /// Immutable current primary offer, or no primary selection.
        offer: Option<ClipboardOffer>,
    },
    /// Installed clipboard or primary source was replaced or cancelled by the compositor.
    SourceCancelled {
        /// Exact installed source that retired.
        source: ClipboardSourceId,
    },
    /// Exactly one outcome for an admitted read, unless the host terminates.
    Received {
        /// Exact admitted read.
        transfer: ClipboardTransferId,
        /// Original target.
        window: WindowId,
        /// Original seat.
        seat: InputSeatId,
        /// Whole terminal result.
        outcome: ClipboardReceiveOutcome,
    },
}
