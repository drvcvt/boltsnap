//! Weak allocation records count escaped Arc clones before each native admission.
use std::{
    cell::RefCell,
    rc::Rc,
    sync::{Arc, Weak},
};
const LIMIT: usize = 128 * 1024;
const RECORDS: usize = 1024;
#[derive(Clone, Default)]
pub(super) struct Budget {
    allocations: Rc<RefCell<Vec<Weak<str>>>>,
}
pub(super) type Payload = Arc<str>;
impl Budget {
    /// A rejection is a data violation of the pending batch, never a host failure.
    pub(super) fn text(&self, text: String) -> Result<Payload, &'static str> {
        if text.len() > 4096 {
            return Err("IME text over 4096 bytes");
        }
        if text.contains('\0') {
            return Err("NUL in IME text");
        }
        let mut entries = self.allocations.borrow_mut();
        entries.retain(|entry| entry.strong_count() != 0);
        let used: usize = entries
            .iter()
            .filter_map(Weak::upgrade)
            .map(|text| text.len())
            .sum();
        if entries.len() >= RECORDS || used.checked_add(text.len()).is_none_or(|next| next > LIMIT)
        {
            return Err("IME payload ledger limit");
        }
        entries
            .try_reserve(1)
            .map_err(|_| "IME payload ledger allocation")?;
        let text: Arc<str> = Arc::from(text);
        entries.push(Arc::downgrade(&text));
        Ok(text)
    }
    #[cfg(test)]
    pub(super) fn used(&self) -> usize {
        self.allocations
            .borrow()
            .iter()
            .filter_map(Weak::upgrade)
            .map(|text| text.len())
            .sum()
    }
}
