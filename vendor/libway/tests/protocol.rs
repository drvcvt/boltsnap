mod support;
use libway::*;
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
use support::*;
fn options() -> CaptureOptions {
    CaptureOptions {
        timeout: Duration::from_millis(500),
        ..Default::default()
    }
}
#[test]
fn ext_pixels_metadata_and_cleanup() {
    let (server, socket) = Server::start(Config::default());
    let opts = options();
    let mut conn = Connection::from_socket(socket, &opts).unwrap();
    let outputs = conn.outputs(&opts).unwrap();
    assert_eq!(outputs.len(), 1);
    assert_eq!(outputs[0].logical.x, -4);
    assert!(conn.capabilities().ext_output_capture);
    let frame = conn.capture(outputs[0].id, &opts).unwrap();
    assert_eq!(frame.backend, Backend::Ext);
    assert_eq!(frame.presentation_time, Some(Duration::new(123, 456)));
    assert_eq!(
        &frame.rgba8().unwrap()[..8],
        &[16, 32, 48, 255, 17, 32, 48, 255]
    );
    conn.outputs(&opts).unwrap();
    server.settle();
    assert_eq!(server.metrics.frames.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.sessions.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.sources.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
}
#[test]
fn wlr_versions_and_padding() {
    for version in [1, 2, 3] {
        let (_server, socket) = Server::start(Config {
            ext: false,
            wlr: version,
            ..Default::default()
        });
        let opts = options();
        let mut c = Connection::from_socket(socket, &opts).unwrap();
        let id = c.outputs(&opts).unwrap()[0].id;
        let frame = c.capture(id, &opts).unwrap();
        assert_eq!(frame.stride, 24);
        assert_eq!(frame.rgba8().unwrap().len(), 32);
        assert_eq!(&frame.rgba8().unwrap()[16..20], &[16, 33, 48, 255]);
    }
}
#[test]
fn ext_without_shm_falls_back_but_forced_ext_fails() {
    let (_server, socket) = Server::start(Config {
        fault: Fault::NoShm,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    assert_eq!(c.capture(id, &opts).unwrap().backend, Backend::Wlr);
    assert!(matches!(
        c.capture(
            id,
            &CaptureOptions {
                backend: Backend::Ext,
                ..opts
            }
        ),
        Err(Error::UnsupportedFormat)
    ));
}
#[test]
fn failures_are_prompt_and_cleanup() {
    for (ext, fault) in [
        (true, Fault::EarlyFail),
        (false, Fault::EarlyFail),
        (true, Fault::Stopped),
        (true, Fault::InvalidSize),
        (true, Fault::LayoutChange),
    ] {
        let (server, socket) = Server::start(Config {
            ext,
            fault,
            ..Default::default()
        });
        let opts = options();
        let mut c = Connection::from_socket(socket, &opts).unwrap();
        let id = c.outputs(&opts).unwrap()[0].id;
        let start = Instant::now();
        assert!(c.capture(id, &opts).is_err());
        assert!(start.elapsed() < Duration::from_millis(300));
        c.outputs(&opts).unwrap();
        server.settle();
        assert_eq!(server.metrics.frames.load(Ordering::SeqCst), 0);
        assert_eq!(server.metrics.sessions.load(Ordering::SeqCst), 0);
        assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn deadline_and_cancellation_do_not_leave_workers() {
    for cancel in [false, true] {
        let (server, socket) = Server::start(Config {
            fault: Fault::Stall,
            ..Default::default()
        });
        let mut opts = options();
        let mut c = Connection::from_socket(socket, &opts).unwrap();
        let id = c.outputs(&opts).unwrap()[0].id;
        opts.timeout = Duration::from_millis(80);
        let token = opts.cancellation.clone();
        let trigger = cancel.then(|| {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(25));
                token.cancel();
            })
        });
        let start = Instant::now();
        let result = c.capture(id, &opts);
        assert!(if cancel {
            matches!(result, Err(Error::Cancelled))
        } else {
            matches!(result, Err(Error::Timeout))
        });
        assert!(start.elapsed() < Duration::from_millis(250));
        if let Some(t) = trigger {
            t.join().unwrap();
        }
        c.outputs(&options()).unwrap();
        server.settle();
        assert_eq!(server.metrics.frames.load(Ordering::SeqCst), 0);
        assert_eq!(server.metrics.sessions.load(Ordering::SeqCst), 0);
    }
}
#[test]
fn persistent_stream_releases_frames_and_stops_on_idle_timeout() {
    let (server, socket) = Server::start(Config {
        fault: Fault::IdleSecond,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    {
        let mut stream = c
            .stream(
                id,
                CaptureOptions {
                    timeout: Duration::from_millis(80),
                    ..opts
                },
                BufferKind::Cpu,
            )
            .unwrap();
        assert!(stream.next_frame().is_ok());
        assert!(matches!(stream.next_frame(), Err(Error::Timeout)));
        assert!(matches!(stream.next_frame(), Err(Error::SessionStopped)));
    }
    c.outputs(&options()).unwrap();
    server.settle();
    assert_eq!(server.metrics.sessions.load(Ordering::SeqCst), 0);
    assert_eq!(server.metrics.frames.load(Ordering::SeqCst), 0);
}
#[test]
fn output_handles_cannot_cross_connections() {
    let (_a, sa) = Server::start(Config::default());
    let (_b, sb) = Server::start(Config::default());
    let opts = options();
    let mut a = Connection::from_socket(sa, &opts).unwrap();
    let mut b = Connection::from_socket(sb, &opts).unwrap();
    let id = a.outputs(&opts).unwrap()[0].id;
    assert!(matches!(b.capture(id, &opts), Err(Error::OutputGone)));
}
#[test]
fn discovery_also_has_a_deadline() {
    let (socket, _silent_server) = std::os::unix::net::UnixStream::pair().unwrap();
    let start = Instant::now();
    assert!(matches!(
        Connection::from_socket(
            socket,
            &CaptureOptions {
                timeout: Duration::from_millis(40),
                ..options()
            }
        ),
        Err(Error::Timeout)
    ));
    assert!(start.elapsed() < Duration::from_millis(200));
}
#[cfg(feature = "image")]
#[test]
fn desktop_negative_origin_region_and_y_invert() {
    let (_server, socket) = Server::start(Config {
        outputs: 2,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let desktop = c.capture_desktop(&opts).unwrap();
    assert_eq!(
        desktop.logical_bounds,
        Rect {
            x: -4,
            y: 0,
            width: 8,
            height: 2
        }
    );
    assert_eq!(desktop.image.dimensions(), (8, 2));
    assert_eq!(desktop.image.get_pixel(4, 0).0, [160, 176, 192, 255]);
    let crop = c
        .capture_region(
            Rect {
                x: -1,
                y: 0,
                width: 2,
                height: 2,
            },
            &opts,
        )
        .unwrap();
    assert_eq!(crop.image.get_pixel(0, 0).0, [19, 32, 48, 255]);
    assert_eq!(crop.image.get_pixel(1, 0).0, [160, 176, 192, 255]);
    let (_server, socket) = Server::start(Config {
        ext: false,
        inverted: true,
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    assert_eq!(
        c.capture(id, &opts)
            .unwrap()
            .to_image()
            .unwrap()
            .get_pixel(0, 0)
            .0,
        [16, 33, 48, 255]
    );
}

#[test]
fn changed_constraints_allocate_a_new_frame_and_preserve_the_old_one() {
    let (server, socket) = Server::start(Config {
        fault: Fault::ChangedConstraints,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    let mut stream = c.stream(id, opts, BufferKind::Cpu).unwrap();
    let first = stream.next_frame().unwrap();
    let original = first.rgba8().unwrap();
    let second = stream.next_frame().unwrap();
    assert_eq!((first.width, first.height), (4, 2));
    assert_eq!((second.width, second.height), (2, 3));
    assert_eq!(second.rgba8().unwrap().len(), 24);
    assert_eq!(first.rgba8().unwrap(), original);
    drop(stream);
    c.outputs(&options()).unwrap();
    server.settle();
    assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
    assert_eq!(first.rgba8().unwrap(), original);
}

#[test]
fn disconnected_server_fails_without_waiting_for_the_deadline() {
    let (server, socket) = Server::start(Config::default());
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    drop(server);
    let start = Instant::now();
    assert!(matches!(c.capture(id, &opts), Err(Error::Wayland(_))));
    assert!(start.elapsed() < Duration::from_millis(200));
}

#[test]
fn empty_and_excessive_output_sets_are_explicit_errors() {
    let (_server, socket) = Server::start(Config {
        outputs: 0,
        ..Default::default()
    });
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    assert!(c.outputs(&options()).unwrap().is_empty());
    #[cfg(feature = "image")]
    assert!(matches!(
        c.capture_desktop(&options()),
        Err(Error::NoOutputs)
    ));
    let (_server, socket) = Server::start(Config {
        outputs: 2,
        ..Default::default()
    });
    let opts = CaptureOptions {
        limits: Limits {
            max_outputs: 1,
            ..Default::default()
        },
        ..options()
    };
    assert!(matches!(
        Connection::from_socket(socket, &opts),
        Err(Error::LimitExceeded)
    ));
}

#[cfg(feature = "gpu")]
#[test]
#[ignore = "requires LIBWAY_TEST_RENDER_NODE; isolated server, no desktop capture"]
fn gpu_fd_transport_import_rejection_and_frame_lifetimes() {
    let path = std::env::var_os("LIBWAY_TEST_RENDER_NODE").expect("explicit render node");
    for ext in [true, false] {
        for reject_dma in [false, true] {
            let (server, socket) = Server::start(Config {
                ext,
                dma: true,
                reject_dma,
                ..Default::default()
            });
            let opts = CaptureOptions {
                backend: if ext { Backend::Ext } else { Backend::Wlr },
                ..options()
            };
            let mut c = Connection::from_socket(socket, &opts).unwrap();
            let id = c.outputs(&opts).unwrap()[0].id;
            let allocator = gpu::GpuAllocator::open(&path).unwrap();
            let result = c.capture_with(id, &opts, BufferKind::Gpu(allocator));
            if reject_dma {
                assert!(matches!(result, Err(Error::CaptureFailed(_))));
            } else {
                let frame = result.unwrap();
                assert!(matches!(frame.rgba8(), Err(Error::Unsupported(_))));
                let FrameStorage::Gpu(buffer) = &frame.storage else {
                    panic!("GPU storage expected")
                };
                c.outputs(&opts).unwrap();
                server.settle();
                assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
                assert!(!buffer.planes().is_empty());
                assert!(buffer.planes()[0].fd().try_clone_to_owned().is_ok());
                assert_eq!(server.metrics.imports.load(Ordering::SeqCst), 1);
            }
            c.outputs(&opts).unwrap();
            server.settle();
            assert_eq!(server.metrics.params.load(Ordering::SeqCst), 0);
            assert_eq!(server.metrics.buffers.load(Ordering::SeqCst), 0);
            assert_eq!(server.metrics.frames.load(Ordering::SeqCst), 0);
        }
    }
}

#[test]
fn output_removal_invalidates_capture_and_handles() {
    let (_server, socket) = Server::start(Config {
        fault: Fault::RemoveOutput,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    assert!(matches!(c.capture(id, &opts), Err(Error::LayoutChanged)));
    assert!(c.outputs(&opts).unwrap().is_empty());
    assert!(matches!(c.capture(id, &opts), Err(Error::OutputGone)));
}
#[test]
fn removed_capture_manager_is_not_reused() {
    let (_server, socket) = Server::start(Config {
        fault: Fault::RemoveExt,
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    assert_eq!(c.capture(id, &opts).unwrap().backend, Backend::Ext);
    assert!(!c.capabilities().ext_output_capture);
    assert_eq!(c.capture(id, &opts).unwrap().backend, Backend::Wlr);
}

#[cfg(feature = "image")]
#[test]
fn mixed_fractional_scales_compose_at_the_highest_density() {
    let (_server, socket) = Server::start(Config {
        outputs: 2,
        mode_sizes: vec![(4, 2), (6, 3)],
        ..Default::default()
    });
    let opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let desktop = c.capture_desktop(&opts).unwrap();
    assert_eq!(desktop.scale, 1.5);
    assert_eq!(desktop.image.dimensions(), (12, 3));
    assert_eq!(desktop.outputs[0].scale(), 1.0);
    assert_eq!(desktop.outputs[1].scale(), 1.5);
    assert_eq!(desktop.image.get_pixel(6, 0).0, [160, 176, 192, 255]);
    let crop = c
        .capture_region(
            Rect {
                x: -1,
                y: 0,
                width: 2,
                height: 2,
            },
            &opts,
        )
        .unwrap();
    assert_eq!(crop.image.dimensions(), (3, 3));
    assert_eq!(crop.image.get_pixel(2, 1).0, [160, 176, 192, 255]);
}

#[cfg(feature = "image")]
#[test]
fn composition_budget_rejects_excess_but_does_not_charge_an_unused_resize() {
    let (_server, socket) = Server::start(Config::default());
    let mut c = Connection::from_socket(socket, &options()).unwrap();
    let mut opts = options();
    opts.limits.max_bytes = 200;
    assert_eq!(c.capture_desktop(&opts).unwrap().image.dimensions(), (4, 2));
    opts.limits.max_bytes = 100;
    assert!(matches!(
        c.capture_desktop(&opts),
        Err(Error::LimitExceeded)
    ));
}

#[test]
fn removing_an_unselected_duplicate_manager_preserves_capture() {
    let (_server, socket) = Server::start(Config {
        fault: Fault::RemoveDuplicateExt,
        ..Default::default()
    });
    let opts = CaptureOptions {
        backend: Backend::Ext,
        ..options()
    };
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    let id = c.outputs(&opts).unwrap()[0].id;
    for _ in 0..2 {
        assert_eq!(c.capture(id, &opts).unwrap().backend, Backend::Ext);
        assert!(c.capabilities().ext_output_capture);
    }
}

#[cfg(feature = "image")]
#[test]
fn direct_desktop_matches_frame_for_every_transform() {
    use wayland_server::protocol::wl_output::Transform as T;
    for transform in [
        T::Normal,
        T::_90,
        T::_180,
        T::_270,
        T::Flipped,
        T::Flipped90,
        T::Flipped180,
        T::Flipped270,
    ] {
        let (_server, socket) = Server::start(Config {
            transform,
            ..Default::default()
        });
        let opts = options();
        let mut c = Connection::from_socket(socket, &opts).unwrap();
        let id = c.outputs(&opts).unwrap()[0].id;
        let expected = c.capture(id, &opts).unwrap().to_image().unwrap();
        let desktop = c.capture_desktop(&opts).unwrap();
        assert_eq!(desktop.image, expected, "{transform:?}");
        let region = c.capture_region(desktop.logical_bounds, &opts).unwrap();
        assert_eq!(region.image, expected, "{transform:?}");
    }
}

#[cfg(feature = "image")]
#[test]
fn anisotropic_resize_accounts_for_the_actual_intermediate() {
    let (_server, socket) = Server::start(Config {
        mode_sizes: vec![(100, 2)],
        ..Default::default()
    });
    let mut opts = options();
    let mut c = Connection::from_socket(socket, &opts).unwrap();
    // Vertical-first scratch is 100 * 2 * 16, not 4 * 2 * 16.
    opts.limits.max_bytes = 4000;
    assert!(matches!(
        c.capture_desktop(&opts),
        Err(Error::LimitExceeded)
    ));
    opts.limits.max_bytes = 6000;
    assert_eq!(c.capture_desktop(&opts).unwrap().image.dimensions(), (4, 2));
}
