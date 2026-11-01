//! The smallest owned window: one CPU-presented window with a solid fill,
//! redrawn when configured or asked to, closed when the compositor asks.
//! Run inside a Wayland session:
//! `cargo run --no-default-features --features window --example window`.
use libway::window::{Configuration, Host, PresentOutcome, WindowEvent, WindowOptions};

const FILL: [u8; 4] = [0x30, 0x30, 0x38, 0xff];

fn main() -> libway::Result<()> {
    let mut host = Host::connect()?;
    let mut window = None;
    let mut configuration: Option<Configuration> = None;
    let mut dirty = false;
    let mut shown = false;
    loop {
        host.pump(None).map_err(|failure| failure.error)?;
        for event in host.events() {
            match event {
                WindowEvent::Ready => {
                    let id = host.create_window(WindowOptions {
                        preferred_width: 480,
                        preferred_height: 270,
                        title: "libway window".into(),
                        app_id: "libway.window".into(),
                    })?;
                    host.enable_cpu(id)?;
                    window = Some(id);
                }
                WindowEvent::Configured {
                    configuration: next,
                    ..
                } => {
                    println!("configured {}x{}", next.width, next.height);
                    (configuration, dirty) = (Some(next), true);
                }
                WindowEvent::RedrawRequested { .. } => dirty = true,
                WindowEvent::CloseRequested { .. } => {
                    if let Some(id) = window {
                        host.close(id)?;
                    }
                    host.shutdown();
                    println!("closed");
                    return Ok(());
                }
                _ => {}
            }
        }
        if let (Some(id), Some(config), true) = (window, configuration, dirty) {
            let pixels = FILL.repeat(config.pixel_width as usize * config.pixel_height as usize);
            // NeedsFrame and NeedsRelease wait for RedrawRequested; Suspended
            // waits for the next configuration.
            let outcome = host.present_cpu(id, config.revision, &pixels)?;
            dirty = false;
            if outcome == PresentOutcome::Presented && !shown {
                shown = true;
                println!("shown");
            }
        }
    }
}
