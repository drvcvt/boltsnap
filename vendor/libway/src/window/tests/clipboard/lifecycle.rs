//! Offer availability and focus/window/capability retirement on the actual queue.
use super::*;

#[test]
fn offer_before_enter_survives_same_client_focus_switch_and_reads_cancel() {
    let (mut fixture, window, seat) = fixture();
    native(&mut fixture, Input::KeyboardLeave(0, 0, 30));
    emit(
        &mut fixture,
        Wire::Offer(0, vec!["text/plain".into()], None),
    );
    let metadata = match events(&mut fixture).pop().unwrap() {
        ClipboardEvent::Selection {
            offer: Some(offer), ..
        } => offer,
        other => panic!("{other:?}"),
    };
    assert!(matches!(
        fixture.host.clipboard_offer(window, seat),
        Err(Error::InvalidInput(_))
    ));
    native(&mut fixture, Input::KeyboardEnter(0, 0, 31, vec![]));
    assert_eq!(
        fixture.host.clipboard_offer(window, seat).unwrap(),
        Some(metadata.clone())
    );
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, metadata.id, 16))
        .unwrap();
    fixture.sync();
    let other = fixture.create(100, 100);
    native(&mut fixture, Input::KeyboardLeave(0, 0, 32));
    native(&mut fixture, Input::KeyboardEnter(0, 1, 33, vec![]));
    assert_eq!(
        events(&mut fixture),
        [received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::Cancelled
        )]
    );
    assert_eq!(
        fixture.host.clipboard_offer(other, seat).unwrap(),
        Some(metadata)
    );
    emit(&mut fixture, Wire::SelectionNone(0));
    assert_eq!(fixture.host.clipboard_offer(other, seat).unwrap(), None);
}

#[test]
fn manager_discovery_after_seat_and_removal_are_explicit() {
    let mut fixture = Fixture::with_options(PeerOptions {
        seat_count: 1,
        ..PeerOptions::default()
    });
    fixture.sync();
    fixture.sync();
    fixture
        .peer
        .emit(PeerEvent::Input(Input::Capabilities(0, 3)));
    fixture.sync();
    let seat = fixture
        .host
        .events()
        .into_iter()
        .find_map(|event| match event {
            WindowEvent::Input(event) => Some(event.seat),
            _ => None,
        })
        .unwrap();
    let window = fixture.create(100, 100);
    native(&mut fixture, Input::KeyboardEnter(0, 0, 10, vec![]));
    assert!(matches!(
        fixture.host.clipboard_offer(window, seat),
        Err(Error::Unsupported(_))
    ));
    emit(&mut fixture, Wire::AddManager);
    fixture.sync();
    assert_eq!(fixture.host.clipboard_offer(window, seat).unwrap(), None);
    native(&mut fixture, Input::Key(0, 11, 1, 30, true));
    fixture
        .host
        .set_clipboard(window, seat, &representation(b"owned"))
        .unwrap();
    fixture.sync();
    emit(&mut fixture, Wire::RemoveManager);
    assert!(matches!(
        fixture.host.clipboard_offer(window, seat),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        fixture
            .host
            .set_clipboard(window, seat, &representation(b"new")),
        Err(Error::Unsupported(_))
    ));
    fixture.peer.take_requests();
    emit(&mut fixture, Wire::AddManager);
    fixture.sync();
    assert!(matches!(
        fixture.host.clipboard_offer(window, seat),
        Err(Error::Unsupported(_))
    ));
    assert!(
        !fixture
            .peer
            .take_requests()
            .contains(&Request::BindClipboard)
    );
}

#[test]
fn capability_loss_and_window_close_retire_reads_exactly_once() {
    let (mut fixture, window, seat) = fixture();
    let offer = offer(&mut fixture, window, seat, None);
    events(&mut fixture);
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 16))
        .unwrap();
    fixture.sync();
    native(&mut fixture, Input::Capabilities(0, 1));
    assert_eq!(
        events(&mut fixture),
        [received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::Cancelled
        )]
    );
    native(&mut fixture, Input::Capabilities(0, 3));
    native(&mut fixture, Input::KeyboardEnter(1, 0, 40, vec![]));
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 16))
        .unwrap();
    fixture.sync();
    fixture.host.close(window).unwrap();
    assert_eq!(
        events(&mut fixture),
        [received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::Cancelled
        )]
    );
    assert!(!fixture.host.cancel_clipboard(id).unwrap());
}

#[test]
fn late_offer_on_retired_legacy_device_is_explicitly_destroyed() {
    let mut fixture = Fixture::with_options(PeerOptions {
        clipboard: true,
        clipboard_version: 1,
        seat_count: 1,
        ..PeerOptions::default()
    });
    fixture.sync();
    fixture.sync();
    fixture.host.events();
    native(&mut fixture, Input::RemoveSeat(0));
    fixture.host.events();
    fixture.peer.take_requests();
    emit(
        &mut fixture,
        Wire::Offer(0, vec!["text/plain".into()], Some(vec![])),
    );
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::ClipboardDestroyOffer(0)]
    );
    assert!(events(&mut fixture).is_empty());
    assert!(fixture.host.failure().is_none());
}
