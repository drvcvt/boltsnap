//! Pipe readiness and deadlines are independent of protocol callback counts.
//! Each test resolves its caller-owned read reservation before servicing DnD.
use super::*;
use crate::support::pipe_fds;
use libway::{
    TransferError,
    dnd::{Outcome, Payload},
};
use std::io::Write;

#[test]
fn pipe_only_completion_has_no_callbacks_or_duplicated_library_fds() {
    let mut fixture = Fixture::new(DndLimits::default());
    let (_, mut writer, inode) = fixture.incoming();
    assert_eq!(pipe_fds(inode), 2);
    let guard = fixture.display.connection().prepare_read().unwrap();
    let view = fixture.dnd.wait_view().unwrap();
    assert!(!view.immediate());
    assert!(view.deadline().is_some());
    assert_eq!(view.interests().len(), 2);
    let interests: Vec<_> = view.interests().collect();
    assert!(interests[1].readable && !interests[1].writable);
    assert_eq!(pipe_fds(inode), 2);
    assert!(
        poll_view(&view, Instant::now())
            .iter()
            .all(|fd| fd.revents == 0)
    );
    writer.write_all(b"owned pipe bytes").unwrap();
    drop(writer);
    let ready = poll_view(&view, Instant::now() + WATCHDOG);
    assert_eq!(
        ready[0].revents, 0,
        "completion must not require socket traffic"
    );
    assert_ne!(ready[1].revents & (libc::POLLIN | libc::POLLHUP), 0);
    assert_eq!(pipe_fds(inode), 1);
    drop(interests);
    drop(guard);
    drop(view);
    let report = fixture.dnd.dispatch().unwrap();
    assert_eq!(report.callbacks, 0);
    assert_eq!(report.pending_events, 1);
    assert_eq!(pipe_fds(inode), 0);
    let events = fixture.dnd.events();
    let [
        DndEvent::Dropped {
            transfer, payload, ..
        },
    ] = events.as_slice()
    else {
        panic!("expected exactly one completed payload: {events:?}");
    };
    assert_eq!(
        payload,
        &Payload::Bytes {
            mime: "a/b".into(),
            data: b"owned pipe bytes".to_vec()
        }
    );
    let view = fixture.dnd.wait_view().unwrap();
    assert_eq!(view.interests().len(), 1);
    assert!(view.deadline().is_none());
    drop(view);
    fixture
        .dnd
        .complete(*transfer, Outcome::Accepted(Action::Copy))
        .unwrap();
    fixture.barrier();
}

#[test]
fn an_open_empty_pipe_expires_on_inactivity_and_total_deadlines_only() {
    for inactivity_first in [true, false] {
        // Longer than the post-admission wire-barrier watchdog, so expiry is
        // observed by poll rather than used as a setup-speed pass threshold.
        let short = Duration::from_secs(3);
        let long = Duration::from_secs(30);
        let mut fixture = Fixture::new(DndLimits {
            inactivity: if inactivity_first { short } else { long },
            total: if inactivity_first { long } else { short },
            ..Default::default()
        });
        let (_, writer, inode) = fixture.incoming();
        let guard = fixture.display.connection().prepare_read().unwrap();
        let view = fixture.dnd.wait_view().unwrap();
        let deadline = view.deadline().unwrap();
        assert!(!view.immediate());
        assert!(
            deadline > Instant::now(),
            "fixture exhausted the transfer deadline"
        );
        assert!(poll_view(&view, deadline).iter().all(|fd| fd.revents == 0));
        assert!(Instant::now() >= deadline);
        drop(guard);
        drop(view);
        let report = fixture.dnd.dispatch().unwrap();
        assert_eq!(report.callbacks, 0);
        assert_eq!(report.pending_events, 1);
        assert!(matches!(
            fixture.dnd.events().as_slice(),
            [DndEvent::TransferFailed {
                reason: TransferError::Timeout,
                ..
            }]
        ));
        assert_eq!(pipe_fds(inode), 1, "only the controlled writer remains");
        let view = fixture.dnd.wait_view().unwrap();
        assert!(view.deadline().is_none());
        assert_eq!(view.interests().len(), 1);
        drop(view);
        drop(writer);
        assert_eq!(pipe_fds(inode), 0);
        fixture.barrier();
    }
}

#[test]
fn cancellation_removes_pipe_interests_without_closing_the_peer() {
    let mut fixture = Fixture::new(DndLimits::default());
    let (target, writer, inode) = fixture.incoming();
    let view = fixture.dnd.wait_view().unwrap();
    assert_eq!(view.interests().len(), 2);
    assert_eq!(pipe_fds(inode), 2);
    drop(view);
    fixture.dnd.unregister_target(target).unwrap();
    let view = fixture.dnd.wait_view().unwrap();
    assert_eq!(view.interests().len(), 1);
    assert!(view.deadline().is_none());
    assert_eq!(pipe_fds(inode), 1);
    drop(view);
    fixture.dnd.events();
    drop(writer);
    assert_eq!(pipe_fds(inode), 0);
    fixture.barrier();
}
