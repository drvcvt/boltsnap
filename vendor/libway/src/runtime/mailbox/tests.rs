//! Deterministic mailbox ownership, capacity and durable wake regression cases.
use super::*;
use std::{
    os::fd::AsRawFd,
    sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
};

fn mailbox<T>(max_queued: usize, max_bytes: usize) -> Mailbox<T> {
    Mailbox::new(Limits {
        max_queued,
        max_bytes,
    })
    .unwrap()
}

fn readable<T>(mailbox: &Mailbox<T>) -> bool {
    let mut interest = libc::pollfd {
        fd: mailbox.wake_fd().as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: interest is initialized and writable for one pollfd; the borrowed
    // wake descriptor remains live. Zero timeout makes this an observation only.
    let result = unsafe { libc::poll(&mut interest, 1, 0) };
    assert!(result >= 0);
    interest.revents & libc::POLLIN != 0
}

#[test]
fn limits_are_validated_before_wake_creation() {
    for (max_queued, max_bytes) in [(0, 1), (1, 0), (0, 0), (usize::MAX, 1)] {
        assert!(
            Mailbox::<u64>::new(Limits {
                max_queued,
                max_bytes
            })
            .is_err()
        );
    }
}

#[test]
fn queued_count_at_limit_minus_one_limit_and_one_beyond() {
    let mut mailbox = mailbox(256, 8 * 1024 * 1024);
    let sender = mailbox.sender();
    for value in 0..255 {
        sender.send(value, 0).unwrap();
    }
    assert_eq!(mailbox.shared.lock().queue.len(), 255);
    sender.send(255, 0).unwrap();
    assert_eq!(mailbox.shared.lock().queue.len(), 256);
    assert!(matches!(sender.send(256, 0), Err(SendError::Full(256))));
    for expected in 0..256 {
        mailbox
            .pop()
            .unwrap()
            .unwrap()
            .apply(|value| assert_eq!(value, expected));
    }
    assert!(!mailbox.has_pending().unwrap());
}

#[test]
fn bytes_at_limit_minus_one_limit_and_one_beyond() {
    let mut mailbox = mailbox(256, 8 * 1024 * 1024);
    let sender = mailbox.sender();
    let limit = mailbox.shared.limits.max_bytes;
    sender.send(1, limit - 1).unwrap();
    assert_eq!(mailbox.shared.lock().retained_bytes, limit - 1);
    sender.send(2, 1).unwrap();
    assert_eq!(mailbox.shared.lock().retained_bytes, limit);
    assert!(matches!(sender.send(3, 1), Err(SendError::Full(3))));
    assert!(matches!(
        sender.send(4, usize::MAX),
        Err(SendError::Full(4))
    ));
    mailbox.pop().unwrap().unwrap().apply(|_| {});
    mailbox.pop().unwrap().unwrap().apply(|_| {});
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
}

#[test]
fn popped_command_keeps_byte_credit_through_application() {
    let mut mailbox = mailbox(1, 8);
    let sender = mailbox.sender();
    sender.send(1, 8).unwrap();
    let received = mailbox.pop().unwrap().unwrap();
    assert!(!mailbox.has_pending().unwrap());
    assert!(matches!(sender.send(2, 1), Err(SendError::Full(2))));
    received.apply(|value| {
        assert_eq!(value, 1);
        assert!(matches!(sender.send(3, 1), Err(SendError::Full(3))));
        // Queue count applies only to queued commands, independently of bytes.
        sender.send(4, 0).unwrap();
    });
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
    mailbox
        .pop()
        .unwrap()
        .unwrap()
        .apply(|value| assert_eq!(value, 4));
    sender.send(5, 8).unwrap();
}

#[test]
fn credit_is_released_after_callback_unwind() {
    let mut mailbox = mailbox(1, 8);
    mailbox.sender().send(1, 8).unwrap();
    let received = mailbox.pop().unwrap().unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        received.apply(|_| panic!("test callback"));
    }));
    assert!(result.is_err());
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
    assert!(!mailbox.shared.state.is_poisoned());
    mailbox.sender().send(2, 8).unwrap();
}

