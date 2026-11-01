//! Finite synthetic protocol traces; no live display or timing sleeps are used.
mod pipes;
mod protocol;
#[cfg(feature = "dnd")]
mod transfer;
mod wake;

use super::{
    Coordinator, QueueState,
    mailbox::Limits,
    trace::{Phase, Trace},
};
use crate::support::{Config, Server};
use std::{
    os::fd::AsRawFd,
    time::{Duration, Instant},
};
use wayland_client::{Connection, Dispatch, QueueHandle, protocol::wl_callback};

const WATCHDOG: Duration = Duration::from_secs(2);
type Pump = Coordinator<SyncState, u32>;

#[derive(Default)]
struct SyncState {
    events: Vec<u32>,
    calls: Vec<u32>,
    #[cfg(feature = "dnd")]
    compositor: Option<wayland_client::protocol::wl_compositor::WlCompositor>,
    #[cfg(feature = "dnd")]
    surface: Option<wayland_client::protocol::wl_surface::WlSurface>,
}

impl QueueState for SyncState {
    fn pending(&self) -> usize {
        self.events.len()
    }
}

impl Dispatch<wl_callback::WlCallback, u32> for SyncState {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        tag: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.calls.push(*tag);
        state.events.push(*tag);
    }
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

fn limits() -> Limits {
    Limits {
        max_queued: 8,
        max_bytes: 64,
    }
}

fn config() -> Config {
    Config {
        ext: false,
        wlr: 0,
        outputs: 0,
        ..Default::default()
    }
}

fn fixture() -> (Server, Pump) {
    let (server, socket) = Server::start(config());
    let pump = Pump::from_socket(socket, limits(), |_, _, _| SyncState::default()).unwrap();
    (server, pump)
}

fn issue_sync(pump: &Pump, tag: u32) {
    pump.display
        .connection()
        .display()
        .sync(&pump.queue.handle(), tag);
}

/// Read preceding wire events into their destination queues, dispatching only this
/// separate barrier queue. Coordinator trace assertions start after this setup.
fn read_barrier(pump: &Pump) {
    let connection = pump.display.connection();
    let mut queue = connection.new_event_queue::<BarrierState>();
    connection.display().sync(&queue.handle(), ());
    let mut state = BarrierState::default();
    let deadline = Instant::now() + WATCHDOG;
    loop {
        queue.dispatch_pending(&mut state).unwrap();
        if state.done {
            return;
        }
        assert!(Instant::now() < deadline, "wire barrier timed out");
        connection.flush().unwrap();
        let Some(guard) = connection.prepare_read() else {
            continue;
        };
        let mut fd = libc::pollfd {
            fd: connection.backend().poll_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            .div_ceil(1_000_000)
            .min(i32::MAX as u128) as i32;
        // SAFETY: the connection retains this fd and the initialized pollfd is
        // writable for one entry; the read reservation is resolved before dispatch.
        let result = unsafe { libc::poll(&mut fd, 1, timeout) };
        if result < 0 {
            assert_eq!(
                std::io::Error::last_os_error().kind(),
                std::io::ErrorKind::Interrupted
            );
            drop(guard);
        } else {
            assert!(result > 0, "wire barrier timed out");
            guard.read().unwrap();
        }
    }
}

fn assert_resolved(trace: &Trace, prepared: usize, reads: usize, cancelled: usize, polls: usize) {
    assert_eq!(trace.prepared, prepared);
    assert_eq!(trace.reads, reads);
    assert_eq!(trace.cancelled, cancelled);
    assert_eq!(trace.polls, polls);
    assert_eq!(trace.prepared, trace.reads + trace.cancelled);
    assert!(!trace.reserved, "no reservation may escape a turn");
}
