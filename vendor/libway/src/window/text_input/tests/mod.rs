//! Literal socketpair protocol oracles; never connects to a desktop compositor.
mod admission;
mod bounds;
mod lifecycle;
mod transactions;
use super::*;
use crate::window::tests::peer::{ImeWireEvent as Wire, InputWireEvent};
use crate::window::{ImeCause, ImeEvent, ImeRectangle, ImeSession, ImeState, ImeSurrounding};
use wayland_protocols::wp::text_input::zv3::client::zwp_text_input_v3::Event as Ev;

fn fixture() -> (Fixture, WindowId, InputSeatId, ConfigurationRevision) {
    let mut fixture = Fixture::with_options(PeerOptions {
        ime: true,
        seat_count: 1,
        ..PeerOptions::default()
    });
    let window = fixture.create(80, 20);
    let configuration = fixture.configure(window, 0, 80, 20);
    let seat = fixture.host.coordinator.state.text_input.test_seat();
    fixture
        .peer
        .emit(PeerEvent::Input(InputWireEvent::KeyboardEnter(
            0,
            0,
            1,
            vec![],
        )));
    fixture.peer.emit(PeerEvent::Ime(Wire::Enter(0, 0)));
    fixture.sync();
    fixture.host.events();
    fixture.peer.take_requests();
    (fixture, window, seat, configuration.revision)
}
fn state(revision: ConfigurationRevision) -> ImeState<'static> {
    ImeState {
        configuration: revision,
        surrounding: Some(ImeSurrounding {
            text: "aéz",
            cursor: 3,
            anchor: 1,
        }),
        caret: ImeRectangle {
            x: -2,
            y: 3,
            width: 5,
            height: 9,
        },
        cause: ImeCause::Other,
    }
}
fn activate(
    fixture: &mut Fixture,
    window: WindowId,
    seat: InputSeatId,
    revision: ConfigurationRevision,
) -> ImeSession {
    let session = fixture
        .host
        .enable_ime(window, seat, state(revision))
        .unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    session
}
fn emit(fixture: &Fixture, wire: Wire) {
    fixture.peer.emit(PeerEvent::Ime(wire));
}
fn collected(fixture: &mut Fixture) -> Vec<ImeEvent> {
    fixture.sync();
    fixture
        .host
        .events()
        .into_iter()
        .filter_map(|event| match event {
            WindowEvent::Ime(event) => Some(event),
            _ => None,
        })
        .collect()
}

fn assert_failure_retirement(
    fixture: &mut Fixture,
    window: WindowId,
    seat: InputSeatId,
    session: ImeSession,
) {
    let events = fixture.host.events();
    assert_eq!(
        events.first(),
        Some(&WindowEvent::Ime(ImeEvent::Ended {
            window,
            seat,
            session
        }))
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, WindowEvent::Ime(ImeEvent::Ended { .. })))
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                WindowEvent::Ime(ImeEvent::Focus { entered: false, .. })
            ))
            .count(),
        1
    );
    assert!(events.iter().all(|event| matches!(
        event,
        WindowEvent::Ime(ImeEvent::Ended { .. } | ImeEvent::Focus { entered: false, .. })
            | WindowEvent::Input(InputEvent {
                observation: InputObservation::Cancelled { .. },
                ..
            })
    )));
    assert!(
        fixture
            .host
            .coordinator
            .state
            .text_input
            .test_session_or_none()
            .is_none()
    );
}

/// A data violation retires the session through the normal path: disable plus
/// commit on the wire, one `Ended`, no `Done`, a live host and a working new activation.
fn assert_ended_alive_and_reactivatable(
    fixture: &mut Fixture,
    window: WindowId,
    seat: InputSeatId,
    session: ImeSession,
) {
    let requests = fixture.sync();
    fixture.peer.take_requests();
    assert_eq!(requests, [Request::ImeDisable(0), Request::ImeCommit(0)]);
    let events = fixture.host.events();
    assert_eq!(
        events,
        [WindowEvent::Ime(ImeEvent::Ended {
            window,
            seat,
            session
        })]
    );
    assert!(
        fixture
            .host
            .coordinator
            .state
            .text_input
            .test_session_or_none()
            .is_none()
    );
    let revision = fixture
        .host
        .configuration(window)
        .unwrap()
        .unwrap()
        .revision;
    let next = activate(fixture, window, seat, revision);
    assert_ne!(next, session);
    assert_eq!(
        fixture.host.coordinator.state.text_input.test_session(),
        Some(next)
    );
    emit(fixture, Wire::Done(0, 3));
    assert!(
        matches!(&collected(fixture)[0], ImeEvent::Done { session, transaction, .. }
        if *session == next && transaction == &ImeTransaction::default())
    );
}

fn inject(fixture: &mut Fixture, event: Ev) {
    crate::window::text_input::test_incoming(&mut fixture.host.coordinator.state, event).unwrap();
}
