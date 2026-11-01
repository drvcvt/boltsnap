//! What the test server "renders": a per-output pattern written into the
//! client's buffer before `ready`.
use super::*;

pub(super) fn buffer_dimensions(buffer: &wl_buffer::WlBuffer) -> (u32, u32) {
    if let Some(b) = buffer.data::<Buffer>() {
        return (b.width, b.height);
    }
    #[cfg(feature = "gpu")]
    if let Some(b) = buffer.data::<dma::GpuBufferData>() {
        return (b.width, b.height);
    }
    panic!("unknown test buffer");
}

/// The XRGB pixel the server paints at `(x, y)` of output `index`.
pub fn pixel(index: usize, x: u32, y: u32) -> u32 {
    if index.is_multiple_of(2) {
        0x00102030_u32 + (x << 16) + (y << 8)
    } else {
        0x00a0b0c0
    }
}

pub(super) fn paint(buffer: &wl_buffer::WlBuffer, index: usize) {
    #[cfg(feature = "gpu")]
    if let Some(b) = buffer.data::<dma::GpuBufferData>() {
        // Only LINEAR buffers are painted, by the CPU; other dma-buf tests
        // verify fd transport and lifecycle and do not claim GPU rendering.
        dma::paint(b, index);
        return;
    }
    let Some(b) = buffer.data::<Buffer>() else {
        return;
    };
    // Test server owns its FD and serializes access before sending ready.
    let mut map = unsafe { MmapOptions::new().map_mut(&*b.file).unwrap() };
    for y in 0..b.height {
        for x in 0..b.width {
            let at = b.offset + (y * b.stride + x * 4) as usize;
            map[at..at + 4].copy_from_slice(&pixel(index, x, y).to_le_bytes());
        }
    }
}
