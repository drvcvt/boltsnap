//! Shared byte credits and SHM buffers writable only outside compositor use.
use super::{damage::PixelRect, layout::Layout};
use crate::{Error, Result, buffer::ShmBuffer, window::WindowId};
use std::{cell::Cell, os::fd::AsFd, rc::Rc};
use wayland_client::{
    Dispatch, Proxy, QueueHandle,
    protocol::{wl_buffer::WlBuffer, wl_shm::WlShm, wl_shm_pool::WlShmPool},
};

pub(crate) const MAX_SHM_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Default)]
pub(crate) struct Budget {
    used: Rc<Cell<usize>>,
}

impl Budget {
    pub(crate) fn used(&self) -> usize {
        self.used.get()
    }

    pub(crate) fn remaining(&self) -> usize {
        // All admissions are checked against this constant before changing used.
        MAX_SHM_BYTES - self.used()
    }

    pub(crate) fn reserve(&self, bytes: usize) -> Result<Credit> {
        if bytes == 0 {
            return Err(Error::InvalidDimensions);
        }
        let next = self.used().checked_add(bytes).ok_or(Error::LimitExceeded)?;
        if next > MAX_SHM_BYTES {
            return Err(Error::LimitExceeded);
        }
        self.used.set(next);
        Ok(Credit {
            used: self.used.clone(),
            bytes,
        })
    }
}

pub(crate) struct Credit {
    used: Rc<Cell<usize>>,
    bytes: usize,
}

impl Drop for Credit {
    fn drop(&mut self) {
        // Each non-clonable credit is issued once after adding exactly its charge.
        self.used.set(self.used.get() - self.bytes);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BufferToken {
    pub(crate) window: WindowId,
    pub(crate) sequence: u64,
}

pub(crate) struct Slot {
    buffer: WlBuffer,
    shm: ShmBuffer,
    // Field drop order releases the mapping before returning its byte credit.
    _credit: Credit,
    token: BufferToken,
    layout: Layout,
    busy: bool,
    // Sequence of the presented frame this storage still holds, if known.
    valid: Option<u64>,
}

impl Slot {
    pub(crate) fn new<S>(
        shm: &WlShm,
        qh: &QueueHandle<S>,
        token: BufferToken,
        layout: Layout,
        budget: &Budget,
    ) -> Result<Self>
    where
        S: Dispatch<WlShmPool, ()> + Dispatch<WlBuffer, BufferToken> + 'static,
    {
        if !shm.is_alive() {
            return Err(Error::Unsupported("wl_shm is unavailable"));
        }
        let size = i32::try_from(layout.bytes).map_err(|_| Error::LimitExceeded)?;
        let credit = budget.reserve(layout.bytes)?;
        let storage = ShmBuffer::new(layout.bytes)?;
        let pool = shm.create_pool(storage.file.as_fd(), size, qh, ());
        let buffer = pool.create_buffer(
            0,
            layout.width,
            layout.height,
            layout.stride,
            layout.format.wire(),
            qh,
            token,
        );
        // wl_shm_pool.destroy retains server mappings while its buffers exist.
        pool.destroy();
        if !buffer.is_alive() {
            buffer.destroy();
            return Err(Error::Wayland("CPU SHM buffer creation failed".into()));
        }
        Ok(Self {
            buffer,
            shm: storage,
            _credit: credit,
            token,
            layout,
            busy: false,
            valid: None,
        })
    }

    pub(crate) fn buffer(&self) -> &WlBuffer {
        &self.buffer
    }

    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }

    pub(crate) fn token(&self) -> BufferToken {
        self.token
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.busy
    }

    pub(crate) fn valid(&self) -> Option<u64> {
        self.valid
    }

    // Copies the rects, or the whole frame for None. Content is unknown until
    // mark_busy records the frame, so an early error forces a later full copy.
    pub(crate) fn write(&mut self, rgba: &[u8], rects: Option<&[PixelRect]>) -> Result<()> {
        if self.busy {
            return Err(Error::InvalidInput("CPU buffer awaits compositor release"));
        }
        self.valid = None;
        let Some(rects) = rects else {
            return self.layout.copy_rgba(rgba, &mut self.shm.map);
        };
        rects
            .iter()
            .try_for_each(|rect| self.layout.copy_rgba_rect(rgba, &mut self.shm.map, *rect))
    }

    // The host marks busy before committing, and treats later wire failure as
    // teardown rather than proving that compositor access never began.
    pub(crate) fn mark_busy(&mut self, frame: u64) -> Result<()> {
        if self.busy {
            return Err(Error::InvalidInput("CPU buffer is already busy"));
        }
        self.busy = true;
        self.valid = Some(frame);
        Ok(())
    }

    // Only the matching wl_buffer.release handler may grant this transition.
    // Frame callback completion is independent and must not call it.
    pub(crate) fn release(&mut self) {
        self.busy = false;
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        // A busy buffer may be destroyed during teardown, but its storage must
        // never be rewritten/reused. The compositor owns its independent mapping.
        self.buffer.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_budget_accounts_exact_boundary_and_returns_credits() {
        let budget = Budget::default();
        let other_window = budget.clone();
        let first = budget.reserve(MAX_SHM_BYTES - 1).unwrap();
        let second = other_window.reserve(1).unwrap();
        assert_eq!(budget.used(), MAX_SHM_BYTES);
        assert_eq!(other_window.remaining(), 0);
        assert!(matches!(budget.reserve(1), Err(Error::LimitExceeded)));
        drop(first);
        assert_eq!(other_window.used(), 1);
        drop(second);
        assert_eq!(budget.used(), 0);
    }

    #[test]
    fn failed_admission_preserves_existing_credit() {
        let budget = Budget::default();
        let credit = budget.reserve(32).unwrap();
        for bytes in [0, MAX_SHM_BYTES, usize::MAX] {
            assert!(budget.reserve(bytes).is_err());
            assert_eq!(budget.used(), 32);
        }
        drop(credit);
        assert_eq!(budget.remaining(), MAX_SHM_BYTES);
    }

    #[test]
    fn credits_outlive_the_original_budget_handle() {
        let budget = Budget::default();
        let surviving_owner = budget.clone();
        let credit = budget.reserve(64).unwrap();
        drop(budget);
        assert_eq!(surviving_owner.used(), 64);
        drop(credit);
        assert_eq!(surviving_owner.used(), 0);
    }
}
