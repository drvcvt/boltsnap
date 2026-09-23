#![cfg(feature = "dnd")]
mod support;
use libway::{
    Display, SurfaceHandle,
    dnd::{Accept, Action, Actions, Dnd, DndEvent, DndOptions, LocalRect, TargetSpec},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use support::{
    client::{Client, roundtrip_beside_reader, surface},
    *,
};

fn spec() -> TargetSpec {
    TargetSpec {
        rect: LocalRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        },
        accepts: vec![Accept::Files],
        actions: Actions {
            copy: true,
            move_: false,
            ask: false,
        },
        preferred: Action::Copy,
        priority: 0,
        enabled: true,
    }
}
/// Drain after every wake, exactly like a toolkit's user-event handler.
fn recv_until(
    rx: &mpsc::Receiver<()>,
    dnd: &Dnd,
    mut pred: impl FnMut(&DndEvent) -> bool,
) -> Vec<DndEvent> {
    let end = Instant::now() + Duration::from_secs(2);
    let mut seen = Vec::new();
    while Instant::now() < end {
        if rx.recv_timeout(Duration::from_millis(100)).is_err() {
            continue;
        }
        let events = dnd.events();
        let hit = events.iter().any(&mut pred);
        seen.extend(events);
        if hit {
            return seen;
        }
    }
    panic!("timed out; saw {seen:?}");
}

#[test]
fn reader_wakes_once_per_drain_and_ignores_other_queues() {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    let wakes = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel();
    let counter = wakes.clone();
    let reader = dnd
        .spawn_reader(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let _ = tx.send(());
        })
        .unwrap();
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Ready { .. }));
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
    dnd.register_target(&handle, spec()).unwrap();
    let control = server.dnd();
    control.offer(&["text/uri-list"], 1, &[]);
    control.enter(0, 1., 1.);
    for i in 0..300 {
        control.motion(f64::from(i % 90), 2.);
    }
    server.settle();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "one wake per undrained batch"
    );
    let events = dnd.events();
    let hovers = events
        .iter()
        .filter(|e| matches!(e, DndEvent::Hover { .. }));
    assert_eq!(hovers.count(), 1, "{events:?}");
    let last = events.last().unwrap();
    assert!(
        matches!(last, DndEvent::Hover { x, .. } if *x == f64::from(299 % 90)),
        "{last:?}"
    );

    // Events for another Rust event queue on the same connection must not wake the consumer.
    let conn = display.connection().clone();
    let mut queue = conn.new_event_queue::<Client>();
    let _registry = conn.display().get_registry(&queue.handle(), ());
    let mut state = Client::default();
    for _ in 0..20 {
        roundtrip_beside_reader(&conn, &mut queue, &mut state);
    }
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(wakes.load(Ordering::SeqCst), 2);
    assert!(reader.is_alive());
    let stop = Instant::now();
    drop(reader);
    assert!(stop.elapsed() < Duration::from_millis(500));
    control.leave();
    server.settle();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "no wake after the reader stopped"
    );
}

#[test]
fn reader_services_transfers_and_stops_cleanly_during_one() {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig {
            hold_writer_open: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    let (tx, rx) = mpsc::channel();
    let reader = dnd
        .spawn_reader(move || {
            let _ = tx.send(());
        })
        .unwrap();
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Ready { .. }));
    dnd.register_target(&handle, spec()).unwrap();
    let control = server.dnd();
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", b"file:///x\n")]);
    control.enter(0, 1., 1.);
    control.drop();
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Leave { .. }));
    server.settle();
    let pipe = server.metrics.dnd.pipes.lock().unwrap()[0];
    assert_eq!(pipe_fds(pipe), 2, "our read end and the stalled writer");
    drop(reader);
    drop(dnd);
    server.settle();
    assert_eq!(pipe_fds(pipe), 1, "read end closed");
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn reader_times_out_stalled_transfers_without_display_traffic() {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig {
            hold_writer_open: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let options = DndOptions {
        limits: libway::dnd::DndLimits {
            inactivity: Duration::from_millis(150),
            ..Default::default()
        },
        ..Default::default()
    };
    let dnd = Dnd::new(&display, options).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    let (tx, rx) = mpsc::channel();
    let _reader = dnd
        .spawn_reader(move || {
            let _ = tx.send(());
        })
        .unwrap();
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Ready { .. }));
    dnd.register_target(&handle, spec()).unwrap();
    let control = server.dnd();
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", b"file:///x\n")]);
    control.enter(0, 1., 1.);
    control.drop();
    let events = recv_until(&rx, &dnd, |e| matches!(e, DndEvent::TransferFailed { .. }));
    assert!(events.iter().any(|e| matches!(
        e,
        DndEvent::TransferFailed {
            reason: libway::TransferError::Timeout,
            ..
        }
    )));
}

