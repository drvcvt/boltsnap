//! Live layer-shell check with owned surfaces only. Run inside a Wayland session
//! whose compositor offers `zwlr_layer_shell_v1`.
//!
//! 1. HUD: a 240x120 overlay in the top right corner of the compositor's chosen
//!    output, 24 px from both edges, click-through, no keyboard. Shown for 12
//!    seconds; move the pointer across it. It counts pointer events it receives,
//!    which must stay 0.
//! 2. Prompt: a 360x160 overlay in the middle with on-demand keyboard: it gets
//!    focus like a normal window and gives it up when another window is
//!    clicked. Escape closes it, losing focus closes it, and after 15 seconds
//!    it closes on its own.
//!
//! Static frames only, no animation or flashing.
use libway::window::{
    Anchor, ExclusiveZone, Host, InputEvent, InputObservation, InputRegion, KeyboardInteractivity,
    KeyboardObservation, Layer, LayerOptions, LayerState, PresentOutcome, WindowEvent, WindowId,
};
use std::time::{Duration, Instant};

const HUD_TIME: Duration = Duration::from_secs(12);
const PROMPT_TIME: Duration = Duration::from_secs(15);
// Premultiplied RGBA: a calm translucent slate and an opaque light border.
const HUD_FILL: [u8; 4] = [26, 29, 35, 217];
const PROMPT_FILL: [u8; 4] = [24, 26, 31, 245];
const BORDER: [u8; 4] = [150, 160, 175, 255];
// Linux evdev code.
const ESCAPE: u32 = 1;

#[derive(PartialEq)]
enum Phase {
    Hud,
    Prompt,
}

struct Surface {
    id: WindowId,
    name: &'static str,
    fill: [u8; 4],
    configured: Option<libway::window::Configuration>,
    dirty: bool,
    shown: Option<Instant>,
}

fn main() -> libway::Result<()> {
    let mut host = Host::connect()?;
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut phase = Phase::Hud;
    let mut surface: Option<Surface> = None;
    let mut pointer_events = 0_u32;
    while Instant::now() < deadline {
        host.pump(Some(Instant::now() + Duration::from_millis(50)))
            .map_err(|failure| failure.error)?;
        for event in host.events() {
            match event {
                WindowEvent::Ready => surface = Some(hud(&mut host)?),
                WindowEvent::Configured {
                    window,
                    configuration,
                } => {
                    if let Some(current) = surface.as_mut().filter(|s| s.id == window) {
                        if current.configured.is_none() {
                            println!(
                                "{} configured {}x{}",
                                current.name, configuration.width, configuration.height
                            );
                        }
                        current.configured = Some(configuration);
                        current.dirty = true;
                    }
                }
                WindowEvent::Input(InputEvent { observation, .. }) => {
                    let Some(current) = surface.as_ref() else {
                        continue;
                    };
                    match observation {
                        // The HUD is this client's only surface in its phase,
                        // so every pointer frame would be one for the HUD.
                        InputObservation::PointerFrame(_) if phase == Phase::Hud => {
                            pointer_events += 1;
                        }
                        InputObservation::Keyboard(KeyboardObservation::Enter {
                            window, ..
                        }) if window == current.id && phase == Phase::Prompt => {
                            println!("prompt keyboard enter");
                        }
                        InputObservation::Keyboard(KeyboardObservation::Key {
                            window,
                            physical: ESCAPE,
                            down: true,
                            ..
                        }) if window == current.id && phase == Phase::Prompt => {
                            println!("prompt closed by Escape");
                            host.close(window)?;
                            host.shutdown();
                            return Ok(());
                        }
                        InputObservation::Keyboard(KeyboardObservation::Leave {
                            window, ..
                        }) if window == current.id && phase == Phase::Prompt => {
                            println!("prompt closed by focus loss");
                            host.close(window)?;
                            host.shutdown();
                            return Ok(());
                        }
                        _ => {}
                    }
                }
                WindowEvent::Closed { window }
                    if surface.as_ref().is_some_and(|s| s.id == window) =>
                {
                    println!("closed by the compositor");
                    host.shutdown();
                    return Ok(());
                }
                _ => {}
            }
        }
        let Some(current) = surface.as_mut() else {
            continue;
        };
        if let (true, Some(config)) = (current.dirty, current.configured) {
            let pixels = frame(config.pixel_width, config.pixel_height, current.fill);
            current.dirty = host.present_cpu(current.id, config.revision, &pixels)?
                != PresentOutcome::Presented;
            if !current.dirty && current.shown.is_none() {
                current.shown = Some(Instant::now());
                println!("{} shown", current.name);
            }
        }
        match (&phase, current.shown) {
            (Phase::Hud, Some(shown)) if shown.elapsed() >= HUD_TIME => {
                println!("hud pointer events: {pointer_events}");
                host.close(current.id)?;
                surface = Some(prompt(&mut host)?);
                phase = Phase::Prompt;
            }
            (Phase::Prompt, Some(shown)) if shown.elapsed() >= PROMPT_TIME => {
                println!("prompt timeout");
                host.close(current.id)?;
                break;
            }
            _ => {}
        }
    }
    host.shutdown();
    Ok(())
}

fn hud(host: &mut Host) -> libway::Result<Surface> {
    let id = host.create_layer(LayerOptions {
        namespace: "libway.hud".into(),
        output: None,
        state: LayerState {
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::RIGHT,
            width: 240,
            height: 120,
            margin: [24, 24, 0, 0],
            exclusive_zone: ExclusiveZone::Respect,
            exclusive_edge: None,
            keyboard: KeyboardInteractivity::None,
        },
    })?;
    host.set_input_region(id, InputRegion::None)?;
    host.enable_cpu(id)?;
    Ok(surface(id, "hud", HUD_FILL))
}

fn prompt(host: &mut Host) -> libway::Result<Surface> {
    let id = host.create_layer(LayerOptions {
        namespace: "libway.prompt".into(),
        output: None,
        state: LayerState {
            layer: Layer::Overlay,
            anchor: Anchor::NONE,
            width: 360,
            height: 160,
            margin: [0; 4],
            exclusive_zone: ExclusiveZone::Respect,
            exclusive_edge: None,
            keyboard: KeyboardInteractivity::OnDemand,
        },
    })?;
    host.enable_cpu(id)?;
    Ok(surface(id, "prompt", PROMPT_FILL))
}

fn surface(id: WindowId, name: &'static str, fill: [u8; 4]) -> Surface {
    Surface {
        id,
        name,
        fill,
        configured: None,
        dirty: false,
        shown: None,
    }
}

/// A flat fill with a 2 pixel border.
fn frame(width: u32, height: u32, fill: [u8; 4]) -> Vec<u8> {
    let (width, height) = (width as usize, height as usize);
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let edge = x < 2 || y < 2 || x + 2 >= width || y + 2 >= height;
            pixels.extend_from_slice(if edge { &BORDER } else { &fill });
        }
    }
    pixels
}
