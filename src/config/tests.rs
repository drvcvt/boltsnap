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
    // The same error is reported once, even after more edits; another error
    // is reported again.
    assert_eq!(remembered.see("record_fps = 120\nx = \n").1, None);
    assert_eq!(remembered.see("record_fps = 120\nx = \ny = \n").1, None);
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
fn selector_prefs_write_only_the_keys_that_changed() {
    let path = temp_config("selector-prefs");
    let original = "record_show_frame = true\nrecord_cursor = \"Quick\"\n";
    std::fs::write(&path, original).unwrap();
    save_selector_prefs_at(&path, Some(false), None).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "record_show_frame = false\nrecord_cursor = \"Quick\"\n"
    );
    save_selector_prefs_at(&path, None, Some(false)).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "record_show_frame = false\nrecord_cursor = \"Quick\"\nrecord_audio_enabled = false\n"
    );
    // Nothing changed: a broken file is not even read.
    std::fs::write(&path, "x = = 1").unwrap();
    save_selector_prefs_at(&path, None, None).unwrap();
    assert!(save_selector_prefs_at(&path, Some(true), None).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "x = = 1");
    remove_config(&path);
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

#[test]
fn pref_changes_name_only_the_changed_keys() {
    use super::{RecordCursor, RecordingPrefs, pref_changes};
    use boltsnap::settings::{Change, Value};
    let old = RecordingPrefs::default();
    assert!(pref_changes(&old, &old).is_empty());
    let new = RecordingPrefs {
        cursor: RecordCursor::Quick,
        ..old.clone()
    };
    assert_eq!(
        pref_changes(&old, &new),
        vec![("record_cursor", Change::Set(Value::Text("quick".into())))]
    );
}

#[test]
fn reading_notes_a_deleted_file_and_keeps_the_last_text_when_unreadable() {
    use super::{Noted, Read, Remembered, read_noted};
    let path = temp_config("read-noted");
    let remembered = std::sync::Mutex::new(Remembered::default());
    std::fs::write(&path, "record_fps = 60\n").unwrap();
    assert!(matches!(
        read_noted(&path, &remembered).0,
        Noted::Text(_, Read::Valid)
    ));
    // Deleted means defaults, so a broken file after it falls back to empty.
    std::fs::remove_file(&path).unwrap();
    assert!(
        matches!(read_noted(&path, &remembered).0, Noted::Text(text, Read::Valid) if text.is_empty())
    );
    std::fs::write(&path, "record_fps = \n").unwrap();
    assert!(matches!(
        read_noted(&path, &remembered).0,
        Noted::Text(_, Read::Broken { fallback: Some(good) }) if good.is_empty()
    ));
    // A file that cannot be read is not noted: the last valid text stays.
    std::fs::write(&path, "record_fps = 120\n").unwrap();
    read_noted(&path, &remembered);
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(matches!(
        read_noted(&path, &remembered),
        (Noted::Unreadable { good: Some(good), .. }, None) if good == "record_fps = 120\n"
    ));
    assert_eq!(
        remembered.lock().unwrap().good.as_deref(),
        Some("record_fps = 120\n")
    );
    std::fs::remove_dir(&path).unwrap();
    remove_config(&path);
}
