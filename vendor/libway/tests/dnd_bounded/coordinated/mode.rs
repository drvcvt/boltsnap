//! Bounded admission keeps the active drag intact; terminal causes forbid sleep.
//! This module uses only the owner API and never starts a legacy reader or run.
use super::*;
use libway::{Error, dnd::OutgoingLimits};
use std::{
    io::Read,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
fn empty_and_retained_event_views_distinguish_runnable_work() {
    let mut fixture = Fixture::new(DndLimits::default());
    let connection_fd = fixture.display.connection().backend().poll_fd().as_raw_fd();
    let view = fixture.dnd.wait_view().unwrap();
    assert!(!view.immediate());
    assert!(view.deadline().is_none());
    let interest = view.interests().next().unwrap();
    assert_eq!(interest.fd.as_raw_fd(), connection_fd);
    assert!(interest.readable && !interest.writable);
    assert_eq!(view.interests().len(), 1);
    drop(view);
    let request = fixture.request(DragData::Text("queued".into()));
    let drag = fixture
        .dnd
        .start_drag(request, OutgoingLimits::default())
        .unwrap();
    fixture.dnd.cancel_drag(drag).unwrap();
    let report = fixture.dnd.dispatch().unwrap();
    assert_eq!(report.callbacks, 0);
    assert_eq!(report.pending_events, 1);
    let view = fixture.dnd.wait_view().unwrap();
    assert!(view.immediate());
    assert!(view.deadline().is_none());
    drop(view);
    assert!(matches!(
        fixture.dnd.events().as_slice(),
        [DndEvent::DragEnded { .. }]
    ));
    assert!(!fixture.dnd.wait_view().unwrap().immediate());
    fixture.barrier();
}

#[test]
fn lazy_and_oversize_refusals_preserve_the_admitted_eager_drag() {
    let mut fixture = Fixture::new(DndLimits::default());
    let limits = OutgoingLimits {
        max_payload: 8,
        max_eager_bytes: 32,
    };
    let request = fixture.request(DragData::Static(vec![("a/b".into(), b"eager".to_vec())]));
    let drag = fixture.dnd.start_drag(request, limits).unwrap();
    fixture.barrier();
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let request = fixture.request(DragData::Lazy {
        mimes: vec!["a/b".into()],
        provide: Box::new(move |_| {
            counter.fetch_add(1, Ordering::SeqCst);
            Some(b"lazy".to_vec())
        }),
    });
    assert!(matches!(
        fixture.dnd.start_drag(request, limits),
        Err(Error::Unsupported(_))
    ));
    let request = fixture.request(DragData::Static(vec![("a/b".into(), vec![0; 9])]));
    assert!(matches!(
        fixture.dnd.start_drag(request, limits),
        Err(Error::LimitExceeded)
    ));
    assert!(fixture.dnd.failure().is_none());
    fixture.barrier();
    assert_single_source(&fixture);
    fixture.server.dnd().request_send("a/b");
    fixture.barrier();
    let PipePeer::Outgoing { mime, read } = fixture.peers.recv_timeout(WATCHDOG).unwrap() else {
        panic!("expected the outgoing reader");
    };
    assert_eq!(mime, "a/b");
    let view = fixture.dnd.wait_view().unwrap();
    assert_eq!(
        view.interests().len(),
        1,
        "small eager send must have reached EOF"
    );
    drop(view);
    let mut bytes = Vec::new();
    File::from(read).read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"eager");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    fixture.dnd.cancel_drag(drag).unwrap();
    fixture.barrier();
}

fn assert_single_source(fixture: &Fixture) {
    assert_eq!(
        fixture
            .server
            .metrics
            .dnd
            .start_drags
            .lock()
            .unwrap()
            .as_slice(),
        &[(1, false)]
    );
    assert_eq!(
        fixture
            .server
            .metrics
            .dnd
            .sources_alive
            .load(Ordering::SeqCst),
        1
    );
}

#[test]
fn terminal_disconnect_retains_its_cause_and_refuses_wait() {
    let Fixture {
        server,
        display,
        mut dnd,
        ..
    } = Fixture::new(DndLimits::default());
    drop(server);
    let guard = display.connection().prepare_read().unwrap();
    let view = dnd.wait_view().unwrap();
    let ready = poll_view(&view, Instant::now() + WATCHDOG);
    assert_ne!(
        ready[0].revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR),
        0
    );
    drop(view);
    assert!(guard.read().is_err());
    assert!(dnd.wait_view().is_err());
    let first = dnd.failure().unwrap().to_owned();
    assert!(!first.is_empty());
    assert!(matches!(dnd.dispatch(), Err(Error::SessionStopped)));
    dnd.events();
    assert!(dnd.wait_view().is_err());
    assert_eq!(dnd.failure(), Some(first.as_str()));
}

#[test]
fn event_overflow_terminalizes_the_owner_before_another_wait() {
    let mut fixture = Fixture::new(DndLimits {
        max_events: 1,
        ..Default::default()
    });
    fixture.target();
    fixture.barrier();
    fixture.server.dnd().offer(&["a/b"], 1, &[]);
    fixture.server.dnd().enter(0, 1., 1.);
    fixture.server.dnd().leave();
    fixture.read_barrier();
    assert!(matches!(fixture.dnd.dispatch(), Err(Error::LimitExceeded)));
    let first = fixture.dnd.failure().unwrap().to_owned();
    assert!(fixture.dnd.wait_view().is_err());
    fixture.dnd.events();
    assert!(matches!(fixture.dnd.dispatch(), Err(Error::SessionStopped)));
    assert!(fixture.dnd.wait_view().is_err());
    assert_eq!(fixture.dnd.failure(), Some(first.as_str()));
}
