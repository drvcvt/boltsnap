//! The pages' values from the config, as `Config::load()` has them in
//! effect, and the replay buffer's state from its control.

use std::path::Path;

use boltsnap::app::pages::{AudioSource, Corner, Cursor, Display, Prefs};

use crate::config::{
    Config, RecordAudioSource, RecordBothMode, RecordCursor, RecordDefaultTarget,
    resolve_record_dir, resolve_save_dir,
};
use crate::shelf::layout;

/// `path` with the home folder written as `~`, as people type it.
pub(super) fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match home
        .as_deref()
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// `displays`: the connected monitors as (connector, label), from the
/// window's own Wayland outputs.
/// `path` as typed, with a leading `~` for the home folder.
pub(super) fn untilde(path: &str) -> std::path::PathBuf {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from);
    match (path.strip_prefix('~'), home) {
        (Some(rest), Some(home)) => home.join(rest.trim_start_matches('/')),
        _ => std::path::PathBuf::from(path),
    }
}

pub fn load(displays: Vec<(String, String)>) -> Prefs {
    let cfg = Config::load();
    let shelf = cfg.shelf_prefs();
    let recording = cfg.recording_prefs();
    let replay = Config::replay_settings().unwrap_or_default();
    let buffer = super::save::replay_state(replay.seconds, replay.memory_mib);
    Prefs {
        card_width: shelf.card.w,
        visible_cards: shelf.visible as u32,
        corner: match shelf.corner {
            layout::Corner::TopLeft => Corner::TopLeft,
            layout::Corner::TopRight => Corner::TopRight,
            layout::Corner::BottomLeft => Corner::BottomLeft,
            layout::Corner::BottomRight => Corner::BottomRight,
        },
        save_dir: tilde(&resolve_save_dir(None, &cfg)),
        both_combined: recording.both_mode == RecordBothMode::Combined,
        display: match &recording.default_target {
            RecordDefaultTarget::Focused => Display::Focused,
            RecordDefaultTarget::Both => Display::Both,
            RecordDefaultTarget::Output(name) => Display::Output(name.clone()),
        },
        displays,
        codec: cfg.record_codec.clone().unwrap_or_else(|| "auto".into()),
        codecs: super::save::codecs(),
        can_browse: super::folder::available(),
        ui_font: cfg.ui_font.clone().filter(|font| !font.is_empty()),
        fonts: super::save::fonts(),
        fps: cfg.record_profile().unwrap_or_default().fps(),
        cursor: match recording.cursor {
            RecordCursor::System => Cursor::System,
            RecordCursor::Mellow => Cursor::Mellow,
            RecordCursor::Quick => Cursor::Quick,
        },
        show_frame: recording.show_frame,
        audio: recording.audio_enabled,
        audio_source: match recording.audio_source {
            RecordAudioSource::SystemAndMic => AudioSource::SystemAndMic,
            RecordAudioSource::Mic => AudioSource::Mic,
            RecordAudioSource::System => AudioSource::System,
        },
        record_dir: tilde(&resolve_record_dir(&cfg)),
        disk_to_shelf: recording.disk_add_to_shelf,
        shelf_to_disk_after: cfg.shelf_to_disk_after().map(|after| after.as_secs()),
        replay_autostart: replay.autostart,
        replay_running: buffer.running,
        replay_stalled: buffer.stalled,
        replay_stale: buffer.stale,
        recording: super::save::recording(),
        set_keys: crate::config::settings_state()
            .doc
            .keys()
            .map(str::to_owned)
            .collect(),
        replay_seconds: replay.seconds,
        replay_memory_mib: replay.memory_mib,
        replay_bitrate: replay.bitrate_kbps,
        replay_fps: replay.fps,
    }
}
