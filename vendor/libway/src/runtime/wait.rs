//! One prepared read and one poll, with borrows retaining every descriptor.
use super::{
    WaitReport,
    mailbox::Mailbox,
    trace::{Phase, Trace},
};
use crate::{Error, Result};
use std::{
    os::fd::{AsRawFd, BorrowedFd},
    time::Instant,
};
use wayland_client::{Connection, backend::ReadEventsGuard};

pub(crate) struct Interest<'a> {
    pub(crate) fd: BorrowedFd<'a>,
    pub(crate) readable: bool,
    pub(crate) writable: bool,
}

pub(super) struct WaitSet<'a> {
    pub(super) interests: Vec<Interest<'a>>,
    pub(super) deadline: Option<Instant>,
}

struct Reservation<'a> {
    guard: Option<ReadEventsGuard>,
    trace: &'a mut Trace,
}

impl Reservation<'_> {
    fn resolve(mut self, read: bool) -> Result<()> {
        let Some(guard) = self.guard.take() else {
            return Err(Error::SessionStopped);
        };
        let result = if read {
            guard.read().map(|_| ())
        } else {
            drop(guard);
            Ok(())
        };
        self.trace.resolved(read);
        match result {
            Ok(()) => Ok(()),
            Err(error) if super::would_block(&error) => Ok(()),
            Err(error) => Err(Error::Wayland(error.to_string())),
        }
    }
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if let Some(guard) = self.guard.take() {
            drop(guard);
            self.trace.resolved(false);
        }
    }
}

pub(super) fn run<C>(
    connection: &Connection,
    mailbox: &Mailbox<C>,
    set: WaitSet<'_>,
    trace: &mut Trace,
) -> Result<WaitReport> {
    trace.phase(Phase::BeforePrepare);
    let Some(guard) = connection.prepare_read() else {
        connection
            .backend()
            .dispatch_inner_queue()
            .map_err(|error| Error::Wayland(error.to_string()))?;
        return Ok(WaitReport {
            retry: true,
            ..WaitReport::default()
        });
    };
    trace.prepare();
    let reservation = Reservation {
        guard: Some(guard),
        trace,
    };
    reservation.trace.phase(Phase::AfterPrepare);
    if mailbox.has_pending().map_err(super::mailbox_error)? {
        return Ok(WaitReport {
            #[cfg(test)]
            wake: true,
            ..WaitReport::default()
        });
    }
    reservation.trace.phase(Phase::BeforePoll);
    reservation.trace.poll();
    #[cfg(test)]
    let fault = reservation.trace.poll_fault.take();
    let report = poll(
        &set,
        #[cfg(test)]
        fault,
    )?;
    reservation.resolve(report.read)?;
    Ok(report)
}

fn poll(
    set: &WaitSet<'_>,
    #[cfg(test)] fault: Option<super::trace::PollFault>,
) -> Result<WaitReport> {
    let mut fds: Vec<_> = set
        .interests
        .iter()
        .map(|interest| libc::pollfd {
            fd: interest.fd.as_raw_fd(),
            events: (if interest.readable { libc::POLLIN } else { 0 })
                | (if interest.writable { libc::POLLOUT } else { 0 }),
            revents: 0,
        })
        .collect();
    let (ret, error) = poll_fds(
        &mut fds,
        timeout(set.deadline),
        #[cfg(test)]
        fault,
    );
    if ret < 0 {
        if error.kind() == std::io::ErrorKind::Interrupted {
            return Ok(WaitReport {
                #[cfg(test)]
                interrupted: true,
                ..WaitReport::default()
            });
        }
        return Err(error.into());
    }
    if fds.iter().any(|fd| fd.revents & libc::POLLNVAL != 0) {
        return Err(std::io::Error::from_raw_os_error(libc::EBADF).into());
    }
    Ok(readiness(&fds, ret == 0))
}

fn poll_fds(
    fds: &mut [libc::pollfd],
    timeout: i32,
    #[cfg(test)] fault: Option<super::trace::PollFault>,
) -> (i32, std::io::Error) {
    #[cfg(test)]
    if let Some(fault) = fault {
        use super::trace::PollFault;
        return match fault {
            PollFault::Interrupted => (-1, std::io::Error::from_raw_os_error(libc::EINTR)),
            PollFault::Fatal => (-1, std::io::Error::from_raw_os_error(libc::EIO)),
            PollFault::Invalid => {
                fds[0].revents = libc::POLLNVAL;
                (1, std::io::Error::from_raw_os_error(0))
            }
        };
    }
    // SAFETY: the initialized poll array is live and every fd has a retained borrow.
    let ret = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
    (ret, std::io::Error::last_os_error())
}

fn readiness(fds: &[libc::pollfd], _timed_out: bool) -> WaitReport {
    // The builder always supplies the connection first and wake last.
    let connection = fds.first().map_or(0, |fd| fd.revents);
    WaitReport {
        read: connection & (libc::POLLIN | libc::POLLERR | libc::POLLHUP) != 0,
        #[cfg(test)]
        writable: connection & libc::POLLOUT != 0,
        #[cfg(test)]
        pipe: fds
            .iter()
            .skip(1)
            .take(fds.len().saturating_sub(2))
            .any(|fd| fd.revents != 0),
        #[cfg(test)]
        wake: fds.last().is_some_and(|fd| fd.revents != 0),
        #[cfg(test)]
        timed_out: _timed_out,
        ..WaitReport::default()
    }
}

pub(super) fn earliest(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn timeout(deadline: Option<Instant>) -> i32 {
    deadline.map_or(-1, |deadline| {
        let millis = deadline
            .saturating_duration_since(Instant::now())
            .as_nanos()
            .div_ceil(1_000_000);
        millis.min(i32::MAX as u128) as i32
    })
}
