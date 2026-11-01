//! Shared immutable payload ownership and bounded descriptor ingestion.
use super::types::{HeldKeys, Keymap, SeatName};
use crate::{Error, Result};
use std::{
    fmt,
    fs::File,
    ops::Deref,
    os::{fd::OwnedFd, unix::fs::FileExt},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Clone)]
pub(super) struct Budget {
    used: Arc<AtomicUsize>,
    limit: usize,
}
impl Budget {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            used: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }
    pub(super) fn reserve(&self, amount: usize) -> Result<Charge> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(amount).filter(|next| *next <= self.limit)
            })
            .map_err(|_| Error::LimitExceeded)?;
        Ok(Charge {
            budget: self.clone(),
            amount,
        })
    }
}
pub(super) struct Charge {
    budget: Budget,
    amount: usize,
}
impl Charge {
    pub(super) fn grow(&mut self) -> Result<()> {
        let mut credit = self.budget.reserve(1)?;
        self.amount += credit.amount;
        credit.amount = 0;
        Ok(())
    }
}
impl Drop for Charge {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.amount, Ordering::AcqRel);
    }
}
pub(super) struct Data<T> {
    pub(super) value: T,
    _charge: Charge,
}
pub(super) struct Owned<T>(Arc<Data<T>>);
impl<T> Clone for Owned<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> Owned<T> {
    pub(super) fn new(value: T, charge: Charge) -> Self {
        Self(Arc::new(Data {
            value,
            _charge: charge,
        }))
    }
}
impl<T> Deref for Owned<T> {
    type Target = Data<T>;
    fn deref(&self) -> &Data<T> {
        &self.0
    }
}
impl<T: fmt::Debug> fmt::Debug for Owned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(f)
    }
}
impl<T: PartialEq> PartialEq for Owned<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}
impl<T: Eq> Eq for Owned<T> {}

pub(super) fn keymap(fd: OwnedFd, size: u32, budget: &Budget) -> Result<Keymap> {
    let file = File::from(fd); // Exclusive fd ownership; every exit drops this File once.
    let size = size as usize;
    if size == 0 || size > 1024 * 1024 {
        return Err(Error::LimitExceeded);
    }
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file() || metadata.len() < size as u64 {
        return Err(Error::InvalidInput(
            "keymap requires a regular file of sufficient length",
        ));
    }
    let charge = budget.reserve(size)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| Error::LimitExceeded)?;
    bytes.resize(size, 0);
    // A concurrent truncation is a checked short read, never an untrusted mmap.
    file.read_exact_at(&mut bytes, 0)?;
    if bytes.last() != Some(&0)
        || bytes[..size - 1].contains(&0)
        || std::str::from_utf8(&bytes[..size - 1]).is_err()
    {
        return Err(Error::InvalidInput(
            "keymap must be UTF-8 with exactly one final NUL",
        ));
    }
    Ok(Keymap(Owned::new(bytes.into_boxed_slice(), charge)))
}

pub(super) fn held_keys(bytes: &[u8], budget: &Budget) -> Result<HeldKeys> {
    if !bytes.len().is_multiple_of(4) || bytes.len() / 4 > 1024 {
        return Err(Error::InvalidInput("invalid held-key array"));
    }
    let charge = budget.reserve(bytes.len() / 4)?;
    let mut keys = Vec::new();
    keys.try_reserve_exact(bytes.len() / 4)
        .map_err(|_| Error::LimitExceeded)?;
    for code in bytes.as_chunks::<4>().0 {
        let code = u32::from_ne_bytes(*code);
        if keys.contains(&code) {
            return Err(Error::InvalidInput("duplicate held key"));
        }
        keys.push(code);
    }
    Ok(HeldKeys(Owned::new(keys.into_boxed_slice(), charge)))
}

pub(super) fn seat_name(name: String, budget: &Budget) -> Result<SeatName> {
    if name.len() > 256 || name.contains('\0') {
        return Err(Error::InvalidInput("invalid seat name"));
    }
    let charge = budget.reserve(name.len())?;
    Ok(SeatName(Owned::new(name.into_boxed_str(), charge)))
}

#[cfg(test)]
#[path = "tests/payload.rs"]
mod tests;
