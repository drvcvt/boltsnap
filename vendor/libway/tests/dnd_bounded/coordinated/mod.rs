//! Caller-driven DnD waits with owned protocol and pipe peers.
mod backpressure;
mod mode;
mod wait;

use crate::support::{
    Config, DndConfig, PipePeer, Server,
    client::{Client, roundtrip_beside_reader, surface},
};
use libway::{
    Display, SurfaceHandle,
    dnd::{
        Accept, Action, Actions, CoordinatedDnd, DispatchReport, DndEvent, DndLimits, DndOptions,
        DragData, DragRequest, LocalRect, TargetId, TargetSpec, WaitView,
    },
};
use std::{
    fs::File,
    os::fd::AsRawFd,
    sync::mpsc::{Receiver, channel},
    time::{Duration, Instant},
};
use wayland_client::protocol::wl_surface;

const WATCHDOG: Duration = Duration::from_secs(2);

struct Fixture {
    server: Server,
    display: Display,
    dnd: CoordinatedDnd,
    origin: wl_surface::WlSurface,
    peers: Receiver<PipePeer>,
}

impl Fixture {
    fn new(limits: DndLimits) -> Self {
        let (tx, peers) = channel();
        let (server, socket) = Server::start(Config {
            ext: false,
            wlr: 0,
            outputs: 0,
            dnd: Some(DndConfig {
                controlled_pipes: Some(tx),
                ..Default::default()
            }),
            ..Default::default()
        });
        let display = Display::from_socket(socket).unwrap();
        let origin = surface(&display);
        let dnd = CoordinatedDnd::new(
            &display,
            DndOptions {
                limits,
                track_input: false,
            },
        )
        .unwrap();
        let mut fixture = Self {
            server,
            display,
            dnd,
            origin,
            peers,
        };
        let end = Instant::now() + WATCHDOG;
        loop {
            assert!(Instant::now() < end, "bootstrap timed out");
            fixture.barrier();
            if fixture
                .dnd
                .events()
                .iter()
                .any(|e| matches!(e, DndEvent::Ready { .. }))
            {
                break;
            }
        }
        fixture.barrier();
        assert!(fixture.dnd.events().is_empty());
        fixture
    }

    fn read_barrier(&self) {
        self.server.dnd().barrier();
        let mut queue = self.display.connection().new_event_queue::<Client>();
        roundtrip_beside_reader(
            self.display.connection(),
            &mut queue,
            &mut Client::default(),
        );
    }

    fn barrier(&mut self) -> DispatchReport {
        self.read_barrier();
        self.dnd.dispatch().unwrap()
    }

    fn request(&self, data: DragData) -> DragRequest {
        crate::request(&SurfaceHandle::from_surface(&self.origin), data)
    }

    fn target(&mut self) -> TargetId {
        self.dnd
            .register_target(
                &SurfaceHandle::from_surface(&self.origin),
                TargetSpec {
                    rect: LocalRect {
                        x: 0.,
                        y: 0.,
                        width: 100.,
                        height: 100.,
                    },
                    accepts: vec![Accept::Mime(vec!["a/b".into()])],
                    actions: Actions {
                        copy: true,
                        move_: false,
                        ask: false,
                    },
                    preferred: Action::Copy,
                    priority: 0,
                    enabled: true,
                },
            )
            .unwrap()
    }

    fn incoming(&mut self) -> (TargetId, File, u64) {
        let target = self.target();
        self.barrier();
        self.server.dnd().offer(&["a/b"], 1, &[]);
        self.server.dnd().enter(0, 1., 1.);
        self.barrier();
        self.barrier();
        self.dnd.events();
        self.server.dnd().drop();
        self.barrier();
        self.barrier();
        let PipePeer::Incoming { mime, write } = self.peers.recv_timeout(WATCHDOG).unwrap() else {
            panic!("expected the incoming writer");
        };
        assert_eq!(mime, "a/b");
        self.dnd.events();
        let inode = self.server.metrics.dnd.pipes.lock().unwrap()[0];
        (target, File::from(write), inode)
    }
}

fn poll_view(view: &WaitView<'_>, deadline: Instant) -> Vec<libc::pollfd> {
    let mut fds: Vec<_> = view
        .interests()
        .map(|interest| libc::pollfd {
            fd: interest.fd.as_raw_fd(),
            events: if interest.readable { libc::POLLIN } else { 0 }
                | if interest.writable { libc::POLLOUT } else { 0 },
            revents: 0,
        })
        .collect();
    loop {
        let nanos = deadline
            .saturating_duration_since(Instant::now())
            .as_nanos();
        let timeout = nanos.div_ceil(1_000_000).min(i32::MAX as u128) as i32;
        // SAFETY: poll borrows initialized entries; the view retains every live fd.
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        if result >= 0 {
            assert!(fds.iter().all(|fd| fd.revents & libc::POLLNVAL == 0));
            return fds;
        }
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::Interrupted
        );
        assert!(
            Instant::now() < deadline,
            "interrupted wait exceeded its watchdog"
        );
    }
}
