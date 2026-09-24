use std::env;
use std::io;
use std::path::{Path, PathBuf};

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

/// Parsed `~/.config/boltsnap/config.toml`. A missing file or an unset key leaves
/// the field `None`; defaults are applied by the `resolve_*` helpers.
#[derive(Default, Debug, PartialEq, Eq)]
pub struct Config {
    pub save_dir: Option<String>,
    pub record_codec: Option<String>,
    record_profile: Option<String>,
    /// `Some(None)`: set, but not a whole number.
    record_fps: Option<Option<i64>>,
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
}

impl Config {
    /// `record_fps` (1-240) overrides the profile's frame rate.
    pub fn record_profile(&self) -> Result<RecordProfile, String> {
        if let Some(fps) = self.record_fps {
            return fps
                .filter(|fps| (1..=240).contains(fps))
                .map(|fps| RecordProfile::Fps(fps as u32))
                .ok_or_else(|| "record_fps must be a whole number from 1 to 240".into());
        }
        match self.record_profile.as_deref() {
            None | Some("quality") => Ok(RecordProfile::Quality),
            Some("quiet") => Ok(RecordProfile::Quiet),
            _ => Err("record_profile must be \"quality\" or \"quiet\"".into()),
        }
    }

    pub fn replay_settings() -> Result<boltsnap::replay::settings::Settings, String> {
        let text = match std::fs::read_to_string(config_path()) {
            Ok(text) => text,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(format!("read replay settings: {error}")),
        };
        Self::parse_replay_settings(&text)
    }

    fn parse_replay_settings(text: &str) -> Result<boltsnap::replay::settings::Settings, String> {
        let value: toml::Value =
            toml::from_str(text).map_err(|e| format!("invalid config: {e}"))?;
        let mut settings = boltsnap::replay::settings::Settings::default();
        let Some(value) = value.get("replay") else {
            return Ok(settings);
        };
        if !value.is_table() {
            return Err("replay must be a settings table".into());
        }
        let number = |key, default, min, max| -> Result<u64, String> {
            let Some(value) = value.get(key) else {
                return Ok(default);
            };
            let n = value
                .as_integer()
                .and_then(|n| u64::try_from(n).ok())
                .ok_or_else(|| format!("invalid replay {key}"))?;
            if n < min || n > max {
                return Err(format!("replay {key} must be {min}..{max}"));
            }
            Ok(n)
        };
        settings.seconds = number("duration_seconds", 60, 1, 600)?;
        settings.memory_mib = number("memory_mib", 512, 64, 4096)? as usize;
        settings.fps = number("fps", 60, 1, 240)? as u32;
        if let Some(value) = value.get("encoder") {
            settings.encoder = value
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("invalid replay encoder")?
                .into();
        }
        if let Some(value) = value.get("output") {
            settings.output = Some(
                value
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or("invalid replay output")?
                    .into(),
            );
        }
        if let Some(value) = value.get("autostart") {
            settings.autostart = value.as_bool().ok_or("invalid replay autostart")?;
        }
        Ok(settings)
    }

    /// Parse config text. Unknown keys are ignored; a parse error logs and yields
    /// defaults (so a typo never bricks the daemon).
    pub fn parse(text: &str) -> Config {
        match toml::from_str::<toml::Value>(text) {
            Ok(v) => Config {
                save_dir: v.get("save_dir").and_then(|x| x.as_str()).map(String::from),
                record_codec: v
                    .get("record_codec")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                record_profile: v
                    .get("record_profile")
                    .map(|x| x.as_str().unwrap_or("").to_owned()),
                record_fps: v.get("record_fps").map(|x| x.as_integer()),
                record_dir: v
                    .get("record_dir")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                record_default_target: v
                    .get("record_default_target")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                record_cursor: v
                    .get("record_cursor")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                record_both_mode: v
                    .get("record_both_mode")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                record_show_frame: v.get("record_show_frame").and_then(|x| x.as_bool()),
                record_disk_add_to_shelf: v
                    .get("record_disk_add_to_shelf")
                    .and_then(|x| x.as_bool()),
                record_audio_enabled: v.get("record_audio_enabled").and_then(|x| x.as_bool()),
                record_audio_source: v
                    .get("record_audio_source")
                    .and_then(|x| x.as_str())
                    .map(String::from),
                record_shelf_to_disk_after_seconds: v
                    .get("record_shelf_to_disk_after_seconds")
                    .and_then(|x| x.as_integer()),
                ui_font: v.get("ui_font").and_then(|x| x.as_str()).map(String::from),
            },
            Err(e) => {
                eprintln!("boltsnap: ignoring malformed config: {e}");
                Config::default()
            }
        }
    }

    /// Load from `config_path()`; a missing file yields defaults.
    pub fn load() -> Config {
        match std::fs::read_to_string(config_path()) {
            Ok(s) => Config::parse(&s),
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

pub fn save_recording_prefs(prefs: &RecordingPrefs) -> io::Result<()> {
    save_recording_prefs_at(&config_path(), prefs)
}

pub fn save_recording_prefs_at(path: &Path, prefs: &RecordingPrefs) -> io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    let mut table = match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str::<toml::Table>(&text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => toml::Table::new(),
        Err(error) => return Err(error),
    };
    let target = match &prefs.default_target {
        RecordDefaultTarget::Focused => "focused".to_string(),
        RecordDefaultTarget::Output(name) => format!("output:{name}"),
        RecordDefaultTarget::Both => "both".to_string(),
    };
    table.insert("record_default_target".into(), toml::Value::String(target));
    table.insert(
        "record_both_mode".into(),
        toml::Value::String(
            match prefs.both_mode {
                RecordBothMode::Separate => "separate",
                RecordBothMode::Combined => "combined",
            }
            .into(),
        ),
    );
    table.insert(
        "record_cursor".into(),
        toml::Value::String(prefs.cursor.key().into()),
    );
    table.insert(
        "record_show_frame".into(),
        toml::Value::Boolean(prefs.show_frame),
    );
    table.insert(
        "record_disk_add_to_shelf".into(),
        toml::Value::Boolean(prefs.disk_add_to_shelf),
    );
    table.insert(
        "record_audio_enabled".into(),
        toml::Value::Boolean(prefs.audio_enabled),
    );
    table.insert(
        "record_audio_source".into(),
        toml::Value::String(
            match prefs.audio_source {
                RecordAudioSource::SystemAndMic => "system-and-mic",
                RecordAudioSource::Mic => "mic",
                RecordAudioSource::System => "system",
            }
            .into(),
        ),
    );

    let text = toml::to_string_pretty(&table).map_err(io::Error::other)?;
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let temporary = path.with_file_name(format!("{file_name}.tmp-{}", std::process::id()));
    let result = (|| {
        let mut file = std::fs::File::create(&temporary)?;
        io::Write::write_all(&mut file, text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
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
        assert!(saved.contains("record_audio_enabled = true"));
        assert!(saved.contains("record_audio_source = \"mic\""));
        assert!(saved.contains("unrelated = \"keep-me\""));
        assert_eq!(
            Config::parse(&saved).recording_prefs().audio_source,
            RecordAudioSource::Mic
        );
        let _ = std::fs::remove_file(path);
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
        std::fs::remove_file(path).unwrap();
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
        std::fs::remove_file(path).unwrap();
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
