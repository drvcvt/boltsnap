//! Private QueueState pipe interests join the existing prepared-read wait.
use super::*;
use crate::runtime::Interest;
use std::{
    fs::File,
    io::{Read, Write},
    os::fd::AsFd,
};
use wayland_client::delegate_noop;

struct PipeState {
    pipe: Option<File>,
    deadline: Instant,
    received: Option<Vec<u8>>,
    progress_only: bool,
}

impl QueueState for PipeState {
    fn pending(&self) -> usize {
        usize::from(!self.progress_only && self.received.is_some())
    }

    fn service_transfers(&mut self, now: Instant) -> crate::Result<bool> {
        let Some(pipe) = &mut self.pipe else {
            return Ok(false);
        };
        if now >= self.deadline {
            self.pipe = None;
            self.received = Some(Vec::new());
            return Ok(false);
        }
        let mut bytes = [0u8; 32];
        match pipe.read(&mut bytes) {
            Ok(count) => {
                self.received = Some(bytes[..count].to_vec());
                self.pipe = None;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        Ok(self.progress_only && self.received.is_some())
    }

    fn transfer_interests<'a>(&'a self, output: &mut Vec<Interest<'a>>) -> crate::Result<()> {
        if let Some(pipe) = &self.pipe {
            output
                .try_reserve(1)
                .map_err(|_| crate::Error::LimitExceeded)?;
            output.push(Interest {
                fd: pipe.as_fd(),
                readable: true,
                writable: false,
            });
        }
        Ok(())
    }

    fn transfer_deadline(&self) -> Option<Instant> {
        self.pipe.as_ref().map(|_| self.deadline)
    }

    fn stop(&mut self) {
        self.pipe = None;
    }
}

delegate_noop!(PipeState: ignore wl_callback::WlCallback);

fn pipe_fixture() -> (Server, Coordinator<PipeState, u32>, File) {
    let (server, socket) = Server::start(config());
    let mut fds = [-1; 2];
    // SAFETY: pipe2 initializes two unique descriptors into this live array.
    assert_eq!(
        unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
        0
    );
    use std::os::fd::FromRawFd;
    // SAFETY: each newly created descriptor acquires exactly one File owner.
    let read = unsafe { File::from_raw_fd(fds[0]) };
    // SAFETY: the second distinct descriptor is not owned elsewhere.
    let write = unsafe { File::from_raw_fd(fds[1]) };
    let pump = Coordinator::from_socket(socket, limits(), |_, _, _| PipeState {
        pipe: Some(read),
        deadline: Instant::now() + WATCHDOG,
        received: None,
        progress_only: false,
    })
    .unwrap();
    (server, pump, write)
}

#[test]
fn pipe_only_wakeup_cancels_the_connection_read_before_transfer_service() {
    let (_server, mut pump, peer) = pipe_fixture();
    let mut peer = Some(peer);
    pump.trace.hook = Some(Box::new(move |phase| {
        if phase == Phase::BeforePoll {
            peer.as_mut().unwrap().write_all(b"private pipe").unwrap();
            drop(peer.take());
        }
    }));
    let turn = pump.step(None).unwrap();
    assert_eq!(pump.state.received.as_deref(), Some(&b"private pipe"[..]));
    assert!(turn.wait.pipe && !turn.wait.read && !turn.wait.wake);
    assert_eq!(turn.callbacks, 0);
    assert_eq!(turn.events, 1);
    assert_resolved(&pump.trace, 1, 0, 1, 1);
    assert!(pump.state.pipe.is_none());
}

#[test]
fn blocked_transfer_is_not_runnable_and_caller_deadline_still_bounds_the_wait() {
    let (_server, mut pump, _peer) = pipe_fixture();
    let turn = pump.step(Some(Instant::now())).unwrap();
    assert!(!turn.runnable());
    assert!(turn.wait.timed_out && !turn.wait.pipe);
    assert!(pump.state.pipe.is_some() && pump.state.received.is_none());
    assert_resolved(&pump.trace, 1, 0, 1, 1);
    let end = pump.state.deadline;
    assert_eq!(super::super::wait::earliest(None, Some(end)), Some(end));
    assert_eq!(
        super::super::wait::earliest(Some(end + WATCHDOG), Some(end)),
        Some(end)
    );
}

#[test]
fn expired_transfer_is_serviced_before_sleep_and_removes_its_interest() {
    let (_server, mut pump, _peer) = pipe_fixture();
    pump.state.deadline = Instant::now();
    let turn = pump.step(None).unwrap();
    assert!(turn.runnable());
    assert_eq!(pump.state.received.as_deref(), Some(&[][..]));
    assert!(pump.state.transfer_deadline().is_none());
    assert_resolved(&pump.trace, 0, 0, 0, 0);
}

#[test]
fn byte_progress_without_events_returns_before_a_second_slice_or_wait() {
    let (_server, mut pump, mut peer) = pipe_fixture();
    pump.state.progress_only = true;
    peer.write_all(b"one slice").unwrap();
    let turn = pump.step(None).unwrap();
    assert_eq!(turn.events, 0);
    assert!(turn.transfers_ready && turn.runnable());
    assert_resolved(&pump.trace, 0, 0, 0, 0);
    assert_eq!(pump.trace.dispatches, 1);
    pump.trace = Trace::default();
    let idle = pump.step(Some(Instant::now())).unwrap();
    assert!(!idle.runnable() && idle.wait.timed_out);
    assert_resolved(&pump.trace, 1, 0, 1, 1);
}