#[cfg(feature = "foreign-display")]
#[test]
fn guest_reader_coexists_with_the_owner_loop() {
    use std::{os::fd::AsRawFd, sync::atomic::AtomicBool};
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let owner = wayland_client::Connection::from_socket(socket).unwrap();
    let ptr = std::ptr::NonNull::new(owner.backend().display_ptr().cast()).unwrap();
    let guest = unsafe { Display::from_raw(ptr) };
    let dnd = Dnd::new(&guest, DndOptions::default()).unwrap();
    assert!(
        dnd.run(Duration::ZERO).is_err(),
        "guests never read themselves"
    );
    let handle = SurfaceHandle::from_surface(&surface(&Display::from_connection(owner.clone())));
    let (tx, rx) = mpsc::channel();
    let reader = dnd
        .spawn_reader(move || {
            let _ = tx.send(());
        })
        .unwrap();
    // The owner keeps reading on its own thread with its own queue, like a toolkit loop.
    let stop = Arc::new(AtomicBool::new(false));
    let owner_loop = {
        let (owner, stop) = (owner.clone(), stop.clone());
        std::thread::spawn(move || {
            let mut queue = owner.new_event_queue::<Client>();
            let _registry = owner.display().get_registry(&queue.handle(), ());
            let mut state = Client::default();
            let fd = owner.backend().poll_fd().as_raw_fd();
            while !stop.load(Ordering::SeqCst) {
                queue.dispatch_pending(&mut state).unwrap();
                queue.flush().unwrap();
                let Some(guard) = queue.prepare_read() else {
                    continue;
                };
                let mut p = libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                };
                if unsafe { libc::poll(&mut p, 1, 20) } > 0 {
                    let _ = guard.read();
                }
            }
        })
    };
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Ready { .. }));
    dnd.register_target(&handle, spec()).unwrap();
    let control = server.dnd();
    for _ in 0..20 {
        control.offer(&["text/uri-list"], 1, &[]);
        control.enter(0, 1., 1.);
        control.motion(2., 2.);
        let events = recv_until(&rx, &dnd, |e| {
            matches!(
                e,
                DndEvent::Hover {
                    target: Some(_),
                    ..
                }
            )
        });
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DndEvent::Enter { surface, .. } if *surface == handle))
        );
        control.leave();
        recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Leave { .. }));
    }
    drop(reader);
    drop(dnd);
    stop.store(true, Ordering::SeqCst);
    owner_loop.join().unwrap();
    drop(guest);
    server.settle();
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.dnd.devices.load(Ordering::SeqCst), 0);
}

#[test]
fn events_from_consumer_calls_wake_the_consumer() {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    let (tx, rx) = mpsc::channel();
    let _reader = dnd
        .spawn_reader(move || {
            let _ = tx.send(());
        })
        .unwrap();
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Ready { .. }));
    let target = dnd.register_target(&handle, spec()).unwrap();
    server.dnd().offer(&["text/uri-list"], 1, &[]);
    server.dnd().enter(0, 1., 1.);
    recv_until(&rx, &dnd, |e| {
        matches!(
            e,
            DndEvent::Hover {
                target: Some(_),
                ..
            }
        )
    });
    // No protocol traffic follows; only the consumer's own call produces the next event.
    dnd.update_target(target, None, Some(false)).unwrap();
    recv_until(&rx, &dnd, |e| {
        matches!(e, DndEvent::Hover { target: None, .. })
    });
}

