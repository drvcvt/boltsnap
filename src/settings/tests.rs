use std::path::PathBuf;

use super::*;

fn issues(text: &str) -> Vec<Issue> {
    match load(text) {
        Loaded::Ok { issues, .. } => issues,
        broken => panic!("{broken:?}"),
    }
}

fn doc(text: &str) -> Doc {
    match load(text) {
        Loaded::Ok { doc, .. } => doc,
        broken => panic!("{broken:?}"),
    }
}

fn messages(text: &str) -> Vec<(String, Option<usize>, String)> {
    issues(text)
        .into_iter()
        .map(|issue| (issue.key, issue.line, issue.message))
        .collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "boltsnap-settings-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn an_empty_file_has_no_values_and_no_issues() {
    assert_eq!(
        load(""),
        Loaded::Ok {
            doc: Doc::default(),
            issues: vec![]
        }
    );
}

#[test]
fn valid_values_are_read_with_their_types() {
    let doc = doc("record_fps = 120\n\
         record_show_frame = false\n\
         record_default_target = \"output:DP-3\"\n\
         save_dir = \"~/shots\"\n\
         [replay]\n\
         fps = 60\n\
         output = \"DP-3\"\n");
    assert_eq!(doc.int("record_fps"), Some(120));
    assert_eq!(doc.bool("record_show_frame"), Some(false));
    assert_eq!(doc.text("record_default_target"), Some("output:DP-3"));
    assert_eq!(doc.text("save_dir"), Some("~/shots"));
    assert_eq!(doc.int("replay.fps"), Some(60));
    assert_eq!(doc.text("replay.output"), Some("DP-3"));
    assert_eq!(doc.get("record_cursor"), None);
}

#[test]
fn invalid_values_are_left_out_and_reported_with_their_line() {
    let text = "# comment\n\
         record_show_frame = \"no\"\n\
         record_cursor = \"Quick\"\n\
         record_default_target = \"output:\"\n";
    assert_eq!(doc(text).get("record_show_frame"), None);
    assert_eq!(
        messages(text),
        vec![
            (
                "record_default_target".into(),
                Some(4),
                "invalid record_default_target: expected focused, both or output:<name>".into()
            ),
            (
                "record_cursor".into(),
                Some(3),
                "invalid record_cursor: expected system, mellow, quick".into()
            ),
            (
                "record_show_frame".into(),
                Some(2),
                "invalid record_show_frame".into()
            ),
        ]
    );
}

#[test]
fn strict_keys_keep_the_messages_their_users_always_showed() {
    let first = |text: &str| issues(text).remove(0).message;
    assert_eq!(
        first("record_fps = 0"),
        "record_fps must be a whole number from 1 to 240"
    );
    assert_eq!(
        first("record_fps = '120'"),
        "record_fps must be a whole number from 1 to 240"
    );
    assert_eq!(
        first("record_profile = 60"),
        "record_profile must be \"quality\" or \"quiet\""
    );
    assert_eq!(
        first("[replay]\nduration_seconds = 601"),
        "replay duration_seconds must be 1..600"
    );
    assert_eq!(first("[replay]\nfps = 'x'"), "invalid replay fps");
    assert_eq!(
        first("[replay]\nduration_seconds = -1"),
        "invalid replay duration_seconds"
    );
    assert_eq!(first("[replay]\nencoder = ''"), "invalid replay encoder");
    assert_eq!(
        first("[replay]\nautostart = 'yes'"),
        "invalid replay autostart"
    );
    assert_eq!(first("replay = 1"), "replay must be a settings table");
    // The old replay parser stopped at the first bad key in this order.
    assert_eq!(
        first("[replay]\nfps = 0\nduration_seconds = 0"),
        "replay duration_seconds must be 1..600"
    );
    assert!(super::schema::find("replay.fps").unwrap().strict);
    assert!(!super::schema::find("record_cursor").unwrap().strict);
}

#[test]
fn unknown_keys_are_reported_and_known_ones_still_read() {
    let text =
        "custom = 7\nrecord_fps = 60\n\"replay.fps\" = 30\n[replay]\nfsp = 60\n[editor]\nx = 1\n";
    assert_eq!(doc(text).int("record_fps"), Some(60));
    let unknown: Vec<_> = messages(text)
        .into_iter()
        .map(|(key, line, _)| (key, line))
        .collect();
    assert_eq!(
        unknown,
        vec![
            ("custom".into(), Some(1)),
            ("editor".into(), None),
            ("replay.fsp".into(), Some(5)),
            ("replay.fps".into(), None),
        ]
    );
}

