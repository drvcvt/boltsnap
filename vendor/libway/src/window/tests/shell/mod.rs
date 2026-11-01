//! Decoration negotiation, toplevel states, serial-gated window management and
//! cursor shapes, each against literal wire traces from the fake compositor.
mod cursor;
mod decoration;
mod layer;
mod layer_runtime;
mod manage;
mod region;
mod states;
mod toplevel;
use super::*;

const MAXIMIZED: u32 = 1;
const FULLSCREEN: u32 = 2;
const RESIZING: u32 = 3;
const ACTIVATED: u32 = 4;
const TILED_LEFT: u32 = 5;
const TILED_RIGHT: u32 = 6;
const TILED_TOP: u32 = 7;
const TILED_BOTTOM: u32 = 8;
const SUSPENDED: u32 = 9;

/// Native-endian xdg_toplevel state array.
fn state_array(values: &[u32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// Deliver one complete configure sequence and return its single configuration.
fn configure(fixture: &mut Fixture, id: WindowId, native: usize, values: &[u32]) -> Configuration {
    fixture
        .peer
        .configure(native, 200, 100, 73, state_array(values));
    fixture.sync();
    let events = fixture.host.events();
    let [
        WindowEvent::Configured {
            window,
            configuration,
        },
    ] = events.as_slice()
    else {
        panic!("expected exactly one configure: {events:?}");
    };
    assert_eq!(*window, id);
    assert_eq!(
        fixture.host.configuration(id).unwrap(),
        Some(*configuration)
    );
    *configuration
}
