//! Saving from the window (settings plan, Phase 5b): one thread sends each
//! write in order, to the daemon or to the replay control, and hands back
//! what came of it. A replay start takes seconds, so none of this runs on
//! the window's loop.

use std::sync::mpsc;
use std::thread::JoinHandle;

use boltsnap::app::Write;
use boltsnap::app::pages::Notice;
use boltsnap::settings::Change;
use serde_json::json;

use crate::ipc::{Request, call_daemon};

/// Which row a write came from: (page, block).
pub(super) type Tag = (usize, usize);

pub(super) struct Saver {
    writes: mpsc::Sender<(Tag, Write)>,
    thread: JoinHandle<()>,
}

impl Saver {
    /// `done` gets every write's tag back with its outcome.
    pub(super) fn spawn(done: calloop::channel::Sender<(Tag, Result<(), String>)>) -> Self {
        let (writes, queue) = mpsc::channel::<(Tag, Write)>();
        let thread = std::thread::spawn(move || {
            for (tag, write) in queue {
                let result = send(&write);
                if let Err(error) = &result {
                    eprintln!("boltsnap settings: {error}");
                }
                let _ = done.send((tag, result));
            }
        });
        Self { writes, thread }
    }

    pub(super) fn send(&self, tag: Tag, write: Write) {
        let _ = self.writes.send((tag, write));
    }

    /// Wait until every write sent so far has gone out; the window is closing.
    pub(super) fn finish(self) {
        drop(self.writes);
        let _ = self.thread.join();
    }
}

/// The replay session in which a key that only a restart applies was
/// written: that session runs with old settings.
static STALE_SESSION: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Replay keys the running buffer does not show and `status` cannot tell
/// apart (length and memory it can).
const RESTART_ONLY: [&str; 4] = [
    "replay.fps",
    "replay.encoder",
    "replay.output",
    "replay.bitrate_kbps",
];

fn replay(command: &str) -> Result<serde_json::Value, String> {
    super::super::replay::call(&json!({"version": 1, "command": command}))
}

/// One change to the daemon; its refusal as the reason.
fn patch(key: &str, change: Change) -> Result<(), String> {
    let response = call_daemon(Request::SetSettings {
        patch: vec![(key.to_owned(), change)],
    })
    .map_err(|error| format!("Boltsnap is not running: {error}"))?;
    if response.ok {
        Ok(())
    } else {
        Err(refusal(
            key,
            response
                .error
                .unwrap_or_else(|| format!("{key} was not saved")),
        ))
    }
}

/// The daemon's refusal as the banner says it: a key this window knows but
/// the daemon does not means the running Boltsnap is an older build.
fn refusal(key: &str, error: String) -> String {
    if error == format!("unknown setting {key}") && boltsnap::settings::schema::find(key).is_some()
    {
        "Boltsnap is older than this window. Restart it to save this".into()
    } else {
        error
    }
}

fn send(write: &Write) -> Result<(), String> {
    match write {
        Write::RestartReplay => {
            replay("stop")?;
            replay("start").map(|_| ())
        }
        // Setting a key or removing it (a reset, "Auto", "Like recordings"):
        // either way the running buffer keeps its old settings.
        Write::Reset(key) | Write::Key(key, _) => {
            let change = match write {
                Write::Key(_, value) => Change::Set(value.clone()),
                _ => Change::Remove,
            };
            let saved = patch(key, change);
            if saved.is_ok() && RESTART_ONLY.contains(key) {
                let session = replay("status")
                    .ok()
                    .and_then(|status| status["session"].as_str().map(str::to_owned));
                if session.is_some() {
                    *STALE_SESSION.lock().unwrap() = session;
                }
            }
            saved
        }
        Write::Replay(on) => replay(if *on { "start" } else { "stop" }).map(|_| ()),
    }
}

/// The editors the daemon can start, by name; `None` without a daemon or
/// from one that does not say.
pub(super) fn daemon_editors() -> Option<Vec<String>> {
    call_daemon(Request::GetSettings).ok()?.editors
}

