//! Real protocol and DnD queues share the pump; peer pipes remain test-owned.
use super::*;
use crate::{
    SurfaceHandle,
    dnd::{
        Accept, Action, Actions, DndEvent, DndOptions, LocalRect, Outcome, Payload, TargetId,
        TargetSpec,
    },
    support::{DndConfig, PipePeer, pipe_fds},
};
use std::{
    fs::File,
    io::Write,
    sync::mpsc::{Receiver, channel},
};
use wayland_client::{
    delegate_noop,
    protocol::{wl_compositor, wl_registry, wl_surface},
};

impl Dispatch<wl_registry::WlRegistry, ()> for SyncState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
            && interface == "wl_compositor"
        {
            state.compositor = Some(registry.bind(name, 4, qh, ()));
        }
    }
}

delegate_noop!(SyncState: ignore wl_compositor::WlCompositor);
delegate_noop!(SyncState: ignore wl_surface::WlSurface);

struct DndFixture {
    server: Server,
    pump: Pump,
    peers: Receiver<PipePeer>,
}

impl DndFixture {
    fn new() -> Self {
        let (tx, peers) = channel();
        let (server, socket) = Server::start(Config {
            dnd: Some(DndConfig {
                controlled_pipes: Some(tx),
                ..Default::default()
            }),
            ..config()
        });
        let mut pump = Pump::from_socket(socket, limits(), |connection, qh, _| {
            connection.display().get_registry(qh, ());
            SyncState::default()
        })
        .unwrap();
        read_barrier(&pump);
        pump.step(Some(Instant::now())).unwrap();
        pump.state.surface = Some(
            pump.state
                .compositor
                .as_ref()
                .unwrap()
                .create_surface(&pump.queue.handle(), ()),
        );
        pump.enable_dnd(DndOptions::default()).unwrap();
        let mut fixture = Self {
            server,
            pump,
            peers,
        };
        let end = Instant::now() + WATCHDOG;
        loop {
            assert!(Instant::now() < end, "DnD bootstrap timed out");
            fixture.barrier();
            if fixture
                .pump
                .dnd
                .as_mut()
                .unwrap()
                .events()
                .iter()
                .any(|event| matches!(event, DndEvent::Ready { .. }))
            {
                break;
            }
        }
        fixture.barrier();
        assert!(fixture.pump.dnd.as_mut().unwrap().events().is_empty());
        fixture.pump.trace = Trace::default();
        fixture
    }

    fn barrier(&mut self) -> super::super::Turn {
        self.server.dnd().barrier();
        read_barrier(&self.pump);
        self.pump.step(Some(Instant::now())).unwrap()
    }

