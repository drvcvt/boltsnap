//! The readback's host buffers: idle ones for reuse, and leased ones whose
//! mapping a tile reads directly until it is dropped.
use super::{host::HostBuffer, owner::DeviceOwner};
use crate::capture::ReadbackFallback;
use ash::vk;
use std::{
    mem::ManuallyDrop,
    sync::{Arc, Mutex, MutexGuard},
};

/// Idle host buffers kept for reuse; more are freed when they come back. Two
/// cover a two-output shot without allocating; measured on two 1920x1080
/// outputs: 1 costs about a millisecond per shot, 4 keeps a burst's buffers
/// (+16 MiB idle). See docs/plans/phase-a/2026-11-01-mapped-tiles.md.
pub(super) const MAX_IDLE: usize = 2;
/// Host buffers leased out at once: four outputs, two shots in flight. At the
/// cap, reads copy into the heap instead of leasing, so the pool stays bounded.
pub(super) const MAX_LEASED: usize = 8;

#[derive(Default)]
struct PoolState {
    /// Unused buffers, oldest first.
    idle: Vec<HostBuffer>,
    leased: usize,
    /// The readback was dropped: returned buffers are freed, not kept.
    closed: bool,
}

/// Host buffers of one readback, shared with its leases.
#[derive(Default)]
pub(crate) struct HostPool(Arc<Mutex<PoolState>>);

impl HostPool {
    fn lock(&self) -> MutexGuard<'_, PoolState> {
        lock(&self.0)
    }

    /// An idle buffer of exactly `size` bytes, or a new one.
    pub(super) fn take(
        &self,
        device: &ash::Device,
        memory: &vk::PhysicalDeviceMemoryProperties,
        size: u64,
    ) -> Result<HostBuffer, ReadbackFallback> {
        let reused = {
            let mut state = self.lock();
            let slot = state.idle.iter().position(|host| host.size == size);
            slot.map(|slot| state.idle.remove(slot))
        };
        match reused {
            Some(host) => Ok(host),
            None => HostBuffer::new(device, memory, size, vk::BufferUsageFlags::TRANSFER_DST),
        }
    }

    /// Whether a buffer may be leased now (`MAX_LEASED`), counting it if so.
    pub(super) fn reserve_lease(&self) -> bool {
        let mut state = self.lock();
        let free = state.leased < MAX_LEASED;
        state.leased += usize::from(free);
        free
    }

    /// Wrap `host`, reserved with `reserve_lease`, as a lease of `len` bytes.
    pub(super) fn lease(
        &self,
        host: HostBuffer,
        len: usize,
        owner: &Arc<DeviceOwner>,
    ) -> HostLease {
        HostLease {
            host: ManuallyDrop::new(host),
            len,
            pool: Arc::clone(&self.0),
            owner: Arc::clone(owner),
        }
    }

    /// Return an unused buffer: kept idle, evicting the oldest past `MAX_IDLE`.
    pub(super) fn give(&self, device: &ash::Device, host: HostBuffer) {
        let evicted = {
            let mut state = self.lock();
            state.idle.push(host);
            (state.idle.len() > MAX_IDLE).then(|| state.idle.remove(0))
        };
        if let Some(old) = evicted {
            old.destroy(device);
        }
    }

    /// Free every idle buffer; leases still out free theirs when dropped.
    /// The device must be idle.
    pub(super) fn close(&self, device: &ash::Device) {
        let idle = {
            let mut state = self.lock();
            state.closed = true;
            std::mem::take(&mut state.idle)
        };
        for host in idle {
            host.destroy(device);
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> (usize, usize) {
        let state = self.lock();
        (state.idle.len(), state.leased)
    }
}

/// A tile's bytes: a host buffer's mapping, owned until drop. Keeps the
/// device alive, so it may outlive the readback and the connection.
pub(crate) struct HostLease {
    host: ManuallyDrop<HostBuffer>,
    len: usize,
    pool: Arc<Mutex<PoolState>>,
    owner: Arc<DeviceOwner>,
}

impl HostLease {
    pub(crate) fn bytes(&self) -> &[u8] {
        self.host.bytes(self.len)
    }
}

impl Drop for HostLease {
    fn drop(&mut self) {
        // SAFETY: taken once, here; `self.host` is not used afterwards.
        let host = unsafe { ManuallyDrop::take(&mut self.host) };
        let mut state = lock(&self.pool);
        state.leased -= 1;
        if self.owner.is_leaking() {
            // The device could not be drained: never free its memory. A
            // `HostBuffer` has no `Drop`, so letting it go leaks it.
            let _leaked = host;
        } else if state.closed || state.idle.len() >= MAX_IDLE {
            drop(state);
            host.destroy(&self.owner.device);
        } else {
            state.idle.push(host);
        }
    }
}

/// The pool's lock; a panic elsewhere never makes the counts unusable.
fn lock(pool: &Mutex<PoolState>) -> MutexGuard<'_, PoolState> {
    pool.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
