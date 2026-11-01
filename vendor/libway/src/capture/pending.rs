//! Per-capture protocol state collected from events until a frame is ready.
use super::*;

#[derive(Default, Clone)]
pub(crate) struct Constraints {
    pub size: (u32, u32),
    pub shm: Vec<wl_shm::Format>,
    pub stride: Option<u32>,
    pub dma: Vec<(u32, Vec<u64>)>,
    pub dma_any_modifier: bool,
    pub device: Option<u64>,
}
pub(super) struct Pending {
    pub(super) token: u64,
    pub(super) constraints: Constraints,
    pub(super) frame_size: (u32, u32),
    pub(super) batch: Constraints,
    pub(super) batch_open: bool,
    pub(super) formats_done: bool,
    pub(super) ready: bool,
    pub(super) error: Option<Error>,
    pub(super) transform: Option<Transform>,
    pub(super) inverted: bool,
    pub(super) timestamp: Option<Duration>,
    pub(super) damage: Vec<(u32, u32, u32, u32)>,
    pub(super) imported: Option<wl_buffer::WlBuffer>,
    pub(super) import_done: bool,
}
impl Pending {
    pub(super) fn new(token: u64) -> Self {
        Self {
            token,
            constraints: Constraints::default(),
            frame_size: (0, 0),
            batch: Constraints::default(),
            batch_open: false,
            formats_done: false,
            ready: false,
            error: None,
            transform: None,
            inverted: false,
            timestamp: None,
            damage: Vec::new(),
            imported: None,
            import_done: false,
        }
    }
    pub(super) fn batch(&mut self) -> &mut Constraints {
        if !self.batch_open {
            self.batch = Constraints::default();
            self.batch_open = true;
        }
        &mut self.batch
    }
    pub(super) fn reset_frame(&mut self) {
        self.ready = false;
        self.frame_size = (0, 0);
        self.error = None;
        self.transform = None;
        self.inverted = false;
        self.timestamp = None;
        self.damage.clear();
        self.import_done = false;
        if let Some(buffer) = self.imported.take() {
            buffer.destroy();
        }
    }
}
impl Drop for Pending {
    fn drop(&mut self) {
        // An import can finish in the same event batch as a failure/cancellation.
        // Keep ownership here until it has explicitly moved into Objects.
        if let Some(buffer) = self.imported.take() {
            buffer.destroy();
        }
    }
}
