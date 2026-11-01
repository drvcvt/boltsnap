//! Actual-press admission and eager source lifetime independent of the editor.
use super::*;

#[test]
fn serial_grants_require_actual_press_in_current_focus_epoch() {
    let (mut fixture, window, seat) = fixture();
    let data = representation(b"copied");
    assert!(matches!(
        fixture.host.set_clipboard(window, seat, &data),
        Err(Error::NotReady)
    ));
    native(&mut fixture, Input::Key(0, 20, 1, 30, false));
    native(&mut fixture, Input::Modifiers(0, 21, 0, 0, 0, 0));
    assert!(matches!(
        fixture.host.set_clipboard(window, seat, &data),
        Err(Error::NotReady)
    ));
    native(&mut fixture, Input::Key(0, u32::MAX, 2, 30, true));
    let first = fixture.host.set_clipboard(window, seat, &data).unwrap();
    fixture.sync();
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::ClipboardSet(Some(0), u32::MAX))
    );
    native(&mut fixture, Input::KeyboardLeave(0, 0, 22));
    native(
        &mut fixture,
        Input::KeyboardEnter(0, 0, 23, vec![30, 0, 0, 0]),
    );
    assert!(matches!(
        fixture.host.set_clipboard(window, seat, &data),
        Err(Error::NotReady)
    ));
    native(&mut fixture, Input::PointerEnter(0, 0, 24, 2., 2.));
    native(&mut fixture, Input::Button(0, 0, 3, 272, true));
    let second = fixture.host.set_clipboard(window, seat, &data).unwrap();
    fixture.sync();
    assert_ne!(first, second);
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::ClipboardSet(Some(1), 0))
    );
    assert!(events(&mut fixture).contains(&ClipboardEvent::SourceCancelled { source: first }));
}

#[test]
fn invalid_replacement_preserves_source_and_close_preserves_exact_bytes() {
    let (mut fixture, window, seat) = fixture();
    native(&mut fixture, Input::Key(0, 40, 1, 30, true));
    let data = representation("A\0é\n".as_bytes());
    let source = fixture.host.set_clipboard(window, seat, &data).unwrap();
    fixture.sync();
    let invalid = vec![data[0].clone(), data[0].clone()];
    assert!(matches!(
        fixture.host.set_clipboard(window, seat, &invalid),
        Err(Error::InvalidInput(_))
    ));
    fixture.host.close(window).unwrap();
    fixture.sync();
    assert!(!events(&mut fixture).contains(&ClipboardEvent::SourceCancelled { source }));
    emit(&mut fixture, Wire::Send(0, "text/plain".into()));
    emit(&mut fixture, Wire::DrainSend(0));
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::ClipboardBytes("A\0é\n".as_bytes().to_vec()))
    );
    emit(&mut fixture, Wire::CancelSource(0));
    assert_eq!(
        events(&mut fixture),
        [ClipboardEvent::SourceCancelled { source }]
    );
}

#[test]
fn accepted_send_retains_original_bytes_after_source_replacement() {
    let (mut fixture, window, seat) = fixture();
    native(&mut fixture, Input::Key(0, 50, 1, 30, true));
    let bytes = vec![0x9b; 192 * 1024];
    let source = fixture
        .host
        .set_clipboard(window, seat, &representation(&bytes))
        .unwrap();
    fixture.sync();
    emit(&mut fixture, Wire::Send(0, "text/plain".into()));
    fixture
        .host
        .set_clipboard(window, seat, &representation(b"new"))
        .unwrap();
    fixture.sync();
    assert!(events(&mut fixture).contains(&ClipboardEvent::SourceCancelled { source }));
    let mut received = Vec::new();
    for _ in 0..5 {
        emit(&mut fixture, Wire::DrainSend(0));
        for request in fixture.peer.take_requests() {
            if let Request::ClipboardBytes(bytes) = request {
                received.extend(bytes);
            }
        }
    }
    assert_eq!(received, bytes);
}

#[test]
fn repeated_keyboard_only_capabilities_preserve_a_fresh_press() {
    let (mut fixture, window, seat) = fixture();
    native(&mut fixture, Input::Capabilities(0, 2));
    native(&mut fixture, Input::Key(0, 51, 1, 30, true));
    native(&mut fixture, Input::Capabilities(0, 2));
    fixture
        .host
        .set_clipboard(window, seat, &representation(b"grant"))
        .unwrap();
    fixture.sync();
    assert!(
        fixture
            .peer
            .take_requests()
            .contains(&Request::ClipboardSet(Some(0), 51))
    );
}
