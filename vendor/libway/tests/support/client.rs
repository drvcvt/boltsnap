//! A cooperating toolkit stand-in: owns surfaces on the same connection as the `Dnd` session.
use libway::{
    Display,
    dnd::{Dnd, DndEvent},
};
use std::time::{Duration, Instant};
use wayland_client::{
    Dispatch, delegate_noop,
    protocol::{wl_callback, wl_compositor, wl_registry, wl_surface},
};

#[derive(Default)]
pub struct Client {
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub synced: bool,
}
impl Dispatch<wl_callback::WlCallback, ()> for Client {
    fn event(
        s: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &wayland_client::Connection,
        _: &wayland_client::QueueHandle<Self>,
    ) {
        s.synced = true;
    }
}
/// A roundtrip that cannot sleep on events another thread already read for this queue. The
/// Rust backend's `prepare_read` does not look at the queue, so `EventQueue::roundtrip` next to
/// libway's reader can block forever; this polls with a timeout and re-dispatches instead.
pub fn roundtrip_beside_reader(
    conn: &wayland_client::Connection,
    queue: &mut wayland_client::EventQueue<Client>,
    state: &mut Client,
) {
    use std::os::fd::AsRawFd;
    state.synced = false;
    conn.display().sync(&queue.handle(), ());
    let end = Instant::now() + Duration::from_secs(2);
    loop {
        queue.dispatch_pending(state).unwrap();
        if state.synced {
            return;
        }
        assert!(Instant::now() < end, "roundtrip timed out");
        conn.flush().unwrap();
        let Some(guard) = conn.prepare_read() else {
            continue;
        };
        let mut fd = libc::pollfd {
            fd: conn.backend().poll_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        if unsafe { libc::poll(&mut fd, 1, 10) } > 0 {
            let _ = guard.read();
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for Client {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &wayland_client::Connection,
        qh: &wayland_client::QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = e
            && interface == "wl_compositor"
        {
            s.compositor = Some(registry.bind(name, 4, qh, ()));
        }
    }
}
delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);

/// The test owns one surface on the same connection, like a cooperating toolkit would.
pub fn surface(display: &Display) -> wl_surface::WlSurface {
    let conn = display.connection().clone();
    let mut queue = conn.new_event_queue::<Client>();
    let _registry = conn.display().get_registry(&queue.handle(), ());
    let mut state = Client::default();
    queue.roundtrip(&mut state).unwrap();
    let surface = state
        .compositor
        .unwrap()
        .create_surface(&queue.handle(), ());
    conn.flush().unwrap();
    surface
}
/// Run the session until an event matches; returns everything seen on the way.
pub fn wait_for(dnd: &Dnd, mut pred: impl FnMut(&DndEvent) -> bool) -> Vec<DndEvent> {
    let end = Instant::now() + Duration::from_secs(2);
    let mut seen = Vec::new();
    while Instant::now() < end {
        dnd.run(Duration::from_millis(20)).unwrap();
        let events = dnd.events();
        let hit = events.iter().any(&mut pred);
        seen.extend(events);
        if hit {
            return seen;
        }
    }
    panic!("timed out waiting; saw {seen:?}");
}
