//! Surface scale selection and pixel rounding through actual protocol events.
use super::peer::State;
use super::*;
use wayland_protocols::wp::{
    fractional_scale::v1::server::{wp_fractional_scale_manager_v1, wp_fractional_scale_v1},
    viewporter::server::{wp_viewport, wp_viewporter},
};
use wayland_server::protocol::wl_output;
use wayland_server::{Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource};
impl GlobalDispatch<wl_output::WlOutput, usize> for State {
    fn bind(
        state: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<wl_output::WlOutput>,
        index: &usize,
        init: &mut DataInit<'_, Self>,
    ) {
        let output = init.init(resource, *index);
        state
            .outputs
            .resize(state.outputs.len().max(index + 1), None);
        output.scale(1);
        if output.version() >= 4 {
            output.name(format!("OUT-{index}"));
            output.description(format!("Test output {index}"));
        }
        if !state.hold_output_done {
            output.done();
        }
        state.outputs[*index] = Some(output);
        state.ledger.push(Request::BindOutput(*index));
    }
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wl_output::WlOutput,
        request: wl_output::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        assert!(matches!(request, wl_output::Request::Release));
        state.ledger.push(Request::ReleaseOutput(*index));
    }
}

impl Dispatch<wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wp_fractional_scale_manager_v1::WpFractionalScaleManagerV1,
        request: wp_fractional_scale_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_fractional_scale_manager_v1::Request::GetFractionalScale { id, surface } => {
                let index = *surface.data::<usize>().unwrap();
                state.windows[index].fractional = Some(init.init(id, index));
                state.ledger.push(Request::Fractional(index));
            }
            wp_fractional_scale_manager_v1::Request::Destroy => {}
            request => panic!("unexpected fractional manager request: {request:?}"),
        }
    }
}

impl Dispatch<wp_fractional_scale_v1::WpFractionalScaleV1, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wp_fractional_scale_v1::WpFractionalScaleV1,
        request: wp_fractional_scale_v1::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        assert!(matches!(request, wp_fractional_scale_v1::Request::Destroy));
        state.ledger.push(Request::DestroyFractional(*index));
    }
}

impl Dispatch<wp_viewporter::WpViewporter, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wp_viewporter::WpViewporter,
        request: wp_viewporter::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_viewporter::Request::GetViewport { id, surface } => {
                let index = *surface.data::<usize>().unwrap();
                state.windows[index].viewport = Some(init.init(id, index));
                state.ledger.push(Request::Viewport(index));
            }
            wp_viewporter::Request::Destroy => {}
            request => panic!("unexpected viewporter request: {request:?}"),
        }
    }
}

impl Dispatch<wp_viewport::WpViewport, usize> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &wp_viewport::WpViewport,
        request: wp_viewport::Request,
        index: &usize,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        match request {
            wp_viewport::Request::SetDestination { width, height } => state
                .ledger
                .push(Request::ViewportDestination(*index, width, height)),
            wp_viewport::Request::Destroy => state.ledger.push(Request::DestroyViewport(*index)),
            request => panic!("unexpected viewport request: {request:?}"),
        }
    }
}

fn configured(fixture: &mut Fixture, id: WindowId, scale: u32, pixels: u32) {
    let events = fixture.host.events();
    let [
        WindowEvent::Configured {
            window,
            configuration,
        },
    ] = events.as_slice()
    else {
        panic!("expected one scale configuration: {events:?}");
    };
    assert_eq!(*window, id);
    assert_eq!(configuration.scale_120, scale);
    assert_eq!(
        (configuration.pixel_width, configuration.pixel_height),
        (pixels, pixels)
    );
    assert_eq!(
        fixture.host.configuration(id).unwrap(),
        Some(*configuration)
    );
}

