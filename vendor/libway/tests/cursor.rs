#![cfg(feature = "capture")]
mod support;
use libway::*;
use std::{sync::atomic::Ordering, time::Duration};
use support::*;
fn options() -> CaptureOptions {
    CaptureOptions {
        timeout: Duration::from_millis(500),
        ..Default::default()
    }
}
fn next(stream: &mut CursorStream<'_>) -> CursorEvent {
    stream
        .poll_event(Duration::from_millis(500))
        .unwrap()
        .unwrap()
}
#[test]
fn cursor_hotspot_is_published_with_the_matching_image_and_alpha() {
    let (server, socket) = Server::start(Config {
        cursor: Some(CursorFault::Resize),
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    let mut stream = c.cursor_stream(output, options()).unwrap();
    assert!(matches!(next(&mut stream), CursorEvent::Enter { .. }));
    assert!(matches!(
        next(&mut stream),
        CursorEvent::Position { x: -2, y: 3, .. }
    ));
    let CursorEvent::Image {
        frame: first,
        hotspot,
        generation,
        ..
    } = next(&mut stream)
    else {
        panic!("expected image")
    };
    assert_eq!(hotspot, (1, 1));
    assert_eq!(generation, 1);
    assert_eq!((first.width, first.height), (4, 2));
    let original = first.rgba8().unwrap();
    assert_eq!(&original[..4], &[16, 32, 48, 128]);
    let CursorEvent::Image {
        frame: second,
        hotspot,
        generation,
        ..
    } = next(&mut stream)
    else {
        panic!("expected image")
    };
    assert_eq!(hotspot, (9, 7));
    assert_eq!(generation, 2);
    assert_eq!((second.width, second.height), (2, 3));
    drop(stream);
    c.outputs(&options()).unwrap();
    server.settle();
    assert_eq!(first.rgba8().unwrap(), original);
    for value in [
        &server.metrics.frames,
        &server.metrics.buffers,
        &server.metrics.cursor_sessions,
        &server.metrics.sessions,
        &server.metrics.sources,
        &server.metrics.seats,
        &server.metrics.pointers,
    ] {
        assert_eq!(value.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn position_and_visibility_continue_while_cursor_image_is_idle() {
    let (_server, socket) = Server::start(Config {
        cursor: Some(CursorFault::IdleImage),
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    let mut stream = c.cursor_stream(output, options()).unwrap();
    for _ in 0..3 {
        next(&mut stream);
    }
    assert!(matches!(
        next(&mut stream),
        CursorEvent::Position { x: 41, y: -3, .. }
    ));
    assert!(matches!(next(&mut stream), CursorEvent::Leave { .. }));
    assert!(matches!(next(&mut stream), CursorEvent::Enter { .. }));
    assert!(matches!(
        next(&mut stream),
        CursorEvent::Position { x: 5, y: 6, .. }
    ));
    for _ in 0..2 {
        assert!(
            stream
                .poll_event(Duration::from_millis(10))
                .unwrap()
                .is_none()
        );
    }
}
#[test]
fn cursor_errors_are_bounded_and_do_not_poison_the_connection() {
    for fault in [
        CursorFault::Flood,
        CursorFault::NoPointer,
        CursorFault::Stopped,
    ] {
        let (server, socket) = Server::start(Config {
            cursor: Some(fault),
            ..Default::default()
        });
        let mut c = Connection::from_socket(socket, &options()).unwrap();
        let output = c.outputs(&options()).unwrap()[0].id;
        let result = (|| -> Result<()> {
            let mut stream = c.cursor_stream(output, options())?;
            for _ in 0..5 {
                stream.poll_event(Duration::from_millis(100))?;
            }
            Ok(())
        })();
        // libwayland (`foreign-display`) reads 4 KiB per call and pump stops reading while
        // notices are queued, so depending on how the flood splits it becomes backpressure
        // or overflows. Both stay bounded; an overflow must be the explicit limit error.
        let backpressure = cfg!(feature = "foreign-display") && matches!(fault, CursorFault::Flood);
        match &result {
            Ok(()) => assert!(backpressure, "this fault must fail"),
            Err(e) if backpressure => assert!(matches!(e, Error::LimitExceeded), "{e:?}"),
            Err(_) => {}
        }
        c.outputs(&options()).unwrap();
        server.settle();
        assert_eq!(server.metrics.seats.load(Ordering::SeqCst), 0);
        assert_eq!(server.metrics.cursor_sessions.load(Ordering::SeqCst), 0);
        assert_eq!(server.metrics.frames.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn ordinary_capture_does_not_bind_seats_and_cursor_requires_ext() {
    let (server, socket) = Server::start(Config {
        ext: false,
        cursor: Some(CursorFault::None),
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    c.capture(output, &options()).unwrap();
    assert!(matches!(
        c.cursor_stream(output, options()),
        Err(Error::Unsupported(_))
    ));
    assert_eq!(server.metrics.seats.load(Ordering::SeqCst), 0);
}

#[test]
fn dropping_an_idle_image_and_reopening_resets_cursor_state() {
    let (server, socket) = Server::start(Config {
        cursor: Some(CursorFault::IdleImage),
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    for _ in 0..3 {
        let mut stream = c.cursor_stream(output, options()).unwrap();
        assert!(matches!(next(&mut stream), CursorEvent::Enter { .. }));
        assert!(matches!(
            next(&mut stream),
            CursorEvent::Position { x: -2, y: 3, .. }
        ));
        assert!(matches!(
            next(&mut stream),
            CursorEvent::Image {
                generation: 1,
                hotspot: (1, 1),
                ..
            }
        ));
        assert!(matches!(
            next(&mut stream),
            CursorEvent::Position { x: 41, .. }
        ));
        // Drop with a buffer in flight and unconsumed visibility/position events.
        drop(stream);
    }
    c.outputs(&options()).unwrap();
    server.settle();
    for value in [
        &server.metrics.frames,
        &server.metrics.buffers,
        &server.metrics.sessions,
        &server.metrics.cursor_sessions,
        &server.metrics.seats,
        &server.metrics.pointers,
    ] {
        assert_eq!(value.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn cancellation_releases_cursor_and_connection_can_capture_again() {
    let (server, socket) = Server::start(Config {
        cursor: Some(CursorFault::IdleImage),
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    let opts = options();
    let token = opts.cancellation.clone();
    let mut stream = c.cursor_stream(output, opts).unwrap();
    for _ in 0..7 {
        next(&mut stream);
    }
    token.cancel();
    assert!(matches!(
        stream.poll_event(Duration::from_secs(1)),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        stream.poll_event(Duration::from_secs(1)),
        Err(Error::SessionStopped)
    ));
    drop(stream);
    c.capture(output, &options()).unwrap();
    c.outputs(&options()).unwrap();
    server.settle();
    assert_eq!(server.metrics.cursor_sessions.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
}

#[test]
fn cursor_requires_a_seat_and_respects_image_allocation_limits() {
    let (_server, socket) = Server::start(Config::default());
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    assert!(matches!(
        c.cursor_stream(output, options()),
        Err(Error::Unsupported(_))
    ));
    let (_server, socket) = Server::start(Config {
        cursor: Some(CursorFault::None),
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let output = c.outputs(&options()).unwrap()[0].id;
    let mut opts = options();
    opts.limits.max_pixels = 1;
    let mut stream = c.cursor_stream(output, opts).unwrap();
    next(&mut stream);
    next(&mut stream);
    assert!(matches!(
        stream.poll_event(Duration::from_secs(1)),
        Err(Error::LimitExceeded)
    ));
}
