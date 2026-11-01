//! Bounded nonblocking pipes; protocol ownership and result credits remain above.
mod read;
mod signal;
#[cfg(test)]
mod tests;
mod write;

use super::{ClipboardReceiveOutcome, ClipboardTransferId, Payload};
use crate::{Error, Result, runtime::Interest, window::InputSeatId};
use std::{
    os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
    time::Instant,
};

const READS: usize = 8;
const SENDS: usize = 16;
const MAX_BYTES: usize = 1024 * 1024;
const JOB_BYTES: usize = 64 * 1024;
const TURN_BYTES: usize = 256 * 1024;

pub(super) struct ReadAdmission {
    pub(super) id: ClipboardTransferId,
    pub(super) fd: OwnedFd,
    pub(super) max_bytes: usize,
    pub(super) deadline: Instant,
    pub(super) replace: Option<ClipboardTransferId>,
}

pub(super) struct Completion {
    pub(super) id: ClipboardTransferId,
    pub(super) outcome: ClipboardReceiveOutcome,
}

pub(super) struct Service {
    pub(super) completions: [Option<Completion>; READS],
    pub(super) runnable: bool,
    pub(super) bytes: usize,
    pub(super) failure: Option<Error>,
}

enum Job {
    Read(read::Read),
    Send(write::Send),
}

struct Progress {
    bytes: usize,
    runnable: bool,
    done: bool,
    completion: Option<Completion>,
    failure: Option<Error>,
}

impl Job {
    fn service(&mut self, now: Instant, allowance: usize) -> Progress {
        match self {
            Self::Read(read) => {
                let progress = read.service(now, allowance);
                Progress {
                    bytes: progress.bytes,
                    runnable: progress.runnable,
                    done: progress.outcome.is_some(),
                    completion: progress.outcome.map(|outcome| Completion {
                        id: read.id,
                        outcome,
                    }),
                    failure: None,
                }
            }
            Self::Send(send) => {
                let progress = send.service(now, allowance);
                Progress {
                    bytes: progress.bytes,
                    runnable: progress.runnable,
                    done: progress.done,
                    completion: None,
                    failure: progress.failure,
                }
            }
        }
    }

    fn deadline(&self) -> Instant {
        match self {
            Self::Read(job) => job.deadline,
            Self::Send(job) => job.deadline,
        }
    }

    fn interest(&self) -> Interest<'_> {
        match self {
            Self::Read(job) => Interest {
                fd: job.fd.as_fd(),
                readable: true,
                writable: false,
            },
            Self::Send(job) => Interest {
                fd: job.fd.as_fd(),
                readable: false,
                writable: true,
            },
        }
    }
}

#[derive(Default)]
pub(crate) struct Transfers {
    jobs: Vec<Job>,
    start: usize,
}

impl Transfers {
    pub(super) fn read(&mut self, request: ReadAdmission) -> Result<Option<Completion>> {
        let reads = self
            .jobs
            .iter()
            .filter(|job| matches!(job, Job::Read(_)))
            .count();
        let replacing = request
            .replace
            .is_some_and(|id| self.read_index(id).is_some());
        if reads == READS && !replacing {
            return Err(Error::LimitExceeded);
        }
        if self.read_index(request.id).is_some() {
            return Err(Error::InvalidInput("duplicate clipboard transfer"));
        }
        self.jobs.try_reserve(1).map_err(|_| Error::LimitExceeded)?;
        // All fallible setup precedes retirement of the previous accepted read.
        let replacement = request.replace;
        let job = read::Read::new(request)?;
        let cancelled = replacement.and_then(|id| self.cancel(id));
        self.jobs.push(Job::Read(job));
        Ok(cancelled)
    }

