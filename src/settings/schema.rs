//! Every key boltsnap reads from `config.toml`, with what it accepts.

/// What a key accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Bool,
    Int {
        min: i64,
        max: i64,
    },
    Choice(&'static [&'static str]),
    /// Any string; `empty_ok: false` also rejects `""`.
    Text {
        empty_ok: bool,
    },
    /// A directory; `~` and `$VAR` are expanded when it is used.
    Path,
    /// `focused`, `both` or `output:<name>`.
    Target,
}

/// When a changed value takes effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    Live,
    NextRecording,
    ReplayRestart,
    DaemonStart,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// `record_fps`, or `replay.fps` for a key in a table.
    pub path: &'static str,
    pub kind: Kind,
    pub applies: Applies,
    /// An invalid value stops its user instead of falling back to the default:
    /// recording at another frame rate or a replay with other limits than
    /// configured would be worse than a clear error.
    pub strict: bool,
    /// Replaces the generated message for an invalid value, where an older
    /// parser said it differently.
    pub message: Option<&'static str>,
}

/// Tables that hold keys; any other table is unknown.
pub const TABLES: &[&str] = &["replay", "shelf"];

const fn key(path: &'static str, kind: Kind, applies: Applies) -> Key {
    Key {
        path,
        kind,
        applies,
        strict: false,
        message: None,
    }
}

const fn strict(path: &'static str, kind: Kind, applies: Applies) -> Key {
    Key {
        path,
        kind,
        applies,
        strict: true,
        message: None,
    }
}

const fn with_message(key: Key, message: &'static str) -> Key {
    Key {
        message: Some(message),
        ..key
    }
}

use Applies::*;
use Kind::*;

/// In the order the old replay parser checked them, so a file with several
/// invalid replay keys reports the same one first.
pub const KEYS: &[Key] = &[
    key("save_dir", Path, Live),
    key("record_codec", Text { empty_ok: true }, NextRecording),
    with_message(
        strict(
            "record_profile",
            Choice(&["quality", "quiet"]),
            NextRecording,
        ),
        "record_profile must be \"quality\" or \"quiet\"",
    ),
    with_message(
        strict("record_fps", Int { min: 1, max: 240 }, NextRecording),
        "record_fps must be a whole number from 1 to 240",
    ),
    key("record_dir", Path, Live),
    key("record_default_target", Target, NextRecording),
    key(
        "record_both_mode",
        Choice(&["separate", "combined"]),
        NextRecording,
    ),
    key(
        "record_cursor",
        Choice(&["system", "mellow", "quick"]),
        NextRecording,
    ),
    key("record_show_frame", Bool, Live),
    key("record_disk_add_to_shelf", Bool, Live),
    key("record_audio_enabled", Bool, NextRecording),
    key(
        "record_audio_source",
        Choice(&["system-and-mic", "mic", "system"]),
        NextRecording,
    ),
    // Zero or less turns it off, as it always did.
    key(
        "record_shelf_to_disk_after_seconds",
        Int {
            min: i64::MIN,
            max: i64::MAX,
        },
        NextRecording,
    ),
    key("ui_font", Text { empty_ok: true }, Live),
    key("shelf.card_width", Int { min: 140, max: 380 }, Live),
    key("shelf.visible_cards", Int { min: 1, max: 8 }, Live),
    key(
        "shelf.corner",
        Choice(&["bottom-left", "bottom-right", "top-left", "top-right"]),
        Live,
    ),
    key("copy_to_clipboard", Bool, Live),
    key("selector_magnifier", Choice(&["alt", "always"]), Live),
    key("shelf.open_with", Text { empty_ok: false }, Live),
    key("shelf.card_opacity", Int { min: 30, max: 100 }, Live),
    key("shelf.solid_on_hover", Bool, Live),
    key("shelf.card_shadow", Bool, Live),
    key("shelf.animations", Bool, Live),
    strict(
        "replay.duration_seconds",
        Int { min: 1, max: 600 },
        ReplayRestart,
    ),
    strict(
        "replay.memory_mib",
        Int { min: 64, max: 4096 },
        ReplayRestart,
    ),
    strict("replay.fps", Int { min: 1, max: 240 }, ReplayRestart),
    strict("replay.encoder", Text { empty_ok: false }, ReplayRestart),
    strict("replay.output", Text { empty_ok: false }, ReplayRestart),
    strict("replay.autostart", Bool, DaemonStart),
    strict(
        "replay.bitrate_kbps",
        Int {
            min: 500,
            max: 200_000,
        },
        ReplayRestart,
    ),
];

pub fn find(path: &str) -> Option<&'static Key> {
    KEYS.iter().find(|key| key.path == path)
}

/// `("replay", "fps")` for `replay.fps`, `(None, key)` at the top level.
pub fn split(path: &str) -> (Option<&str>, &str) {
    match path.split_once('.') {
        Some((table, key)) => (Some(table), key),
        None => (None, path),
    }
}
