#![cfg(feature = "dnd")]
mod support;
use libway::{
    Display, SurfaceHandle,
    dnd::{Accept, Action, Actions, Dnd, DndEvent, DndOptions, LocalRect, TargetSpec},
};
use std::{sync::atomic::Ordering, time::Duration};
use support::{
    client::{surface, wait_for},
    *,
};

fn spec(rect: LocalRect, priority: i32) -> TargetSpec {
    TargetSpec {
        rect,
        accepts: vec![Accept::Files],
        actions: Actions {
            copy: true,
            move_: false,
            ask: false,
        },
        preferred: Action::Copy,
        priority,
        enabled: true,
    }
}
fn rect(x: f64, y: f64, width: f64, height: f64) -> LocalRect {
    LocalRect {
        x,
        y,
        width,
        height,
    }
}
fn session() -> (Server, Display, Dnd) {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let dnd = Dnd::new(&display, DndOptions::default()).unwrap();
    (server, display, dnd)
}

#[test]
fn ready_after_registry_sync_and_seat_binding() {
    let (server, _display, dnd) = session();
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    let events = dnd.events();
    assert!(
        matches!(events.first(), Some(DndEvent::Ready { version: 3, seats }) if seats.len() == 1),
        "{events:?}"
    );
    server.settle();
    let m = &server.metrics.dnd;
    assert_eq!(m.devices.load(Ordering::SeqCst), 1);
    assert_eq!(
        m.pointers.load(Ordering::SeqCst),
        0,
        "no input tracking by default"
    );
    drop(dnd);
    server.settle();
    assert_eq!(m.devices.load(Ordering::SeqCst), 0);
}

#[test]
fn wait_durations_and_transfer_limits_are_checked() {
    let (server, display, dnd) = session();
    let paused = server.dnd().pause();
    assert!(matches!(
        dnd.run(Duration::MAX),
        Err(libway::Error::InvalidInput(_))
    ));
    assert!(matches!(
        dnd.wait_ready(Duration::MAX),
        Err(libway::Error::InvalidInput(_))
    ));
    assert!(matches!(
        dnd.wait_ready(Duration::ZERO),
        Err(libway::Error::Timeout)
    ));
    let start = std::time::Instant::now();
    dnd.run(Duration::ZERO).unwrap();
    assert!(start.elapsed() < Duration::from_millis(100));
    for duration in [Duration::ZERO, Duration::MAX] {
        for total in [false, true] {
            let mut options = DndOptions::default();
            if total {
                options.limits.total = duration;
            } else {
                options.limits.inactivity = duration;
            }
            assert!(matches!(
                Dnd::new(&display, options),
                Err(libway::Error::InvalidInput(_))
            ));
        }
    }
    drop(paused);
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.wait_ready(Duration::ZERO).unwrap();
}

#[test]
fn foreign_and_destroyed_surfaces_cannot_register_targets() {
    use wayland_client::Proxy;
    let (_server, display, dnd) = session();
    let (_other_server, other, _other_dnd) = session();
    let own = surface(&display);
    let foreign = surface(&other);
    assert_eq!(own.id().protocol_id(), foreign.id().protocol_id());
    let spec = || spec(rect(0., 0., 10., 10.), 0);
    assert!(matches!(
        dnd.register_target(&SurfaceHandle::from_surface(&foreign), spec()),
        Err(libway::Error::InvalidInput(_))
    ));
    let handle = SurfaceHandle::from_surface(&own);
    own.destroy();
    assert!(matches!(
        dnd.register_target(&handle, spec()),
        Err(libway::Error::InvalidInput(_))
    ));
    let shared = Display::from_connection(display.clone().connection().clone());
    let live = surface(&shared);
    let id = dnd
        .register_target(&SurfaceHandle::from_surface(&live), spec())
        .unwrap();
    dnd.unregister_target(id).unwrap();
}