    pub(super) fn send(
        &mut self,
        fd: OwnedFd,
        payload: Payload,
        deadline: Instant,
        seat: InputSeatId,
    ) -> Result<()> {
        if self
            .jobs
            .iter()
            .filter(|job| matches!(job, Job::Send(_)))
            .count()
            == SENDS
        {
            return Err(Error::LimitExceeded);
        }
        self.jobs.try_reserve(1).map_err(|_| Error::LimitExceeded)?;
        nonblocking(&fd)?;
        self.jobs.push(Job::Send(write::Send {
            fd,
            payload,
            deadline,
            seat,
            offset: 0,
        }));
        Ok(())
    }

    pub(super) fn cancel(&mut self, id: ClipboardTransferId) -> Option<Completion> {
        let index = self.read_index(id)?;
        self.jobs.remove(index);
        Some(Completion {
            id,
            outcome: ClipboardReceiveOutcome::Cancelled,
        })
    }

    pub(super) fn cancel_sends(&mut self, seat: InputSeatId) {
        self.jobs
            .retain(|job| !matches!(job, Job::Send(send) if send.seat == seat));
    }

    pub(super) fn clear(&mut self) {
        self.jobs.clear();
        self.start = 0;
    }

    pub(crate) fn deadline(&self) -> Option<Instant> {
        self.jobs.iter().map(Job::deadline).min()
    }

    pub(crate) fn interests<'a>(&'a self, output: &mut Vec<Interest<'a>>) -> Result<()> {
        output
            .try_reserve(self.jobs.len())
            .map_err(|_| Error::LimitExceeded)?;
        output.extend(self.jobs.iter().map(Job::interest));
        Ok(())
    }

    fn read_index(&self, id: ClipboardTransferId) -> Option<usize> {
        self.jobs
            .iter()
            .position(|job| matches!(job, Job::Read(read) if read.id == id))
    }

    pub(super) fn service(&mut self, now: Instant) -> Service {
        self.service_slice(now, JOB_BYTES, TURN_BYTES)
    }

    fn service_slice(&mut self, now: Instant, job_bytes: usize, turn_bytes: usize) -> Service {
        let mut report = Service {
            completions: std::array::from_fn(|_| None),
            runnable: false,
            bytes: 0,
            failure: None,
        };
        let count = self.jobs.len();
        if count == 0 {
            return report;
        }
        // Rotate the vector before visiting; stable retain preserves that order.
        let start = self.start % count;
        self.jobs.rotate_left(start);
        let mut completed = 0;
        let mut visited = 0;
        self.jobs.retain_mut(|job| {
            if report.failure.is_some() {
                return true;
            }
            if report.bytes == turn_bytes && job.deadline() > now {
                report.runnable = true;
                return true;
            }
            visited += 1;
            let allowance = job_bytes.min(turn_bytes - report.bytes);
            let progress = job.service(now, allowance);
            report.bytes += progress.bytes;
            report.runnable |= progress.runnable;
            report.failure = progress.failure;
            if let Some(completion) = progress.completion {
                report.completions[completed] = Some(completion);
                completed += 1;
            }
            !progress.done
        });
        self.start = if self.jobs.is_empty() {
            0
        } else {
            if visited == count {
                1 % self.jobs.len()
            } else {
                visited % self.jobs.len()
            }
        };
        report
    }
}

pub(super) fn open_pipe() -> Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: pipe2 initializes exactly two entries; no fd is owned on failure.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: successful pipe2 created two unique descriptors owned by this call.
    let read = unsafe { OwnedFd::from_raw_fd(fds[0]) };
    // SAFETY: the second distinct pipe descriptor has not acquired another owner.
    let write = unsafe { OwnedFd::from_raw_fd(fds[1]) };
    Ok((read, write))
}

fn nonblocking(fd: &OwnedFd) -> Result<()> {
    // SAFETY: these fcntl commands take no pointer; fd stays owned throughout.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: flags came from F_GETFL for this live owned descriptor.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: F_GETFD takes no pointer and does not transfer descriptor ownership.
    let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: descriptor flags are preserved while adding close-on-exec.
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