    fn target(&mut self) -> TargetId {
        let surface = SurfaceHandle::from_surface(self.pump.state.surface.as_ref().unwrap());
        self.pump
            .dnd
            .as_mut()
            .unwrap()
            .register_target(
                &surface,
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

    fn offer(&self) {
        self.server.dnd().offer(&["a/b"], 1, &[]);
        self.server.dnd().enter(0, 1., 1.);
    }

    fn incoming(&mut self) -> (TargetId, File, u64) {
        let target = self.target();
        self.barrier();
        self.offer();
        self.barrier();
        self.barrier();
        self.pump.dnd.as_mut().unwrap().events();
        self.server.dnd().drop();
        self.barrier();
        self.barrier();
        let PipePeer::Incoming { mime, write } = self.peers.recv_timeout(WATCHDOG).unwrap() else {
            panic!("expected the controlled incoming writer");
        };
        assert_eq!(mime, "a/b");
        self.pump.dnd.as_mut().unwrap().events();
        let inode = self.server.metrics.dnd.pipes.lock().unwrap()[0];
        self.pump.trace = Trace::default();
        (target, File::from(write), inode)
    }
}

#[test]
fn already_read_protocol_only_dnd_only_and_both_are_serviced_before_poll() {
    for (protocol, dnd) in [(true, false), (false, true), (true, true)] {
        let mut fixture = DndFixture::new();
        fixture.target();
        fixture.barrier();
        if protocol {
            issue_sync(&fixture.pump, 101);
        }
        if dnd {
            fixture.offer();
        }
        fixture.server.dnd().barrier();
        read_barrier(&fixture.pump);
        fixture.pump.trace = Trace::default();
        let turn = fixture.pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
        assert_eq!(turn.callbacks, usize::from(protocol));
        assert_eq!(turn.events, usize::from(protocol));
        assert_eq!(turn.dnd_callbacks > 0, dnd);
        assert_eq!(turn.dnd_events > 0, dnd);
        assert_resolved(&fixture.pump.trace, 0, 0, 0, 0);
        assert_eq!(
            (fixture.pump.trace.dispatches, fixture.pump.trace.flushes),
            (1, 1)
        );
        let retained = fixture.pump.step(Some(Instant::now())).unwrap();
        assert_eq!((retained.callbacks, retained.dnd_callbacks), (0, 0));
        assert_eq!(retained.events, turn.events);
        assert_eq!(retained.dnd_events, turn.dnd_events);
        assert!(retained.runnable());
        assert_resolved(&fixture.pump.trace, 0, 0, 0, 0);
        assert_eq!(
            fixture.pump.state.calls,
            if protocol { vec![101] } else { vec![] }
        );
    }
}

#[test]
fn pipe_only_completion_cancels_the_socket_read_before_delivering_dnd_events() {
    let mut fixture = DndFixture::new();
    let (_, writer, inode) = fixture.incoming();
    assert_eq!(pipe_fds(inode), 2);
    let mut writer = Some(writer);
    // Only the peer writes here. No queue service or application callback is
    // performed by the fixture while the coordinator's read is reserved.
    fixture.pump.trace.hook = Some(Box::new(move |phase| {
        if phase == Phase::BeforePoll {
            assert_eq!(
                pipe_fds(inode),
                2,
                "wait interests must not duplicate pipe fds"
            );
            writer
                .as_mut()
                .unwrap()
                .write_all(b"pump-owned completion")
                .unwrap();
            drop(writer.take());
            assert_eq!(pipe_fds(inode), 1);
        }
    }));
    let turn = fixture.pump.step(Some(Instant::now() + WATCHDOG)).unwrap();
    fixture.pump.trace.hook = None;
    assert!(turn.wait.pipe);
    assert!(!turn.wait.read && !turn.wait.wake && !turn.wait.timed_out);
    assert_eq!(
        (
            turn.callbacks,
            turn.dnd_callbacks,
            turn.events,
            turn.dnd_events
        ),
        (0, 0, 0, 1)
    );
    assert_resolved(&fixture.pump.trace, 1, 0, 1, 1);
    assert_eq!(
        (fixture.pump.trace.dispatches, fixture.pump.trace.flushes),
        (2, 2)
    );
    assert_eq!(pipe_fds(inode), 0);
    accept_completed_drop(&mut fixture);
}

fn accept_completed_drop(fixture: &mut DndFixture) {
    let events = fixture.pump.dnd.as_mut().unwrap().events();
    let [
        DndEvent::Dropped {
            transfer, payload, ..
        },
    ] = events.as_slice()
    else {
        panic!("expected one delivered drop: {events:?}");
    };
    assert_eq!(
        payload,
        &Payload::Bytes {
            mime: "a/b".into(),
            data: b"pump-owned completion".to_vec(),
        }
    );
    fixture
        .pump
        .dnd
        .as_mut()
        .unwrap()
        .complete(*transfer, Outcome::Accepted(Action::Copy))
        .unwrap();
    fixture.barrier();
}

#[test]
fn cancelled_dnd_transfer_is_absent_from_the_next_connection_wait() {
    let mut fixture = DndFixture::new();
    let (target, writer, inode) = fixture.incoming();
    fixture
        .pump
        .dnd
        .as_mut()
        .unwrap()
        .unregister_target(target)
        .unwrap();
    fixture.pump.dnd.as_mut().unwrap().events();
    assert_eq!(pipe_fds(inode), 1);
    // Drain any protocol response to cancellation before measuring the quiet wait.
    fixture.barrier();
    fixture.pump.dnd.as_mut().unwrap().events();
    fixture.pump.trace = Trace::default();
    let turn = fixture.pump.step(Some(Instant::now())).unwrap();
    assert!(turn.wait.timed_out);
    assert!(!turn.wait.pipe && !turn.runnable());
    assert_resolved(&fixture.pump.trace, 1, 0, 1, 1);
    drop(writer);
    assert_eq!(pipe_fds(inode), 0);
}
