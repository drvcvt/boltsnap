use super::*;
#[test]
fn overwrite_done_reset_and_null_empty_distinction() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    emit(&fixture, Wire::Commit(0, Some("discard".into())));
    emit(&fixture, Wire::Commit(0, Some(String::new())));
    emit(&fixture, Wire::Preedit(0, Some("old".into()), 0, 3));
    emit(&fixture, Wire::Preedit(0, Some("éa".into()), 3, 2));
    emit(&fixture, Wire::Delete(0, 1, 2));
    emit(&fixture, Wire::Delete(0, 3, 4));
    emit(&fixture, Wire::Done(0, 1));
    let events = collected(&mut fixture);
    let ImeEvent::Done {
        transaction,
        session: got,
        synchronized,
        ..
    } = &events[0]
    else {
        panic!()
    };
    assert_eq!(*got, session);
    assert!(*synchronized);
    assert_eq!(transaction.commit.as_ref().unwrap().as_str(), "");
    assert_eq!(
        (transaction.delete_before, transaction.delete_after),
        (3, 4)
    );
    assert_eq!(transaction.preedit.as_ref().unwrap().text.as_str(), "éa");
    assert_eq!(
        transaction.preedit.as_ref().unwrap().cursor,
        Some(ImePreeditCursor { begin: 3, end: 2 })
    );
    emit(&fixture, Wire::Commit(0, None));
    emit(&fixture, Wire::Preedit(0, None, -1, -1));
    emit(&fixture, Wire::Done(0, 1));
    let events = collected(&mut fixture);
    assert!(
        matches!(&events[0], ImeEvent::Done { transaction, .. } if transaction == &ImeTransaction::default())
    );
    emit(&fixture, Wire::Done(0, 1));
    assert!(
        matches!(&collected(&mut fixture)[0], ImeEvent::Done { transaction, .. } if transaction == &ImeTransaction::default())
    );
}
#[test]
fn mismatch_applies_complete_block_but_defers_latest_state_until_matching_count() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    fixture.host.update_ime(session, state(revision)).unwrap();
    fixture.sync();
    fixture.peer.take_requests();
    emit(&fixture, Wire::Commit(0, Some("one".into())));
    emit(&fixture, Wire::Done(0, 1));
    assert!(
        matches!(&collected(&mut fixture)[0], ImeEvent::Done { synchronized: false, transaction, .. }
        if transaction.commit.as_ref().unwrap().as_str() == "one")
    );
    let mut update = state(revision);
    update.caret.x = 11;
    fixture.host.update_ime(session, update).unwrap();
    update.caret.x = 22;
    fixture.host.update_ime(session, update).unwrap();
    fixture.sync();
    assert!(fixture.peer.take_requests().is_empty());
    emit(&fixture, Wire::Done(0, 2));
    assert!(matches!(
        &collected(&mut fixture)[0],
        ImeEvent::Done {
            synchronized: true,
            ..
        }
    ));
    fixture.sync();
    assert_eq!(
        fixture.peer.take_requests(),
        [
            Request::ImeSurrounding(0, "aéz".into(), 3, 1),
            Request::ImeCause(0, 1),
            Request::ImeContent(0, 0, 0),
            Request::ImeCaret(0, 22, 3, 5, 9),
            Request::ImeCommit(0)
        ]
    );
}
#[test]
fn old_activation_blocks_cannot_acquire_replacement() {
    let (mut fixture, window, seat, revision) = fixture();
    let old = activate(&mut fixture, window, seat, revision);
    let current = fixture
        .host
        .enable_ime(window, seat, state(revision))
        .unwrap();
    assert_ne!(old, current);
    fixture.sync();
    fixture.host.events();
    emit(&fixture, Wire::Commit(0, Some("old".into())));
    emit(&fixture, Wire::Done(0, 1));
    assert!(collected(&mut fixture).is_empty());
    emit(&fixture, Wire::Commit(0, Some("current".into())));
    emit(&fixture, Wire::Done(0, 3));
    assert!(
        matches!(&collected(&mut fixture)[0], ImeEvent::Done { session, transaction, .. }
        if *session == current && transaction.commit.as_ref().unwrap().as_str() == "current")
    );
}
#[test]
fn malformed_cursor_ends_the_session_without_partial_transaction_or_host_failure() {
    for cursor in [(1, 2), (-1, 0), (-2, -2), (0, 99)] {
        let (mut fixture, window, seat, revision) = fixture();
        let session = activate(&mut fixture, window, seat, revision);
        emit(&fixture, Wire::Delete(0, 1, 1));
        emit(
            &fixture,
            Wire::Preedit(0, Some("é".into()), cursor.0, cursor.1),
        );
        emit(&fixture, Wire::Done(0, 1));
        assert_ended_alive_and_reactivatable(&mut fixture, window, seat, session);
    }
}
#[test]
fn data_violations_end_the_session_non_fatally() {
    // Injected past the transport: the wire cannot carry NUL or over 4096-byte messages.
    let violations = [
        (
            "NUL",
            Ev::CommitString {
                text: Some("a\0b".into()),
            },
        ),
        (
            "oversize commit",
            Ev::CommitString {
                text: Some("x".repeat(4097)),
            },
        ),
        (
            "oversize preedit",
            Ev::PreeditString {
                text: Some("x".repeat(4097)),
                cursor_begin: -1,
                cursor_end: -1,
            },
        ),
        (
            "cursor on null preedit",
            Ev::PreeditString {
                text: None,
                cursor_begin: 2,
                cursor_end: 3,
            },
        ),
    ];
    for (name, bad) in violations {
        let (mut fixture, window, seat, revision) = fixture();
        let session = activate(&mut fixture, window, seat, revision);
        inject(&mut fixture, bad);
        inject(&mut fixture, Ev::Done { serial: 1 });
        assert_ended_alive_and_reactivatable(&mut fixture, window, seat, session);
        assert_eq!(
            fixture.host.coordinator.state.text_input.test_used(),
            0,
            "{name}"
        );
    }
}
#[test]
fn one_bad_event_drops_the_whole_batch_and_credits_return() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    inject(
        &mut fixture,
        Ev::DeleteSurroundingText {
            before_length: 2,
            after_length: 2,
        },
    );
    inject(
        &mut fixture,
        Ev::CommitString {
            text: Some("good".into()),
        },
    );
    inject(
        &mut fixture,
        Ev::PreeditString {
            text: Some("pre".into()),
            cursor_begin: 0,
            cursor_end: 1,
        },
    );
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 7);
    inject(
        &mut fixture,
        Ev::CommitString {
            text: Some("x".repeat(4097)),
        },
    );
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
    inject(
        &mut fixture,
        Ev::PreeditString {
            text: Some("later".into()),
            cursor_begin: 0,
            cursor_end: 1,
        },
    );
    inject(&mut fixture, Ev::Done { serial: 1 });
    assert_ended_alive_and_reactivatable(&mut fixture, window, seat, session);
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
    emit(&fixture, Wire::Commit(0, Some("fresh".into())));
    emit(&fixture, Wire::Done(0, 3));
    assert!(
        matches!(&collected(&mut fixture)[0], ImeEvent::Done { transaction, .. }
        if transaction.commit.as_ref().unwrap().as_str() == "fresh")
    );
}
#[test]
fn future_count_stays_fatal() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    emit(&fixture, Wire::Commit(0, Some("x".into())));
    emit(&fixture, Wire::Done(0, 2));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    assert_failure_retirement(&mut fixture, window, seat, session);
}