#[test]
fn hover_routes_to_registered_targets_and_negotiates() {
    let (server, display, dnd) = session();
    let surface = surface(&display);
    let handle = SurfaceHandle::from_surface(&surface);
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    let left = dnd
        .register_target(&handle, spec(rect(0., 0., 100., 100.), 0))
        .unwrap();
    let right = dnd
        .register_target(&handle, spec(rect(100., 0., 100., 100.), 0))
        .unwrap();
    let overlay = dnd
        .register_target(&handle, spec(rect(90., 0., 20., 100.), 5))
        .unwrap();
    let control = server.dnd();
    control.offer(&["text/uri-list", "text/plain"], 1 | 2, &[]);
    control.enter(0, 10., 10.);
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Hover { .. }));
    assert!(events.iter().any(
        |e| matches!(e, DndEvent::Enter { offered, surface, .. } if offered.len() == 2 && *surface == handle)
    ));
    assert!(events.iter().any(|e| matches!(e,
        DndEvent::Hover { target: Some(t), x, y, .. } if *t == left && *x == 10. && *y == 10.)));
    control.motion(150., 10.);
    wait_for(
        &dnd,
        |e| matches!(e, DndEvent::Hover { target: Some(t), .. } if *t == right),
    );
    control.motion(95., 10.);
    wait_for(
        &dnd,
        |e| matches!(e, DndEvent::Hover { target: Some(t), .. } if *t == overlay),
    );
    control.motion(150., 150.);
    wait_for(&dnd, |e| matches!(e, DndEvent::Hover { target: None, .. }));
    dnd.update_target(right, None, Some(false)).unwrap();
    control.motion(150., 10.);
    wait_for(&dnd, |e| matches!(e, DndEvent::Hover { target: None, .. }));
    control.leave();
    wait_for(&dnd, |e| matches!(e, DndEvent::Leave { .. }));
    server.settle();
    let m = &server.metrics.dnd;
    let uri = Some("text/uri-list".to_string());
    // left, right, overlay, outside; the disabled target keeps the refusal without a request.
    assert_eq!(
        m.accepts.lock().unwrap().clone(),
        vec![uri.clone(), uri.clone(), uri, None]
    );
    let actions = m.actions.lock().unwrap().clone();
    assert_eq!(actions[0], (1, 1));
    assert_eq!(*actions.last().unwrap(), (0, 0));
    assert_eq!(m.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn targets_that_accept_nothing_offered_refuse_the_hover() {
    let (server, display, dnd) = session();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.register_target(&handle, spec(rect(0., 0., 100., 100.), 0))
        .unwrap();
    let control = server.dnd();
    control.offer(&["image/png"], 1, &[]);
    control.enter(0, 10., 10.);
    wait_for(&dnd, |e| matches!(e, DndEvent::Hover { target: None, .. }));
    server.settle();
    assert!(server.metrics.dnd.accepts.lock().unwrap().is_empty());
}

#[test]
fn motion_is_coalesced_and_ordered_events_are_kept() {
    let (server, display, dnd) = session();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.register_target(&handle, spec(rect(0., 0., 100., 100.), 0))
        .unwrap();
    let control = server.dnd();
    control.offer(&["text/uri-list"], 1, &[]);
    control.enter(0, 1., 1.);
    for i in 0..500 {
        control.motion(f64::from(i % 100), 1.);
    }
    control.leave();
    server.settle();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Leave { .. }));
    let hovers = events
        .iter()
        .filter(|e| matches!(e, DndEvent::Hover { .. }))
        .count();
    assert!(hovers < 500, "hover events were not coalesced: {hovers}");
    let leave = events
        .iter()
        .position(|e| matches!(e, DndEvent::Leave { .. }))
        .unwrap();
    let last_hover = events
        .iter()
        .rposition(|e| matches!(e, DndEvent::Hover { .. }))
        .unwrap();
    assert!(last_hover < leave);
}

#[test]
fn selection_offers_are_released_without_events() {
    let (server, _display, dnd) = session();
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.events();
    let control = server.dnd();
    control.offer(&["text/plain"], 0, &[]);
    control.selection();
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    assert!(dnd.events().is_empty());
    server.settle();
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn stale_announcements_are_evicted_and_later_drags_still_work() {
    let (server, display, dnd) = session();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.register_target(&handle, spec(rect(0., 0., 100., 100.), 0))
        .unwrap();
    let control = server.dnd();
    dnd.events();
    for _ in 0..20 {
        control.offer(&["text/uri-list"], 1, &[]);
    }
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    server.settle();
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 8);
    control.enter(0, 5., 5.);
    wait_for(&dnd, |e| {
        matches!(
            e,
            DndEvent::Hover {
                target: Some(_),
                ..
            }
        )
    });
}

#[test]
fn specs_that_would_trip_a_compositor_are_rejected() {
    let (_server, display, dnd) = session();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    let mut bad = spec(rect(0., 0., 10., 10.), 0);
    bad.actions = Actions {
        copy: false,
        move_: true,
        ask: false,
    };
    assert!(matches!(
        dnd.register_target(&handle, bad.clone()),
        Err(libway::Error::InvalidInput(_))
    ));
    bad.actions = Actions::default();
    bad.preferred = Action::None;
    assert!(matches!(
        dnd.register_target(&handle, bad),
        Err(libway::Error::InvalidInput(_))
    ));
}

#[test]
fn event_overflow_fails_the_session_for_good() {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig::default()),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let options = DndOptions {
        limits: libway::dnd::DndLimits {
            max_events: 2,
            ..Default::default()
        },
        ..Default::default()
    };
    let dnd = Dnd::new(&display, options).unwrap();
    let _surface = surface(&display);
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.events();
    let control = server.dnd();
    control.offer(&["text/uri-list"], 1, &[]);
    control.enter(0, 1., 1.);
    control.leave();
    control.offer(&["text/uri-list"], 1, &[]);
    control.enter(0, 1., 1.);
    server.settle();
    let mut failed = false;
    for _ in 0..10 {
        failed |= dnd.run(Duration::from_millis(20)).is_err();
    }
    assert!(failed);
    assert!(dnd.dispatch().is_err(), "overflow stays fatal");
    let events = dnd.events();
    assert!(
        matches!(events.last(), Some(DndEvent::Failed(_))),
        "{events:?}"
    );
    assert!(events.len() <= 3, "{events:?}");
}
