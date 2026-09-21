//! Opt-in stage timings, correlated across CLI/daemon using a numeric request ID.
//! CLOCK_MONOTONIC timestamps share a clock domain on this Linux host. No pixels,
//! titles, output names or paths are logged. Disabled mode does not read a clock.
use std::sync::OnceLock;
use std::time::Instant;

fn monotonic_us() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: valid writable timespec, CLOCK_MONOTONIC is supported on Linux.
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000 + ts.tv_nsec as u64 / 1_000
}

pub fn request_id() -> Option<u64> {
    static ID: OnceLock<Option<u64>> = OnceLock::new();
    *ID.get_or_init(|| {
        std::env::var_os("BOLTSNAP_TIMINGS")
            .map(|_| (u64::from(std::process::id()) << 32) ^ monotonic_us())
    })
}

pub fn mark(stage: &'static str) {
    mark_request(stage, request_id());
}
pub fn mark_request(stage: &'static str, id: Option<u64>) {
    if let Some(id) = id {
        eprintln!(
            "boltsnap timing request={id} pid={} stage={stage} at_us={}",
            std::process::id(),
            monotonic_us()
        );
    }
}

pub struct Span(Option<(&'static str, u64, Instant)>);
impl Span {
    pub fn new(stage: &'static str) -> Self {
        Self::for_request(stage, request_id())
    }
    pub fn for_request(stage: &'static str, id: Option<u64>) -> Self {
        Self(id.map(|id| (stage, id, Instant::now())))
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, id, start)) = self.0 {
            eprintln!(
                "boltsnap timing request={id} pid={} stage={stage} at_us={} duration_us={}",
                std::process::id(),
                monotonic_us(),
                start.elapsed().as_micros()
            );
        }
    }
}