#[test]
fn fractional_pair_precedes_initial_commit_and_scales_round_nearest_ties_up() {
    let mut fixture = Fixture::with_options(PeerOptions {
        fractional: true,
        viewport: true,
        ..Default::default()
    });
    let id = fixture.create(101, 101);
    let requests = fixture.peer.take_requests();
    let commit = requests
        .iter()
        .position(|r| *r == Request::Commit(0, false))
        .unwrap();
    for extension in [Request::Fractional(0), Request::Viewport(0)] {
        assert!(requests.iter().position(|r| *r == extension).unwrap() < commit);
    }
    fixture.peer.emit(PeerEvent::Fractional(0, 150));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(fixture.host.configuration(id).unwrap(), None);
    let first = fixture.configure(id, 0, 101, 101);
    assert_eq!(
        (first.scale_120, first.pixel_width, first.pixel_height),
        (150, 126, 126)
    );
    assert_eq!(first.revision.0, 1);
    assert_eq!(fixture.peer.take_requests(), [Request::Ack(0, 73)]);
    fixture.peer.emit(PeerEvent::Fractional(0, 180));
    fixture.sync();
    configured(&mut fixture, id, 180, 152);
    let second = fixture.host.configuration(id).unwrap().unwrap();
    assert_eq!(second.revision.0, 2);
    assert!(fixture.peer.take_requests().is_empty());
    fixture.peer.emit(PeerEvent::Fractional(0, 180));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(fixture.host.configuration(id).unwrap(), Some(second));
}

#[test]
fn incomplete_fractional_pair_creates_neither_surface_extension() {
    for (fractional, viewport) in [(false, false), (true, false), (false, true)] {
        let mut fixture = Fixture::with_options(PeerOptions {
            fractional,
            viewport,
            ..Default::default()
        });
        let id = fixture.create(101, 101);
        let requests = fixture.peer.take_requests();
        assert!(
            !requests
                .iter()
                .any(|r| matches!(r, Request::Fractional(_) | Request::Viewport(_)))
        );
        let config = fixture.configure(id, 0, 101, 101);
        assert_eq!(
            (config.scale_120, config.pixel_width, config.pixel_height),
            (120, 101, 101)
        );
    }
}

#[test]
fn legacy_surface_uses_output_maximum_only_after_done_and_tracks_leave() {
    let mut fixture = Fixture::with_options(PeerOptions {
        outputs: true,
        surface_version: 5,
        ..Default::default()
    });
    let id = fixture.create(101, 101);
    let initial = fixture.configure(id, 0, 101, 101);
    assert_eq!(initial.scale_120, 120);
    fixture.peer.take_requests();
    fixture.peer.emit(PeerEvent::Enter(0, 0));
    fixture.peer.emit(PeerEvent::Output(0, 2, false));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(fixture.host.configuration(id).unwrap(), Some(initial));
    fixture.peer.emit(PeerEvent::Output(0, 2, true));
    fixture.sync();
    configured(&mut fixture, id, 240, 202);
    fixture.peer.emit(PeerEvent::Output(1, 3, true));
    fixture.peer.emit(PeerEvent::Enter(0, 1));
    fixture.sync();
    configured(&mut fixture, id, 360, 303);
    fixture.peer.emit(PeerEvent::Leave(0, 1));
    fixture.sync();
    configured(&mut fixture, id, 240, 202);
    fixture.peer.emit(PeerEvent::Leave(0, 0));
    fixture.sync();
    configured(&mut fixture, id, 120, 101);
    assert!(fixture.peer.take_requests().is_empty());
}

#[test]
fn version_six_surface_preference_overrides_entered_outputs() {
    let mut fixture = Fixture::with_options(PeerOptions {
        outputs: true,
        ..Default::default()
    });
    let id = fixture.create(101, 101);
    let initial = fixture.configure(id, 0, 101, 101);
    fixture.peer.take_requests();
    fixture.peer.emit(PeerEvent::Output(0, 3, true));
    fixture.peer.emit(PeerEvent::Enter(0, 0));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(fixture.host.configuration(id).unwrap(), Some(initial));
    fixture.peer.emit(PeerEvent::Preferred(0, 2));
    fixture.sync();
    configured(&mut fixture, id, 240, 202);
    fixture.peer.emit(PeerEvent::Output(0, 4, true));
    fixture.sync();
    assert!(fixture.host.events().is_empty());
    assert_eq!(
        fixture.host.configuration(id).unwrap().unwrap().scale_120,
        240
    );
    assert!(fixture.peer.take_requests().is_empty());
}

