//! Each send retains its charged immutable source independently of replacement.
use super::{Payload, signal};
use crate::window::InputSeatId;
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    time::Instant,
};

pub(super) struct Send {
    pub(super) fd: OwnedFd,
    pub(super) payload: Payload,
    pub(super) deadline: Instant,
    pub(super) seat: InputSeatId,
    pub(super) offset: usize,
}

pub(super) struct Progress {
    pub(super) bytes: usize,
    pub(super) runnable: bool,
    pub(super) done: bool,
    pub(super) failure: Option<crate::Error>,
}

impl Send {
    pub(super) fn service(&mut self, now: Instant, allowance: usize) -> Progress {
        let mut progress = Progress {
            bytes: 0,
            runnable: false,
            done: now >= self.deadline || self.offset == self.payload.bytes().len(),
            failure: None,
        };
        if progress.done {
            return progress;
        }
        for _ in 0..32 {
            let count = allowance - progress.bytes;
            if count == 0 {
                progress.runnable = true;
                return progress;
            }
            let bytes = self.payload.bytes();
            let end = (self.offset + count).min(bytes.len());
            match signal::write(self.fd.as_raw_fd(), &bytes[self.offset..end]) {
                Ok(0) => {
                    progress.done = true;
                    return progress;
                }
                Ok(count) => {
                    self.offset += count;
                    progress.bytes += count;
                    if self.offset == bytes.len() {
                        progress.done = true;
                        return progress;
                    }
                }
                Err(signal::WriteError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => {
                    return progress;
                }
                Err(signal::WriteError::Io(error))
                    if error.kind() == io::ErrorKind::Interrupted =>
                {
                    continue;
                }
                Err(signal::WriteError::Io(_)) => {
                    progress.done = true;
                    return progress;
                }
                Err(signal::WriteError::Signal(error)) => {
                    progress.failure = Some(error.into());
                    progress.done = true;
                    return progress;
                }
            }
        }
        progress.runnable = true;
        progress
    }
}
