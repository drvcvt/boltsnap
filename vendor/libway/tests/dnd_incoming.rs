#![cfg(feature = "dnd")]
mod support;
use libway::{
    Display, Error, SurfaceHandle, TransferError,
    dnd::{
        Accept, Action, Actions, Dnd, DndEvent, DndLimits, DndOptions, LocalRect, Outcome, Payload,
        TargetSpec,
    },
};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};
use support::{
    client::{surface, wait_for},
    *,
};

fn setup(config: DndConfig, limits: DndLimits) -> (Server, Display, Dnd, SurfaceHandle) {
    let (server, socket) = Server::start(Config {
        dnd: Some(config),
        ..Default::default()
    });
    let display = Display::from_socket(socket).unwrap();
    let options = DndOptions {
        limits,
        track_input: false,
    };
    let dnd = Dnd::new(&display, options).unwrap();
    let handle = SurfaceHandle::from_surface(&surface(&display));
    dnd.wait_ready(Duration::from_secs(2)).unwrap();
    dnd.events();
    (server, display, dnd, handle)
}
fn files_target() -> TargetSpec {
    TargetSpec {
        rect: LocalRect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 100.,
        },
        accepts: vec![Accept::Files, Accept::Text],
        actions: Actions {
            copy: true,
            move_: true,
            ask: false,
        },
        preferred: Action::Copy,
        priority: 0,
        enabled: true,
    }
}
fn dropped(events: &[DndEvent]) -> Option<(libway::dnd::TransferId, &Payload, Action)> {
    events.iter().find_map(|e| match e {
        DndEvent::Dropped {
            transfer,
            payload,
            action,
            ..
        } => Some((*transfer, payload, *action)),
        _ => None,
    })
}
fn failed(events: &[DndEvent]) -> Option<TransferError> {
    events.iter().find_map(|e| match e {
        DndEvent::TransferFailed { reason, .. } => Some(*reason),
        _ => None,
    })
}

#[test]
fn drop_delivers_paths_then_finish_only_after_completion() {
    for version in [1, 2, 3] {
        let config = DndConfig {
            version,
            ..Default::default()
        };
        let (server, _display, dnd, handle) = setup(config, DndLimits::default());
        let m = &server.metrics.dnd;
        let target = dnd.register_target(&handle, files_target()).unwrap();
        let control = server.dnd();
        let list: &[u8] = b"file:///tmp/a%20b\r\nfile:///c\r\n";
        control.offer(
            &["text/plain", "text/uri-list"],
            1 | 2,
            &[("text/uri-list", list)],
        );
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
        control.drop();
        let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
        assert!(events.iter().any(|e| matches!(e, DndEvent::Leave { .. })));
        let (transfer, payload, action) = dropped(&events).unwrap();
        let paths = vec![PathBuf::from("/tmp/a b"), PathBuf::from("/c")];
        assert_eq!(payload, &Payload::Files(paths));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DndEvent::Dropped { target: t, .. } if *t == target))
        );
        let expected = if version >= 3 {
            Action::Copy
        } else {
            Action::None
        };
        assert_eq!(action, expected, "v{version}");
        server.settle();
        assert_eq!(m.finishes.load(Ordering::SeqCst), 0);
        assert_eq!(m.offers_alive.load(Ordering::SeqCst), 1);
        assert_eq!(m.receives.lock().unwrap().as_slice(), ["text/uri-list"]);
        dnd.complete(transfer, Outcome::Accepted(Action::Copy))
            .unwrap();
        server.settle();
        assert_eq!(m.finishes.load(Ordering::SeqCst), usize::from(version >= 3));
        assert_eq!(m.offers_alive.load(Ordering::SeqCst), 0);
        assert!(matches!(
            dnd.complete(transfer, Outcome::Rejected),
            Err(Error::UnknownTransfer)
        ));
        let pipes = m.pipes.lock().unwrap().clone();
        assert_eq!(pipes.len(), 1);
        assert!(!pipe_open(pipes[0]), "pipe leak at version {version}");
    }
}