#[test]
fn suspended_configure_blocks_cpu_without_allocating_then_resume_presents() {
    let mut fixture = Fixture::with_options(PeerOptions {
        shm: true,
        ..Default::default()
    });
    let id = fixture.create(1, 1);
    fixture.host.enable_cpu(id).unwrap();
    fixture.peer.take_requests();
    fixture
        .peer
        .configure(0, 1, 1, 84, 9_u32.to_ne_bytes().to_vec());
    fixture.sync();
    let suspended = fixture.host.configuration(id).unwrap().unwrap();
    assert!(suspended.suspended);
    fixture.host.events();
    fixture.peer.take_requests();
    assert_eq!(
        fixture
            .host
            .present_cpu(id, suspended.revision, &[0, 0, 0, 255])
            .unwrap(),
        PresentOutcome::Suspended
    );
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    assert!(fixture.peer.buffers().is_empty());
    let resumed = fixture.configure(id, 0, 1, 1);
    assert!(!resumed.suspended);
    assert!(resumed.revision > suspended.revision);
    assert_eq!(
        fixture
            .host
            .present_cpu(id, resumed.revision, &[0, 0, 0, 255])
            .unwrap(),
        PresentOutcome::Presented
    );
    fixture.sync();
    assert_eq!(fixture.peer.buffers().len(), 1);
}

#[test]
fn invalid_native_scale_callbacks_stop_without_configuration_or_ack() {
    for event in [
        PeerEvent::Fractional(0, 0),
        PeerEvent::Preferred(0, 0),
        PeerEvent::Preferred(0, -1),
        PeerEvent::Output(0, 0, true),
        PeerEvent::Output(0, -1, true),
    ] {
        let mut fixture = Fixture::with_options(PeerOptions {
            fractional: true,
            viewport: true,
            outputs: true,
            ..Default::default()
        });
        let id = fixture.create(2, 1);
        let lease = fixture.host.lease(id).unwrap();
        fixture.configure(id, 0, 2, 1);
        fixture.peer.take_requests();
        fixture.peer.emit(event);
        read_barrier(&fixture.host);
        assert!(fixture.host.pump(Some(Instant::now())).is_err());
        assert!(fixture.host.events().is_empty());
        assert_eq!(
            fixture.peer.take_requests(),
            [Request::ReleaseOutput(0), Request::ReleaseOutput(1)]
        );
        assert!(lease.is_retired());
        assert!(matches!(
            fixture.host.proxy().try_send(WindowCommand::Close(id)),
            Err(CommandSendError::Closed(_))
        ));
        let cause = fixture.host.failure().unwrap().to_owned();
        assert!(fixture.host.pump(Some(Instant::now())).is_err());
        assert_eq!(fixture.host.failure(), Some(cause.as_str()));
        assert!(fixture.peer.take_requests().is_empty());
    }
}

fn assert_output_bindings(peer: &Peer, count: usize) {
    let mut expected = vec![Request::BindCompositor, Request::BindShell];
    expected.extend((0..count).map(Request::BindOutput));
    assert_eq!(peer.requests(), expected);
}

fn assert_retired_output_bindings(peer: &Peer, count: usize) {
    let mut expected = vec![Request::BindCompositor, Request::BindShell];
    expected.extend((0..count).map(Request::BindOutput));
    expected.extend((0..count).map(Request::ReleaseOutput));
    assert_eq!(peer.requests(), expected);
}

