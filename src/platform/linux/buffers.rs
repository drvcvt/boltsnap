//! `wl_buffer`s kept across frames. A new buffer every frame is a new texture
//! and a full upload in the compositor whatever the damage says, and at the
//! display's rate that slows every screen (settings plan, 4f and 5c S1).

use smithay_client_toolkit::shm::slot::{Buffer, SlotPool};
use wayland_client::protocol::wl_shm;

/// Enough for the compositor to hold one on screen and one queued while the
/// next is drawn.
const MAX_BUFFERS: usize = 3;

#[derive(Debug, PartialEq, Eq)]
enum Choice {
    Reuse(usize),
    /// A new one; `drop_oldest` keeps the count at `MAX_BUFFERS`.
    Create {
        drop_oldest: bool,
    },
}

/// `free[i]`: the compositor has released kept buffer `i`.
fn choose(free: &[bool]) -> Choice {
    match free.iter().position(|free| *free) {
        Some(index) => Choice::Reuse(index),
        None => Choice::Create {
            drop_oldest: free.len() >= MAX_BUFFERS,
        },
    }
}

#[derive(Default)]
pub struct Buffers {
    kept: Vec<Buffer>,
}

impl Buffers {
    /// A buffer `w` × `h` the compositor does not hold, and its pixels. Kept
    /// buffers of another size go; a buffer still on screen is destroyed by
    /// sctk once it is released.
    pub fn next<'a>(
        &'a mut self,
        pool: &'a mut SlotPool,
        (w, h): (i32, i32),
        format: wl_shm::Format,
    ) -> Option<(&'a Buffer, &'a mut [u8])> {
        let stride = w * 4;
        self.kept
            .retain(|buffer| (buffer.height(), buffer.stride()) == (h, stride));
        let free: Vec<bool> = self
            .kept
            .iter()
            .map(|buffer| buffer.canvas(pool).is_some())
            .collect();
        let index = match choose(&free) {
            Choice::Reuse(index) => index,
            Choice::Create { drop_oldest } => {
                let (buffer, _) = pool.create_buffer(w, h, stride, format).ok()?;
                if drop_oldest {
                    self.kept.remove(0);
                }
                self.kept.push(buffer);
                self.kept.len() - 1
            }
        };
        let buffer = &self.kept[index];
        let pixels = buffer.canvas(pool)?;
        Some((buffer, pixels))
    }
}

#[cfg(test)]
mod tests {
    use super::{Choice, choose};

    #[test]
    fn a_released_buffer_is_reused_before_a_new_one_is_made() {
        assert_eq!(choose(&[]), Choice::Create { drop_oldest: false });
        assert_eq!(choose(&[false, true, true]), Choice::Reuse(1));
        assert_eq!(
            choose(&[false, false]),
            Choice::Create { drop_oldest: false }
        );
        assert_eq!(choose(&[false; 3]), Choice::Create { drop_oldest: true });
    }
}