#[test]
fn a_syntax_error_reports_its_line() {
    match load("record_fps = 60\nrecord_show_frame = \n") {
        Loaded::Broken {
            line,
            message,
            reason,
        } => {
            assert_eq!(line, Some(2));
            assert!(
                message.starts_with("TOML parse error at line 2"),
                "{message}"
            );
            assert_eq!(reason, "invalid string");
        }
        ok => panic!("{ok:?}"),
    }
}

#[test]
fn apply_changes_only_the_named_keys() {
    let text = "# boltsnap\n\
         save_dir = \"~/shots\"  # where screenshots go\n\
         # how fast\n\
         record_fps = 60\n\
         record_cursor = \"Quick\"\n\
         custom = 7\n\
         [replay]\n\
         fps = 30\n";
    let out = apply(
        text,
        &[
            ("record_fps", Change::Set(Value::Int(120))),
            ("replay.fps", Change::Set(Value::Int(60))),
        ],
    )
    .unwrap();
    assert_eq!(
        out,
        "# boltsnap\n\
         save_dir = \"~/shots\"  # where screenshots go\n\
         # how fast\n\
         record_fps = 120\n\
         record_cursor = \"Quick\"\n\
         custom = 7\n\
         [replay]\n\
         fps = 60\n"
    );
}

#[test]
fn apply_adds_missing_keys_and_tables_and_removes_keys() {
    let out = apply(
        "record_fps = 60\n",
        &[
            ("record_fps", Change::Remove),
            ("record_show_frame", Change::Set(Value::Bool(false))),
            ("replay.autostart", Change::Set(Value::Bool(true))),
        ],
    )
    .unwrap();
    assert_eq!(
        out,
        "record_show_frame = false\n\n[replay]\nautostart = true\n"
    );
    // Removing a key of a table that is not there adds no empty table.
    assert_eq!(
        apply("record_fps = 60\n", &[("replay.fps", Change::Remove)]).unwrap(),
        "record_fps = 60\n"
    );
    let inline = apply(
        "replay = { fps = 30 }\n",
        &[("replay.fps", Change::Set(Value::Int(60)))],
    )
    .unwrap();
    assert_eq!(doc(&inline).int("replay.fps"), Some(60));
}

#[test]
fn apply_rejects_bad_changes_and_broken_files() {
    let text = "record_fps = 60\n";
    for change in [
        ("record_fps", Change::Set(Value::Int(500))),
        ("record_fps", Change::Set(Value::Text("60".into()))),
        ("record_cursor", Change::Set(Value::Text("wobbly".into()))),
        ("nope", Change::Remove),
    ] {
        assert!(
            apply(text, std::slice::from_ref(&change)).is_err(),
            "{change:?}"
        );
    }
    assert!(
        apply(
            "replay = 1\n",
            &[("replay.fps", Change::Set(Value::Int(60)))]
        )
        .is_err()
    );
    assert!(apply("record_fps = \n", &[("record_fps", Change::Remove)]).is_err());
}