#[test]
fn wake_is_readable_before_drain_and_unreadable_after() {
    let mut mailbox = mailbox(4, 4);
    let sender = mailbox.sender();
    assert!(!readable(&mailbox));
    sender.send(1, 1).unwrap();
    sender.send(2, 1).unwrap();
    assert!(readable(&mailbox));
    mailbox.drain_wake().unwrap();
    assert!(!readable(&mailbox));
    assert!(mailbox.has_pending().unwrap());
    mailbox.drain_wake().unwrap();
    assert!(!readable(&mailbox));
    mailbox
        .pop()
        .unwrap()
        .unwrap()
        .apply(|value| assert_eq!(value, 1));
    mailbox
        .pop()
        .unwrap()
        .unwrap()
        .apply(|value| assert_eq!(value, 2));
    assert!(mailbox.pop().unwrap().is_none());
}

#[test]
fn producer_after_drain_or_final_empty_check_leaves_a_durable_token() {
    for after_empty_check in [false, true] {
        let mut mailbox = mailbox(1, 1);
        let sender = mailbox.sender();
        let start = Arc::new(Barrier::new(2));
        let admitted = Arc::new(Barrier::new(2));
        let producer_start = start.clone();
        let producer_admitted = admitted.clone();
        let worker = thread::spawn(move || {
            producer_start.wait();
            sender.send(7, 1).unwrap();
            producer_admitted.wait();
        });
        mailbox.drain_wake().unwrap();
        if after_empty_check {
            assert!(!mailbox.has_pending().unwrap());
        }
        start.wait();
        admitted.wait();
        // This is the exact final-check/poll gap, acknowledged without sleeps.
        assert!(readable(&mailbox));
        assert!(mailbox.has_pending().unwrap());
        mailbox.drain_wake().unwrap();
        mailbox
            .pop()
            .unwrap()
            .unwrap()
            .apply(|value| assert_eq!(value, 7));
        assert!(!mailbox.has_pending().unwrap());
        assert!(!readable(&mailbox));
        worker.join().unwrap();
    }
}

#[test]
fn saturated_socket_is_successful_already_readable_notification() {
    let mut mailbox = mailbox(1, 1);
    let mut saturated = false;
    for _ in 0..65_536 {
        match mailbox.shared.wake.try_notify_once() {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                saturated = true;
                break;
            }
            Err(error) => panic!("unexpected saturation error: {error}"),
        }
    }
    assert!(saturated, "fixture must reach an actual full socket");
    mailbox.sender().send(9, 1).unwrap();
    assert!(readable(&mailbox));
    // Saturation may retain more than one finite drain chunk; no producers run.
    for _ in 0..65_536 {
        mailbox.drain_wake().unwrap();
        if !readable(&mailbox) {
            break;
        }
    }
    assert!(!readable(&mailbox));
    mailbox
        .pop()
        .unwrap()
        .unwrap()
        .apply(|value| assert_eq!(value, 9));
}

#[test]
fn admitted_wake_failure_retains_command_and_first_cause() {
    let mut mailbox = mailbox(2, 2);
    let sender = mailbox.sender();
    mailbox.drain_wake().unwrap();
    assert!(!mailbox.has_pending().unwrap());
    let before = mailbox.wake_fd().as_raw_fd();
    mailbox.shared.wake.fail_next_write(libc::EIO);
    let Err(SendError::AdmittedWakeFailed(first)) = sender.send(1, 1) else {
        panic!("expected admitted wake failure");
    };
    // This same live read fd wakes even when failure follows the empty check.
    assert_eq!(mailbox.wake_fd().as_raw_fd(), before);
    assert!(readable(&mailbox));
    assert_eq!(mailbox.shared.lock().queue.len(), 1);
    assert_eq!(mailbox.shared.lock().retained_bytes, 1);
    assert!(matches!(sender.send(2, 1), Err(SendError::Failed(2, cause)) if cause == first));
    assert!(matches!(mailbox.pop(), Err(cause) if cause == first));
    assert!(matches!(mailbox.has_pending(), Err(cause) if cause == first));
    mailbox
        .shared
        .fail(Failure::new("later", "does not replace first"));
    assert_eq!(mailbox.failure(), Some(first));
    mailbox.close();
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
}

