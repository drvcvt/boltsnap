//! Input and opaque regions of any window's `wl_surface`, toplevel or layer.
//! Both are double-buffered surface state; each setter commits once without a
//! buffer so the change applies without a new frame.
use super::{Host, state::WindowState, types::WindowId};
use crate::{Error, Result};
use wayland_client::{delegate_noop, protocol::wl_region::WlRegion};

/// Most rectangles accepted per region.
const MAX_RECTS: usize = 64;

/// A rectangle in surface-local logical coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogicalRect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Positive width, at most `i32::MAX`.
    pub width: u32,
    /// Positive height, at most `i32::MAX`.
    pub height: u32,
}

/// Which part of a surface accepts pointer and touch input.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum InputRegion {
    /// The whole surface takes input (protocol default).
    All,
    /// No input at all: pointer and touch pass through (click-through).
    None,
    /// Only the union of these 1..=64 rectangles takes input.
    Rects(Vec<LogicalRect>),
}

impl Host {
    /// Set the input region of any window. A layer commits once without a
    /// buffer, so the region applies at once; a toplevel applies it with its
    /// next presented frame, because a bufferless commit there would also
    /// apply an already acknowledged configure with an old-sized buffer. Empty `Rects` or a zero-sized rectangle return `InvalidInput`, a side
    /// above `i32::MAX` `InvalidDimensions`, more than 64 rectangles
    /// `LimitExceeded`; a retired or foreign window `InvalidInput`. Nothing is
    /// sent on error.
    pub fn set_input_region(&mut self, window: WindowId, region: InputRegion) -> Result<()> {
        self.coordinator.mutate(|state| {
            let rects = match &region {
                InputRegion::All => None,
                InputRegion::None => Some(&[][..]),
                InputRegion::Rects(rects) if rects.is_empty() => {
                    return Err(Error::InvalidInput(
                        "empty input rectangles; use InputRegion::None",
                    ));
                }
                InputRegion::Rects(rects) => Some(rects.as_slice()),
            };
            state.set_region(window, Kind::Input, rects)
        })
    }

    /// Hint which parts of a window are fully opaque, applied like
    /// [`Host::set_input_region`]. An empty slice clears the hint (protocol
    /// default). Bounds and errors as for [`Host::set_input_region`].
    pub fn set_opaque_region(&mut self, window: WindowId, rects: &[LogicalRect]) -> Result<()> {
        self.coordinator.mutate(|state| {
            let rects = (!rects.is_empty()).then_some(rects);
            state.set_region(window, Kind::Opaque, rects)
        })
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Input,
    Opaque,
}

fn validate(rects: &[LogicalRect]) -> Result<()> {
    if rects.len() > MAX_RECTS {
        return Err(Error::LimitExceeded);
    }
    for rect in rects {
        if rect.width == 0 || rect.height == 0 {
            return Err(Error::InvalidInput("empty region rectangle"));
        }
        if rect.width > i32::MAX as u32 || rect.height > i32::MAX as u32 {
            return Err(Error::InvalidDimensions);
        }
    }
    Ok(())
}

impl WindowState {
    /// `None` sends a null region: infinite for input, nothing for opaque.
    /// `Some` creates a region from the rectangles (possibly none), sets it and
    /// destroys it at once; the surface keeps its own copy.
    fn set_region(
        &mut self,
        window: WindowId,
        kind: Kind,
        rects: Option<&[LogicalRect]>,
    ) -> Result<()> {
        self.ensure_active()?;
        if let Some(rects) = rects {
            validate(rects)?;
        }
        let record = self.record(window)?;
        let surface = record.native.surface();
        let layer = record.layer.is_some();
        let compositor = &self.globals.as_ref().ok_or(Error::NotReady)?.compositor;
        let region = rects.map(|rects| {
            let region = compositor.create_region(&self.qh, ());
            for rect in rects {
                // validate bounds every side to i32::MAX.
                region.add(rect.x, rect.y, rect.width as i32, rect.height as i32);
            }
            region
        });
        match kind {
            Kind::Input => surface.set_input_region(region.as_ref()),
            Kind::Opaque => surface.set_opaque_region(region.as_ref()),
        }
        if let Some(region) = region {
            region.destroy();
        }
        if layer {
            surface.commit();
        }
        self.requested()
    }
}

delegate_noop!(WindowState: ignore WlRegion);