#[test]
fn write_creates_replaces_and_leaves_no_temporary_files() {
    let dir = temp_dir("write");
    let path = dir.join("config.toml");
    write(&path, &[("record_fps", Change::Set(Value::Int(120)))]).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "record_fps = 120\n"
    );
    write(&path, &[("record_fps", Change::Set(Value::Int(60)))]).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "record_fps = 60\n");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["config.toml", "config.toml.lock"]);
    // An invalid change leaves the file alone.
    assert!(write(&path, &[("record_fps", Change::Set(Value::Int(0)))]).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "record_fps = 60\n");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn parallel_writers_keep_each_others_changes() {
    let dir = temp_dir("parallel");
    let path = dir.join("config.toml");
    let writers: Vec<_> = [("record_show_frame", 0), ("replay.autostart", 1)]
        .into_iter()
        .map(|(key, offset)| {
            let path = path.clone();
            std::thread::spawn(move || {
                for round in 0..20 {
                    let value = (round + offset) % 2 == 0;
                    write(&path, &[(key, Change::Set(Value::Bool(value)))]).unwrap();
                }
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap();
    }
    let doc = doc(&std::fs::read_to_string(&path).unwrap());
    // Round 19 wrote the last value of each writer.
    assert_eq!(doc.bool("record_show_frame"), Some(false));
    assert_eq!(doc.bool("replay.autostart"), Some(true));
    let _ = std::fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn write_keeps_a_symlink_and_the_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("symlink");
    let real = dir.join("dotfiles-config.toml");
    std::fs::write(&real, "custom = 7\n").unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o600)).unwrap();
    let link = dir.join("config.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    write(&link, &[("record_fps", Change::Set(Value::Int(120)))]).unwrap();
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        std::fs::read_to_string(&real).unwrap(),
        "custom = 7\nrecord_fps = 120\n"
    );
    let mode = std::fs::metadata(&real).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    // A link that points nowhere is an error, not a new file over the link.
    let dangling = dir.join("dangling.toml");
    std::os::unix::fs::symlink(dir.join("missing.toml"), &dangling).unwrap();
    assert!(write(&dangling, &[("record_fps", Change::Remove)]).is_err());
    assert!(!dir.join("missing.toml").exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn state_reads_the_file_or_the_last_valid_text_while_it_is_broken() {
    let path = PathBuf::from("/c/config.toml");
    let good = "record_fps = 120\nrecord_cursor = \"Quick\"\n";
    let valid = State::read(path.clone(), good, None);
    assert_eq!(valid.doc.int("record_fps"), Some(120));
    assert_eq!(valid.issues.len(), 1);
    assert_eq!(valid.broken, None);

    let broken = State::read(path.clone(), "record_fps = \n", Some(good));
    assert_eq!(broken.doc, valid.doc);
    assert_eq!(broken.issues, valid.issues);
    assert_eq!(
        broken.broken,
        Some(Broken {
            line: Some(1),
            reason: "invalid string".into()
        })
    );

    let never_valid = State::read(path, "record_fps = \n", None);
    assert_eq!(never_valid.doc, Doc::default());
    assert!(never_valid.issues.is_empty());
    assert!(never_valid.broken.is_some());
}

#[test]
fn state_and_patch_round_trip_through_json() {
    let text = "record_fps = 120\nrecord_show_frame = false\nsave_dir = \"~/s\"\n\
                record_cursor = 1\n[replay]\nfps = 60\n";
    let mut state = State::read(PathBuf::from("/c/config.toml"), text, None);
    let json = state.to_json();
    assert_eq!(json["values"]["replay.fps"], 60);
    assert_eq!(json["values"]["record_show_frame"], false);
    assert_eq!(json["issues"][0]["key"], "record_cursor");
    assert_eq!(json["broken"], serde_json::Value::Null);
    assert_eq!(State::from_json(&json).unwrap(), state);
    state.broken = Some(Broken {
        line: None,
        reason: "x".into(),
    });
    assert_eq!(State::from_json(&state.to_json()).unwrap(), state);

    let patch = vec![
        ("record_fps".to_string(), Change::Set(Value::Int(60))),
        ("record_dir".to_string(), Change::Remove),
        (
            "ui_font".to_string(),
            Change::Set(Value::Text("Outfit".into())),
        ),
        (
            "record_show_frame".to_string(),
            Change::Set(Value::Bool(true)),
        ),
    ];
    let json = state::patch_to_json(&patch);
    assert_eq!(
        json.to_string(),
        r#"[["record_fps",60],["record_dir",null],["ui_font","Outfit"],["record_show_frame",true]]"#
    );
    assert_eq!(state::patch_from_json(&json).unwrap(), patch);
}

#[test]
fn a_patch_of_the_wrong_form_is_refused() {
    for bad in [
        r#"{"record_fps":60}"#,
        r#"[["record_fps",60.5]]"#,
        r#"[["record_fps",[1]]]"#,
        r#"[["record_fps"]]"#,
        r#"[[1,2]]"#,
        r#"[["record_fps",1,2]]"#,
    ] {
        let json: serde_json::Value = serde_json::from_str(bad).unwrap();
        assert!(state::patch_from_json(&json).is_err(), "{bad}");
    }
    let empty = serde_json::json!([]);
    assert_eq!(state::patch_from_json(&empty).unwrap(), vec![]);
}