#[test]
fn wake_read_error_is_terminal_without_consuming_queued_work() {
    let mut mailbox = mailbox(1, 1);
    mailbox.sender().send(1, 1).unwrap();
    mailbox.shared.wake.fail_next_read(libc::EIO);
    let cause = mailbox.drain_wake().unwrap_err();
    assert!(matches!(mailbox.pop(), Err(first) if first == cause));
    assert!(readable(&mailbox));
    assert_eq!(mailbox.shared.lock().queue.len(), 1);
}

#[test]
fn interrupted_wake_operations_retry_without_losing_admission() {
    let mut mailbox = mailbox(1, 1);
    mailbox.shared.wake.fail_next_write(libc::EINTR);
    mailbox.sender().send(1, 1).unwrap();
    assert!(readable(&mailbox));
    mailbox.shared.wake.fail_next_read(libc::EINTR);
    mailbox.drain_wake().unwrap();
    assert!(!readable(&mailbox));
    mailbox
        .pop()
        .unwrap()
        .unwrap()
        .apply(|value| assert_eq!(value, 1));
    assert!(mailbox.failure().is_none());
}

#[test]
fn exhausted_interrupted_writes_leave_durable_failure_readability() {
    let mut mailbox = mailbox(1, 1);
    mailbox.drain_wake().unwrap();
    assert!(!mailbox.has_pending().unwrap());
    mailbox.shared.wake.fail_writes(libc::EINTR, 5);
    let Err(SendError::AdmittedWakeFailed(cause)) = mailbox.sender().send(1, 1) else {
        panic!("expected exhausted interrupted wake failure");
    };
    assert_eq!(mailbox.shared.wake.remaining_write_failures(), 1);
    assert!(readable(&mailbox));
    assert!(matches!(mailbox.pop(), Err(first) if first == cause));
    assert_eq!(mailbox.shared.lock().queue.len(), 1);
    assert_eq!(mailbox.shared.lock().retained_bytes, 1);
}

#[test]
fn terminal_write_shutdown_is_observed_as_eof() {
    let wake = Wake::new().unwrap();
    wake.fail_next_write(libc::EIO);
    assert_eq!(wake.notify().unwrap_err().raw_os_error(), Some(libc::EIO));
    assert_eq!(
        wake.drain().unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
    // EOF is persistent, rather than a consumable one-shot error indication.
    assert_eq!(
        wake.drain().unwrap_err().kind(),
        std::io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn first_cause_has_a_bounded_utf8_representation() {
    for (context, expected_bytes) in [("wake", 512), ("wakes", 511)] {
        let cause = Failure::new(context, "ä".repeat(1000));
        assert_eq!(cause.as_str().len(), expected_bytes);
        assert!(cause.as_str().starts_with(context));
        assert!(cause.as_str().is_char_boundary(cause.as_str().len()));
    }
}

struct Payload {
    sender: Option<Sender<Self>>,
    drops: Arc<AtomicUsize>,
}

impl Drop for Payload {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Relaxed);
        if let Some(sender) = &self.sender {
            let probe = Self {
                sender: None,
                drops: self.drops.clone(),
            };
            assert!(matches!(sender.send(probe, 0), Err(SendError::Closed(_))));
        }
    }
}

#[test]
fn close_destroys_payloads_outside_lock_and_is_idempotent() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut mailbox = mailbox(2, 2);
    let sender = mailbox.sender();
    let payload = Payload {
        sender: Some(sender.clone()),
        drops: drops.clone(),
    };
    assert!(sender.send(payload, 1).is_ok());
    mailbox.close();
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
    mailbox.close();
    drop(mailbox);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert!(matches!(
        sender.send(
            Payload {
                sender: None,
                drops: drops.clone()
            },
            0
        ),
        Err(SendError::Closed(_))
    ));
    assert_eq!(drops.load(Ordering::Relaxed), 3);
}

