use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use boltsnap::settings::{self, Loaded, schema};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordDefaultTarget {
    Focused,
    Output(String),
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordBothMode {
    Separate,
    Combined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordAudioSource {
    SystemAndMic,
    Mic,
    System,
}

/// How recordings show the pointer. Smooth modes record without the cursor and
/// draw a spring-smoothed one on save.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordCursor {
    #[default]
    System,
    Mellow,
    Quick,
}

impl RecordCursor {
    pub fn key(self) -> &'static str {
        match self {
            RecordCursor::System => "system",
            RecordCursor::Mellow => "mellow",
            RecordCursor::Quick => "quick",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecordProfile {
    #[default]
    Quality,
    Quiet,
    /// `record_fps`: an explicit frame rate.
    Fps(u32),
}

impl RecordProfile {
    pub fn fps(self) -> u32 {
        match self {
            RecordProfile::Quality => 240,
            RecordProfile::Quiet => 60,
            RecordProfile::Fps(fps) => fps,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingPrefs {
    pub default_target: RecordDefaultTarget,
    pub both_mode: RecordBothMode,
    pub show_frame: bool,
    pub disk_add_to_shelf: bool,
    pub audio_enabled: bool,
    pub audio_source: RecordAudioSource,
    pub cursor: RecordCursor,
}

impl Default for RecordingPrefs {
    fn default() -> Self {
        Self {
            default_target: RecordDefaultTarget::Focused,
            both_mode: RecordBothMode::Separate,
            show_frame: true,
            disk_add_to_shelf: true,
            audio_enabled: true,
            audio_source: RecordAudioSource::SystemAndMic,
            cursor: RecordCursor::System,
        }
    }
}

/// Parsed `~/.config/boltsnap/config.toml`. A missing file, an unset key or an
/// invalid value leaves the field `None`; defaults are applied by the
/// `resolve_*` helpers. What was invalid is kept in `issues`.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Config {
    pub save_dir: Option<String>,
    pub record_codec: Option<String>,
    record_profile: Option<String>,
    record_fps: Option<u32>,
    pub record_dir: Option<String>,
    record_default_target: Option<String>,
    record_both_mode: Option<String>,
    record_cursor: Option<String>,
    record_show_frame: Option<bool>,
    record_disk_add_to_shelf: Option<bool>,
    record_audio_enabled: Option<bool>,
    record_audio_source: Option<String>,
    record_shelf_to_disk_after_seconds: Option<i64>,
    /// Font family for boltsnap's own chrome (selector overlay, shelf). Unset
    /// follows the desktop font.
    pub ui_font: Option<String>,
    issues: Vec<settings::Issue>,
}

impl Config {
    /// `record_fps` (1-240) overrides the profile's frame rate. An invalid
    /// value of either is an error, never a recording at another rate.
    pub fn record_profile(&self) -> Result<RecordProfile, String> {
        let issue = |key: &str| self.issues.iter().find(|issue| issue.key == key);
        if let Some(issue) = issue("record_fps") {
            return Err(issue.message.clone());
        }
        if let Some(fps) = self.record_fps {
            return Ok(RecordProfile::Fps(fps));
        }
        if let Some(issue) = issue("record_profile") {
            return Err(issue.message.clone());
        }
        Ok(match self.record_profile.as_deref() {
            Some("quiet") => RecordProfile::Quiet,
            _ => RecordProfile::Quality,
        })
    }

    pub fn replay_settings() -> Result<boltsnap::replay::settings::Settings, String> {
        let text = match std::fs::read_to_string(config_path()) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(format!("read replay settings: {error}")),
        };
        match remember(&text) {
            Read::Broken {
                fallback: Some(good),
            } => Self::parse_replay_settings(&good),
            // With nothing better known, the broken text gives its error.
            _ => Self::parse_replay_settings(&text),
        }
    }

    /// Every `[replay]` key is strict: the first invalid one is the error.
    fn parse_replay_settings(text: &str) -> Result<boltsnap::replay::settings::Settings, String> {
        let (doc, issues) = match settings::load(text) {
            Loaded::Ok { doc, issues } => (doc, issues),
            Loaded::Broken { message, .. } => return Err(format!("invalid config: {message}")),
        };
        let replay_issue = issues.iter().find(|issue| {
            issue.key == "replay"
                || issue.key.starts_with("replay.")
                    && schema::find(&issue.key).is_some_and(|key| key.strict)
        });
        if let Some(issue) = replay_issue {
            return Err(issue.message.clone());
        }
        // The schema checked every range, so the casts cannot truncate.
        let defaults = boltsnap::replay::settings::Settings::default();
        Ok(boltsnap::replay::settings::Settings {
            seconds: doc
                .int("replay.duration_seconds")
                .map_or(defaults.seconds, |n| n as u64),
            memory_mib: doc
                .int("replay.memory_mib")
                .map_or(defaults.memory_mib, |n| n as usize),
            fps: doc.int("replay.fps").map_or(defaults.fps, |n| n as u32),
            encoder: doc
                .text("replay.encoder")
                .map_or(defaults.encoder, String::from),
            output: doc.text("replay.output").map(String::from),
            autostart: doc.bool("replay.autostart").unwrap_or(defaults.autostart),
            bitrate_kbps: doc.int("replay.bitrate_kbps").map(|n| n as u32),
        })
    }

    /// Parse config text. Unknown keys are ignored; text that does not parse
    /// yields defaults (so a typo never bricks the daemon).
    pub fn parse(text: &str) -> Config {
        match settings::load(text) {
            Loaded::Ok { doc, issues } => Config::from_doc(&doc, issues),
            Loaded::Broken { .. } => Config::default(),
        }
    }

    fn from_doc(doc: &settings::Doc, issues: Vec<settings::Issue>) -> Config {
        let text = |key| doc.text(key).map(String::from);
        Config {
            save_dir: text("save_dir"),
            record_codec: text("record_codec"),
            record_profile: text("record_profile"),
            record_fps: doc.int("record_fps").map(|fps| fps as u32),
            record_dir: text("record_dir"),
            record_default_target: text("record_default_target"),
            record_both_mode: text("record_both_mode"),
            record_cursor: text("record_cursor"),
            record_show_frame: doc.bool("record_show_frame"),
            record_disk_add_to_shelf: doc.bool("record_disk_add_to_shelf"),
            record_audio_enabled: doc.bool("record_audio_enabled"),
            record_audio_source: text("record_audio_source"),
            record_shelf_to_disk_after_seconds: doc.int("record_shelf_to_disk_after_seconds"),
            ui_font: text("ui_font"),
            issues,
        }
    }

    /// Load from `config_path()`; a missing file yields defaults. While the
    /// file does not parse, the last version that did is read instead.
    pub fn load() -> Config {
        match std::fs::read_to_string(config_path()) {
            Ok(text) => match remember(&text) {
                Read::Valid => Config::parse(&text),
                Read::Broken { fallback } => {
                    fallback.map_or_else(Config::default, |good| Config::parse(&good))
                }
            },
            Err(_) => Config::default(),
        }
    }

    /// Like `load`, but `None` when the file is missing or does not parse, so
    /// a half-edited file never replaces known settings with defaults.
    pub fn load_valid() -> Option<Config> {
        let text = std::fs::read_to_string(config_path()).ok()?;
        matches!(remember(&text), Read::Valid).then(|| Config::parse(&text))
    }

    /// Recordings at least this long are saved permanently to `record_dir` even
    /// when saved to the shelf (they still get a shelf card), and stop counting
    /// toward the temporary cache quota once they reach it. Default 60 s; 0
    /// turns it off.
    pub fn shelf_to_disk_after(&self) -> Option<std::time::Duration> {
        match self.record_shelf_to_disk_after_seconds {
            None => Some(std::time::Duration::from_secs(60)),
            Some(seconds) if seconds > 0 => Some(std::time::Duration::from_secs(seconds as u64)),
            Some(_) => None,
        }
    }

    pub fn recording_prefs(&self) -> RecordingPrefs {
        let defaults = RecordingPrefs::default();
        RecordingPrefs {
            default_target: match self.record_default_target.as_deref() {
                Some("focused") => RecordDefaultTarget::Focused,
                Some("both") => RecordDefaultTarget::Both,
                Some(value) => value
                    .strip_prefix("output:")
                    .filter(|name| !name.is_empty())
                    .map(|name| RecordDefaultTarget::Output(name.to_string()))
                    .unwrap_or(defaults.default_target),
                None => defaults.default_target,
            },
            both_mode: match self.record_both_mode.as_deref() {
                Some("combined") => RecordBothMode::Combined,
                Some("separate") => RecordBothMode::Separate,
                _ => defaults.both_mode,
            },
            show_frame: self.record_show_frame.unwrap_or(defaults.show_frame),
            disk_add_to_shelf: self
                .record_disk_add_to_shelf
                .unwrap_or(defaults.disk_add_to_shelf),
            audio_enabled: self.record_audio_enabled.unwrap_or(defaults.audio_enabled),
            audio_source: match self.record_audio_source.as_deref() {
                Some("system-and-mic") => RecordAudioSource::SystemAndMic,
                Some("mic") => RecordAudioSource::Mic,
                Some("system") => RecordAudioSource::System,
                _ => defaults.audio_source,
            },
            cursor: match self.record_cursor.as_deref() {
                Some("mellow") => RecordCursor::Mellow,
                Some("quick") => RecordCursor::Quick,
                _ => defaults.cursor,
            },
        }
    }
}

/// The last `config.toml` that parsed, and the last broken one reported, so a
/// half-edited file never puts every setting back to its default (CF1).
#[derive(Default)]
struct Remembered {
    good: Option<String>,
    reported: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Read {
    Valid,
    /// The file does not parse; `fallback` is the last text that did.
    Broken {
        fallback: Option<String>,
    },
}

impl Remembered {
    /// Notes the file's `text`, with a report the first time a broken text is
    /// seen.
    fn see(&mut self, text: &str) -> (Read, Option<String>) {
        let Loaded::Broken { line, reason, .. } = settings::load(text) else {
            self.good = Some(text.into());
            self.reported = None;
            return (Read::Valid, None);
        };
        let report = (self.reported.as_deref() != Some(text)).then(|| {
            self.reported = Some(text.into());
            let place = line.map_or("config.toml".into(), |line| {
                format!("config.toml line {line}")
            });
            let using = if self.good.is_some() {
                "keeping the last valid settings"
            } else {
                "using the defaults"
            };
            format!("{place} is invalid: {reason}; {using}")
        });
        let fallback = self.good.clone();
        (Read::Broken { fallback }, report)
    }
}

static REMEMBERED: Mutex<Remembered> = Mutex::new(Remembered {
    good: None,
    reported: None,
});
static ON_BROKEN: OnceLock<fn(&str)> = OnceLock::new();

/// Where a broken `config.toml` is reported besides stderr, such as a
/// desktop notification. Set once; later calls are ignored.
pub fn on_broken(report: fn(&str)) {
    let _ = ON_BROKEN.set(report);
}

fn remember(text: &str) -> Read {
    let (read, report) = REMEMBERED.lock().unwrap().see(text);
    if let Some(report) = report {
        eprintln!("boltsnap: {report}");
        if let Some(hook) = ON_BROKEN.get() {
            hook(&report);
        }
    }
    read
}

pub fn save_recording_prefs(prefs: &RecordingPrefs) -> io::Result<()> {
    save_recording_prefs_at(&config_path(), prefs)
}

/// Writes only the prefs whose value differs from what the file has in effect,
/// so an invalid value nobody changed stays for the user to see, and keys
/// that were never set are not added.
pub fn save_recording_prefs_at(path: &Path, prefs: &RecordingPrefs) -> io::Result<()> {
    let wanted = pref_values(prefs);
    settings::update(path, |text| {
        // A broken file gets no changes; the writer refuses it as it is.
        let Loaded::Ok { doc, issues } = settings::load(text) else {
            return Vec::new();
        };
        let current = pref_values(&Config::from_doc(&doc, issues).recording_prefs());
        wanted
            .iter()
            .zip(current)
            .filter(|((_, wanted), (_, current))| wanted != current)
            .map(|((key, value), _)| (*key, settings::Change::Set(value.clone())))
            .collect()
    })
}

fn pref_values(prefs: &RecordingPrefs) -> [(&'static str, settings::Value); 7] {
    use settings::Value::{Bool, Text};
    let target = match &prefs.default_target {
        RecordDefaultTarget::Focused => "focused".to_string(),
        RecordDefaultTarget::Output(name) => format!("output:{name}"),
        RecordDefaultTarget::Both => "both".to_string(),
    };
    let both_mode = match prefs.both_mode {
        RecordBothMode::Separate => "separate",
        RecordBothMode::Combined => "combined",
    };
    let audio_source = match prefs.audio_source {
        RecordAudioSource::SystemAndMic => "system-and-mic",
        RecordAudioSource::Mic => "mic",
        RecordAudioSource::System => "system",
    };
    [
        ("record_default_target", Text(target)),
        ("record_both_mode", Text(both_mode.into())),
        ("record_cursor", Text(prefs.cursor.key().into())),
        ("record_show_frame", Bool(prefs.show_frame)),
        ("record_disk_add_to_shelf", Bool(prefs.disk_add_to_shelf)),
        ("record_audio_enabled", Bool(prefs.audio_enabled)),
        ("record_audio_source", Text(audio_source.into())),
    ]
}

pub fn config_path() -> PathBuf {
    crate::paths::config_dir().join("config.toml")
}

/// Expand a leading `~` (to `$HOME`) and `$VAR` / `${VAR}` tokens from the env.
/// Unset variables expand to empty.
pub fn expand_path(raw: &str) -> PathBuf {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    let mut first = true;
    while let Some(c) = chars.next() {
        match c {
            '~' if first => out.push_str(&crate::paths::home_dir().to_string_lossy()),
            '$' => {
                let braced = chars.peek() == Some(&'{');
                if braced {
                    chars.next();
                }
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    let ok = if braced {
                        n != '}'
                    } else {
                        n.is_ascii_alphanumeric() || n == '_'
                    };
                    if ok {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                if braced && chars.peek() == Some(&'}') {
                    chars.next();
                }
                if let Some(val) = env::var_os(&name) {
                    out.push_str(&val.to_string_lossy());
                }
            }
            _ => out.push(c),
        }
        first = false;
    }
    PathBuf::from(out)
}

/// The shelf save directory: CLI flag > `$BOLTSNAP_SAVE_DIR` > config > `~/Bilder/boltsnap`.
pub fn resolve_save_dir(cli: Option<&Path>, cfg: &Config) -> PathBuf {
    if let Some(p) = cli {
        return p.to_path_buf();
    }
    if let Ok(e) = env::var("BOLTSNAP_SAVE_DIR")
        && !e.is_empty()
    {
        return expand_path(&e);
    }
    if let Some(s) = &cfg.save_dir {
        return expand_path(s);
    }
    default_save_dir()
}

fn default_save_dir() -> PathBuf {
    crate::paths::default_screenshot_dir()
}

/// Requested recording encoder: CLI flag > $BOLTSNAP_RECORD_CODEC > config > `auto`
/// (hardware H.264 through gpu-screen-recorder; `h264_nvenc` for wf-recorder).
pub fn resolve_record_codec(cli: Option<&str>, cfg: &Config) -> String {
    if let Some(c) = cli {
        return c.to_string();
    }
    if let Ok(e) = env::var("BOLTSNAP_RECORD_CODEC")
        && !e.is_empty()
    {
        return e;
    }
    if let Some(c) = &cfg.record_codec {
        return c.clone();
    }
    "auto".to_string()
}

/// Where confirmed recordings are saved: config `record_dir` (expanded) else the
/// regular `save_dir`.
pub fn resolve_record_dir(cfg: &Config) -> PathBuf {
    if let Some(s) = &cfg.record_dir {
        return expand_path(s);
    }
    resolve_save_dir(None, cfg)
}

#[cfg(test)]
mod tests {
    #[test]
    fn record_profile_defaults_and_validation() {
        use super::{Config, RecordProfile};
        assert_eq!(
            Config::parse("").record_profile().unwrap(),
            RecordProfile::Quality
        );
        assert_eq!(
            Config::parse("record_profile = 'quality'")
                .record_profile()
                .unwrap(),
            RecordProfile::Quality
        );
        assert_eq!(
            Config::parse("record_profile = 'quiet'")
                .record_profile()
                .unwrap(),
            RecordProfile::Quiet
        );
        let fps = |text: &str| Config::parse(text).record_profile().map(RecordProfile::fps);
        assert_eq!(fps("record_profile = 'quiet'\nrecord_fps = 120"), Ok(120));
        for text in ["record_fps = 0", "record_fps = 241", "record_fps = '120'"] {
            assert!(fps(text).is_err(), "{text}");
        }
        for text in [
            "record_profile = 'typo'",
            "record_profile = 60",
            "record_profile = ''",
        ] {
            assert!(Config::parse(text).record_profile().is_err());
        }
    }

    #[test]
    fn replay_duration_defaults_and_limits_are_explicit() {
        assert_eq!(
            super::Config::parse_replay_settings("").unwrap().seconds,
            60
        );
        assert_eq!(
            super::Config::parse_replay_settings("[replay]\nduration_seconds = 30")
                .unwrap()
                .seconds,
            30
        );
        for text in [
            "[replay]\nduration_seconds = 0",
            "[replay]\nduration_seconds = 601",
            "[replay]\nmemory_mib = 0",
            "[replay]\nbitrate_kbps = 100",
            "[replay]\nautostart = 'yes'",
            "replay = 1",
        ] {
            assert!(super::Config::parse_replay_settings(text).is_err());
        }
    }

    use super::*;

    fn temp_config(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "boltsnap-{name}-{}-{}.toml",
            std::process::id(),
            crate::paths::timestamp()
        ))
    }

    /// The file and the lock the writer leaves beside it.
    fn remove_config(path: &Path) {
        let _ = std::fs::remove_file(path);
        let lock = format!("{}.lock", path.file_name().unwrap().to_string_lossy());
        let _ = std::fs::remove_file(path.with_file_name(lock));
    }

    #[test]
    fn audio_preferences_default_on_and_system_plus_mic() {
        let prefs = Config::default().recording_prefs();
        assert!(prefs.audio_enabled);
        assert_eq!(prefs.audio_source, RecordAudioSource::SystemAndMic);
    }

    #[test]
    fn audio_preferences_parse_and_round_trip() {
        let parsed = Config::parse(
            r#"
record_audio_enabled = false
record_audio_source = "system"
unrelated = "keep-me"
"#,
        )
        .recording_prefs();
        assert!(!parsed.audio_enabled);
        assert_eq!(parsed.audio_source, RecordAudioSource::System);

        let path = temp_config("record-audio-round-trip");
        std::fs::write(&path, "unrelated = \"keep-me\"\n").unwrap();
        save_recording_prefs_at(
            &path,
            &RecordingPrefs {
                audio_enabled: true,
                audio_source: RecordAudioSource::Mic,
                ..RecordingPrefs::default()
            },
        )
        .unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        // Unchanged from the default, so not written (CF3).
        assert!(!saved.contains("record_audio_enabled"));
        assert!(saved.contains("record_audio_source = \"mic\""));
        assert!(saved.contains("unrelated = \"keep-me\""));
        assert_eq!(
            Config::parse(&saved).recording_prefs().audio_source,
            RecordAudioSource::Mic
        );
        remove_config(&path);
    }

    #[test]
    fn invalid_audio_source_uses_default() {
        let prefs = Config::parse("record_audio_source = \"bluetooth\"\n").recording_prefs();
        assert_eq!(prefs.audio_source, RecordAudioSource::SystemAndMic);
    }

    #[test]
    fn long_shelf_recordings_go_to_disk_after_a_minute_by_default() {
        let after = |text: &str| Config::parse(text).shelf_to_disk_after();
        assert_eq!(after(""), Some(std::time::Duration::from_secs(60)));
        assert_eq!(
            after("record_shelf_to_disk_after_seconds = 300\n"),
            Some(std::time::Duration::from_secs(300))
        );
        assert_eq!(after("record_shelf_to_disk_after_seconds = 0\n"), None);
        assert_eq!(after("record_shelf_to_disk_after_seconds = -5\n"), None);
    }

    #[test]
    fn recording_prefs_default_to_focused_separate_and_visible() {
        assert_eq!(
            Config::default().recording_prefs(),
            RecordingPrefs {
                default_target: RecordDefaultTarget::Focused,
                both_mode: RecordBothMode::Separate,
                show_frame: true,
                disk_add_to_shelf: true,
                audio_enabled: true,
                audio_source: RecordAudioSource::SystemAndMic,
                cursor: RecordCursor::System,
            }
        );
    }

    #[test]
    fn recording_prefs_parse_all_keys_and_named_output() {
        let prefs = Config::parse(
            "record_default_target = \"output:DP-3\"\n\
             record_both_mode = \"combined\"\n\
             record_show_frame = false\n\
             record_disk_add_to_shelf = false\n",
        )
        .recording_prefs();
        assert_eq!(
            prefs,
            RecordingPrefs {
                default_target: RecordDefaultTarget::Output("DP-3".into()),
                both_mode: RecordBothMode::Combined,
                show_frame: false,
                disk_add_to_shelf: false,
                audio_enabled: true,
                audio_source: RecordAudioSource::SystemAndMic,
                cursor: RecordCursor::System,
            }
        );
    }

    #[test]
    fn recording_prefs_malformed_values_fall_back_safely() {
        let prefs = Config::parse(
            "record_default_target = \"output:\"\n\
             record_both_mode = \"fast\"\n\
             record_show_frame = \"no\"\n\
             record_disk_add_to_shelf = 0\n\
             record_cursor = \"wobbly\"\n",
        )
        .recording_prefs();
        assert_eq!(prefs, RecordingPrefs::default());
    }

    #[test]
    fn recording_prefs_write_preserves_unknown_keys() {
        let path = temp_config("prefs-preserve");
        std::fs::write(&path, "custom = 7\n").unwrap();
        let prefs = RecordingPrefs {
            default_target: RecordDefaultTarget::Output("DP-3".into()),
            both_mode: RecordBothMode::Combined,
            show_frame: false,
            disk_add_to_shelf: false,
            cursor: RecordCursor::Quick,
            ..RecordingPrefs::default()
        };
        save_recording_prefs_at(&path, &prefs).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("custom = 7"));
        assert_eq!(Config::parse(&written).recording_prefs(), prefs);
        remove_config(&path);
    }

    #[test]
    fn recording_prefs_write_keeps_comments_order_and_tables() {
        let path = temp_config("prefs-comments");
        let original = "# my settings\nzeta = 1\nrecord_cursor = \"system\" # smooth later\nalpha = 2\n\n[replay]\n# ten seconds\nduration_seconds = 10\n";
        std::fs::write(&path, original).unwrap();
        let prefs = RecordingPrefs {
            cursor: RecordCursor::Mellow,
            ..RecordingPrefs::default()
        };
        save_recording_prefs_at(&path, &prefs).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.starts_with(
            "# my settings\nzeta = 1\nrecord_cursor = \"mellow\" # smooth later\nalpha = 2\n"
        ));
        assert!(written.contains("[replay]\n# ten seconds\nduration_seconds = 10\n"));
        // New keys stay top-level, not inside [replay].
        assert_eq!(Config::parse(&written).recording_prefs(), prefs);
        assert_eq!(Config::parse_replay_settings(&written).unwrap().seconds, 10);
        remove_config(&path);
    }

    #[test]
    fn recording_prefs_write_is_readable_and_rejects_malformed_existing_config() {
        let path = temp_config("prefs-roundtrip");
        let prefs = RecordingPrefs {
            default_target: RecordDefaultTarget::Both,
            both_mode: RecordBothMode::Separate,
            show_frame: true,
            disk_add_to_shelf: false,
            ..RecordingPrefs::default()
        };
        save_recording_prefs_at(&path, &prefs).unwrap();
        assert_eq!(
            Config::parse(&std::fs::read_to_string(&path).unwrap()).recording_prefs(),
            prefs
        );

        std::fs::write(&path, "custom = = broken").unwrap();
        assert!(save_recording_prefs_at(&path, &RecordingPrefs::default()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "custom = = broken");
        remove_config(&path);
    }

    #[test]
    fn recording_prefs_write_only_changed_keys_and_keeps_invalid_ones() {
        let path = temp_config("prefs-changed-only");
        let original = "record_cursor = \"Quick\" # typo\nrecord_show_frame = \"no\"\n";
        std::fs::write(&path, original).unwrap();
        save_recording_prefs_at(&path, &RecordingPrefs::default()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        let prefs = RecordingPrefs {
            cursor: RecordCursor::Mellow,
            ..RecordingPrefs::default()
        };
        save_recording_prefs_at(&path, &prefs).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "record_cursor = \"mellow\" # typo\nrecord_show_frame = \"no\"\n"
        );
        std::fs::remove_file(&path).unwrap();
        save_recording_prefs_at(&path, &prefs).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "record_cursor = \"mellow\"\n"
        );
        remove_config(&path);
    }

    #[test]
    fn a_broken_file_falls_back_to_the_last_valid_text_and_is_reported_once() {
        let mut remembered = Remembered::default();
        let (read, report) = remembered.see("record_fps = \n");
        assert_eq!(read, Read::Broken { fallback: None });
        assert_eq!(
            report.as_deref(),
            Some("config.toml line 1 is invalid: invalid string; using the defaults")
        );
        assert_eq!(remembered.see("record_fps = 120\n"), (Read::Valid, None));
        let (read, report) = remembered.see("record_fps = 120\nx = \n");
        assert_eq!(
            read,
            Read::Broken {
                fallback: Some("record_fps = 120\n".into())
            }
        );
        assert_eq!(
            report.as_deref(),
            Some("config.toml line 2 is invalid: invalid string; keeping the last valid settings")
        );
        // The same broken text is reported once, another one again.
        assert_eq!(remembered.see("record_fps = 120\nx = \n").1, None);
        assert!(remembered.see("record_fps = = 1\n").1.is_some());
        // Fixed and broken the same way again: reported again.
        remembered.see("record_fps = 60\n");
        let (read, report) = remembered.see("record_fps = = 1\n");
        assert_eq!(
            read,
            Read::Broken {
                fallback: Some("record_fps = 60\n".into())
            }
        );
        assert!(report.is_some());
    }

    #[test]
    fn parse_reads_save_dir_and_ignores_unknown() {
        let c = Config::parse("save_dir = \"/tmp/shots\"\nbogus = 1\n");
        assert_eq!(c.save_dir.as_deref(), Some("/tmp/shots"));
    }

    #[test]
    fn parse_missing_keys_are_none() {
        let c = Config::parse("");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn parse_malformed_yields_defaults() {
        let c = Config::parse("save_dir = = =");
        assert_eq!(c, Config::default());
    }

    #[test]
    fn expand_tilde_and_env() {
        let home = crate::paths::home_dir();
        unsafe {
            env::set_var("BOLT_T", "sub");
        }
        assert_eq!(expand_path("~/Bilder"), home.join("Bilder"));
        assert_eq!(expand_path("/a/$BOLT_T/b"), PathBuf::from("/a/sub/b"));
        assert_eq!(expand_path("/a/${BOLT_T}x"), PathBuf::from("/a/subx"));
    }

    #[test]
    fn save_dir_precedence_flag_over_config() {
        let cfg = Config {
            save_dir: Some("/from/config".into()),
            ..Config::default()
        };
        let got = resolve_save_dir(Some(Path::new("/from/flag")), &cfg);
        assert_eq!(got, PathBuf::from("/from/flag"));
    }

    #[test]
    fn save_dir_falls_back_to_config_then_default() {
        // BOLTSNAP_SAVE_DIR is boltsnap-private; removing it is race-free. The
        // expectations come from the selected platform path provider.
        let home = crate::paths::home_dir();
        unsafe {
            env::remove_var("BOLTSNAP_SAVE_DIR");
        }
        let cfg = Config {
            save_dir: Some("~/cfgshots".into()),
            ..Config::default()
        };
        assert_eq!(
            resolve_save_dir(None, &cfg),
            PathBuf::from(&home).join("cfgshots")
        );
        assert_eq!(
            resolve_save_dir(None, &Config::default()),
            crate::paths::default_screenshot_dir()
        );
    }

    #[test]
    fn parse_record_keys() {
        let c = Config::parse("record_codec = \"libx264\"\nrecord_dir = \"/tmp/rec\"\n");
        assert_eq!(c.record_codec.as_deref(), Some("libx264"));
        assert_eq!(c.record_dir.as_deref(), Some("/tmp/rec"));
    }

    #[test]
    fn record_codec_defaults_to_auto() {
        // BOLTSNAP_RECORD_CODEC is boltsnap-private (read by nothing else), so
        // removing it without restore is race-free and keeps the default assertion
        // self-contained.
        unsafe {
            env::remove_var("BOLTSNAP_RECORD_CODEC");
        }
        assert_eq!(resolve_record_codec(None, &Config::default()), "auto");
        let cfg = Config {
            record_codec: Some("libx264".into()),
            ..Config::default()
        };
        assert_eq!(resolve_record_codec(None, &cfg), "libx264");
        assert_eq!(resolve_record_codec(Some("hevc_nvenc"), &cfg), "hevc_nvenc");
    }
}