#[test]
fn completion_must_match_negotiation_and_invalid_answers_are_retryable() {
    for version in [1, 2, 3] {
        for action in [Action::None, Action::Copy, Action::Move] {
            let (server, _display, dnd, handle) = setup(
                DndConfig {
                    version,
                    ..Default::default()
                },
                DndLimits::default(),
            );
            let mut spec = files_target();
            spec.preferred = action;
            dnd.register_target(&handle, spec).unwrap();
            let control = server.dnd();
            let bits = match action {
                Action::Copy => 1,
                Action::Move => 2,
                _ => 0,
            };
            control.offer(&["text/plain"], bits, &[("text/plain", b"data")]);
            control.enter(0, 5., 5.);
            wait_for(&dnd, |e| {
                matches!(e, DndEvent::Hover { action: a, target: Some(_), .. }
                if *a == if version >= 3 { action } else { Action::None })
            });
            control.drop();
            let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
            let (transfer, _, negotiated) = dropped(&events).unwrap();
            assert_eq!(negotiated, if version >= 3 { action } else { Action::None });
            for chosen in [Action::None, Action::Copy, Action::Move, Action::Ask] {
                let valid = chosen != Action::Ask
                    && (version < 3
                        || matches!(action, Action::Copy | Action::Move) && chosen == action);
                if valid {
                    continue;
                }
                assert!(
                    matches!(
                        dnd.complete(transfer, Outcome::Accepted(chosen)),
                        Err(Error::InvalidInput(_))
                    ),
                    "v{version}: {action:?} accepted as {chosen:?}"
                );
                server.settle();
                assert_eq!(server.metrics.dnd.finishes.load(Ordering::SeqCst), 0);
                assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 1);
            }
            let outcome = if version >= 3 && action == Action::None {
                Outcome::Rejected
            } else {
                Outcome::Accepted(action)
            };
            dnd.complete(transfer, outcome).unwrap();
            server.settle();
            assert_eq!(
                server.metrics.dnd.finishes.load(Ordering::SeqCst),
                usize::from(version >= 3 && action != Action::None)
            );
            assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
            assert_eq!(server.metrics.dnd.protocol_errors.load(Ordering::SeqCst), 0);
        }
    }
}

#[test]
fn rejected_completion_destroys_without_finish() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let mime = "text/plain;charset=utf-8";
    control.offer(&[mime], 1, &[(mime, "h\u{e9}llo".as_bytes())]);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
    let (transfer, payload, _) = dropped(&events).unwrap();
    assert_eq!(payload, &Payload::Text("h\u{e9}llo".into()));
    dnd.complete(transfer, Outcome::Rejected).unwrap();
    server.settle();
    assert_eq!(server.metrics.dnd.finishes.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn raw_mime_targets_get_bytes_and_latin1_is_decoded() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    let mut spec = files_target();
    spec.accepts = vec![Accept::Mime(vec!["image/png".into()]), Accept::Text];
    dnd.register_target(&handle, spec).unwrap();
    let control = server.dnd();
    control.offer(&["image/png"], 1, &[("image/png", b"\x89PNG")]);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
    let (transfer, payload, _) = dropped(&events).unwrap();
    let png = Payload::Bytes {
        mime: "image/png".into(),
        data: b"\x89PNG".to_vec(),
    };
    assert_eq!(payload, &png);
    dnd.complete(transfer, Outcome::Accepted(Action::Copy))
        .unwrap();
    control.offer(&["STRING"], 1, &[("STRING", b"caf\xe9")]);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
    assert_eq!(
        dropped(&events).unwrap().1,
        &Payload::Text("caf\u{e9}".into())
    );
}

#[test]
fn drop_outside_targets_or_without_accept_is_released() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    control.offer(&["image/png"], 1, &[("image/png", b"png")]);
    control.enter(0, 5., 5.);
    wait_for(&dnd, |e| matches!(e, DndEvent::Hover { target: None, .. }));
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Leave { .. }));
    server.settle();
    dnd.run(Duration::from_millis(50)).unwrap();
    assert!(dropped(&events).is_none() && failed(&events).is_none());
    assert!(dnd.events().is_empty());
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
    assert!(server.metrics.dnd.receives.lock().unwrap().is_empty());
}

