//! Live toplevel request check with one owned window only. Run inside a Wayland
//! session; the compositor should offer `xdg_activation_v1` and
//! `zwp_idle_inhibit_manager_v1`.
//!
//! 1. Opens a 480x300 window titled "libway attention" with size limits of
//!    320x200 to 640x400, set before the first frame. Try to resize it below
//!    or above those limits; every size change is printed as `configured WxH`.
//! 2. Once the first frame is shown it enables idle inhibit (`idle inhibit on`)
//!    and disables it again after 5 seconds (`idle inhibit off`).
//! 3. Give the window keyboard focus, then click another window. On that focus
//!    loss it requests attention once (`attention requested`); the compositor
//!    may mark it urgent (Hyprland: border or bar). 10 seconds later it closes.
//!
//! Closing the window exits early, and it gives up after 90 seconds. Static
//! frame only, no animation or flashing.
use libway::window::{
    Configuration, Host, InputEvent, InputObservation, KeyboardObservation, PresentOutcome,
    SizeLimits, WindowEvent, WindowId, WindowOptions,
};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(90);
const INHIBIT_TIME: Duration = Duration::from_secs(5);
const ATTENTION_TIME: Duration = Duration::from_secs(10);
// Premultiplied RGBA: a calm opaque slate and a light border.
const FILL: [u8; 4] = [26, 29, 35, 255];
const BORDER: [u8; 4] = [150, 160, 175, 255];
const LIMITS: SizeLimits = SizeLimits {
    min_width: 320,
    min_height: 200,
    max_width: 640,
    max_height: 400,
};

fn main() -> libway::Result<()> {
    let mut host = Host::connect()?;
    let deadline = Instant::now() + DEADLINE;
    let mut window: Option<WindowId> = None;
    let mut configuration: Option<Configuration> = None;
    let mut size: Option<(u32, u32)> = None;
    let mut dirty = false;
    let mut shown = false;
    let mut inhibit_since: Option<Instant> = None;
    let mut focused_once = false;
    let mut attention_at: Option<Instant> = None;
    while Instant::now() < deadline {
        host.pump(Some(Instant::now() + Duration::from_millis(50)))
            .map_err(|failure| failure.error)?;
        for event in host.events() {
            match event {
                WindowEvent::Ready => {
                    let id = host.create_window(WindowOptions {
                        preferred_width: 480,
                        preferred_height: 300,
                        title: "libway attention".into(),
                        app_id: "libway.attention".into(),
                    })?;
                    host.set_size_limits(id, LIMITS)?;
                    host.enable_cpu(id)?;
                    window = Some(id);
                }
                WindowEvent::Configured {
                    window: target,
                    configuration: next,
                } if Some(target) == window => {
                    let next_size = (next.width, next.height);
                    if size != Some(next_size) {
                        println!("configured {}x{}", next.width, next.height);
                        size = Some(next_size);
                    }
                    configuration = Some(next);
                    dirty = true;
                }
                WindowEvent::Input(InputEvent {
                    observation: InputObservation::Keyboard(keyboard),
                    ..
                }) => match keyboard {
                    KeyboardObservation::Enter { window: target, .. } if Some(target) == window => {
                        focused_once = true;
                    }
                    KeyboardObservation::Leave { window: target, .. }
                        if Some(target) == window && focused_once && attention_at.is_none() =>
                    {
                        match host.request_attention(target) {
                            Ok(()) => println!("attention requested"),
                            Err(error) => println!("attention not requested: {error}"),
                        }
                        attention_at = Some(Instant::now());
                    }
                    _ => {}
                },
                WindowEvent::CloseRequested { window: target } if Some(target) == window => {
                    println!("closed by the user");
                    host.close(target)?;
                    host.shutdown();
                    return Ok(());
                }
                _ => {}
            }
        }
        let Some(id) = window else {
            continue;
        };
        if let (true, Some(config)) = (dirty, configuration) {
            let pixels = frame(config.pixel_width, config.pixel_height);
            dirty = host.present_cpu(id, config.revision, &pixels)? != PresentOutcome::Presented;
            if !dirty && !shown {
                shown = true;
                println!("shown");
                match host.set_idle_inhibit(id, true) {
                    Ok(()) => {
                        println!("idle inhibit on");
                        inhibit_since = Some(Instant::now());
                    }
                    Err(error) => println!("idle inhibit not enabled: {error}"),
                }
            }
        }
        if inhibit_since.is_some_and(|since| since.elapsed() >= INHIBIT_TIME) {
            host.set_idle_inhibit(id, false)?;
            println!("idle inhibit off");
            inhibit_since = None;
        }
        if attention_at.is_some_and(|at| at.elapsed() >= ATTENTION_TIME) {
            println!("done");
            host.close(id)?;
            host.shutdown();
            return Ok(());
        }
    }
    println!("deadline reached");
    if let Some(id) = window {
        host.close(id)?;
    }
    host.shutdown();
    Ok(())
}

/// A flat fill with a 2 pixel border.
fn frame(width: u32, height: u32) -> Vec<u8> {
    let (width, height) = (width as usize, height as usize);
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let edge = x < 2 || y < 2 || x + 2 >= width || y + 2 >= height;
            pixels.extend_from_slice(if edge { &BORDER } else { &FILL });
        }
    }
    pixels
}
