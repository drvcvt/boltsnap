//! Worker publications survive descriptor drains, coalescing and bounded admission.
use super::*;
use std::{
    sync::{Arc, Barrier, mpsc::channel},
    thread,
};

#[test]
fn concurrent_wakes_coalesce_until_the_retained_event_is_collected() {
    let mut fixture = Fixture::new();
    let wake = fixture.host.wake();
    let barrier = Arc::new(Barrier::new(9));
    let workers: Vec<_> = (0..8)
        .map(|_| {
            let wake = wake.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                wake.wake().unwrap();
            })
        })
        .collect();
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }
    let turn = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert!(turn.commands.is_empty() && turn.runnable);
    assert_eq!(turn.events_pending, 1);
    wake.wake().unwrap();
    assert_eq!(
        fixture
            .host
            .pump(Some(Instant::now()))
            .unwrap()
            .events_pending,
        1
    );
    assert_eq!(fixture.host.events(), [WindowEvent::Wake]);
    wake.wake().unwrap();
    fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(fixture.host.events(), [WindowEvent::Wake]);
}

#[test]
fn draining_the_descriptor_before_host_application_cannot_consume_the_wake_token() {
    let mut fixture = Fixture::new();
    let wake = fixture.host.wake();
    wake.wake().unwrap();
    // Coordinator servicing drains the socket but leaves application commands
    // for Host::pump. The retained queue token must outlive that early drain.
    let turn = fixture.host.coordinator.step(Some(Instant::now())).unwrap();
    assert!(turn.commands);
    assert_eq!(turn.events, 0);
    wake.wake().unwrap();
    fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(fixture.host.events(), [WindowEvent::Wake]);
    fixture.host.coordinator.step(Some(Instant::now())).unwrap();
    wake.wake().unwrap();
    fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(fixture.host.events(), [WindowEvent::Wake]);
}

#[test]
fn wake_counts_toward_the_64_command_batch_without_a_fake_window_outcome() {
    let mut fixture = Fixture::new();
    let id = fixture.create(1, 1);
    fixture.peer.take_requests();
    fixture.host.wake().wake().unwrap();
    let proxy = fixture.host.proxy();
    for value in 0..64 {
        proxy
            .try_send(WindowCommand::SetTitle(id, value.to_string()))
            .unwrap();
    }
    let first = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(first.commands.len(), 63);
    assert!(first.runnable);
    assert!(
        first
            .commands
            .iter()
            .all(|outcome| outcome.window == id && outcome.result.is_ok())
    );
    assert_eq!(fixture.host.events(), [WindowEvent::Wake]);
    let second = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(second.commands.len(), 1);
    assert_eq!(second.commands[0].window, id);
    assert!(!second.runnable);
    let expected: Vec<_> = (0..64)
        .map(|value| Request::Title(0, value.to_string()))
        .collect();
    assert_eq!(fixture.peer.requests(), expected);
}

#[test]
fn full_admission_resets_coalescing_and_queued_commands_provide_the_service_turn() {
    let mut fixture = Fixture::new();
    let id = fixture.create(1, 1);
    let proxy = fixture.host.proxy();
    for value in 0..256 {
        proxy
            .try_send(WindowCommand::SetTitle(id, value.to_string()))
            .unwrap();
    }
    let wake = fixture.host.wake();
    let (published, results) = channel();
    published.send(73).unwrap();
    assert!(matches!(wake.wake(), Err(Error::LimitExceeded)));
    assert!(matches!(wake.wake(), Err(Error::LimitExceeded)));
    let first = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(first.commands.len(), 64);
    assert!(first.runnable);
    assert!(fixture.host.events().is_empty());
    // Mandatory servicing after this ordinary command turn sees the published
    // result even though capacity refused its additional Wake command.
    assert_eq!(results.try_recv().unwrap(), 73);
    assert!(fixture.host.failure().is_none());
    wake.wake().unwrap();
    for _ in 0..3 {
        let turn = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
        assert_eq!(turn.commands.len(), 64);
        assert!(turn.runnable);
        assert!(fixture.host.events().is_empty());
    }
    let wake_turn = fixture.host.pump(Some(Instant::now() + WATCHDOG)).unwrap();
    assert!(wake_turn.commands.is_empty());
    assert_eq!(fixture.host.events(), [WindowEvent::Wake]);
}

#[test]
fn temporary_wake_upgrade_and_weak_clones_do_not_retain_native_ownership_on_drop() {
    let mut fixture = Fixture::new();
    fixture.create(1, 1);
    fixture.peer.take_requests();
    let wake = fixture.host.wake();
    let clone = wake.clone();
    let retained = wake.retain_state().unwrap();
    let (holding, held) = channel();
    let (release, released) = channel();
    let worker = thread::spawn(move || {
        holding.send(()).unwrap();
        released.recv_timeout(WATCHDOG).unwrap();
        drop(retained);
    });
    held.recv_timeout(WATCHDOG).unwrap();
    let Fixture { host, peer } = fixture;
    drop(host);
    assert_eq!(
        peer.take_requests(),
        [
            Request::DestroyToplevel(0),
            Request::DestroyRole(0),
            Request::DestroySurface(0),
            Request::DestroyShell,
        ]
    );
    // The upgraded state retains only an already-closed mailbox sender.
    wake.wake().unwrap();
    clone.wake().unwrap();
    release.send(()).unwrap();
    worker.join().unwrap();
    assert!(wake.retain_state().is_none());
    wake.wake().unwrap();
    clone.wake().unwrap();
}

#[test]
fn admitted_wake_failure_is_terminal_and_keeps_the_first_cause() {
    let mut fixture = Fixture::new();
    let id = fixture.create(1, 1);
    let lease = fixture.host.lease(id).unwrap();
    let wake = fixture.host.wake();
    wake.fail_next_wake(libc::EIO);
    let Error::Wayland(cause) = wake.wake().unwrap_err() else {
        panic!("wake failure must be terminal");
    };
    assert!(cause.starts_with("mailbox wake write: "));
    assert!(matches!(wake.wake(), Err(Error::Wayland(later)) if later == cause));
    let failure = fixture
        .host
        .pump(Some(Instant::now() + WATCHDOG))
        .unwrap_err();
    assert!(matches!(failure.error, Error::Wayland(actual) if actual == cause));
    assert!(failure.commands.is_empty());
    assert!(lease.is_retired());
    let first = fixture.host.failure().unwrap().to_owned();
    assert!(first.contains(&cause));
    assert!(matches!(
        fixture.host.pump(Some(Instant::now())).unwrap_err().error,
        Error::SessionStopped
    ));
    assert_eq!(fixture.host.failure(), Some(first.as_str()));
}

#[test]
fn inability_to_retain_an_admitted_wake_event_terminalizes_the_host() {
    let mut fixture = Fixture::new();
    fixture
        .host
        .coordinator
        .state
        .events
        .extend(std::iter::repeat_n(
            WindowEvent::Ready,
            super::super::state::MAX_EVENTS,
        ));
    fixture.host.wake().wake().unwrap();
    let failure = fixture.host.pump(Some(Instant::now())).unwrap_err();
    assert!(failure.commands.is_empty());
    assert!(fixture.host.failure().is_some());
    assert_eq!(fixture.host.events().len(), super::super::state::MAX_EVENTS);
}
