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
/// A session the watchdog started that dies sooner counts as one more failed
/// restart, so a capture that keeps dying backs off instead of restarting
/// every 15 s.
const FLAP: Duration = Duration::from_secs(300);

/// Waits after the n-th failed restart in a row, so a monitor that stays gone
/// or a full disk does not bring a notification and a probe every 15 s.
fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs(match failures {
        0 | 1 => 30,
        2 => 60,
        _ => 300,
    })
}

/// Failed restarts of one series. Another session name (a manual start, a
/// stop) starts a new one; a restart carries it on to the session it started.
#[derive(Default)]
struct Backoff {
    name: Option<String>,
    failures: u32,
    retry_at: Option<Instant>,
    /// When the watchdog started this session, until its first dead look.
    born: Option<Instant>,
}

impl Backoff {
    fn track(&mut self, name: &str) {
        if self.name.as_deref() != Some(name) {
            *self = Self {
                name: Some(name.into()),
                ..Self::default()
            };
        }
    }

    /// At the first dead look at a session the watchdog started: how long it
    /// lived if that was under `FLAP`, then a failed restart, and whether it
    /// is the first of its series. Living longer starts a new series.
    fn flapped(&mut self, name: &str, now: Instant) -> Option<(Duration, bool)> {
        self.track(name);
        let lived = now.saturating_duration_since(self.born.take()?);
        if lived >= FLAP {
            self.failures = 0;
            return None;
        }
        Some((lived, self.failed(now)))
    }

    fn due(&mut self, name: &str, now: Instant) -> bool {
        self.track(name);
        self.retry_at.is_none_or(|at| now >= at)
    }

    /// `name` replaced the dead session; its series goes on.
    fn restarted(&mut self, name: &str, now: Instant) {
        self.name = Some(name.into());
        self.retry_at = None;
        self.born = Some(now);
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
            if strikes < STRIKES {
                continue;
            }
            let now = Instant::now();
            if let Some((lived, first)) = backoff.flapped(&name, now) {
                let wait = retry_delay(backoff.failures).as_secs();
                log!(
                    "boltsnap replay: capture stopped again {} s after a restart; next try in {wait} s",
                    lived.as_secs()
                );
                if first {
                    notify_error(&format!(
                        "replay capture stopped again {} s after restarting; next try in {wait} s",
                        lived.as_secs()
                    ));
                }
            }
            if !backoff.due(&name, now) {
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
            let restarts = old.restarts + 1;
            log!("boltsnap replay: capture of {name} stopped; restarting");
            starting.store(true, Ordering::Relaxed);
            cancel_start.store(false, Ordering::Relaxed);
            drop(guard);
            let restarted = super::start::start(settings, cancel_start.clone());
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
                    log!("boltsnap replay: restarted as {}", session.name);
                    backoff.restarted(&session.name, Instant::now());
                    session.restarts = restarts;
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
    log!("boltsnap replay: restart failed: {error}");
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
    fn a_restart_that_dies_again_soon_backs_off_like_a_failed_one() {
        let start = Instant::now();
        let mut backoff = Backoff::default();
        assert!(backoff.due("a", start));
        backoff.restarted("b", start);
        // Dead 20 s after the restart: the series goes on, nothing for 30 s.
        let dead = start + Duration::from_secs(20);
        assert_eq!(
            backoff.flapped("b", dead),
            Some((Duration::from_secs(20), true))
        );
        assert!(!backoff.due("b", dead));
        assert!(backoff.due("b", dead + Duration::from_secs(30)));
        backoff.restarted("c", dead + Duration::from_secs(30));
        let dead = dead + Duration::from_secs(40);
        assert_eq!(
            backoff.flapped("c", dead),
            Some((Duration::from_secs(10), false))
        );
        assert!(!backoff.due("c", dead + Duration::from_secs(59)));
        assert!(backoff.due("c", dead + Duration::from_secs(60)));
        backoff.restarted("d", dead + Duration::from_secs(60));
        let dead = dead + Duration::from_secs(70);
        assert!(backoff.flapped("d", dead).is_some());
        assert!(!backoff.due("d", dead + Duration::from_secs(299)));
        assert!(backoff.due("d", dead + Duration::from_secs(300)));
    }

    #[test]
    fn a_restart_that_lived_long_starts_a_new_series() {
        let start = Instant::now();
        let mut backoff = Backoff::default();
        backoff.due("a", start);
        backoff.failed(start);
        backoff.restarted("b", start + Duration::from_secs(30));
        let dead = start + Duration::from_secs(30 + 360);
        assert_eq!(backoff.flapped("b", dead), None);
        assert!(backoff.due("b", dead));
        assert!(backoff.failed(dead), "a new series notifies again");
        // Only the first dead look counts.
        assert_eq!(backoff.flapped("b", dead), None);
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
