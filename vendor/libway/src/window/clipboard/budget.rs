//! Shared host-held payload charges, independent of application-owned Arc aliases.
use crate::{Error, Result};
use std::sync::{
    Arc, Weak,
    atomic::{AtomicUsize, Ordering},
};

#[cfg(test)]
#[path = "budget_tests.rs"]
mod tests;

const LIMIT: usize = 32 * 1024 * 1024;
#[derive(Default)]
pub(super) struct Budget {
    used: Arc<AtomicUsize>,
    allocations: Vec<Weak<Data>>,
}
pub(super) struct Charge {
    used: Arc<AtomicUsize>,
    amount: usize,
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.used.fetch_sub(self.amount, Ordering::AcqRel);
    }
}
struct Data {
    bytes: Arc<[u8]>,
    _charge: Charge,
}
/// Immutable bytes and their one host charge, retained by sources and accepted sends.
#[derive(Clone)]
pub(crate) struct Payload(Arc<Data>);
impl Payload {
    pub(crate) fn bytes(&self) -> &Arc<[u8]> {
        &self.0.bytes
    }
}
impl Budget {
    pub(super) fn reserve(&self, amount: usize) -> Result<Charge> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(amount).filter(|next| *next <= LIMIT)
            })
            .map_err(|_| Error::LimitExceeded)?;
        Ok(Charge {
            used: self.used.clone(),
            amount,
        })
    }
    pub(super) fn payload(&mut self, bytes: &Arc<[u8]>) -> Result<Payload> {
        self.allocations.retain(|entry| entry.strong_count() > 0);
        if let Some(data) = self
            .allocations
            .iter()
            .filter_map(Weak::upgrade)
            .find(|data| Arc::ptr_eq(&data.bytes, bytes))
        {
            return Ok(Payload(data));
        }
        self.allocations
            .try_reserve(1)
            .map_err(|_| Error::LimitExceeded)?;
        let charge = self.reserve(bytes.len())?;
        let data = Arc::new(Data {
            bytes: bytes.clone(),
            _charge: charge,
        });
        self.allocations.push(Arc::downgrade(&data));
        Ok(Payload(data))
    }
}
