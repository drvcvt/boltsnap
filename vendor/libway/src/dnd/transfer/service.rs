//! Fair pipe slices under the shared session lock; protocol callbacks are separate.
use super::{DndLimits, Done, Incoming, Outgoing, Reactor, TransferError, expired};
use std::{
    io,
    os::fd::AsRawFd,
    time::{Duration, Instant},
};

const BYTES: usize = 256 * 1024;
const CHUNK: usize = 64 * 1024;
const TIME: Duration = Duration::from_millis(1);

struct Allowance {
    remaining: usize,
    until: Instant,
    progressed: bool,
    yielded: bool,
}
impl Allowance {
    fn new() -> Self {
        Self {
            remaining: BYTES,
            until: Instant::now() + TIME,
            progressed: false,
            yielded: false,
        }
    }
    fn take(&mut self) -> usize {
        if self.remaining == 0 || Instant::now() >= self.until {
            self.yielded = true;
            return 0;
        }
        self.remaining.min(CHUNK)
    }
    fn consume(&mut self, bytes: usize) {
        self.remaining -= bytes;
        self.progressed = true;
    }
}

impl Reactor {
    /// Share byte/time allowances across both directions, retaining exact offsets on yield.
    pub fn service(&mut self, now: Instant, limits: &DndLimits) -> Vec<Done> {
        self.service_with(now, limits, Allowance::new())
    }
    fn service_with(
        &mut self,
        now: Instant,
        limits: &DndLimits,
        mut allowance: Allowance,
    ) -> Vec<Done> {
        let mut done = Vec::new();
        if !self.incoming.is_empty() {
            self.incoming.rotate_left(1);
        }
        if !self.outgoing.is_empty() {
            self.outgoing.rotate_left(1);
        }
        // A busy direction cannot starve the other, nor a busy pipe its peers.
        if self.outgoing_first {
            self.write_slice(now, limits, &mut allowance, &mut done);
            self.read_slice(now, limits, &mut allowance, &mut done);
        } else {
            self.read_slice(now, limits, &mut allowance, &mut done);
            self.write_slice(now, limits, &mut allowance, &mut done);
        }
        self.outgoing_first = !self.outgoing_first;
        self.ready = allowance.yielded
            && allowance.progressed
            && (!self.incoming.is_empty() || !self.outgoing.is_empty());
        if !done.is_empty() {
            self.generation += 1;
        }
        done
    }
    fn read_slice(
        &mut self,
        now: Instant,
        limits: &DndLimits,
        allowance: &mut Allowance,
        done: &mut Vec<Done>,
    ) {
        let mut chunk = [0; CHUNK];
        self.incoming.retain_mut(|transfer| {
            let Some(result) = read(transfer, now, limits, allowance, &mut chunk) else {
                return true;
            };
            done.push(Done::Incoming {
                id: transfer.id,
                result,
            });
            false
        });
    }
    fn write_slice(
        &mut self,
        now: Instant,
        limits: &DndLimits,
        allowance: &mut Allowance,
        done: &mut Vec<Done>,
    ) {
        self.outgoing.retain_mut(|transfer| {
            if write(transfer, now, limits, allowance) {
                done.push(Done::Outgoing);
                return false;
            }
            true
        });
    }
}

fn read(
    t: &mut Incoming,
    now: Instant,
    limits: &DndLimits,
    allowance: &mut Allowance,
    chunk: &mut [u8; CHUNK],
) -> Option<Result<Vec<u8>, TransferError>> {
    loop {
        if expired(t.started, t.progressed, now.max(Instant::now()), limits) {
            return Some(Err(TransferError::Timeout));
        }
        let count = allowance.take();
        if count == 0 {
            return None;
        }
        // SAFETY: The owned fd is live and the writable slice has at least count bytes.
        let n = unsafe { libc::read(t.fd.as_raw_fd(), chunk.as_mut_ptr().cast(), count) };
        let current = now.max(Instant::now());
        if expired(t.started, t.progressed, current, limits) {
            return Some(Err(TransferError::Timeout));
        }
        if n > 0 {
            let n = n as usize;
            allowance.consume(n);
            if n > limits.max_payload.saturating_sub(t.buf.len()) {
                return Some(Err(TransferError::TooLarge));
            }
            t.buf.extend_from_slice(&chunk[..n]);
            t.progressed = current;
            continue;
        }
        if n == 0 {
            return Some(Ok(std::mem::take(&mut t.buf)));
        }
        match io::Error::last_os_error().kind() {
            io::ErrorKind::WouldBlock => return None,
            io::ErrorKind::Interrupted => continue,
            _ => return Some(Err(TransferError::Io)),
        }
    }
}
fn write(t: &mut Outgoing, now: Instant, limits: &DndLimits, allowance: &mut Allowance) -> bool {
    loop {
        if expired(t.started, t.progressed, now.max(Instant::now()), limits) {
            return true;
        }
        let rest = &t.data[t.offset..];
        if rest.is_empty() {
            return true;
        }
        let count = allowance.take().min(rest.len());
        if count == 0 {
            return false;
        }
        // SAFETY: The owned nonblocking fd is live and rest contains count readable bytes.
        // A closed reader yields EPIPE; Rust binaries ignore SIGPIPE by default.
        let n = unsafe { libc::write(t.fd.as_raw_fd(), rest.as_ptr().cast(), count) };
        if n > 0 {
            let n = n as usize;
            allowance.consume(n);
            t.offset += n;
            t.progressed = now.max(Instant::now());
            continue;
        }
        if n == 0 {
            return false;
        }
        match io::Error::last_os_error().kind() {
            io::ErrorKind::WouldBlock => return false,
            io::ErrorKind::Interrupted => continue,
            _ => return true,
        }
    }
}

#[cfg(test)]
mod tests;
