//! Replaces a replay session whose capture died.
//!
//! The recorder can lose its video stream while its process and its audio
//! stream keep going, and a buffer in that state can never produce a clip
//! again. Nothing downstream notices, so the service has to: it replaces the
//! session the way a manual Start would. Until a new session runs, the old
//! worker keeps its ring, which stays saveable up to where capture stopped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{SAVE_TIMEOUT, Session, holds_buffer, notify_error, session_in_use, socket_call};

/// How often the watchdog looks at a running session, and how many consecutive
/// unready looks it takes to call the capture dead.
const INTERVAL: Duration = Duration::from_secs(5);
const STRIKES: u32 = 3;

/// Waits after the n-th failed restart in a row, so a monitor that stays gone
/// or a full disk does not bring a notification and a probe every 15 s.
fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(match failures {
        0 | 1 => 30,
        2 => 60,
        _ => 300,
    })
}

/// Failed restarts of one session. Another session name (a successful
/// restart, a manual start, a stop) starts a new series.
#[derive(Default)]
struct Backoff {
    name: Option<String>,
    failures: u32,
    retry_at: Option<Instant>,
}

impl Backoff {
    fn due(&mut self, name: &str, now: Instant) -> bool {
        if self.name.as_deref() != Some(name) {
            *self = Self {
                name: Some(name.into()),
                ..Self::default()
            };
        }
        self.retry_at.is_none_or(|at| now >= at)
    }

    /// True for the first failure of a series, the one worth a notification.
    fn failed(&mut self, now: Instant) -> bool {
        self.failures += 1;
        self.retry_at = Some(now + retry_delay(self.failures));
        self.failures == 1
    }
}

pub(super) fn spawn(
    owner: Arc<Mutex<Option<Session>>>,
    stopping: Arc<AtomicBool>,
    cancel_start: Arc<AtomicBool>,
    starting: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        let mut strikes = 0;
        let mut backoff = Backoff::default();
        while !stopping.load(Ordering::Relaxed) {
            std::thread::sleep(INTERVAL);
            // Never interrupt a start in flight, and never take the buffer away
            // from a selection or an export that is still using it.
            if starting.load(Ordering::Relaxed) {
                strikes = 0;
                continue;
            }
            let Some(name) = owner.lock().unwrap().as_ref().map(|s| s.name.clone()) else {
                strikes = 0;
                backoff = Backoff::default();
                continue;
            };
            if session_in_use(&name) {
                strikes = 0;
                continue;
            }
            strikes += 1;
            if strikes < STRIKES || !backoff.due(&name, Instant::now()) {
                continue;
            }
            strikes = 0;
            let settings = match crate::config::Config::replay_settings() {
                Ok(settings) => settings,
                Err(error) => {
                    restart_failed(&mut backoff, &error);
                    continue;
                }
            };
            let mut guard = owner.lock().unwrap();
            // Only replace the session the strikes were counted against, and
            // ask once more under the lock: a selection may have opened since.
            let Some(old) = guard.as_mut().filter(|s| s.name == name) else {
                continue;
            };
            if session_in_use(&name) {
                continue;
            }
            // Only the dead capture goes, so the next gsr never shares the GPU
            // with it. The worker keeps the ring until a new session runs.
            old.stop_capture();
            starting.store(true, Ordering::Relaxed);
            cancel_start.store(false, Ordering::Relaxed);
            drop(guard);
            let restarted = super::start(settings, cancel_start.clone());
            let mut guard = owner.lock().unwrap();
            starting.store(false, Ordering::Relaxed);
            if cancel_start.load(Ordering::Relaxed) {
                // Stop took the old session; the new one goes with the lock
                // released.
                drop(guard);
                drop(restarted);
                continue;
            }
            match restarted {
                Ok(mut session) => {
                    let notice = session.notice.take();
                    let old = guard.replace(session);
                    drop(guard);
                    if let Some(old) = old {
                        retire(old);
                    }
                    if let Some(notice) = notice {
                        notify_error(&notice);
                    }
                }
                Err(error) => {
                    drop(guard);
                    restart_failed(&mut backoff, &error);
                }
            }
        }
    });
}

/// Logs every failed restart and notifies the first of a series.
fn restart_failed(backoff: &mut Backoff, error: &str) {
    eprintln!("boltsnap replay: restart failed: {error}");
    if backoff.failed(Instant::now()) {
        notify_error(&format!(
            "replay capture stopped and could not restart: {error}; \
             the buffer up to the stop can still be saved"
        ));
    }
}

/// Ends a replaced session once its worker holds nothing any more: an export
/// started on it while the new session was starting finishes first.
fn retire(old: Session) {
    std::thread::spawn(move || {
        let deadline = Instant::now() + SAVE_TIMEOUT;
        while Instant::now() < deadline
            && socket_call(&old.name, &json!({"version":1,"command":"status"}))
                .is_ok_and(|(status, _)| holds_buffer(&status))
        {
            std::thread::sleep(Duration::from_secs(1));
        }
        drop(old);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_back_off_and_notify_once_per_series() {
        let start = Instant::now();
        let mut backoff = Backoff::default();
        assert!(backoff.due("a", start));
        assert!(backoff.failed(start));
        let mut at = start;
        for (failure, delay) in [30, 60, 300, 300].into_iter().enumerate() {
            if failure > 0 {
                assert!(!backoff.failed(at));
            }
            let next = at + Duration::from_secs(delay);
            assert!(!backoff.due("a", next - Duration::from_secs(1)));
            assert!(backoff.due("a", next));
            at = next;
        }
    }

    #[test]
    fn another_session_starts_a_new_series() {
        let now = Instant::now();
        let mut backoff = Backoff::default();
        backoff.due("a", now);
        backoff.failed(now);
        backoff.failed(now);
        assert!(!backoff.due("a", now));
        assert!(backoff.due("b", now));
        assert!(backoff.failed(now), "the first failure of b notifies again");
        assert!(!backoff.due("b", now + Duration::from_secs(29)));
        assert!(backoff.due("b", now + Duration::from_secs(30)));
    }
}
