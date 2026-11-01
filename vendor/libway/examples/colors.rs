//! Live channel-order check for CPU presentation, with one owned window.
//!
//! Shows four static quadrants for 8 seconds, then closes: red (top left),
//! green (top right), blue (bottom left) and half-transparent white (bottom
//! right, premultiplied). No animation, no flashing. Run with
//! `WAYLAND_DEBUG=1` to see which `wl_shm` format the buffers use.
use libway::window::{Host, PresentOutcome, WindowEvent, WindowId, WindowOptions};
use std::time::{Duration, Instant};

const SHOWN: Duration = Duration::from_secs(8);
// Premultiplied RGBA.
const QUADRANTS: [[u8; 4]; 4] = [
    [255, 0, 0, 255],
    [0, 255, 0, 255],
    [0, 0, 255, 255],
    [128, 128, 128, 128],
];

fn main() -> libway::Result<()> {
    let mut host = Host::connect()?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut window: Option<WindowId> = None;
    let mut shown: Option<Instant> = None;
    let mut configuration = None;
    let mut dirty = false;
    while Instant::now() < deadline {
        host.pump(Some(Instant::now() + Duration::from_millis(50)))
            .map_err(|failure| failure.error)?;
        for event in host.events() {
            match event {
                WindowEvent::Ready => {
                    let id = host.create_window(WindowOptions {
                        preferred_width: 400,
                        preferred_height: 300,
                        title: "libway colors".into(),
                        app_id: "libway.colors".into(),
                    })?;
                    host.enable_cpu(id)?;
                    window = Some(id);
                }
                WindowEvent::Configured {
                    window: id,
                    configuration: next,
                } if Some(id) == window => {
                    configuration = Some(next);
                    dirty = true;
                }
                WindowEvent::CloseRequested { window: id } if Some(id) == window => {
                    host.close(id)?;
                    host.shutdown();
                    return Ok(());
                }
                _ => {}
            }
        }
        if let (Some(id), Some(config), true) = (window, configuration, dirty) {
            let pixels = frame(config.pixel_width, config.pixel_height);
            dirty = host.present_cpu(id, config.revision, &pixels)? != PresentOutcome::Presented;
            if !dirty && shown.is_none() {
                println!("shown {}x{}", config.pixel_width, config.pixel_height);
                shown = Some(Instant::now());
            }
        }
        if let (Some(id), Some(at)) = (window, shown)
            && at.elapsed() >= SHOWN
        {
            host.close(id)?;
            break;
        }
    }
    host.shutdown();
    Ok(())
}

fn frame(width: u32, height: u32) -> Vec<u8> {
    let (width, height) = (width as usize, height as usize);
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let quadrant = usize::from(x >= width / 2) + 2 * usize::from(y >= height / 2);
            pixels.extend_from_slice(&QUADRANTS[quadrant]);
        }
    }
    pixels
}
