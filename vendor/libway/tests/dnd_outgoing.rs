#![cfg(feature = "dnd")]
mod support;
use libway::{
    Display, Error, SurfaceHandle,
    dnd::{
        Action, Actions, Dnd, DndEvent, DndOptions, DragData, DragIcon, DragOutcome, DragRequest,
    },
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};
use support::{
    client::{surface, wait_for},
    *,
};

fn setup(version: u32, track_input: bool) -> (Server, Display, Dnd, SurfaceHandle) {
    let (server, socket) = Server::start(Config {
        dnd: Some(DndConfig {
            version,
            ..Default::default()
        }),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let options = DndOptions {
        track_input,
        ..Default::default()
    };
    let dnd = Dnd::new(&display, options).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.events();
    (server, display, dnd, handle)
}
fn request(origin: &SurfaceHandle, data: DragData, serial: Option<u32>) -> DragRequest {
    DragRequest {
        seat: None,
        origin: origin.clone(),
        serial,
        data,
        actions: Actions {
            copy: true,
            move_: true,
            ask: false,
        },
        icon: None,
    }
}
fn ended(events: &[DndEvent]) -> Option<DragOutcome> {
    events.iter().find_map(|e| match e {
        DndEvent::DragEnded { outcome, .. } => Some(*outcome),
        _ => None,
    })
}

#[test]
fn tracked_serial_starts_a_drag_and_serves_every_mime() {
    let (server, _display, dnd, origin) = setup(3, true);
    let m = &server.metrics.dnd;
    let control = server.dnd();
    control.button(777);
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    dnd.start_drag(request(&origin, DragData::Text("x".into()), None))
        .unwrap();
    server.settle();
    assert_eq!(m.start_drags.lock().unwrap().clone(), vec![(777, false)]);
    let offered = m.source_mimes.lock().unwrap().clone();
    assert_eq!(
        offered,
        [
            "text/plain;charset=utf-8",
            "text/plain",
            "UTF8_STRING",
            "TEXT",
            "STRING"
        ]
    );
    assert_eq!(m.actions.lock().unwrap().last().copied(), Some((3, 0)));
    control.target(Some("UTF8_STRING"), 1);
    let events = wait_for(&dnd, |e| {
        matches!(
            e,
            DndEvent::DragFeedback {
                action: Action::Copy,
                ..
            }
        )
    });
    assert!(
        events
            .iter()
            .any(|e| matches!(e, DndEvent::DragFeedback { accepted: true, .. }))
    );
    control.request_send("UTF8_STRING");
    control.request_send("STRING");
    control.drop_performed();
    control.finished();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::DragEnded { .. }));
    assert_eq!(ended(&events), Some(DragOutcome::Dropped(Action::Copy)));
    server.settle();
    let mut received = m.received.lock().unwrap().clone();
    received.sort();
    assert_eq!(
        received,
        vec![
            ("STRING".into(), b"x".to_vec()),
            ("UTF8_STRING".into(), b"x".to_vec())
        ]
    );
    assert_eq!(m.sources_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_serial_is_rejected_and_explicit_serial_works_without_tracking() {
    let (server, _display, dnd, origin) = setup(3, false);
    let m = &server.metrics.dnd;
    let text = request(&origin, DragData::Text("x".into()), None);
    assert!(matches!(dnd.start_drag(text), Err(Error::InvalidSerial)));
    let files = DragData::Files(vec![PathBuf::from("/tmp/a b")]);
    dnd.start_drag(request(&origin, files, Some(42))).unwrap();
    server.settle();
    assert_eq!(m.start_drags.lock().unwrap().clone(), vec![(42, false)]);
    let offered = m.source_mimes.lock().unwrap().clone();
    assert_eq!(
        offered,
        ["text/uri-list", "text/plain;charset=utf-8", "text/plain"]
    );
    let control = server.dnd();
    control.request_send("text/uri-list");
    control.request_send("text/plain");
    control.cancel_source();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::DragEnded { .. }));
    assert_eq!(ended(&events), Some(DragOutcome::Cancelled));
    server.settle();
    let received = m.received.lock().unwrap().clone();
    assert_eq!(received.len(), 2);
    assert!(received.iter().all(|(_, b)| b == b"file:///tmp/a%20b\r\n"));
    assert_eq!(m.sources_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn lazy_payloads_large_data_and_icons() {
    let (server, _display, dnd, origin) = setup(3, false);
    let m = &server.metrics.dnd;
    let big = vec![b'q'; 4 * 1024 * 1024];
    let payload = big.clone();
    let data = DragData::Lazy {
        mimes: vec!["application/x-libway-test".into()],
        provide: Box::new(move |_| Some(payload.clone())),
    };
    let mut req = request(&origin, data, Some(1));
    req.icon = Some(DragIcon {
        width: 2,
        height: 2,
        hotspot: (1, 1),
        rgba_premultiplied: vec![255; 16],
    });
    let drag = dnd.start_drag(req).unwrap();
    server.settle();
    assert_eq!(m.start_drags.lock().unwrap().clone(), vec![(1, true)]);
    assert_eq!(m.surfaces.load(Ordering::SeqCst), 2, "origin and icon");
    let control = server.dnd();
    control.request_send("application/x-libway-test");
    control.request_send("nonexistent/type");
    control.drop_performed();
    control.finished();
    let events = wait_for(
        &dnd,
        |e| matches!(e, DndEvent::DragEnded { drag: d, .. } if *d == drag),
    );
    assert!(matches!(ended(&events), Some(DragOutcome::Dropped(_))));
    // The 4 MiB send outlives the drag and needs several POLLOUT rounds over a 1 MiB pipe.
    let end = std::time::Instant::now() + Duration::from_secs(5);
    while m.received.lock().unwrap().len() < 2 && std::time::Instant::now() < end {
        dnd.run(Duration::from_millis(20)).unwrap();
    }
    let received = m.received.lock().unwrap().clone();
    assert_eq!(received.len(), 2);
    assert!(
        received
            .iter()
            .any(|(m, b)| m == "application/x-libway-test" && *b == big)
    );
    assert!(
        received
            .iter()
            .any(|(m, b)| m == "nonexistent/type" && b.is_empty())
    );
    server.settle();
    assert_eq!(m.sources_alive.load(Ordering::SeqCst), 0);
    assert_eq!(
        m.surfaces.load(Ordering::SeqCst),
        1,
        "icon surface destroyed, origin stays"
    );
}

#[test]
fn invalid_icons_are_rejected_before_any_request() {
    let (server, _display, dnd, origin) = setup(3, false);
    let mut req = request(&origin, DragData::Text("t".into()), Some(1));
    req.icon = Some(DragIcon {
        width: 2,
        height: 2,
        hotspot: (0, 0),
        rgba_premultiplied: vec![0; 15],
    });
    assert!(matches!(dnd.start_drag(req), Err(Error::InvalidDimensions)));
    server.settle();
    assert!(server.metrics.dnd.start_drags.lock().unwrap().is_empty());
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn cancel_drag_destroys_the_source_and_v1_has_no_actions() {
    let (server, _display, dnd, origin) = setup(1, false);
    let drag = dnd
        .start_drag(request(&origin, DragData::Text("t".into()), Some(5)))
        .unwrap();
    server.settle();
    assert!(server.metrics.dnd.actions.lock().unwrap().is_empty());
    dnd.cancel_drag(drag).unwrap();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::DragEnded { .. }));
    assert_eq!(ended(&events), Some(DragOutcome::Cancelled));
    server.settle();
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 0);
    assert!(matches!(dnd.cancel_drag(drag), Err(Error::UnknownTransfer)));
}

