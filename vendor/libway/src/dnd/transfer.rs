//! Non-blocking pipe transfers with progress and total deadlines.
use super::{DndLimits, TransferId};
use crate::{Result, TransferError};
use std::sync::Arc;
use std::{
    io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    time::Instant,
};

pub(crate) enum Done {
    Incoming {
        id: TransferId,
        result: std::result::Result<Vec<u8>, TransferError>,
    },
    /// Nobody waits for sends; the drag ends through protocol events.
    Outgoing,
}

struct Incoming {
    id: TransferId,
    fd: OwnedFd,
    buf: Vec<u8>,
    started: Instant,
    progressed: Instant,
}
struct Outgoing {
    fd: OwnedFd,
    data: Arc<[u8]>,
    offset: usize,
    started: Instant,
    progressed: Instant,
}
#[derive(Default)]
pub(crate) struct Reactor {
    incoming: Vec<Incoming>,
    outgoing: Vec<Outgoing>,
    /// Bumped whenever the set of polled fds changes, so a sleeping reader can be told.
    pub generation: u64,
}
/// Both ends `CLOEXEC`; only the read end non-blocking so the foreign writer keeps blocking
/// semantics. Pipe capacity is raised best-effort so slow readers rarely stall the source.
pub(crate) fn open_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [0; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(io::Error::last_os_error().into());
    }
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
    set_nonblocking(&read);
    unsafe { libc::fcntl(write.as_raw_fd(), libc::F_SETPIPE_SZ, 1024 * 1024) };
    Ok((read, write))
}
pub(crate) fn set_nonblocking(fd: &OwnedFd) {
    unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}
