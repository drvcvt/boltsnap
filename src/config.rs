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

/// The file's valid values and issues as `get_settings` reports them; while
/// it does not parse, those of the last text that did, and where it broke.
pub fn settings_state() -> settings::State {
    let path = config_path();
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let good = match remember(&text) {
        Read::Valid => None,
        Read::Broken { fallback } => fallback,
    };
    settings::State::read(path, &text, good.as_deref())
}

/// The two prefs the selector shows, each only when it changed (`Some`). Every
/// other key stays as the file has it, even one the daemon wrote meanwhile.
pub fn save_selector_prefs(
    show_frame: Option<bool>,
    audio_enabled: Option<bool>,
) -> io::Result<()> {
    save_selector_prefs_at(&config_path(), show_frame, audio_enabled)
}

fn save_selector_prefs_at(
    path: &Path,
    show_frame: Option<bool>,
    audio_enabled: Option<bool>,
) -> io::Result<()> {
    let changes: Vec<_> = [
        ("record_show_frame", show_frame),
        ("record_audio_enabled", audio_enabled),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key, settings::Change::Set(settings::Value::Bool(value?)))))
    .collect();
    if changes.is_empty() {
        return Ok(());
    }
    settings::write(path, &changes)
}

// The Linux daemon writes single keys through `pref_changes` instead.
#[cfg(target_os = "windows")]
pub fn save_recording_prefs(prefs: &RecordingPrefs) -> io::Result<()> {
    save_recording_prefs_at(&config_path(), prefs)
}

/// Writes only the prefs whose value differs from what the file has in effect,
/// so an invalid value nobody changed stays for the user to see, and keys
/// that were never set are not added.
#[cfg(any(target_os = "windows", test))]
pub fn save_recording_prefs_at(path: &Path, prefs: &RecordingPrefs) -> io::Result<()> {
    settings::update(path, |text| {
        // A broken file gets no changes; the writer refuses it as it is.
        let Loaded::Ok { doc, issues } = settings::load(text) else {
            return Vec::new();
        };
        pref_changes(&Config::from_doc(&doc, issues).recording_prefs(), prefs)
    })
}

/// The keys whose value differs between `old` and `new`, set to `new`'s.
pub fn pref_changes(
    old: &RecordingPrefs,
    new: &RecordingPrefs,
) -> Vec<(&'static str, settings::Change)> {
    pref_values(old)
        .into_iter()
        .zip(pref_values(new))
        .filter(|((_, old), (_, new))| old != new)
        .map(|(_, (key, value))| (key, settings::Change::Set(value)))
        .collect()
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
mod tests;
