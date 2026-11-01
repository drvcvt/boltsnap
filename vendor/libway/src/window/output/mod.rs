//! Host-owned CPU presentation, independent buffer release and frame pacing.
mod damage;
mod layout;
mod presentation;
mod protocol;
mod slots;

use super::{
    lease::PresenterLease,
    state::WindowState,
    types::{Configuration, ConfigurationRevision, WindowEvent, WindowId},
};
use crate::{Error, Result};
use damage::{History, HistoryKey};
use layout::Layout;
use slots::{Budget, BufferToken, Slot};
use std::collections::{BTreeMap, VecDeque};
use wayland_client::{Proxy, QueueHandle, protocol::wl_shm::WlShm};

pub use damage::PixelRect;
pub(super) use presentation::Presentation;
pub use presentation::{PresentationFlags, PresentationOutcome, PresentationTiming};

const MAX_WINDOW_BUFFERS: usize = 3;

/// Result of one nonblocking CPU presentation attempt. An unpresented attempt
/// retains no caller pixels; keep dirty output until the relevant native event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PresentOutcome {
    /// The matching buffer, damage and frame callback were committed once.
    Presented,
    /// Buffer or byte credits require an actual compositor buffer release.
    NeedsRelease,
    /// The preceding surface frame callback has not arrived.
    NeedsFrame,
    /// The current configuration suspends presentation until a native change.
    Suspended,
}

pub(crate) struct CpuFrame<'a> {
    pub(crate) configuration: Configuration,
    pub(crate) revision: ConfigurationRevision,
    pub(crate) rgba: &'a [u8],
    /// Changed regions since the previous presented frame; `None` is everything.
    pub(crate) damage: Option<&'a [PixelRect]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FrameToken {
    window: WindowId,
    sequence: u64,
}

struct CpuOutput {
    // Slots precede the lease so local buffer destruction precedes role teardown.
    slots: Vec<Slot>,
    lease: PresenterLease,
    pending_frame: Option<FrameToken>,
    next_buffer: u64,
    next_frame: u64,
    history: History,
    retired: bool,
    waiting_release: bool,
}

#[derive(Default)]
pub(crate) struct CpuOutputs {
    entries: BTreeMap<WindowId, CpuOutput>,
    budget: Budget,
}

impl CpuOutputs {
    pub(crate) fn contains(&self, window: WindowId) -> bool {
        self.entries.contains_key(&window)
    }

    pub(crate) fn enable(&mut self, window: WindowId, lease: PresenterLease) -> Result<()> {
        if lease.id() != window {
            return Err(Error::InvalidInput("CPU lease belongs to another window"));
        }
        if self.contains(window) {
            return Err(Error::InvalidInput("CPU presenter is already enabled"));
        }
        let mut slots = Vec::new();
        slots
            .try_reserve_exact(MAX_WINDOW_BUFFERS)
            .map_err(|_| Error::LimitExceeded)?;
        self.entries.insert(
            window,
            CpuOutput {
                slots,
                lease,
                pending_frame: None,
                next_buffer: 1,
                next_frame: 1,
                history: History::default(),
                retired: false,
                waiting_release: false,
            },
        );
        Ok(())
    }

