//! Exact EOF bytes, pinned offers and transactional receive supersession.
use super::*;

#[test]
fn native_reads_preserve_empty_invalid_utf8_and_exact_limit_bytes() {
    let (mut fixture, window, seat) = fixture();
    for bytes in [vec![], vec![0xff, 0, 0xc3, 0xa9]] {
        let offer = offer(&mut fixture, window, seat, Some(bytes.clone()));
        events(&mut fixture);
        let id = fixture
            .host
            .receive_clipboard(request(window, seat, offer.id, bytes.len().max(1)))
            .unwrap();
        fixture.sync();
        assert_eq!(
            events(&mut fixture),
            [received(
                id,
                window,
                seat,
                ClipboardReceiveOutcome::Bytes(bytes)
            )]
        );
        assert!(!fixture.host.cancel_clipboard(id).unwrap());
    }
    let offer = offer(&mut fixture, window, seat, Some(vec![1, 2, 3]));
    events(&mut fixture);
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 2))
        .unwrap();
    fixture.sync();
    assert_eq!(
        events(&mut fixture),
        [received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::TooLarge
        )]
    );
}

#[test]
fn pinned_read_survives_selection_replacement_and_stale_admission_preserves_it() {
    let (mut fixture, window, seat) = fixture();
    let old = offer(&mut fixture, window, seat, None);
    events(&mut fixture);
    let id = fixture
        .host
        .receive_clipboard(request(window, seat, old.id, 16))
        .unwrap();
    fixture.sync();
    let current = offer(&mut fixture, window, seat, Some(b"new".to_vec()));
    assert_ne!(old.id, current.id);
    assert!(matches!(
        fixture
            .host
            .receive_clipboard(request(window, seat, old.id, 16)),
        Err(Error::InvalidInput(_))
    ));
    events(&mut fixture);
    emit(&mut fixture, Wire::Complete(0, b"original".to_vec()));
    assert_eq!(
        events(&mut fixture),
        [received(
            id,
            window,
            seat,
            ClipboardReceiveOutcome::Bytes(b"original".to_vec())
        )]
    );
}

#[test]
fn supersession_reserves_cancelled_result_and_failed_admission_issues_no_identity() {
    let (mut fixture, window, seat) = fixture();
    let offer = offer(&mut fixture, window, seat, None);
    events(&mut fixture);
    let first = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 16))
        .unwrap();
    fixture.sync();
    let mut invalid = request(window, seat, offer.id, 0);
    assert!(matches!(
        fixture.host.receive_clipboard(invalid.clone()),
        Err(Error::InvalidInput(_))
    ));
    invalid.max_bytes = 16;
    invalid.deadline = Instant::now();
    assert!(matches!(
        fixture.host.receive_clipboard(invalid),
        Err(Error::InvalidInput(_))
    ));
    let never = ClipboardTransferId(first.0, first.1 + 1);
    assert!(matches!(
        fixture.host.cancel_clipboard(never),
        Err(Error::InvalidInput(_))
    ));
    assert!(events(&mut fixture).is_empty());
    let second = fixture
        .host
        .receive_clipboard(request(window, seat, offer.id, 16))
        .unwrap();
    assert_eq!(second, never);
    fixture.sync();
    assert_eq!(
        events(&mut fixture),
        [received(
            first,
            window,
            seat,
            ClipboardReceiveOutcome::Cancelled
        )]
    );
    assert!(!fixture.host.cancel_clipboard(first).unwrap());
    assert!(fixture.host.cancel_clipboard(second).unwrap());
    assert_eq!(
        events(&mut fixture),
        [received(
            second,
            window,
            seat,
            ClipboardReceiveOutcome::Cancelled
        )]
    );
    assert!(!fixture.host.cancel_clipboard(second).unwrap());
    assert!(matches!(
        fixture
            .host
            .cancel_clipboard(ClipboardTransferId(second.0 + 1, second.1)),
        Err(Error::InvalidInput(_))
    ));
}
