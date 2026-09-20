use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

/// A permit stays with queued work until the event loop has consumed it.
#[derive(Clone)]
pub(super) struct Limit {
    active: Arc<AtomicUsize>,
    maximum: usize,
}

impl Limit {
    pub(super) fn new(maximum: usize) -> Self {
        Self {
            active: Arc::new(AtomicUsize::new(0)),
            maximum,
        }
    }

    pub(super) fn acquire(&self) -> Option<Permit> {
        self.active
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                (n < self.maximum).then_some(n + 1)
            })
            .ok()
            .map(|_| Permit(self.active.clone()))
    }
}

pub(crate) struct Permit(Arc<AtomicUsize>);

impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(super) struct ClientLimits {
    pub(super) readers: Limit,
    pub(super) images: Limit,
}

impl Default for ClientLimits {
    fn default() -> Self {
        Self {
            readers: Limit::new(16),
            images: Limit::new(2),
        }
    }
}