    pub(crate) fn present(
        &mut self,
        window: WindowId,
        frame: CpuFrame<'_>,
        shm: &WlShm,
        qh: &QueueHandle<WindowState>,
        feedback: (&mut Presentation, &mut VecDeque<WindowEvent>),
    ) -> Result<PresentOutcome> {
        let output = self.active(window)?;
        if frame.damage.is_some_and(<[PixelRect]>::is_empty) {
            return Err(Error::InvalidInput("empty CPU damage"));
        }
        if frame.revision != frame.configuration.revision {
            return Err(Error::InvalidInput("CPU configuration revision is stale"));
        }
        if frame.configuration.suspended {
            return Ok(PresentOutcome::Suspended);
        }
        let layout = Layout::new(
            frame.configuration.pixel_width,
            frame.configuration.pixel_height,
        )?;
        layout.validate(frame.rgba)?;
        let damage = damage::normalize(frame.damage, layout)?;
        if output.pending_frame.is_some() {
            return Ok(PresentOutcome::NeedsFrame);
        }
        // Exhaustion is checked before allocating resources or writing any pixels.
        output
            .next_frame
            .checked_add(1)
            .ok_or(Error::LimitExceeded)?;
        self.entries
            .get_mut(&window)
            .expect("invariant: active output exists")
            .slots
            .retain(|slot| slot.is_busy() || slot.layout() == layout);
        let index = self.find_slot(window, layout, shm, qh)?;
        let Some(index) = index else {
            return Ok(PresentOutcome::NeedsRelease);
        };
        let output = self
            .entries
            .get_mut(&window)
            .expect("invariant: live output");
        output.commit(index, frame, &damage, qh, feedback)?;
        Ok(PresentOutcome::Presented)
    }

    fn active(&self, window: WindowId) -> Result<&CpuOutput> {
        let output = self
            .entries
            .get(&window)
            .ok_or(Error::InvalidInput("CPU presenter is not enabled"))?;
        if output.retired {
            return Err(Error::InvalidInput("CPU presenter is retired"));
        }
        Ok(output)
    }

    fn find_slot(
        &mut self,
        window: WindowId,
        layout: Layout,
        shm: &WlShm,
        qh: &QueueHandle<WindowState>,
    ) -> Result<Option<usize>> {
        let output = self.active(window)?;
        if let Some(index) = output.slots.iter().position(|slot| !slot.is_busy()) {
            return Ok(Some(index));
        }
        if output.slots.len() == MAX_WINDOW_BUFFERS {
            return Ok(self.wait_release(window));
        }
        if self.budget.remaining() < layout.bytes {
            // Released storage is disposable even if it was the last displayed
            // content: wl_buffer.destroy after release does not alter that content.
            for output in self.entries.values_mut() {
                output.slots.retain(Slot::is_busy);
            }
        }
        if self.budget.remaining() < layout.bytes {
            if self
                .entries
                .values()
                .any(|output| output.slots.iter().any(Slot::is_busy))
            {
                return Ok(self.wait_release(window));
            }
            return Err(Error::LimitExceeded);
        }
        let output = self
            .entries
            .get_mut(&window)
            .expect("invariant: active output");
        let sequence = issue(&mut output.next_buffer)?;
        let slot = Slot::new(
            shm,
            qh,
            BufferToken { window, sequence },
            layout,
            &self.budget,
        )?;
        let index = output.slots.len();
        output.slots.push(slot);
        Ok(Some(index))
    }

    fn wait_release(&mut self, window: WindowId) -> Option<usize> {
        self.entries
            .get_mut(&window)
            .expect("invariant: waiting output exists")
            .waiting_release = true;
        None
    }

    pub(crate) fn release(&mut self, token: BufferToken) -> Vec<WindowId> {
        let Some(output) = self.entries.get_mut(&token.window) else {
            return Vec::new();
        };
        let Some(slot) = output.slots.iter_mut().find(|slot| slot.token() == token) else {
            return Vec::new();
        };
        if !slot.is_busy() {
            return Vec::new();
        }
        slot.release();
        if output.retired {
            output.slots.retain(Slot::is_busy);
            if output.slots.is_empty() {
                self.entries.remove(&token.window);
            }
        }
        // Other windows can be waiting on this buffer's shared byte credits.
        self.entries
            .iter_mut()
            .filter_map(|(window, output)| {
                if output.retired || (*window != token.window && !output.waiting_release) {
                    return None;
                }
                output.waiting_release = false;
                Some(*window)
            })
            .collect()
    }

    pub(crate) fn frame(&mut self, token: FrameToken) -> Option<WindowId> {
        let output = self.entries.get_mut(&token.window)?;
        if output.retired || output.pending_frame != Some(token) {
            return None;
        }
        output.pending_frame = None;
        Some(token.window)
    }

