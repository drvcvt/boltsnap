//! Public admission, source outcomes and dispatch observations on both check backends.
#![cfg(feature = "dnd")]
mod coordinated;
#[path = "../support/mod.rs"]
mod support;
use libway::{
    Display, Error, SurfaceHandle,
    dnd::{
        Actions, Dnd, DndEvent, DndLimits, DndOptions, DragData, DragOutcome, DragRequest,
        OutgoingLimits,
    },
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use support::{
    Config, DndConfig, Server,
    client::{surface, wait_for},
};

fn setup(version: u32, transfers: usize) -> (Server, Display, Dnd, SurfaceHandle) {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig {
            version,
            ..Default::default()
        }),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(
        &display,
        DndOptions {
            limits: DndLimits {
                max_transfers: transfers,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let origin = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.events();
    (server, display, dnd, origin)
}
fn request(origin: &SurfaceHandle, data: DragData) -> DragRequest {
    DragRequest {
        seat: None,
        origin: origin.clone(),
        serial: Some(1),
        data,
        actions: Actions {
            copy: true,
            move_: false,
            ask: false,
        },
        icon: None,
    }
}
#[test]
fn refused_bounded_input_keeps_the_existing_drag_and_legacy_policy() {
    let (_server, _display, dnd, origin) = setup(3, 4);
    let old = dnd
        .start_drag(request(&origin, DragData::Text("old".into())))
        .unwrap();
    let large = || DragData::Text("x".repeat((1 << 20) + 1));
    assert!(matches!(
        dnd.start_drag_bounded(request(&origin, large()), OutgoingLimits::default()),
        Err(Error::LimitExceeded)
    ));
    dnd.cancel_drag(old).unwrap();
    let legacy = dnd.start_drag(request(&origin, large())).unwrap();
    dnd.cancel_drag(legacy).unwrap();
}
#[test]
fn four_exact_boundary_sends_survive_source_cancel_and_release_their_owners() {
    let (server, _display, dnd, origin) = setup(3, 4);
    let drag = dnd
        .start_drag_bounded(
            request(&origin, DragData::Text("x".repeat(1 << 20))),
            OutgoingLimits::default(),
        )
        .unwrap();
    server.settle();
    for mime in [
        "UTF8_STRING",
        "TEXT",
        "text/plain",
        "text/plain;charset=UTF-8",
    ] {
        server.dnd().request_send(mime);
    }
    server.settle();
    dnd.run(Duration::from_millis(10)).unwrap();
    dnd.cancel_drag(drag).unwrap();
    let end = Instant::now() + Duration::from_secs(3);
    while server.metrics.dnd.received.lock().unwrap().len() < 4 {
        assert!(Instant::now() < end);
        dnd.run(Duration::from_millis(10)).unwrap();
    }
    assert!(
        server
            .metrics
            .dnd
            .received
            .lock()
            .unwrap()
            .iter()
            .all(|(_, bytes)| bytes.len() == 1 << 20 && bytes.iter().all(|&byte| byte == b'x'))
    );
    server.settle();
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 0);
}
#[test]
fn refused_sends_end_as_cancelled_and_capacity_is_checked_before_lazy() {
    for version in [1, 3] {
        for transfers in [0, 4] {
            let (server, _display, dnd, origin) = setup(version, transfers);
            let calls = Arc::new(AtomicUsize::new(0));
            let counter = calls.clone();
            let data = DragData::Lazy {
                mimes: vec!["a/b".into()],
                provide: Box::new(move |_| {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Some(vec![0; 9])
                }),
            };
            dnd.start_drag_bounded(
                request(&origin, data),
                OutgoingLimits {
                    max_payload: 8,
                    max_eager_bytes: 32,
                },
            )
            .unwrap();
            server.settle();
            server.dnd().request_send("a/b");
            if version == 1 {
                server.dnd().cancel_source();
            } else {
                server.dnd().finished();
            }
            let events = wait_for(&dnd, |event| matches!(event, DndEvent::DragEnded { .. }));
            assert!(events.iter().any(|event| matches!(
                event,
                DndEvent::DragEnded {
                    outcome: DragOutcome::Cancelled,
                    ..
                }
            )));
            assert_eq!(calls.load(Ordering::Relaxed), usize::from(transfers > 0));
        }
    }
}

#[test]
fn dispatch_report_separates_pre_read_callbacks_from_undrained_events() {
    use support::client::{Client, roundtrip_beside_reader};
    let (_server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    let mut queue = display.connection().new_event_queue::<Client>();
    let mut client = Client::default();
    // This caller reads the socket and dispatches only its own sync callback.
    roundtrip_beside_reader(display.connection(), &mut queue, &mut client);
    assert!(dnd.dispatch_report().unwrap().callbacks > 0);
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    let queued = dnd.dispatch_report().unwrap();
    assert_eq!(queued.callbacks, 0);
    assert!(queued.pending_events > 0);
    assert!(!queued.transfers_ready);
    assert!(!queued.write_pending);
    // The legacy method must not consume app events either.
    dnd.dispatch().unwrap();
    let events = dnd.events();
    assert_eq!(queued.pending_events, events.len());
    assert!(
        events
            .iter()
            .any(|event| matches!(event, DndEvent::Ready { .. }))
    );
    let idle = dnd.dispatch_report().unwrap();
    assert_eq!(idle.callbacks, 0);
    assert_eq!(idle.pending_events, 0);
    assert!(!idle.transfers_ready);
    assert!(!idle.write_pending);
}
