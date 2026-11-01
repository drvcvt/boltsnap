//! Known payload capacity during preparation; MIME/icon bounds remain separate.
use super::OutgoingLimits;
use crate::{Error, Result};

pub(super) struct Budget {
    limits: Option<OutgoingLimits>,
    live: usize,
}
impl Budget {
    pub(super) fn new(limits: Option<OutgoingLimits>, live: usize) -> Result<Self> {
        let mut budget = Self { limits, live: 0 };
        budget.reserve(live)?;
        Ok(budget)
    }
    pub(super) fn payload(&self, length: usize) -> Result<()> {
        if self
            .limits
            .is_some_and(|limits| length > limits.max_payload)
        {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
    pub(super) fn reserve(&mut self, capacity: usize) -> Result<()> {
        let live = self
            .live
            .checked_add(capacity)
            .ok_or(Error::LimitExceeded)?;
        if self
            .limits
            .is_some_and(|limits| live > limits.max_eager_bytes)
        {
            return Err(Error::LimitExceeded);
        }
        self.live = live;
        Ok(())
    }
    pub(super) fn release(&mut self, capacity: usize) {
        self.live = self
            .live
            .checked_sub(capacity)
            .expect("invariant: reserved payload capacities are balanced");
    }
}
