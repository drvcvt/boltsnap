//! Primary selection as a second, independent selection on the shared paths.
use super::*;

fn both() -> (Fixture, WindowId, InputSeatId) {
    fixture_with(PeerOptions {
        clipboard: true,
        primary: true,
        seat_count: 1,
        ..PeerOptions::default()
    })
}
fn primary(fixture: &mut Fixture, event: Primary) {
    fixture.peer.emit(PeerEvent::Primary(event));
    fixture.sync();
}
fn primary_offer(
    fixture: &mut Fixture,
    window: WindowId,
    seat: InputSeatId,
    bytes: Option<Vec<u8>>,
) -> ClipboardOffer {
    primary(fixture, Primary::Offer(0, vec!["text/plain".into()], bytes));
    fixture
        .host
        .primary_selection_offer(window, seat)
        .unwrap()
        .unwrap()
}

#[test]
fn without_the_global_primary_is_unsupported_and_the_clipboard_still_works() {
    let (mut fixture, window, seat) = fixture();
    assert!(matches!(
        fixture.host.primary_selection_offer(window, seat),
        Err(Error::Unsupported(_))
    ));
    native(&mut fixture, Input::Key(0, 11, 1, 30, true));
    assert!(matches!(
        fixture
            .host
            .set_primary_selection(window, seat, &representation(b"p")),
        Err(Error::Unsupported(_))
    ));
    fixture
        .host
        .set_clipboard(window, seat, &representation(b"c"))
        .unwrap();
}

#[test]
fn binds_once_and_creates_one_device_per_seat() {
    let mut fixture = Fixture::with_options(PeerOptions {
        seat_count: 1,
        ..PeerOptions::default()
    });
    fixture.sync();
    fixture.sync();
    fixture.peer.take_requests();
    fixture.peer.emit(PeerEvent::Primary(Primary::Add));
    fixture.sync();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::BindPrimary, Request::PrimaryDevice(0)]
    );
    native(&mut fixture, Input::AddSeat);
    fixture.sync();
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::PrimaryDevice(1))
    );
}

#[test]
fn primary_offers_publish_separately_from_the_clipboard() {
    let (mut fixture, window, seat) = both();
    primary(
        &mut fixture,
        Primary::Offer(0, vec!["text/plain".into(), "UTF8_STRING".into()], None),
    );
    let offer = fixture
        .host
        .primary_selection_offer(window, seat)
        .unwrap()
        .unwrap();
    assert_eq!(offer.mimes, ["text/plain", "UTF8_STRING"]);
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::PrimarySelection {
            seat,
            offer: Some(offer.clone())
        }]
    );
    assert_eq!(fixture.host.clipboard_offer(window, seat).unwrap(), None);
    emit(&mut fixture, Wire::Offer(0, vec!["text/html".into()], None));
    let ordinary = fixture.host.clipboard_offer(window, seat).unwrap().unwrap();
    assert_ne!(ordinary.id, offer.id);
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::Selection {
            seat,
            offer: Some(ordinary)
        }]
    );
    assert_eq!(
        fixture.host.primary_selection_offer(window, seat).unwrap(),
        Some(offer)
    );
}

#[test]
fn a_null_primary_selection_destroys_the_previous_offer() {
    let (mut fixture, window, seat) = both();
    primary_offer(&mut fixture, window, seat, None);
    events(&mut fixture);
    fixture.peer.take_requests();
    primary(&mut fixture, Primary::SelectionNone(0));
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::PrimarySelection { seat, offer: None }]
    );
    assert_eq!(
        fixture.host.primary_selection_offer(window, seat).unwrap(),
        None
    );
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::PrimaryDestroyOffer(0)]
    );
}

#[test]
fn primary_reads_receive_from_the_primary_offer() {
    let (mut fixture, window, seat) = both();
    let offer = primary_offer(&mut fixture, window, seat, Some(b"middle".to_vec()));
    events(&mut fixture);
    fixture.peer.take_requests();
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 64))
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::PrimaryReceive(0, "text/plain".into())]
    );
    assert_eq!(
        events(&mut fixture),
        [received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::Bytes(b"middle".to_vec())
        )]
    );
}

#[test]
fn a_primary_read_supersedes_a_pending_clipboard_read() {
    let (mut fixture, window, seat) = both();
    let ordinary = offer(&mut fixture, window, seat, None);
    let first = fixture
        .host
        .receive_clipboard(request(window, seat, ordinary.id, 64))
        .unwrap();
    fixture.sync();
    let middle = primary_offer(&mut fixture, window, seat, Some(b"p".to_vec()));
    events(&mut fixture);
    let second = fixture
        .host
        .receive_clipboard(request(window, seat, middle.id, 64))
        .unwrap();
    fixture.sync();
    assert_eq!(
        events(&mut fixture),
        [
            received(first, window, seat, ClipboardReceiveOutcome::Cancelled),
            received(
                second,
                window,
                seat,
                ClipboardReceiveOutcome::Bytes(b"p".to_vec())
            )
        ]
    );
}

