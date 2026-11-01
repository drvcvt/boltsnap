//! Protected terminal slots and full-host read replacement at bounded capacity.
use super::*;

#[test]
fn queue_pressure_preserves_old_read_and_reserved_cancellation_slot() {
    let (mut fixture, window, seat) = fixture();
    let offer = offer(&mut fixture, window, seat, None);
    events(&mut fixture);
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 16))
        .unwrap();
    fixture.sync();
    let state = &mut fixture.host.coordinator.state;
    while state.event_room(1) {
        assert!(state.push(WindowEvent::Wake));
    }
    assert!(matches!(
        fixture
            .host
            .receive_clipboard(request(window, seat, offer.id, 16)),
        Err(Error::LimitExceeded)
    ));
    assert!(matches!(
        fixture
            .host
            .cancel_clipboard(ClipboardTransferId(id.0, id.1 + 1)),
        Err(Error::InvalidInput(_))
    ));
    assert!(fixture.host.cancel_clipboard(id).unwrap());
    let all = fixture.host.events();
    assert_eq!(all.len(), super::super::super::state::MAX_EVENTS - 1);
    assert_eq!(
        all.last(),
        Some(&WindowEvent::Clipboard(received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::Cancelled
        )))
    );
    assert!(fixture.host.failure().is_none());
    // Draining preserves native VecDeque allocation and makes admission recover.
    let next = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 16))
        .unwrap();
    assert_eq!(next.1, id.1 + 1);
    assert!(fixture.host.cancel_clipboard(next).unwrap());
}

#[test]
fn eight_active_seats_permit_transactional_same_seat_replacement() {
    let mut fixture = Fixture::with_options(PeerOptions {
        clipboard: true,
        seat_count: 8,
        ..PeerOptions::default()
    });
    fixture.sync();
    fixture.sync();
    fixture.host.events();
    let window = fixture.create(100, 100);
    let mut reads = Vec::new();
    let mut ids = Vec::new();
    for index in 0..8 {
        fixture.host.events();
        native(&mut fixture, Input::Capabilities(index, 3));
        let seat = fixture
            .host
            .events()
            .into_iter()
            .find_map(|event| match event {
                WindowEvent::Input(event) => Some(event.seat),
                _ => None,
            })
            .unwrap();
        native(&mut fixture, Input::KeyboardEnter(index, 0, 10, vec![]));
        emit(
            &mut fixture,
            Wire::Offer(index, vec!["text/plain".into()], None),
        );
        let offer = fixture.host.clipboard_offer(window, seat).unwrap().unwrap();
        let read = request(window, seat, offer.id, 16);
        ids.push(fixture.host.receive_clipboard(read.clone()).unwrap());
        reads.push(read);
        fixture.sync();
    }
    events(&mut fixture);
    let replacement = fixture.host.receive_clipboard(reads[0].clone()).unwrap();
    assert_ne!(replacement, ids[0]);
    assert_eq!(
        events(&mut fixture),
        [received(
            ids[0],
            window,
            reads[0].seat,
            ClipboardReceiveOutcome::Cancelled
        )]
    );
    assert!(!fixture.host.cancel_clipboard(ids[0]).unwrap());
    assert!(fixture.host.cancel_clipboard(replacement).unwrap());
    for id in &ids[1..] {
        assert!(fixture.host.cancel_clipboard(*id).unwrap());
    }
    assert_eq!(events(&mut fixture).len(), 8);
}
