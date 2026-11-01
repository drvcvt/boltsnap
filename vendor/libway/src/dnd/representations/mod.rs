//! Eager byte ownership before dispatch; stateful Lazy providers keep their contract.
mod budget;
mod encoding;

use super::{Actions, DragData, DragIcon, DragRequest, SeatId, interop};
use crate::{Error, Result, SurfaceHandle};
use budget::Budget;
use std::{
    ops::Deref,
    sync::{Arc, Weak},
};

/// Payload policy for [`super::Dnd::start_drag_bounded`], independent of incoming limits.
#[derive(Debug, Clone, Copy)]
pub struct OutgoingLimits {
    /// Maximum bytes in one representation (default 1 MiB); excess is refused whole.
    pub max_payload: usize,
    /// Unique payload-buffer capacity per drag, including preparation overlap (default 2 MiB).
    /// MIME metadata and icons retain their separate count/dimension bounds. A Lazy callback
    /// can allocate or block before returning; only its returned buffer can be admitted.
    pub max_eager_bytes: usize,
}
impl Default for OutgoingLimits {
    fn default() -> Self {
        Self {
            max_payload: 1 << 20,
            max_eager_bytes: 2 << 20,
        }
    }
}

#[derive(Clone)]
pub(crate) struct Bytes(Arc<Vec<u8>>);
impl From<Vec<u8>> for Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(Arc::new(bytes))
    }
}
impl Deref for Bytes {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

enum Payload {
    Eager {
        entries: Vec<(String, Bytes)>,
        insensitive: bool,
    },
    Lazy {
        provide: super::Provider,
        live: Vec<Weak<Vec<u8>>>,
    },
}
pub(crate) struct Data {
    mimes: Vec<String>,
    payload: Payload,
    limits: Option<OutgoingLimits>,
}
pub(crate) struct Request {
    pub origin: SurfaceHandle,
    pub seat: Option<SeatId>,
    pub serial: Option<u32>,
    pub actions: Actions,
    pub icon: Option<DragIcon>,
    pub data: Data,
}
impl Request {
    pub fn new(
        request: DragRequest,
        max_mimes: usize,
        limits: Option<OutgoingLimits>,
    ) -> Result<Self> {
        validate(&request.data, max_mimes)?;
        if let Some(icon) = &request.icon {
            super::drag::validate_icon(icon)?;
        }
        Ok(Self {
            origin: request.origin,
            seat: request.seat,
            serial: request.serial,
            actions: request.actions,
            icon: request.icon,
            data: Data::new(request.data, max_mimes, limits)?,
        })
    }
}
impl Data {
    pub fn new(data: DragData, max_mimes: usize, limits: Option<OutgoingLimits>) -> Result<Self> {
        validate(&data, max_mimes)?;
        let mimes = interop::source_mimes(&data);
        let payload = match data {
            DragData::Lazy { provide, .. } => Payload::Lazy {
                provide,
                live: Vec::new(),
            },
            data => {
                let mut budget = Budget::new(limits, encoding::capacity(&data)?)?;
                let insensitive = !matches!(data, DragData::Static(_));
                let entries = encoding::prepare(data, &mut budget)?;
                Payload::Eager {
                    entries,
                    insensitive,
                }
            }
        };
        Ok(Self {
            mimes,
            payload,
            limits,
        })
    }
    pub fn mimes(&self) -> &[String] {
        &self.mimes
    }
    pub fn bounded(&self) -> bool {
        self.limits.is_some()
    }
    pub fn send(&mut self, mime: &str) -> Option<Bytes> {
        match &mut self.payload {
            Payload::Eager {
                entries,
                insensitive,
            } => entries
                .iter()
                .find(|(name, _)| {
                    if *insensitive {
                        name.eq_ignore_ascii_case(mime)
                    } else {
                        name == mime
                    }
                })
                .map(|(_, bytes)| bytes.clone()),
            Payload::Lazy { provide, live } => {
                if !self.mimes.iter().any(|name| name == mime) {
                    return None;
                }
                // Core's exclusive send admission has already reserved the available pipe slot.
                // The callback remains synchronous and may allocate before it returns.
                let bytes = provide(mime)?;
                live.retain(|owner| owner.strong_count() > 0);
                let retained = live
                    .iter()
                    .filter_map(Weak::upgrade)
                    .try_fold(0usize, |sum, owner| sum.checked_add(owner.capacity()))?;
                let mut budget = Budget::new(self.limits, retained).ok()?;
                budget.payload(bytes.len()).ok()?;
                budget.reserve(bytes.capacity()).ok()?;
                let bytes = Bytes::from(bytes);
                live.push(Arc::downgrade(&bytes.0));
                Some(bytes)
            }
        }
    }
}
fn validate(data: &DragData, max_mimes: usize) -> Result<()> {
    let fixed;
    let mimes: &[String] = match data {
        DragData::Lazy { mimes, .. } => mimes,
        DragData::Static(entries) => {
            if entries.len() > max_mimes {
                return Err(Error::LimitExceeded);
            }
            for (mime, _) in entries {
                validate_mime(mime)?;
            }
            return Ok(());
        }
        _ => {
            fixed = interop::source_mimes(data);
            &fixed
        }
    };
    if mimes.len() > max_mimes {
        return Err(Error::LimitExceeded);
    }
    for mime in mimes {
        validate_mime(mime)?;
    }
    Ok(())
}
fn validate_mime(mime: &str) -> Result<()> {
    if mime.is_empty() || mime.contains('\0') {
        return Err(Error::InvalidInput(
            "MIME names must be nonempty and NUL-free",
        ));
    }
    if mime.len() > 1024 {
        return Err(Error::LimitExceeded);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
