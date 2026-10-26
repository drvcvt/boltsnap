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

pub(super) struct Saver {
    writes: mpsc::Sender<Write>,
    thread: JoinHandle<()>,
}

impl Saver {
    /// `done` gets every write back with its outcome.
    pub(super) fn spawn(done: calloop::channel::Sender<Result<Write, String>>) -> Self {
        let (writes, queue) = mpsc::channel::<Write>();
        let thread = std::thread::spawn(move || {
            for write in queue {
                let result = send(&write).map(|()| write);
                if let Err(error) = &result {
                    eprintln!("boltsnap settings: {error}");
                }
                let _ = done.send(result);
            }
        });
        Self { writes, thread }
    }

    pub(super) fn send(&self, write: Write) {
        let _ = self.writes.send(write);
    }

    /// Wait until every write sent so far has gone out; the window is closing.
    pub(super) fn finish(self) {
        drop(self.writes);
        let _ = self.thread.join();
    }
}

fn send(write: &Write) -> Result<(), String> {
    match write {
        Write::Key(key, value) => {
            let response = call_daemon(Request::SetSettings {
                patch: vec![((*key).to_owned(), Change::Set(value.clone()))],
            })
            .map_err(|error| format!("Boltsnap is not running: {error}"))?;
            if response.ok {
                Ok(())
            } else {
                Err(response
                    .error
                    .unwrap_or_else(|| format!("{key} was not saved")))
            }
        }
        Write::Replay(on) => {
            let command = if *on { "start" } else { "stop" };
            super::super::replay::call(&json!({"version": 1, "command": command})).map(|_| ())
        }
    }
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

/// (running, stalled): whether the replay buffer is up, and up without video.
pub(super) fn replay_state() -> (bool, bool) {
    match super::super::replay::call(&json!({"version": 1, "command": "status"})) {
        Ok(status) => match status["state"].as_str() {
            Some("running" | "starting") => (true, false),
            Some("stalled") => (true, true),
            _ => (false, false),
        },
        Err(_) => (false, false),
    }
}
