//! Real socket capacity determines flush readiness while only the owned server pauses.
//! The test releases each borrowed view before flushing and uses no timing sleeps.
use super::*;

fn fill_socket(fixture: &mut Fixture) {
    let fd = fixture.display.connection().backend().poll_fd().as_raw_fd();
    let size: libc::c_int = 4096;
    // SAFETY: setsockopt borrows a live owned connection and one initialized integer.
    assert_eq!(
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_SNDBUF,
                (&size as *const libc::c_int).cast(),
                std::mem::size_of_val(&size) as _,
            )
        },
        0
    );
    for _ in 0..256 {
        for _ in 0..256 {
            fixture.origin.damage(0, 0, 1, 1);
        }
        if fixture.dnd.dispatch().unwrap().write_pending {
            return;
        }
    }
    panic!("bounded request corpus did not produce real socket backpressure");
}

#[test]
fn real_socket_backpressure_adds_then_clears_connection_writability() {
    let mut fixture = Fixture::new(DndLimits::default());
    fixture.barrier();
    let paused = fixture.server.dnd().pause();
    fill_socket(&mut fixture);
    let view = fixture.dnd.wait_view().unwrap();
    assert_eq!(view.interests().len(), 1);
    assert!(view.interests().next().unwrap().writable);
    assert!(view.deadline().is_none());
    assert!(!view.immediate());
    assert!(
        poll_view(&view, Instant::now())
            .iter()
            .all(|fd| fd.revents == 0)
    );
    drop(view);
    drop(paused);
    let end = Instant::now() + WATCHDOG;
    loop {
        assert!(Instant::now() < end, "flush recovery timed out");
        let guard = fixture.display.connection().prepare_read().unwrap();
        let view = fixture.dnd.wait_view().unwrap();
        let ready = poll_view(&view, end);
        assert_ne!(ready[0].revents & libc::POLLOUT, 0);
        assert_eq!(ready[0].revents & libc::POLLIN, 0);
        drop(guard);
        drop(view);
        let report = fixture.dnd.dispatch().unwrap();
        assert_eq!(report.callbacks, 0);
        if !report.write_pending {
            break;
        }
    }
    let view = fixture.dnd.wait_view().unwrap();
    assert!(!view.interests().next().unwrap().writable);
    assert!(view.deadline().is_none());
    drop(view);
    fixture.barrier();
}
