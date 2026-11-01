//! Reads reserve their full allowance before admission and publish only at EOF.
use super::{ClipboardReceiveOutcome, ClipboardTransferId, MAX_BYTES, ReadAdmission, nonblocking};
use crate::{Error, Result};
use std::{
    io,
    os::fd::{AsRawFd, OwnedFd},
    sync::Arc,
    time::Instant,
};

pub(super) struct Read {
    pub(super) id: ClipboardTransferId,
    pub(super) fd: OwnedFd,
    pub(super) deadline: Instant,
    bytes: Vec<u8>,
    max: usize,
}

pub(super) struct Progress {
    pub(super) bytes: usize,
    pub(super) runnable: bool,
    pub(super) outcome: Option<ClipboardReceiveOutcome>,
}

impl Read {
    #[cfg(test)]
    pub(super) fn received(&self) -> usize {
        self.bytes.len()
    }

    pub(super) fn new(request: ReadAdmission) -> Result<Self> {
        if request.max_bytes == 0 || request.max_bytes > MAX_BYTES {
            return Err(Error::InvalidInput("clipboard receive byte limit"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(request.max_bytes + 1)
            .map_err(|_| Error::LimitExceeded)?;
        nonblocking(&request.fd)?;
        Ok(Self {
            id: request.id,
            fd: request.fd,
            deadline: request.deadline,
            bytes,
            max: request.max_bytes,
        })
    }

    pub(super) fn service(&mut self, now: Instant, allowance: usize) -> Progress {
        let mut progress = Progress {
            bytes: 0,
            runnable: false,
            outcome: None,
        };
        if now >= self.deadline {
            progress.outcome = Some(ClipboardReceiveOutcome::TimedOut);
            return progress;
        }
        let mut storage = [0u8; 8192];
        // Byte and syscall bounds also bound repeated interruptions in one turn.
        for _ in 0..32 {
            let count = storage
                .len()
                .min(allowance - progress.bytes)
                .min(self.max + 1 - self.bytes.len());
            if count == 0 {
                progress.runnable = true;
                return progress;
            }
            // SAFETY: storage is writable for count bytes and fd remains owned.
            let result =
                unsafe { libc::read(self.fd.as_raw_fd(), storage.as_mut_ptr().cast(), count) };
            if result > 0 {
                let bytes = result as usize;
                progress.bytes += bytes;
                self.bytes.extend_from_slice(&storage[..bytes]);
                if self.bytes.len() > self.max {
                    progress.outcome = Some(ClipboardReceiveOutcome::TooLarge);
                    return progress;
                }
            } else if result == 0 {
                progress.outcome = Some(ClipboardReceiveOutcome::Bytes(std::mem::take(
                    &mut self.bytes,
                )));
                return progress;
            } else {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::WouldBlock => return progress,
                    io::ErrorKind::Interrupted => continue,
                    _ => {
                        progress.outcome =
                            Some(ClipboardReceiveOutcome::Failed(Arc::new(error.into())));
                        return progress;
                    }
                }
            }
        }
        progress.runnable = true;
        progress
    }
}
