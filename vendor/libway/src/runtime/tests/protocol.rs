//! Queue callbacks, retained events and terminal transport exits have separate counts.
use super::*;
use crate::Error;

#[test]
fn already_read_queue_work_is_dispatched_before_sleep_and_retained_without_replay() {
    let (_server, mut pump) = fixture();
    issue_sync(&pump, 17);
    read_barrier(&pump);
    assert!(pump.state.calls.is_empty());
    let delivered = pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(delivered.callbacks, 1);
    assert_eq!(delivered.events, 1);
    assert_eq!(pump.state.calls, [17]);
    assert_eq!(pump.state.events, [17]);
    assert_resolved(&pump.trace, 0, 0, 0, 0);
    assert_eq!((pump.trace.dispatches, pump.trace.flushes), (1, 1));
    let retained = pump.step(Some(Instant::now())).unwrap();
    assert_eq!(retained.callbacks, 0);
    assert_eq!(retained.events, 1);
    assert_eq!(pump.state.calls, [17]);
    assert_resolved(&pump.trace, 0, 0, 0, 0);
    assert_eq!((pump.trace.dispatches, pump.trace.flushes), (2, 2));
    pump.state.events.clear();
    pump.trace = Trace::default();
    let idle = pump.step(Some(Instant::now())).unwrap();
    assert!(!idle.runnable());
    assert!(idle.wait.timed_out);
    assert_resolved(&pump.trace, 1, 0, 1, 1);
}

#[test]
fn socket_only_read_dispatches_its_queue_after_resolving_the_guard() {
    let (_server, mut pump) = fixture();
    issue_sync(&pump, 23);
    let turn = pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
    assert!(turn.wait.read);
    assert!(!turn.wait.pipe && !turn.wait.wake && !turn.wait.timed_out);
    assert_eq!(turn.callbacks, 1);
    assert_eq!(turn.events, 1);
    assert_eq!(pump.state.calls, [23]);
    assert_resolved(&pump.trace, 1, 1, 0, 1);
    assert_eq!((pump.trace.dispatches, pump.trace.flushes), (2, 2));
}

#[test]
fn zero_and_future_deadlines_cancel_exactly_one_read_reservation() {
    for delay in [Duration::ZERO, Duration::from_millis(10)] {
        let (_server, mut pump) = fixture();
        let deadline = Instant::now() + delay;
        let turn = pump.step(Some(deadline)).unwrap();
        assert!(turn.wait.timed_out);
        assert!(!turn.wait.read && !turn.wait.wake && !turn.wait.pipe);
        assert!(!turn.runnable());
        assert!(Instant::now() >= deadline);
        assert_resolved(&pump.trace, 1, 0, 1, 1);
        assert_eq!((pump.trace.dispatches, pump.trace.flushes), (2, 2));
    }
}

#[test]
fn disconnect_resolves_the_read_and_sticky_failure_prevents_another_poll() {
    let (server, mut pump) = fixture();
    let sender = pump.sender();
    drop(server);
    assert!(pump.step(Some(Instant::now() + WATCHDOG)).is_err());
    assert_resolved(&pump.trace, 1, 1, 0, 1);
    let first = pump.failure.as_ref().unwrap().clone();
    let before = (pump.trace.dispatches, pump.trace.flushes);
    assert!(matches!(
        pump.step(Some(Instant::now())),
        Err(Error::SessionStopped)
    ));
    assert!(matches!(pump.command(), Err(Error::SessionStopped)));
    assert!(matches!(
        sender.send(31, 1),
        Err(super::super::mailbox::SendError::Closed(31))
    ));
    assert_eq!(pump.failure.as_ref(), Some(&first));
    assert_eq!((pump.trace.dispatches, pump.trace.flushes), before);
    assert_resolved(&pump.trace, 1, 1, 0, 1);
}

#[test]
fn interrupted_poll_cancels_once_and_leaves_the_connection_usable() {
    let (_server, mut pump) = fixture();
    pump.trace.poll_fault = Some(super::super::trace::PollFault::Interrupted);
    let turn = pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
    assert!(turn.wait.interrupted);
    assert!(!turn.wait.writable && !turn.wait.read);
    assert_resolved(&pump.trace, 1, 0, 1, 1);
    assert!(pump.failure.is_none());
    issue_sync(&pump, 41);
    pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
    assert_eq!(pump.state.calls, [41]);
    assert_resolved(&pump.trace, 2, 1, 1, 2);
}

#[test]
fn injected_invalid_and_fatal_poll_returns_cancel_and_stop_without_closing_borrowed_fds() {
    use super::super::trace::PollFault;
    for fault in [PollFault::Invalid, PollFault::Fatal] {
        let (_server, mut pump) = fixture();
        let sender = pump.sender();
        pump.trace.poll_fault = Some(fault);
        assert!(pump.step(Some(Instant::now() + WATCHDOG)).is_err());
        assert_resolved(&pump.trace, 1, 0, 1, 1);
        let cause = pump.failure.as_ref().unwrap().clone();
        assert!(matches!(pump.step(None), Err(Error::SessionStopped)));
        assert_eq!(pump.failure.as_ref(), Some(&cause));
        assert!(matches!(
            sender.send(4, 1),
            Err(super::super::mailbox::SendError::Closed(4))
        ));
        assert_resolved(&pump.trace, 1, 0, 1, 1);
    }
}