#[test]
fn thirty_two_outputs_bind_exactly_and_thirty_third_stops_discovery_stickily() {
    let options = |output_count| PeerOptions {
        outputs: true,
        output_count,
        ..Default::default()
    };
    let (peer, socket) = Peer::start(options(32));
    let mut host = Host::from_socket(socket).unwrap();
    ready(&mut host);
    read_barrier(&host);
    host.pump(Some(Instant::now())).unwrap();
    assert!(host.failure().is_none());
    assert_output_bindings(&peer, 32);
    host.create_window(WindowOptions::default()).unwrap();
    drop(host);
    drop(peer);
    let (peer, socket) = Peer::start(options(33));
    let mut host = Host::from_socket(socket).unwrap();
    let deadline = Instant::now() + WATCHDOG;
    loop {
        assert!(
            Instant::now() < deadline,
            "output-limit discovery timed out"
        );
        if host.pump(Some(deadline)).is_err() {
            break;
        }
    }
    assert!(host.events().is_empty());
    let cause = host.failure().unwrap().to_owned();
    assert!(cause.contains("resource limits"));
    // The fixture's separate sync queue observes admitted binds and teardown;
    // terminal Host pumping never dispatches these queued native callbacks.
    read_barrier(&host);
    assert_retired_output_bindings(&peer, 32);
    assert!(matches!(
        host.create_window(WindowOptions::default()),
        Err(Error::SessionStopped)
    ));
    let later = host.pump(Some(Instant::now())).unwrap_err();
    assert!(matches!(later.error, Error::SessionStopped));
    assert!(later.commands.is_empty());
    assert_eq!(host.failure(), Some(cause.as_str()));
    assert!(host.events().is_empty());
    assert_retired_output_bindings(&peer, 32);
}

/// Drain events until `count` output announcements arrived.
fn announced(fixture: &mut Fixture, count: usize) -> Vec<OutputInfo> {
    let mut infos = Vec::new();
    for _ in 0..8 {
        infos.extend(
            fixture
                .host
                .events()
                .into_iter()
                .filter_map(|event| match event {
                    WindowEvent::OutputAdded(info) => Some(info),
                    _ => None,
                }),
        );
        if infos.len() >= count {
            break;
        }
        fixture.sync();
    }
    infos
}

#[test]
fn outputs_are_announced_with_name_after_done_and_removed_once() {
    let mut fixture = Fixture::with_options(PeerOptions {
        outputs: true,
        ..Default::default()
    });
    let infos = announced(&mut fixture, 2);
    let names: Vec<_> = infos.iter().map(|info| info.name.as_deref()).collect();
    assert_eq!(names, [Some("OUT-0"), Some("OUT-1")]);
    assert_eq!(infos[1].description.as_deref(), Some("Test output 1"));
    assert!(infos[0].id < infos[1].id);
    assert_eq!(fixture.host.outputs(), infos);
    fixture.peer.emit(PeerEvent::RemoveOutput(0));
    fixture.sync();
    assert_eq!(
        fixture.host.events(),
        [WindowEvent::OutputRemoved(infos[0].id)]
    );
    assert_eq!(fixture.host.outputs(), infos[1..]);
}

#[test]
fn an_output_is_announced_only_after_its_first_done() {
    let mut fixture = Fixture::with_options(PeerOptions {
        outputs: true,
        output_count: 1,
        output_done: false,
        ..Default::default()
    });
    fixture.sync();
    fixture.sync();
    assert!(fixture.host.outputs().is_empty());
    assert!(
        !fixture
            .host
            .events()
            .iter()
            .any(|event| matches!(event, WindowEvent::OutputAdded(_)))
    );
    fixture.peer.emit(PeerEvent::Output(0, 1, true));
    fixture.sync();
    let events = fixture.host.events();
    let [WindowEvent::OutputAdded(info)] = events.as_slice() else {
        panic!("expected one announcement: {events:?}");
    };
    assert_eq!(info.name.as_deref(), Some("OUT-0"));
    assert_eq!(fixture.host.outputs(), std::slice::from_ref(info));
}

#[test]
fn output_identities_are_never_reused_and_carry_their_connection() {
    let mut first = Fixture::with_options(PeerOptions {
        outputs: true,
        output_count: 1,
        ..Default::default()
    });
    let mut second = Fixture::with_options(PeerOptions {
        outputs: true,
        output_count: 1,
        ..Default::default()
    });
    let a = announced(&mut first, 1);
    let b = announced(&mut second, 1);
    assert_eq!(a[0].name, b[0].name);
    assert_ne!(a[0].id, b[0].id, "same name, different connection");
}