#[test]
fn primary_sources_need_a_press_and_claim_with_its_serial() {
    let (mut fixture, window, seat) = both();
    let data = representation(b"selected");
    assert!(matches!(
        fixture.host.set_primary_selection(window, seat, &data),
        Err(Error::NotReady)
    ));
    native(&mut fixture, Input::Key(0, 77, 1, 30, true));
    fixture.peer.take_requests();
    fixture
        .host
        .set_primary_selection(window, seat, &data)
        .unwrap();
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::PrimarySource(0),
            Request::PrimaryMime(0, "text/plain;charset=utf-8".into()),
            Request::PrimaryMime(0, "text/plain".into()),
            Request::PrimarySet(Some(0), 77),
        ]
    );
    native(&mut fixture, Input::KeyboardLeave(0, 0, 78));
    assert!(matches!(
        fixture.host.set_primary_selection(window, seat, &data),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn primary_sends_exact_bytes_and_replacement_keeps_the_clipboard_source() {
    let (mut fixture, window, seat) = both();
    native(&mut fixture, Input::Key(0, 5, 1, 30, true));
    let ordinary = fixture
        .host
        .set_clipboard(window, seat, &representation(b"copied"))
        .unwrap();
    let first = fixture
        .host
        .set_primary_selection(window, seat, &representation(b"selected"))
        .unwrap();
    fixture.sync();
    primary(&mut fixture, Primary::Send(0, "text/plain".into()));
    primary(&mut fixture, Primary::DrainSend(0));
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::PrimaryBytes(b"selected".to_vec()))
    );
    fixture
        .host
        .set_primary_selection(window, seat, &representation(b"again"))
        .unwrap();
    fixture.sync();
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::SourceCancelled { source: first }]
    );
    let requests = fixture.peer.take_requests();
    assert!(requests.contains(&Request::PrimaryDestroySource(0)));
    assert!(!requests.contains(&Request::ClipboardDestroySource(0)));
    emit(&mut fixture, Wire::CancelSource(0));
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::SourceCancelled { source: ordinary }]
    );
}

#[test]
fn compositor_cancel_retires_the_primary_source() {
    let (mut fixture, window, seat) = both();
    native(&mut fixture, Input::Key(0, 6, 1, 30, true));
    let source = fixture
        .host
        .set_primary_selection(window, seat, &representation(b"gone"))
        .unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    primary(&mut fixture, Primary::CancelSource(0));
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::SourceCancelled { source }]
    );
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [Request::PrimaryDestroySource(0)]
    );
}

#[test]
fn removing_the_global_disables_primary_until_a_new_host() {
    let (mut fixture, window, seat) = both();
    native(&mut fixture, Input::Key(0, 7, 1, 30, true));
    primary(&mut fixture, Primary::Remove);
    assert!(matches!(
        fixture.host.primary_selection_offer(window, seat),
        Err(Error::Unsupported(_))
    ));
    assert!(matches!(
        fixture
            .host
            .set_primary_selection(window, seat, &representation(b"x")),
        Err(Error::Unsupported(_))
    ));
    fixture.peer.take_requests();
    primary(&mut fixture, Primary::Add);
    fixture.sync();
    assert!(matches!(
        fixture.host.primary_selection_offer(window, seat),
        Err(Error::Unsupported(_))
    ));
    assert!(!fixture.peer.take_requests().contains(&Request::BindPrimary));
    assert_eq!(fixture.host.clipboard_offer(window, seat).unwrap(), None);
}

#[test]
fn seat_removal_destroys_the_primary_source_device_and_offer() {
    let (mut fixture, window, seat) = both();
    native(&mut fixture, Input::Key(0, 8, 1, 30, true));
    let source = fixture
        .host
        .set_primary_selection(window, seat, &representation(b"seat"))
        .unwrap();
    primary_offer(&mut fixture, window, seat, None);
    events(&mut fixture);
    fixture.peer.take_requests();
    native(&mut fixture, Input::RemoveSeat(0));
    fixture.sync();
    assert!(events(&mut fixture).contains(&ClipboardEvent::SourceCancelled { source }));
    let requests = fixture.peer.take_requests();
    for request in [
        Request::PrimaryDestroySource(0),
        Request::PrimaryDestroyDevice(0),
        Request::PrimaryDestroyOffer(0),
    ] {
        assert!(requests.contains(&request), "{request:?} in {requests:?}");
    }
}

#[test]
fn shutdown_destroys_the_primary_manager() {
    let (mut fixture, window, seat) = both();
    native(&mut fixture, Input::Key(0, 9, 1, 30, true));
    fixture
        .host
        .set_primary_selection(window, seat, &representation(b"end"))
        .unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    let Fixture { host, peer } = fixture;
    host.shutdown();
    let requests = peer.requests();
    assert!(requests.contains(&Request::PrimaryDestroySource(0)));
    assert!(requests.contains(&Request::PrimaryDestroyManager));
}
