use super::*;
#[test]
fn native_each_and_combined_payload_limits_and_hidden_cursor() {
    let (mut fixture, window, seat, revision) = fixture();
    activate(&mut fixture, window, seat, revision);
    emit(&fixture, Wire::Commit(0, Some("c".repeat(4000))));
    emit(&fixture, Wire::Preedit(0, Some("p".repeat(4000)), -1, -1));
    emit(&fixture, Wire::Done(0, 1));
    let events = collected(&mut fixture);
    assert!(matches!(&events[0], ImeEvent::Done { transaction, .. } if
        transaction.commit.as_ref().unwrap().as_str().len() == 4000 &&
        transaction.preedit.as_ref().unwrap().text.as_str().len() == 4000 &&
        transaction.preedit.as_ref().unwrap().cursor.is_none()));
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 8000);
    drop(events);
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
}
#[test]
fn escaped_arc_clones_charge_until_final_drop_after_shutdown() {
    let (mut fixture, window, seat, revision) = fixture();
    activate(&mut fixture, window, seat, revision);
    let mut retained = Vec::new();
    for _ in 0..16 {
        emit(&fixture, Wire::Commit(0, Some("c".repeat(4000))));
        emit(&fixture, Wire::Preedit(0, Some("p".repeat(4000)), -1, -1));
        emit(&fixture, Wire::Done(0, 1));
        let events = collected(&mut fixture);
        let ImeEvent::Done { transaction, .. } = &events[0] else {
            panic!()
        };
        retained.push(transaction.commit.as_ref().unwrap().shared());
        retained.push(transaction.preedit.as_ref().unwrap().text.shared());
    }
    retained.push(emitted_commit(&mut fixture, 3072));
    assert_eq!(
        fixture.host.coordinator.state.text_input.test_used(),
        128 * 1024
    );
    let last = retained.pop().unwrap();
    drop(last);
    assert_eq!(
        fixture.host.coordinator.state.text_input.test_used(),
        128_000
    );
    emit(&fixture, Wire::Commit(0, Some("x".repeat(3072))));
    emit(&fixture, Wire::Done(0, 1));
    let events = collected(&mut fixture);
    assert_eq!(
        fixture.host.coordinator.state.text_input.test_used(),
        128 * 1024
    );
    let clone = retained[0].clone();
    fixture.host.coordinator.state.retire_all();
    drop(retained);
    drop(events);
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 4000);
    assert_eq!(clone.len(), 4000);
    drop(clone);
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
}
#[test]
fn queue_overflow_keeps_admitted_prefix_and_terminal_reservations() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    let max = crate::window::state::MAX_EVENTS;
    let reserved = fixture.host.coordinator.state.input.retirement_count(None)
        + fixture.host.coordinator.state.text_input.reserved();
    for index in 0..max - reserved {
        emit(&fixture, Wire::Done(0, 1));
        if index % 64 == 63 {
            read_barrier(&fixture.host);
        }
    }
    emit(&fixture, Wire::Commit(0, Some("overflow".into())));
    emit(&fixture, Wire::Done(0, 1));
    read_barrier(&fixture.host);
    assert!(fixture.host.pump(Some(Instant::now())).is_err());
    let events = fixture.host.events();
    assert_eq!(events.len(), max);
    let prefix = &events[..max - reserved];
    assert!(prefix.iter().all(
        |event| matches!(event, WindowEvent::Ime(ImeEvent::Done { transaction, .. })
        if transaction == &ImeTransaction::default())
    ));
    assert!(
        matches!(&events[max - reserved], WindowEvent::Ime(ImeEvent::Ended { session: got, .. }) if *got == session)
    );
    assert_eq!(events[max - reserved..].len(), reserved);
    assert!(
        events[max - reserved..]
            .iter()
            .all(|event| !matches!(event, WindowEvent::Ime(ImeEvent::Done { .. })))
    );
}
#[test]
fn shared_ledger_overflow_drops_only_the_pending_batch_and_ends_the_session() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    let mut retained = Vec::new();
    for _ in 0..32 {
        emit(&fixture, Wire::Commit(0, Some("x".repeat(4000))));
        emit(&fixture, Wire::Done(0, 1));
        retained.extend(collected(&mut fixture));
    }
    emit(&fixture, Wire::Commit(0, Some("x".repeat(3072))));
    emit(&fixture, Wire::Done(0, 1));
    retained.extend(collected(&mut fixture));
    assert_eq!(
        fixture.host.coordinator.state.text_input.test_used(),
        128 * 1024
    );
    emit(&fixture, Wire::Delete(0, 1, 1));
    emit(&fixture, Wire::Commit(0, Some("x".into())));
    emit(&fixture, Wire::Done(0, 1));
    assert_ended_alive_and_reactivatable(&mut fixture, window, seat, session);
    assert_eq!(retained.len(), 33);
    assert_eq!(
        fixture.host.coordinator.state.text_input.test_used(),
        128 * 1024
    );
    drop(retained);
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
}
#[test]
fn empty_payload_records_cap_at_1024_poisons_batch_and_reaps_after_final_clone() {
    let (mut fixture, window, seat, revision) = fixture();
    let session = activate(&mut fixture, window, seat, revision);
    let mut retained = Vec::new();
    for _ in 0..512 {
        emit(&fixture, Wire::Commit(0, Some(String::new())));
        emit(&fixture, Wire::Preedit(0, Some(String::new()), 0, 0));
        emit(&fixture, Wire::Done(0, 1));
        retained.extend(collected(&mut fixture));
    }
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
    drop(retained.pop()); // Release two exact allocations, even though byte charge is zero.
    emit(&fixture, Wire::Commit(0, Some(String::new())));
    emit(&fixture, Wire::Preedit(0, Some(String::new()), 0, 0));
    emit(&fixture, Wire::Done(0, 1));
    retained.extend(collected(&mut fixture));
    assert_eq!(retained.len(), 512);
    emit(&fixture, Wire::Commit(0, Some(String::new())));
    emit(&fixture, Wire::Done(0, 1));
    assert_ended_alive_and_reactivatable(&mut fixture, window, seat, session);
    drop(retained);
    assert_eq!(fixture.host.coordinator.state.text_input.test_used(), 0);
}

fn emitted_commit(fixture: &mut Fixture, length: usize) -> std::sync::Arc<str> {
    emit(fixture, Wire::Commit(0, Some("x".repeat(length))));
    emit(fixture, Wire::Done(0, 1));
    let events = collected(fixture);
    let ImeEvent::Done { transaction, .. } = &events[0] else {
        panic!()
    };
    transaction.commit.as_ref().unwrap().shared()
}