#[test]
fn leave_after_drop_does_not_abort_and_limits_apply() {
    let limits = DndLimits {
        max_payload: 64,
        inactivity: Duration::from_millis(300),
        total: Duration::from_millis(900),
        ..Default::default()
    };
    let (server, _display, dnd, handle) = setup(DndConfig::default(), limits);
    let target = dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", b"file:///ok\n")]);
    control.enter(0, 5., 5.);
    control.drop();
    control.leave();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
    assert!(failed(&events).is_none());
    let transfer = dropped(&events).unwrap().0;
    dnd.complete(transfer, Outcome::Accepted(Action::Copy))
        .unwrap();

    let mut list = b"file:///".to_vec();
    list.extend([b'x'; 65]);
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", &list)]);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::TransferFailed { .. }));
    assert!(events.iter().any(|e| matches!(e,
        DndEvent::TransferFailed { target: t, reason: TransferError::TooLarge, .. } if *t == target)));
    server.settle();
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn stalled_writer_times_out_and_unregister_aborts() {
    let limits = DndLimits {
        inactivity: Duration::from_millis(200),
        total: Duration::from_millis(2000),
        ..Default::default()
    };
    let config = DndConfig {
        hold_writer_open: true,
        ..Default::default()
    };
    let (server, _display, dnd, handle) = setup(config, limits);
    let target = dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let list: &[u8] = b"file:///never-closed\n";
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", list)]);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::TransferFailed { .. }));
    assert_eq!(failed(&events), Some(TransferError::Timeout));

    control.offer(&["text/uri-list"], 1, &[("text/uri-list", list)]);
    control.enter(0, 5., 5.);
    control.drop();
    wait_for(&dnd, |e| matches!(e, DndEvent::Leave { .. }));
    dnd.unregister_target(target).unwrap();
    assert_eq!(failed(&dnd.events()), Some(TransferError::TargetGone));
    server.settle();
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn a_new_drag_does_not_abort_a_running_transfer() {
    let config = DndConfig {
        writer_delay: Duration::from_millis(40),
        ..Default::default()
    };
    let (server, _display, dnd, handle) = setup(config, DndLimits::default());
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let list = b"file:///slow\n".repeat(500);
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", &list)]);
    control.enter(0, 5., 5.);
    control.drop();
    wait_for(&dnd, |e| matches!(e, DndEvent::Leave { .. }));
    control.offer(&["text/uri-list"], 1, &[]);
    control.enter(0, 5., 5.);
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
    assert!(failed(&events).is_none(), "{events:?}");
    let Payload::Files(paths) = dropped(&events).unwrap().1 else {
        panic!("expected files")
    };
    assert_eq!(paths.len(), 500);
}

#[test]
fn malformed_uri_list_fails_the_whole_drop() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let list: &[u8] = b"file:///ok\nhttp://x/y\n";
    control.offer(&["text/uri-list"], 1, &[("text/uri-list", list)]);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::TransferFailed { .. }));
    assert_eq!(failed(&events), Some(TransferError::Malformed));
    server.settle();
    assert_eq!(server.metrics.dnd.finishes.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn ask_drops_finish_with_the_consumers_choice_only_if_the_source_offers_it() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    let mut spec = files_target();
    spec.actions = Actions {
        copy: true,
        move_: true,
        ask: true,
    };
    spec.preferred = Action::Ask;
    dnd.register_target(&handle, spec).unwrap();
    let control = server.dnd();
    control.offer(
        &["text/uri-list"],
        1 | 4,
        &[("text/uri-list", b"file:///asked\n")],
    );
    control.enter(0, 5., 5.);
    wait_for(&dnd, |e| {
        matches!(
            e,
            DndEvent::Hover {
                action: Action::Ask,
                ..
            }
        )
    });
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
    let (transfer, _, action) = dropped(&events).unwrap();
    assert_eq!(action, Action::Ask);
    for invalid in [Action::Ask, Action::None, Action::Move] {
        let answer = dnd.complete(transfer, Outcome::Accepted(invalid));
        assert!(matches!(answer, Err(Error::InvalidInput(_))), "{invalid:?}");
    }
    dnd.complete(transfer, Outcome::Accepted(Action::Copy))
        .unwrap();
    server.settle();
    let m = &server.metrics.dnd;
    assert_eq!(m.finishes.load(Ordering::SeqCst), 1);
    assert_eq!(m.actions.lock().unwrap().last().copied(), Some((1, 1)));
    assert_eq!(m.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn a_browser_link_falls_back_from_uri_list_to_text() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let link: &[u8] = b"https://example.com/a b";
    let mimes = ["text/uri-list", "text/plain;charset=utf-8"];
    let data: [(&str, &[u8]); 2] = [
        ("text/uri-list", b"https://example.com/a%20b\r\n"),
        ("text/plain;charset=utf-8", link),
    ];
    control.offer(&mimes, 1, &data);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| {
        matches!(
            e,
            DndEvent::Dropped { .. } | DndEvent::TransferFailed { .. }
        )
    });
    let (transfer, payload, _) = dropped(&events).expect("fell back to text");
    assert_eq!(payload, &Payload::Text("https://example.com/a b".into()));
    dnd.complete(transfer, Outcome::Accepted(Action::Copy))
        .unwrap();
    server.settle();
    let m = &server.metrics.dnd;
    assert_eq!(m.receives.lock().unwrap().as_slice(), mimes);
    assert_eq!(m.offers_alive.load(Ordering::SeqCst), 0);
    assert_eq!(m.finishes.load(Ordering::SeqCst), 1);
    for pipe in m.pipes.lock().unwrap().iter() {
        assert!(!pipe_open(*pipe), "both pipes closed");
    }
}