fn expired(started: Instant, progressed: Instant, now: Instant, limits: &DndLimits) -> bool {
    now.saturating_duration_since(progressed) >= limits.inactivity
        || now.saturating_duration_since(started) >= limits.total
}
fn deadline(started: Instant, progressed: Instant, limits: &DndLimits) -> Result<Instant> {
    Ok(super::checked_deadline(progressed, limits.inactivity)?
        .min(super::checked_deadline(started, limits.total)?))
}
impl Reactor {
    /// `started` is when the drop's first read began: falling back to another type of the
    /// same offer keeps the drop's total deadline instead of starting a new one.
    pub fn push_incoming(&mut self, id: TransferId, fd: OwnedFd, started: Instant, now: Instant) {
        self.generation += 1;
        self.incoming.push(Incoming {
            id,
            fd,
            buf: Vec::new(),
            started,
            progressed: now,
        });
    }
    /// Writes never block the dispatching thread; the fd is switched to non-blocking.
    pub fn push_outgoing(&mut self, fd: OwnedFd, data: Arc<[u8]>, now: Instant) {
        self.generation += 1;
        set_nonblocking(&fd);
        self.outgoing.push(Outgoing {
            fd,
            data,
            offset: 0,
            started: now,
            progressed: now,
        });
    }
    pub fn outgoing(&self) -> usize {
        self.outgoing.len()
    }
    pub fn cancel_incoming(&mut self, id: TransferId) {
        self.generation += 1;
        self.incoming.retain(|t| t.id != id);
    }
    /// Index 0 is always the display fd; the second value is the next transfer deadline.
    pub fn poll_set(
        &self,
        display: RawFd,
        write_pending: bool,
        limits: &DndLimits,
    ) -> Result<(Vec<libc::pollfd>, Option<Instant>)> {
        let poll = |fd, events| libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let mut fds = vec![poll(
            display,
            libc::POLLIN | if write_pending { libc::POLLOUT } else { 0 },
        )];
        fds.extend(
            self.incoming
                .iter()
                .map(|t| poll(t.fd.as_raw_fd(), libc::POLLIN)),
        );
        fds.extend(
            self.outgoing
                .iter()
                .map(|t| poll(t.fd.as_raw_fd(), libc::POLLOUT)),
        );
        let incoming = self.incoming.iter().map(|t| (t.started, t.progressed));
        let outgoing = self.outgoing.iter().map(|t| (t.started, t.progressed));
        let mut next: Option<Instant> = None;
        for (started, progressed) in incoming.chain(outgoing) {
            let d = deadline(started, progressed, limits)?;
            next = Some(next.map_or(d, |next| next.min(d)));
        }
        Ok((fds, next))
    }
    /// Reads whatever is ready without blocking; returns finished transfers.
    pub fn service(&mut self, now: Instant, limits: &DndLimits) -> Vec<Done> {
        let mut done = Vec::new();
        let mut chunk = [0u8; 64 * 1024];
        self.incoming.retain_mut(|t| {
            let result = loop {
                if expired(t.started, t.progressed, now.max(Instant::now()), limits) {
                    break Some(Err(TransferError::Timeout));
                }
                let n =
                    unsafe { libc::read(t.fd.as_raw_fd(), chunk.as_mut_ptr().cast(), chunk.len()) };
                let current = now.max(Instant::now());
                if expired(t.started, t.progressed, current, limits) {
                    break Some(Err(TransferError::Timeout));
                }
                if n > 0 {
                    let n = n as usize;
                    if t.buf.len() + n > limits.max_payload {
                        break Some(Err(TransferError::TooLarge));
                    }
                    t.buf.extend_from_slice(&chunk[..n]);
                    t.progressed = current;
                    continue;
                }
                if n == 0 {
                    break Some(Ok(std::mem::take(&mut t.buf)));
                }
                match io::Error::last_os_error().kind() {
                    io::ErrorKind::WouldBlock => break None,
                    io::ErrorKind::Interrupted => continue,
                    _ => break Some(Err(TransferError::Io)),
                }
            };
            let Some(result) = result else {
                return true;
            };
            done.push(Done::Incoming { id: t.id, result });
            false
        });
        self.outgoing.retain_mut(|t| {
            let finished = loop {
                if expired(t.started, t.progressed, now.max(Instant::now()), limits) {
                    break true;
                }
                let rest = &t.data[t.offset..];
                if rest.is_empty() {
                    break true;
                }
                // A closed reader yields EPIPE; Rust binaries ignore SIGPIPE by default.
                let n = unsafe { libc::write(t.fd.as_raw_fd(), rest.as_ptr().cast(), rest.len()) };
                if n > 0 {
                    t.offset += n as usize;
                    t.progressed = now.max(Instant::now());
                    continue;
                }
                if n == 0 {
                    break false;
                }
                match io::Error::last_os_error().kind() {
                    io::ErrorKind::WouldBlock => break false,
                    io::ErrorKind::Interrupted => continue,
                    _ => break true,
                }
            };
            if finished || expired(t.started, t.progressed, now.max(Instant::now()), limits) {
                done.push(Done::Outgoing);
                return false;
            }
            true
        });
        if !done.is_empty() {
            self.generation += 1;
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
        time::Duration,
    };

    #[test]
    fn poll_interest_is_conditional_and_deadline_overflow_is_an_error() {
        let mut reactor = Reactor::default();
        let limits = DndLimits::default();
        for pending in [false, true, false] {
            let (fds, next) = reactor.poll_set(123, pending, &limits).unwrap();
            assert_eq!(fds[0].events & libc::POLLOUT != 0, pending);
            assert!(next.is_none());
        }
        let (read, _write) = open_pipe().unwrap();
        let now = Instant::now();
        reactor.push_incoming(TransferId(1, 1), read, now, now);
        let limits = DndLimits {
            total: Duration::MAX,
            ..limits
        };
        assert!(matches!(
            reactor.poll_set(123, false, &limits),
            Err(crate::Error::InvalidInput(_))
        ));
    }

    #[test]
    fn late_buffered_payload_and_eof_cannot_beat_deadlines() {
        for data in [b"late".as_slice(), b""] {
            for inactivity in [false, true] {
                let (read, write) = open_pipe().unwrap();
                let mut write = File::from(write);
                write.write_all(data).unwrap();
                drop(write);
                let mut reactor = Reactor::default();
                let started = Instant::now();
                let now = started + Duration::from_secs(1);
                let limits = DndLimits {
                    total: if inactivity {
                        Duration::from_secs(10)
                    } else {
                        Duration::from_secs(1)
                    },
                    inactivity: if inactivity {
                        Duration::from_secs(1)
                    } else {
                        Duration::from_secs(10)
                    },
                    ..Default::default()
                };
                reactor.push_incoming(TransferId(1, 1), read, started, started);
                assert!(
                    matches!(
                        reactor.service(now, &limits).as_slice(),
                        [Done::Incoming {
                            result: Err(TransferError::Timeout),
                            ..
                        }]
                    ),
                    "{data:?}, inactivity={inactivity}"
                );
            }
        }
    }

    #[test]
    fn live_payload_succeeds_and_expired_outgoing_closes_without_writing() {
        let mut reactor = Reactor::default();
        let (read, write) = open_pipe().unwrap();
        File::from(write).write_all(b"ok").unwrap();
        let now = Instant::now();
        reactor.push_incoming(TransferId(1, 1), read, now, now);
        assert!(
            matches!(reactor.service(now, &DndLimits::default()).as_slice(),
            [Done::Incoming { result: Ok(data), .. }] if data == b"ok")
        );
        let (read, write) = open_pipe().unwrap();
        reactor.push_outgoing(write, Arc::from(b"late".as_slice()), now);
        let limits = DndLimits {
            total: Duration::from_secs(1),
            ..Default::default()
        };
        assert!(matches!(
            reactor.service(now + limits.total, &limits).as_slice(),
            [Done::Outgoing]
        ));
        let mut bytes = Vec::new();
        File::from(read).read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
    }
}
