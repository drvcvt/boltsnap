//! Three bounded captures. With `gpu`, an optional render-node argument selects DMA-BUF.
use libway::{BufferKind, CaptureOptions, Connection};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = CaptureOptions::default();
    let mut connection = Connection::connect(&options)?;
    let output = connection
        .outputs(&options)?
        .into_iter()
        .next()
        .ok_or("no outputs")?;
    #[cfg(feature = "gpu")]
    let kind = match std::env::args_os().nth(1) {
        Some(node) => BufferKind::Gpu(libway::gpu::GpuAllocator::open(node)?),
        None => BufferKind::Cpu,
    };
    #[cfg(not(feature = "gpu"))]
    let kind = BufferKind::Cpu;
    let mut stream = connection.stream(output.id, options, kind)?;
    for _ in 0..3 {
        let frame = stream.next_frame()?;
        println!(
            "{:?}: {}x{}, {:?}, {:?}",
            frame.backend, frame.width, frame.height, frame.format, frame.presentation_time
        );
        // A recorder would submit frame.storage to its encoder and retain the frame
        // until consumption finishes. This example drops it without retaining history.
    }
    Ok(())
}
