//! Maximized, fullscreen, activated and tiled states from xdg_toplevel configure.
use super::*;

#[test]
fn toplevel_states_are_parsed_and_each_configure_replaces_them() {
    let mut fixture = Fixture::new();
    let id = fixture.create(640, 480);
    let first = configure(
        &mut fixture,
        id,
        0,
        &[MAXIMIZED, ACTIVATED, TILED_LEFT, TILED_TOP],
    );
    assert!(first.maximized && first.activated);
    assert!(!first.fullscreen && !first.suspended);
    assert_eq!(
        first.tiled,
        Tiled {
            left: true,
            top: true,
            ..Tiled::default()
        }
    );
    // Unknown and unused values (resizing, 77) are ignored.
    let second = configure(
        &mut fixture,
        id,
        0,
        &[
            FULLSCREEN,
            TILED_RIGHT,
            TILED_BOTTOM,
            SUSPENDED,
            RESIZING,
            77,
        ],
    );
    assert!(second.fullscreen && second.suspended);
    assert!(!second.maximized && !second.activated);
    assert_eq!(
        second.tiled,
        Tiled {
            right: true,
            bottom: true,
            ..Tiled::default()
        }
    );
    let third = configure(&mut fixture, id, 0, &[]);
    assert!(!third.maximized && !third.fullscreen && !third.activated && !third.suspended);
    assert_eq!(third.tiled, Tiled::default());
    assert_eq!(third.decorations, Decorations::Client);
}

#[test]
fn states_and_decorations_carry_through_rescale() {
    let mut fixture = Fixture::with_options(PeerOptions {
        fractional: true,
        viewport: true,
        decoration: true,
        ..Default::default()
    });
    let id = fixture.create(100, 100);
    fixture.peer.emit(PeerEvent::Decoration(0, 2));
    let first = configure(
        &mut fixture,
        id,
        0,
        &[MAXIMIZED, FULLSCREEN, ACTIVATED, TILED_BOTTOM],
    );
    assert_eq!(first.scale_120, 120);
    fixture.peer.emit(PeerEvent::Fractional(0, 180));
    fixture.sync();
    let events = fixture.host.events();
    let [WindowEvent::Configured { configuration, .. }] = events.as_slice() else {
        panic!("expected one rescale: {events:?}");
    };
    assert_eq!(configuration.scale_120, 180);
    assert!(configuration.revision > first.revision);
    assert_eq!(
        Configuration {
            scale_120: first.scale_120,
            pixel_width: first.pixel_width,
            pixel_height: first.pixel_height,
            revision: first.revision,
            ..*configuration
        },
        first
    );
    assert_eq!(configuration.decorations, Decorations::Server);
    assert!(configuration.maximized && configuration.fullscreen && configuration.activated);
    assert!(configuration.tiled.bottom && !configuration.tiled.top);
}