#[test]
fn v1_drop_that_fetched_data_ends_as_dropped() {
    let (server, _display, dnd, origin) = setup(1, false);
    dnd.start_drag(request(&origin, DragData::Text("t".into()), Some(5)))
        .unwrap();
    server.settle();
    let control = server.dnd();
    control.request_send("text/plain");
    control.cancel_source();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::DragEnded { .. }));
    assert_eq!(ended(&events), Some(DragOutcome::Dropped(Action::None)));
}

#[test]
fn a_second_drag_on_the_same_seat_cancels_the_first() {
    let (server, _display, dnd, origin) = setup(3, false);
    let first = dnd
        .start_drag(request(&origin, DragData::Text("a".into()), Some(1)))
        .unwrap();
    let second = dnd
        .start_drag(request(&origin, DragData::Text("b".into()), Some(2)))
        .unwrap();
    let events = dnd.events();
    assert!(events.iter().any(|e| matches!(e,
        DndEvent::DragEnded { drag, outcome: DragOutcome::Cancelled } if *drag == first)));
    dnd.cancel_drag(second).unwrap();
    server.settle();
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn relative_paths_are_not_turned_into_host_uris() {
    let (server, _display, dnd, origin) = setup(3, false);
    let files = DragData::Files(vec![PathBuf::from("foo.txt")]);
    let answer = dnd.start_drag(request(&origin, files, Some(1)));
    assert!(matches!(answer, Err(Error::InvalidInput(_))));
    server.settle();
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn the_tracked_serial_is_only_used_while_the_button_is_held() {
    let (server, _display, dnd, origin) = setup(3, true);
    let control = server.dnd();
    control.button(777);
    control.release(778);
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    let text = || request(&origin, DragData::Text("x".into()), None);
    assert!(matches!(dnd.start_drag(text()), Err(Error::InvalidSerial)));
    control.button(779);
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    dnd.start_drag(text()).unwrap();
    server.settle();
    assert_eq!(
        server.metrics.dnd.start_drags.lock().unwrap().clone(),
        vec![(779, false)]
    );
}

#[test]
fn files_and_text_serve_uri_lists_to_file_targets_and_text_to_the_rest() {
    let (server, _display, dnd, origin) = setup(3, false);
    let data = DragData::FilesAndText {
        paths: vec![PathBuf::from("/tmp/a b")],
        text: "/tmp/a b".into(),
    };
    dnd.start_drag(request(&origin, data, Some(3))).unwrap();
    server.settle();
    let offered = server.metrics.dnd.source_mimes.lock().unwrap().clone();
    assert_eq!(offered[0], "text/uri-list");
    assert!(offered.iter().any(|m| m == "UTF8_STRING"));
    let control = server.dnd();
    control.request_send("text/uri-list");
    control.request_send("text/plain;charset=utf-8");
    control.request_send("image/png");
    control.drop_performed();
    control.finished();
    wait_for(&dnd, |e| matches!(e, DndEvent::DragEnded { .. }));
    let m = &server.metrics.dnd;
    let end = std::time::Instant::now() + Duration::from_secs(2);
    while m.received.lock().unwrap().len() < 3 && std::time::Instant::now() < end {
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut received = m.received.lock().unwrap().clone();
    received.sort();
    assert_eq!(
        received,
        vec![
            ("image/png".to_string(), Vec::new()),
            ("text/plain;charset=utf-8".to_string(), b"/tmp/a b".to_vec()),
            (
                "text/uri-list".to_string(),
                b"file:///tmp/a%20b\r\n".to_vec()
            ),
        ]
    );
}

#[test]
fn a_started_drag_consumes_the_grab_serial() {
    let (server, _display, dnd, origin) = setup(3, true);
    server.dnd().button(900);
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    let text = || request(&origin, DragData::Text("x".into()), None);
    dnd.start_drag(text()).unwrap();
    // The release that ends the drag may never be reported; the serial must not be reused.
    assert!(matches!(dnd.start_drag(text()), Err(Error::InvalidSerial)));
}

#[test]
fn malformed_mimes_do_not_replace_a_drag_or_call_lazy_providers() {
    let (server, _display, dnd, origin) = setup(3, true);
    let first = dnd
        .start_drag(request(&origin, DragData::Text("old".into()), Some(1)))
        .unwrap();
    server.dnd().button(901);
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    dnd.events();
    for lazy in [false, true] {
        for mime in ["text/\0plain".to_string(), String::new(), "x".repeat(1025)] {
            let too_long = mime.len() > 1024;
            let data = if lazy {
                DragData::Lazy {
                    mimes: vec![mime],
                    provide: Box::new(|_| panic!("validation called provider")),
                }
            } else {
                DragData::Static(vec![(mime, vec![])])
            };
            let result = dnd.start_drag(request(&origin, data, None));
            assert!(if too_long {
                matches!(result, Err(Error::LimitExceeded))
            } else {
                matches!(result, Err(Error::InvalidInput(_)))
            });
            assert!(dnd.events().is_empty());
        }
    }
    server.settle();
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 1);
    assert_eq!(server.metrics.dnd.start_drags.lock().unwrap().len(), 1);
    // The rejected replacements did not consume the tracked serial or cancel the old drag.
    dnd.start_drag(request(
        &origin,
        DragData::Static(vec![("STRING".into(), b"ok".to_vec())]),
        None,
    ))
    .unwrap();
    assert!(dnd.events().iter().any(|e| matches!(e,
        DndEvent::DragEnded { drag, outcome: DragOutcome::Cancelled } if *drag == first)));
    server.settle();
    assert_eq!(server.metrics.dnd.start_drags.lock().unwrap()[1].0, 901);
}

#[test]
fn icon_hotspot_bounds_are_checked_before_replacing_a_drag() {
    let (server, _display, dnd, origin) = setup(3, false);
    let icon = |hotspot| DragIcon {
        width: 2,
        height: 2,
        hotspot,
        rgba_premultiplied: vec![255; 16],
    };
    let mut req = request(&origin, DragData::Text("old".into()), Some(1));
    req.icon = Some(icon((0, 0)));
    let first = dnd.start_drag(req).unwrap();
    for hotspot in [
        (i32::MIN, 0),
        (0, i32::MIN),
        (-1, 0),
        (0, -1),
        (2, 0),
        (0, 2),
        (i32::MAX, i32::MAX),
    ] {
        let mut req = request(&origin, DragData::Text("bad".into()), Some(2));
        req.icon = Some(icon(hotspot));
        assert!(
            matches!(dnd.start_drag(req), Err(Error::InvalidInput(_))),
            "{hotspot:?}"
        );
        assert!(dnd.events().is_empty());
    }
    server.settle();
    assert_eq!(server.metrics.dnd.sources_alive.load(Ordering::SeqCst), 1);
    assert_eq!(server.metrics.dnd.surfaces.load(Ordering::SeqCst), 2);
    assert_eq!(server.metrics.dnd.start_drags.lock().unwrap().len(), 1);
    dnd.cancel_drag(first).unwrap();
    for hotspot in [(0, 1), (1, 0), (1, 1)] {
        let mut req = request(&origin, DragData::Text("edge".into()), Some(3));
        req.icon = Some(icon(hotspot));
        let id = dnd.start_drag(req).unwrap();
        dnd.cancel_drag(id).unwrap();
    }
}

#[test]
fn outgoing_mime_count_applies_to_convenience_static_and_lazy_data() {
    let (server, display, old, origin) = setup(3, false);
    drop(old);
    let options = DndOptions {
        limits: libway::dnd::DndLimits {
            max_mime_types: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let dnd = Dnd::new(&display, options).unwrap();
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    for data in [
        DragData::Text("text".into()),
        DragData::Files(vec!["/tmp/file".into()]),
        DragData::FilesAndText {
            paths: vec![],
            text: String::new(),
        },
        DragData::Static(vec![("one".into(), vec![]), ("two".into(), vec![])]),
        DragData::Lazy {
            mimes: vec!["one".into(), "two".into()],
            provide: Box::new(|_| panic!("provider")),
        },
    ] {
        assert!(matches!(
            dnd.start_drag(request(&origin, data, Some(1))),
            Err(Error::LimitExceeded)
        ));
    }
    dnd.start_drag(request(
        &origin,
        DragData::Static(vec![("x".repeat(1024), vec![])]),
        Some(1),
    ))
    .unwrap();
    server.settle();
    assert_eq!(server.metrics.dnd.start_drags.lock().unwrap().len(), 1);
}

#[test]
fn foreign_or_destroyed_origins_do_not_cancel_the_active_drag() {
    let (server, display, dnd, origin) = setup(3, false);
    let (_other_server, _other, _other_dnd, foreign) = setup(3, false);
    let first = dnd
        .start_drag(request(&origin, DragData::Text("old".into()), Some(1)))
        .unwrap();
    assert!(matches!(
        dnd.start_drag(request(&foreign, DragData::Text("bad".into()), Some(2))),
        Err(Error::InvalidInput(_))
    ));
    let dead = surface(&display);
    let dead_handle = SurfaceHandle::from_surface(&dead);
    dead.destroy();
    assert!(matches!(
        dnd.start_drag(request(&dead_handle, DragData::Text("bad".into()), Some(2))),
        Err(Error::InvalidInput(_))
    ));
    assert!(dnd.events().is_empty());
    server.settle();
    assert_eq!(server.metrics.dnd.start_drags.lock().unwrap().len(), 1);
    dnd.cancel_drag(first).unwrap();
}