    pub(crate) fn retire(&mut self, window: WindowId) {
        let Some(output) = self.entries.get_mut(&window) else {
            return;
        };
        output.retire();
        if output.slots.is_empty() {
            self.entries.remove(&window);
        }
    }

    pub(crate) fn clear(&mut self) {
        // Teardown destroys ownership without waiting or pretending release fired.
        // Surviving external window leases retain their native dependency graph.
        for output in self.entries.values_mut() {
            output.retire();
        }
        self.entries.clear();
    }
}

impl CpuOutput {
    fn retire(&mut self) {
        if self.retired {
            return;
        }
        if self.slots.iter().any(Slot::is_busy) {
            // The lease keeps the surface alive. Removing its last content avoids
            // retaining that content forever while waiting for buffer releases.
            // Busy slots prove that the first configure was already acknowledged.
            self.lease.surface().attach(None, 0, 0);
            self.lease.surface().commit();
        }
        self.retired = true;
        self.pending_frame = None;
        self.waiting_release = false;
        self.slots.retain(Slot::is_busy);
    }

    fn commit(
        &mut self,
        index: usize,
        frame: CpuFrame<'_>,
        damage: &[PixelRect],
        qh: &QueueHandle<WindowState>,
        (presentation, events): (&mut Presentation, &mut VecDeque<WindowEvent>),
    ) -> Result<()> {
        let width =
            i32::try_from(frame.configuration.width).map_err(|_| Error::InvalidDimensions)?;
        let height =
            i32::try_from(frame.configuration.height).map_err(|_| Error::InvalidDimensions)?;
        let token = FrameToken {
            window: self.lease.id(),
            sequence: issue(&mut self.next_frame)?,
        };
        let slot = self
            .slots
            .get_mut(index)
            .ok_or(Error::InvalidInput("unknown CPU slot"))?;
        let config = frame.configuration;
        let key: HistoryKey = (
            config.width,
            config.height,
            config.pixel_width,
            config.pixel_height,
            config.scale_120,
        );
        // Only changes since the slot's own frame are copied; correctness never
        // depends on its old content when that frame is not fully accounted for.
        let stale = self
            .history
            .stale(key, slot.valid(), token.sequence, damage);
        slot.write(frame.rgba, stale.as_deref())?;
        self.lease.configure_surface(config)?;
        slot.mark_busy(token.sequence)?;
        // The compositor's previous content is the preceding presented frame only
        // under unchanged surface state; otherwise the whole buffer is new.
        let partial = self.history.follows(key, token.sequence);
        self.history.record(key, token.sequence, damage);
        self.pending_frame = Some(token);
        self.waiting_release = false;
        let surface = self.lease.surface();
        // From staging through commit, no application code or dispatch may run.
        surface.frame(qh, token);
        surface.attach(Some(slot.buffer()), 0, 0);
        if surface.version() >= 4 {
            let layout = slot.layout();
            if partial {
                for rect in damage {
                    // Clamped to a positive i32 layout, so these casts are lossless.
                    surface.damage_buffer(
                        rect.x as i32,
                        rect.y as i32,
                        rect.width as i32,
                        rect.height as i32,
                    );
                }
            } else {
                surface.damage_buffer(0, 0, layout.width, layout.height);
            }
        } else {
            surface.damage(0, 0, width, height);
        }
        // Feedback attaches to the commit that follows the request.
        presentation.consume(token.window, surface, qh, events);
        surface.commit();
        Ok(())
    }
}

fn issue(next: &mut u64) -> Result<u64> {
    let following = next.checked_add(1).ok_or(Error::LimitExceeded)?;
    let issued = *next;
    *next = following;
    Ok(issued)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issued_tokens_never_wrap_or_reuse_failed_exhaustion() {
        let mut next = u64::MAX - 1;
        assert_eq!(issue(&mut next).unwrap(), u64::MAX - 1);
        assert!(matches!(issue(&mut next), Err(Error::LimitExceeded)));
        assert!(matches!(issue(&mut next), Err(Error::LimitExceeded)));
        assert_eq!(next, u64::MAX);
    }
}