#[test]
fn sends_taken_in_by_a_consumer_dispatch_are_still_written() {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig {
            slow_reader: true,
            ..Default::default()
        }),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    let (tx, rx) = mpsc::channel();
    let _reader = dnd
        .spawn_reader(move || {
            let _ = tx.send(());
        })
        .unwrap();
    recv_until(&rx, &dnd, |e| matches!(e, DndEvent::Ready { .. }));
    let request = libway::dnd::DragRequest {
        seat: None,
        origin: handle,
        serial: Some(1),
        data: libway::dnd::DragData::Text("k".repeat(4 << 20)),
        actions: Actions {
            copy: true,
            move_: false,
            ask: false,
        },
        icon: None,
    };
    dnd.start_drag(request).unwrap();
    server.settle();
    let m = &server.metrics.dnd;
    // 4 MiB never fits one pipe buffer: the rest needs POLLOUT from the reader.
    for round in 0..3 {
        server.dnd().request_send("UTF8_STRING");
        // Race the reader for the send event; whoever dispatches it, the data must go out.
        let end = Instant::now() + Duration::from_millis(30);
        while Instant::now() < end {
            dnd.dispatch().unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while m.received.lock().unwrap().len() <= round && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(m.received.lock().unwrap().len(), round + 1, "round {round}");
    }
    assert!(
        m.received
            .lock()
            .unwrap()
            .iter()
            .all(|(_, b)| b.len() == 4 << 20)
    );
}

#[test]
fn one_session_has_at_most_one_reader() {
    let (_server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    let first = dnd.spawn_reader(|| {}).unwrap();
    assert!(matches!(
        dnd.spawn_reader(|| {}),
        Err(libway::Error::Unsupported(_))
    ));
    drop(first);
    let second = dnd.spawn_reader(|| {}).unwrap();
    assert!(second.is_alive());
}

#[test]
fn socket_backpressure_resumes_without_consumer_flush() {
    use std::os::fd::AsRawFd;
    // Reader already asleep, reader started with pending writes, caller-driven loop.
    for mode in 0..4 {
        let (server, socket) = Server::start(Config {
            dnd: Some(DndConfig::default()),
            ..Default::default()
        });
        let display = Display::from_socket(socket).unwrap();
        let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
        let origin = surface(&display);
        dnd.wait_ready(Duration::from_secs(2)).unwrap();
        dnd.events();
        let mut reader = (mode == 0).then(|| dnd.spawn_reader(|| {}).unwrap());
        // With no events or transfer deadlines the existing reader sleeps indefinitely.
        std::thread::sleep(Duration::from_millis(30));
        let paused = server.dnd().pause();
        let conn = display.connection();
        let size: libc::c_int = 4096;
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    conn.backend().poll_fd().as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&size as *const libc::c_int).cast(),
                    std::mem::size_of_val(&size) as _,
                )
            },
            0
        );
        for _ in 0..10_000 {
            origin.damage(0, 0, 1, 1);
        }
        assert!(
            matches!(conn.flush(), Err(wayland_client::backend::WaylandError::Io(e))
            if e.kind() == std::io::ErrorKind::WouldBlock)
        );
        dnd.start_drag(libway::dnd::DragRequest {
            seat: None,
            origin: SurfaceHandle::from_surface(&origin),
            serial: Some(42),
            data: libway::dnd::DragData::Text("queued".into()),
            actions: Actions {
                copy: true,
                move_: false,
                ask: false,
            },
            icon: None,
        })
        .unwrap();
        if mode == 1 || mode == 3 {
            reader = Some(dnd.spawn_reader(|| {}).unwrap());
        }
        // Ensure the reader has encountered the full socket before releasing the server.
        std::thread::sleep(Duration::from_millis(30));
        if mode == 3 {
            let stop = Instant::now();
            drop(reader);
            assert!(stop.elapsed() < Duration::from_millis(500));
            drop(paused);
            continue;
        }
        drop(paused);
        let end = Instant::now() + Duration::from_secs(2);
        while server.metrics.dnd.start_drags.lock().unwrap().is_empty() && Instant::now() < end {
            if mode == 2 {
                dnd.run(Duration::from_millis(20)).unwrap();
            } else {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        assert_eq!(
            *server.metrics.dnd.start_drags.lock().unwrap(),
            [(42, false)],
            "mode {mode}"
        );
        drop(reader);
    }
}
