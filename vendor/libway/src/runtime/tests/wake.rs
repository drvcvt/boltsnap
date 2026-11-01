//! Explicit phase barriers cover drain/prepare/final-check races without sleeps.
use super::*;
use crate::{
    Error,
    runtime::mailbox::{Failure, SendError},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc::channel,
    },
    thread,
};

#[test]
fn commands_at_each_wake_boundary_are_retained_and_applied_exactly_once() {
    let cases = [
        (Phase::BeforeDrain, 0, 0),
        (Phase::AfterDrain, 0, 0),
        (Phase::BeforePrepare, 1, 0),
        (Phase::AfterPrepare, 1, 0),
        (Phase::BeforePoll, 1, 1),
    ];
    for (index, (target, prepared, polls)) in cases.into_iter().enumerate() {
        let (_server, mut pump) = fixture();
        let sender = pump.sender();
        let admissions = Arc::new(AtomicUsize::new(0));
        let observed = admissions.clone();
        let value = index as u32 + 1;
        pump.trace.hook = Some(Box::new(move |phase| {
            if phase == target && observed.load(Ordering::SeqCst) == 0 {
                sender.send(value, 1).unwrap();
                observed.fetch_add(1, Ordering::SeqCst);
            }
        }));
        let turn = pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
        assert_eq!(admissions.load(Ordering::SeqCst), 1, "{target:?}");
        assert!(turn.commands, "step must retain commands for the host");
        assert_eq!(turn.callbacks, 0);
        assert_resolved(&pump.trace, prepared, 0, prepared, polls);
        assert_eq!(turn.wait.wake, prepared != 0);
        let mut applied = Vec::new();
        pump.command().unwrap().unwrap().apply(|command| {
            assert!(!pump.trace.reserved);
            applied.push(command);
        });
        assert_eq!(applied, [value]);
        assert!(pump.command().unwrap().is_none());
        pump.trace = Trace::default();
        let idle = pump.step(Some(Instant::now())).unwrap();
        assert!(!idle.commands && idle.wait.timed_out);
        assert_resolved(&pump.trace, 1, 0, 1, 1);
        assert!(pump.command().unwrap().is_none());
    }
}

#[test]
fn acknowledged_worker_in_the_final_check_poll_gap_leaves_a_durable_wake() {
    let (_server, mut pump) = fixture();
    let sender = pump.sender();
    let (start, ready) = channel();
    let (admitted, ack) = channel();
    let worker = thread::spawn(move || {
        ready.recv_timeout(WATCHDOG).unwrap();
        sender.send(71, 1).unwrap();
        admitted.send(()).unwrap();
    });
    let mut start = Some(start);
    pump.trace.hook = Some(Box::new(move |phase| {
        if phase == Phase::BeforePoll {
            start.take().unwrap().send(()).unwrap();
            ack.recv_timeout(WATCHDOG).unwrap();
        }
    }));
    let turn = pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
    worker.join().unwrap();
    assert!(turn.commands && turn.wait.wake);
    assert!(!turn.wait.read && !turn.wait.timed_out);
    assert_resolved(&pump.trace, 1, 0, 1, 1);
    pump.trace.hook = None;
    pump.command().unwrap().unwrap().apply(|command| {
        assert!(!pump.trace.reserved);
        assert_eq!(command, 71);
    });
    assert!(pump.command().unwrap().is_none());
}

#[test]
fn admitted_wake_failure_breaks_an_indefinite_poll_without_a_wayland_read() {
    let (server, mut pump) = fixture();
    let sender = pump.sender();
    let hook_sender = sender.clone();
    let wake_fd = pump.mailbox.wake_fd().as_raw_fd();
    let (admitted, admission) = channel();
    pump.trace.hook = Some(Box::new(move |phase| {
        if phase == Phase::BeforePoll {
            hook_sender.fail_next_wake(libc::EIO);
            let Err(SendError::AdmittedWakeFailed(cause)) = hook_sender.send(73, 1) else {
                panic!("fault must retain the admitted command and report wake failure");
            };
            admitted.send(cause).unwrap();
        }
    }));
    let (finished, finish_ack) = channel();
    let watchdog = thread::spawn(move || {
        if finish_ack.recv_timeout(WATCHDOG).is_ok() {
            // Return ownership so the server survives every result observation.
            (true, Some(server))
        } else {
            drop(server);
            (false, None)
        }
    });
    let result = pump.step(None);
    let _ = finished.send(());
    let (finished_in_time, _server) = watchdog.join().unwrap();
    assert!(
        finished_in_time,
        "watchdog had to close the fixture to break poll"
    );
    let cause = admission.try_recv().unwrap();
    assert!(cause.as_str().starts_with("mailbox wake write: "));
    let error = result.unwrap_err();
    assert!(matches!(&error, Error::Wayland(actual) if actual == cause.as_str()));
    assert_resolved(&pump.trace, 1, 0, 1, 1);
    assert_eq!(pump.mailbox.wake_fd().as_raw_fd(), wake_fd);
    assert_eq!(pump.mailbox.failure().as_ref(), Some(&cause));
    let first = pump.failure().unwrap().to_owned();
    assert_eq!(first, Failure::new("coordinator", &error).as_str());
    assert!(matches!(pump.step(None), Err(Error::SessionStopped)));
    assert_eq!(pump.failure(), Some(first.as_str()));
    // Failure takes precedence over closed rejection, retaining the unsent value.
    assert!(matches!(sender.send(74, 1), Err(SendError::Failed(74, later)) if later == cause));
    drop(pump);
    assert!(matches!(sender.send(75, 1), Err(SendError::Failed(75, later)) if later == cause));
}