#[test]
fn a_drop_fails_only_after_every_representation_fails() {
    let (server, _display, dnd, handle) = setup(DndConfig::default(), DndLimits::default());
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let data: [(&str, &[u8]); 2] = [
        ("text/uri-list", b"ftp://x/y\n"),
        ("text/plain", b"\xff\xfe"),
    ];
    control.offer(&["text/uri-list", "text/plain"], 1, &data);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| matches!(e, DndEvent::TransferFailed { .. }));
    assert_eq!(failed(&events), Some(TransferError::Malformed));
    server.settle();
    assert_eq!(server.metrics.dnd.receives.lock().unwrap().len(), 2);
    assert_eq!(server.metrics.dnd.offers_alive.load(Ordering::SeqCst), 0);
}

#[test]
fn falling_back_to_another_type_keeps_the_total_deadline() {
    // Each 4 KiB chunk arrives after 150 ms: the bad uri-list takes 150 ms, the text 450 ms.
    // Alone the text would fit the 500 ms total; after the fallback the drop must not.
    let limits = DndLimits {
        total: Duration::from_millis(500),
        ..Default::default()
    };
    let config = DndConfig {
        writer_delay: Duration::from_millis(150),
        ..Default::default()
    };
    let (server, _display, dnd, handle) = setup(config, limits);
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    let text = vec![b'x'; 3 * 4096];
    let data: [(&str, &[u8]); 2] = [("text/uri-list", b"https://x/\n"), ("text/plain", &text)];
    control.offer(&["text/uri-list", "text/plain"], 1, &data);
    control.enter(0, 5., 5.);
    control.drop();
    let events = wait_for(&dnd, |e| {
        matches!(
            e,
            DndEvent::Dropped { .. } | DndEvent::TransferFailed { .. }
        )
    });
    assert_eq!(failed(&events), Some(TransferError::Timeout), "{events:?}");
    server.settle();
    assert_eq!(server.metrics.dnd.receives.lock().unwrap().len(), 2);
}

#[test]
fn incoming_mime_limit_keeps_only_the_announced_prefix() {
    let limits = DndLimits {
        max_mime_types: 1,
        ..Default::default()
    };
    let (server, _display, dnd, handle) = setup(DndConfig::default(), limits);
    dnd.register_target(&handle, files_target()).unwrap();
    let control = server.dnd();
    for (mimes, accepted) in [
        (["image/png", "text/plain"], false),
        (["text/plain", "image/png"], true),
    ] {
        control.offer(&mimes, 1, &[("text/plain", b"prefix")]);
        control.enter(0, 5., 5.);
        let events = wait_for(&dnd, |e| matches!(e, DndEvent::Hover { .. }));
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DndEvent::Enter { offered, .. }
            if offered.as_slice() == [mimes[0]]))
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, DndEvent::Hover { target, .. }
            if target.is_some() == accepted))
        );
        control.drop();
        if accepted {
            let events = wait_for(&dnd, |e| matches!(e, DndEvent::Dropped { .. }));
            let (id, payload, _) = dropped(&events).unwrap();
            assert_eq!(payload, &Payload::Text("prefix".into()));
            dnd.complete(id, Outcome::Rejected).unwrap();
        } else {
            let events = wait_for(&dnd, |e| matches!(e, DndEvent::Leave { .. }));
            assert!(failed(&events).is_none());
        }
    }
}