/// Why nothing can be saved right now, if something stops it.
pub(super) fn notice() -> Option<Notice> {
    let locked = |text: String| Some(Notice { text, locked: true });
    match call_daemon(Request::GetSettings) {
        Err(error) => locked(format!(
            "Boltsnap is not running, so changes cannot be saved: {error}"
        )),
        Ok(response) => match response.settings.and_then(|state| state.broken) {
            Some(broken) => locked(match broken.line {
                Some(line) => {
                    format!(
                        "config.toml has an error on line {line}, fix it to change settings here"
                    )
                }
                None => "config.toml cannot be read, fix it to change settings here".into(),
            }),
            None => None,
        },
    }
}

/// Follow the daemon: `changed` gets a signal for every settings state it
/// sends, for every change of the recording state (not its running time),
/// and when the daemon goes away; then each watch starts again every 2 s
/// (starting the daemon, like any command).
pub(super) fn watch(changed: calloop::channel::Sender<()>) {
    follow(changed.clone(), crate::ipc::watch_settings, |_| true);
    let mut last = None;
    follow(changed, crate::ipc::watch_recording, move |line| {
        let state = serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .map(|value| value["state"].clone());
        let new = state != last;
        last = state;
        new
    });
}

fn follow(
    changed: calloop::channel::Sender<()>,
    connect: fn() -> std::io::Result<std::os::unix::net::UnixStream>,
    mut tells: impl FnMut(&str) -> bool + Send + 'static,
) {
    std::thread::spawn(move || {
        loop {
            if let Ok(stream) = connect() {
                use std::io::BufRead;
                for line in std::io::BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if tells(&line) && changed.send(()).is_err() {
                        return;
                    }
                }
            }
            if changed.send(()).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    });
}

static CODECS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// What gpu-screen-recorder can encode here, once [`probe_codecs`] has
/// asked it.
pub(super) fn codecs() -> Option<Vec<String>> {
    CODECS.get().cloned()
}

/// Ask gpu-screen-recorder once, off the window's loop (`--info` takes about
/// half a second); `changed` gets a signal when the answer is in.
pub(super) fn probe_codecs(changed: calloop::channel::Sender<()>) {
    std::thread::spawn(move || {
        if let Ok(info) = super::super::gsr::info() {
            let _ = CODECS.set(info.codecs);
            let _ = changed.send(());
        }
    });
}

static FONTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// The installed font families, once [`probe_fonts`] has listed them.
pub(super) fn fonts() -> Option<Vec<String>> {
    FONTS.get().cloned()
}

/// List the families with fontconfig (`fc-list : family`, about 10 ms here)
/// off the window's loop: the first name of each line, sorted, once each.
pub(super) fn probe_fonts(changed: calloop::channel::Sender<()>) {
    std::thread::spawn(move || {
        let Ok(output) = std::process::Command::new("fc-list")
            .args([":", "family"])
            .output()
        else {
            return;
        };
        let mut families: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| line.split(',').next())
            .map(str::trim)
            .filter(|family| !family.is_empty())
            .map(str::to_owned)
            .collect();
        families.sort_by_key(|family| family.to_lowercase());
        families.dedup();
        let _ = FONTS.set(families);
        let _ = changed.send(());
    });
}

