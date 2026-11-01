//! Actual wire traces for the owned role boundary, without live windows or buffers.
mod clipboard;
mod commands;
#[path = "../input/tests/mod.rs"]
mod input;
mod lifetime;
#[cfg(feature = "owned-native-system")]
mod native;
mod output;
mod peer;
mod protocol;
mod scaling;
mod shell;
#[path = "../text_input/tests/mod.rs"]
mod text_input;
mod wake;

use super::*;
use crate::Error;
use peer::{Event as PeerEvent, Options as PeerOptions, Peer, Request};
use std::{
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    backend::{ReadEventsGuard, WaylandError},
    protocol::wl_callback,
};

const WATCHDOG: Duration = Duration::from_secs(2);

struct Fixture {
    host: Host,
    peer: Peer,
}

impl Fixture {
    fn new() -> Self {
        let (peer, socket) = Peer::start(PeerOptions::default());
        let mut host = Host::from_socket(socket).unwrap();
        ready(&mut host);
        let fixture = Self { host, peer };
        assert_eq!(
            fixture.peer.take_requests(),
            [Request::BindCompositor, Request::BindShell]
        );
        fixture
    }

    fn with_options(options: PeerOptions) -> Self {
        let (peer, socket) = Peer::start(options);
        let mut host = Host::from_socket(socket).unwrap();
        ready(&mut host);
        let fixture = Self { host, peer };
        fixture.peer.take_requests();
        fixture
    }

    fn configure(&mut self, id: WindowId, native: usize, width: i32, height: i32) -> Configuration {
        self.peer.configure(native, width, height, 73, vec![]);
        self.sync();
        self.host.events();
        self.host.configuration(id).unwrap().unwrap()
    }

    fn sync(&mut self) -> Vec<Request> {
        read_barrier(&self.host);
        self.host.pump(Some(Instant::now())).unwrap();
        self.peer.requests()
    }

    fn create(&mut self, width: u32, height: u32) -> WindowId {
        let id = self
            .host
            .create_window(WindowOptions {
                preferred_width: width,
                preferred_height: height,
                title: "owned".into(),
                app_id: "test.libway".into(),
            })
            .unwrap();
        self.sync();
        id
    }
}

fn ready(host: &mut Host) {
    let deadline = Instant::now() + WATCHDOG;
    loop {
        assert!(Instant::now() < deadline, "window discovery timed out");
        host.pump(Some(deadline)).unwrap();
        let events = host.events();
        if events.contains(&WindowEvent::Ready) {
            return;
        }
    }
}

fn assert_children_destroyed(peer: &Peer, index: usize) {
    assert_eq!(
        peer.take_requests(),
        [
            Request::DestroyToplevel(index),
            Request::DestroyRole(index),
            Request::DestroySurface(index),
        ]
    );
}

#[derive(Default)]
struct BarrierState {
    done: bool,
}

impl Dispatch<wl_callback::WlCallback, ()> for BarrierState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.done = true;
    }
}

/// A separate internal queue reads preceding native events but only dispatches
/// its sync callback. The Host dispatches its own queue after the guard resolves.
fn read_barrier(host: &Host) {
    let connection = &host.coordinator.state.connection;
    let mut queue = connection.new_event_queue::<BarrierState>();
    connection.display().sync(&queue.handle(), ());
    let mut state = BarrierState::default();
    let deadline = Instant::now() + WATCHDOG;
    loop {
        queue.dispatch_pending(&mut state).unwrap();
        if state.done {
            return;
        }
        assert!(Instant::now() < deadline, "owned wire barrier timed out");
        let write_pending = match connection.flush() {
            Ok(()) => false,
            Err(WaylandError::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => true,
            Err(error) => panic!("owned wire barrier flush failed: {error}"),
        };
        let Some(guard) = connection.prepare_read() else {
            continue;
        };
        if barrier_readable(&guard, write_pending, deadline) {
            // read consumes the guard even when readiness raced with EAGAIN.
            barrier_read(guard);
        } else {
            // Writable-only readiness or EINTR cancels before flushing or dispatch.
            drop(guard);
        }
    }
}

fn barrier_readable(guard: &ReadEventsGuard, write_pending: bool, deadline: Instant) -> bool {
    let mut fd = libc::pollfd {
        fd: guard.connection_fd().as_raw_fd(),
        events: libc::POLLIN | if write_pending { libc::POLLOUT } else { 0 },
        revents: 0,
    };
    let timeout = deadline
        .saturating_duration_since(Instant::now())
        .as_nanos()
        .div_ceil(1_000_000)
        .min(i32::MAX as u128) as i32;
    // SAFETY: guard retains the borrowed descriptor throughout this poll and fd
    // is initialized for one writable entry. No callback dispatch occurs here.
    let result = unsafe { libc::poll(&mut fd, 1, timeout) };
    if result < 0 {
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::Interrupted
        );
        return false;
    }
    assert!(result > 0, "owned wire barrier timed out");
    assert_eq!(
        fd.revents & libc::POLLNVAL,
        0,
        "owned wire barrier invalid fd"
    );
    fd.revents & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0
}

fn barrier_read(guard: ReadEventsGuard) {
    match guard.read() {
        Ok(_) => {}
        Err(WaylandError::Io(error))
            if matches!(
                error.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
            ) => {}
        Err(error) => panic!("owned wire barrier read failed: {error}"),
    }
}
