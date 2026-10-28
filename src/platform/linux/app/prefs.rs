//! The pages' values from the config, as `Config::load()` has them in
//! effect, and the replay buffer's state from its control.

use std::path::Path;

use boltsnap::app::pages::{AudioSource, Corner, Cursor, Display, Prefs, Screen};

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

/// The editors to offer: those the daemon, which starts them, can find; the
/// window's own `PATH` when the daemon does not say.
fn editors(from_daemon: Option<Vec<String>>) -> Vec<(String, String)> {
    crate::platform::linux::shelf::EDITORS
        .iter()
        .filter(|editor| match &from_daemon {
            Some(names) => names.iter().any(|name| name == editor.key),
            None => crate::paths::has_cmd(editor.program),
        })
        .map(|editor| (editor.key.to_owned(), editor.label.to_owned()))
        .collect()
}

pub fn load(displays: Vec<Screen>) -> Prefs {
    let cfg = Config::load();
    let shelf = cfg.shelf_prefs();
    let recording = cfg.recording_prefs();
    let replay = Config::replay_settings().unwrap_or_default();
    let buffer =
        super::save::replay_state(replay.seconds, replay.memory_mib, replay.output.as_deref());
    Prefs {
        card_width: shelf.card.w,
        visible_cards: shelf.visible as u32,
        card_opacity: shelf.opacity.into(),
        solid_on_hover: shelf.solid_on_hover,
        card_shadow: shelf.shadow,
        card_animations: shelf.animations,
        card_hover_fade: shelf.hover_fade,
        corner: match shelf.corner {
            layout::Corner::TopLeft => Corner::TopLeft,
            layout::Corner::TopRight => Corner::TopRight,
            layout::Corner::BottomLeft => Corner::BottomLeft,
            layout::Corner::BottomRight => Corner::BottomRight,
        },
        save_dir: tilde(&resolve_save_dir(None, &cfg)),
        copy_to_clipboard: cfg.copy_to_clipboard(),
        magnifier_always: cfg.magnifier_always(),
        open_with: cfg.shelf_open_with.clone(),
        editors: editors(super::save::daemon_editors()),
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
        replay_output: replay.output.clone(),
        replay_encoder: replay.encoder.clone(),
    }
}

#[cfg(test)]
mod editor_tests {
    #[test]
    fn the_daemons_list_decides_which_editors_are_offered() {
        let names = |list: Vec<(String, String)>| -> Vec<String> {
            list.into_iter().map(|(name, _)| name).collect()
        };
        assert_eq!(
            names(super::editors(Some(vec!["gimp".into(), "eddy".into()]))),
            ["eddy", "gimp"],
            "in the list's order, only what the daemon can start"
        );
        assert!(super::editors(Some(Vec::new())).is_empty());
    }
}