/// How many seconds the replay buffer holds, once a second, for the bar on
/// the replay page; `None` while it does not run. Asks the control socket
/// directly (`replay::status_quiet`), so it never starts the daemon.
/// Once a second: what the replay buffer holds, and what it costs
/// (`load.rs`), both `None` while it is off.
pub(super) fn poll_replay(
    held: calloop::channel::Sender<(Option<f32>, Option<boltsnap::app::ReplayLoad>)>,
) {
    std::thread::spawn(move || {
        let mut meter = super::load::Meter::new();
        loop {
            let status = super::super::replay::status_quiet()
                .filter(|status| matches!(status["state"].as_str(), Some("running" | "stalled")));
            let seconds = status
                .as_ref()
                .and_then(|status| status["duration_us"].as_u64())
                .map(|us| us as f32 / 1e6);
            let load = status
                .as_ref()
                .and_then(|status| status["session"].as_str())
                .and_then(|session| meter.sample(session));
            if held.send((seconds, load)).is_err() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    });
}

/// The replay buffer as its control reports it.
#[derive(Default)]
pub(super) struct ReplayState {
    pub running: bool,
    /// Up, but no video arrives.
    pub stalled: bool,
    /// Up with other settings than `seconds` and `memory_mib` say, or after
    /// a restart-only key was written in this session.
    pub stale: bool,
}

/// What the buffer runs with, against what the file asks for: `output` is
/// `[replay] output` (`None`: like recordings, which the buffer resolved
/// itself and is not compared).
pub(super) fn replay_state(seconds: u64, memory_mib: usize, output: Option<&str>) -> ReplayState {
    let Ok(status) = replay("status") else {
        return ReplayState::default();
    };
    let (running, stalled) = match status["state"].as_str() {
        Some("running" | "starting") => (true, false),
        Some("stalled") => (true, true),
        _ => (false, false),
    };
    // The worker keeps a third of the memory for the live buffer
    // (`replay/worker/src/live.rs`, `budget = memory / 3`).
    let budget = (memory_mib * 1024 * 1024 / 3) as u64;
    let session = status["session"].as_str();
    let stale = running
        && (status["seconds"].as_u64().is_some_and(|s| s != seconds)
            || status["budget_bytes"].as_u64().is_some_and(|b| b != budget)
            || other_display(&status, output)
            || session.is_some() && STALE_SESSION.lock().unwrap().as_deref() == session);
    ReplayState {
        running,
        stalled,
        stale,
    }
}

/// The buffer captures another display than the file names: changed in
/// another window, or before this one opened, it would otherwise look current
/// (Nutzer 28.10.: both displays set, the buffer kept DP-3, no restart offered).
/// Compared with what the session was started for (`asked`), not with what
/// it captures: "both" on displays of different scales runs on one on
/// purpose, and a restart would not change that. An older daemon says only
/// what it captures.
fn other_display(status: &serde_json::Value, output: Option<&str>) -> bool {
    let started = match status.get("asked") {
        Some(asked) => asked.as_str(),
        None => status["output"].as_str(),
    };
    match (output, started) {
        (Some(wanted), Some(started)) => wanted != started,
        (Some(_), None) => status.get("asked").is_some_and(serde_json::Value::is_null),
        _ => false,
    }
}

/// Whether a recording runs now.
pub(super) fn recording() -> bool {
    crate::ipc::call_daemon(Request::RecordingStatus)
        .ok()
        .and_then(|response| response.snapshot)
        .is_some_and(|snapshot| snapshot.state != crate::protocol::PublicRecordingState::Idle)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_buffer_on_another_display_than_the_file_asks_for_a_restart() {
        let running = |output: &str| serde_json::json!({"output": output});
        assert!(super::other_display(&running("DP-3"), Some("both")));
        assert!(super::other_display(&running("both"), Some("DP-3")));
        assert!(!super::other_display(&running("both"), Some("both")));
        assert!(!super::other_display(&running("DP-3"), Some("DP-3")));
        // Like recordings: the buffer chose, nothing to compare.
        assert!(!super::other_display(&running("DP-3"), None));
        assert!(!super::other_display(&serde_json::json!({}), Some("DP-3")));
        // Asked for both, running on one (different scales): no restart helps.
        let fell_back = serde_json::json!({"output": "DP-1", "asked": "both"});
        assert!(!super::other_display(&fell_back, Some("both")));
        // Started like recordings, the file now names a display.
        let inherited = serde_json::json!({"output": "DP-3", "asked": null});
        assert!(super::other_display(&inherited, Some("DP-3")));
    }

    #[test]
    fn a_key_only_an_older_daemon_lacks_asks_for_a_restart() {
        assert_eq!(
            super::refusal("shelf.open_with", "unknown setting shelf.open_with".into()),
            "Boltsnap is older than this window. Restart it to save this"
        );
        // A key neither knows, and other refusals, stay as they are.
        assert_eq!(
            super::refusal("nope", "unknown setting nope".into()),
            "unknown setting nope"
        );
        assert_eq!(super::refusal("save_dir", "disk full".into()), "disk full");
    }
}