#[test]
fn owner_drop_retires_queued_work_but_received_credit_remains_live() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut mailbox = mailbox(2, 2);
    let sender = mailbox.sender();
    for _ in 0..2 {
        assert!(
            sender
                .send(
                    Payload {
                        sender: None,
                        drops: drops.clone()
                    },
                    1
                )
                .is_ok()
        );
    }
    let received = mailbox.pop().unwrap().unwrap();
    drop(mailbox);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    assert_eq!(sender.shared.lock().retained_bytes, 1);
    drop(received);
    assert_eq!(drops.load(Ordering::Relaxed), 2);
    assert_eq!(sender.shared.lock().retained_bytes, 0);
}

#[test]
fn weak_notifier_wakes_without_command_admission() {
    let mut mailbox = mailbox::<u64>(1, 1);
    let notifier = mailbox.notifier();
    assert!(!readable(&mailbox));
    notifier.clone().notify().unwrap();
    assert!(readable(&mailbox));
    assert!(!mailbox.has_pending().unwrap());
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
    // Notification remains available even when every command credit is used.
    mailbox.sender().send(1, 1).unwrap();
    mailbox.drain_wake().unwrap();
    notifier.notify().unwrap();
    assert!(readable(&mailbox));
    assert_eq!(mailbox.shared.lock().queue.len(), 1);
    assert_eq!(mailbox.shared.lock().retained_bytes, 1);
}

#[test]
fn weak_notifier_cannot_retain_receiver_or_queued_payloads() {
    struct Native {
        notifier: Notifier<Self>,
        drops: Arc<AtomicUsize>,
    }
    impl Drop for Native {
        fn drop(&mut self) {
            // The queued payload owns its finalizer wake. Closing moves this
            // destructor outside the lock and the weak handle cannot cycle.
            self.notifier.notify().unwrap();
            self.drops.fetch_add(1, Ordering::Relaxed);
        }
    }
    let drops = Arc::new(AtomicUsize::new(0));
    let mailbox = mailbox(1, 1);
    let notifier = mailbox.notifier();
    let sender = mailbox.sender();
    assert!(
        sender
            .send(
                Native {
                    notifier: mailbox.notifier(),
                    drops: drops.clone()
                },
                1
            )
            .is_ok()
    );
    drop(mailbox);
    assert_eq!(drops.load(Ordering::Relaxed), 1);
    // The remaining sender keeps Shared live, but owner shutdown is a no-op.
    notifier.notify().unwrap();
    drop(sender);
    assert!(notifier.shared.upgrade().is_none());
    notifier.clone().notify().unwrap();
    assert_eq!(drops.load(Ordering::Relaxed), 1);
}

#[test]
fn weak_notifier_failure_latches_first_cause_without_admission() {
    let mut mailbox = mailbox::<u64>(1, 1);
    let notifier = mailbox.notifier();
    mailbox.shared.wake.fail_next_write(libc::EIO);
    let cause = notifier.notify().unwrap_err();
    assert_eq!(mailbox.failure(), Some(cause.clone()));
    assert!(matches!(notifier.notify(), Err(first) if first == cause));
    assert_eq!(mailbox.shared.lock().queue.len(), 0);
    assert_eq!(mailbox.shared.lock().retained_bytes, 0);
    assert!(matches!(mailbox.pop(), Err(first) if first == cause));
    mailbox.close();
    // Closed overrides failure for this best-effort teardown-only handle.
    notifier.notify().unwrap();
}
