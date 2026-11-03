use super::capture::{PostCapture, decide_post_capture};
use super::recording::{is_legacy_recording_status_eof, recording_state_from_status};
use super::*;

#[test]
fn legacy_recording_status_fallback_accepts_only_eof() {
    assert!(is_legacy_recording_status_eof(&std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        "old daemon closed the connection",
    )));
    assert!(!is_legacy_recording_status_eof(&std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "daemon stalled",
    )));
    assert!(!is_legacy_recording_status_eof(&std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "malformed response",
    )));
}

#[test]
fn successful_recording_status_requires_snapshot() {
    let error = recording_state_from_status(crate::ipc::Response::ok(None)).unwrap_err();
    assert_eq!(error, "daemon returned no recording status");

    let state = recording_state_from_status(crate::ipc::Response::ok(Some(
        crate::ipc::RecordingSnapshot::idle(),
    )))
    .unwrap();
    assert_eq!(state, crate::record::session::PublicRecordingState::Idle);
}

#[test]
fn daemon_ingest_error_is_reported_to_capture_cli() {
    let error = require_daemon_success(
        crate::ipc::Response::error("decode PNG failed"),
        "shelf ingest failed",
    )
    .unwrap_err();
    assert_eq!(error, "decode PNG failed");
}

#[test]
fn parser_recording_cli_forms() {
    for action in [
        "show-controls",
        "pause",
        "resume",
        "save-shelf",
        "save-disk",
        "discard",
    ] {
        let args = parse_args(&["boltsnap".into(), "recording".into(), action.into()]).unwrap();
        assert_eq!(args.command, "recording");
        assert_eq!(args.tail, [action]);
        assert!(!args.json);
    }
}

#[test]
fn parser_recording_status_and_watch_require_json_flag_data() {
    for action in ["status", "watch"] {
        let args = parse_args(&[
            "boltsnap".into(),
            "recording".into(),
            action.into(),
            "--json".into(),
        ])
        .unwrap();
        assert_eq!(args.tail, [action]);
        assert!(args.json);
    }
}

#[test]
fn parser_defaults_to_area_copy() {
    let args = parse_args(&["boltsnap".into()]).unwrap();
    assert_eq!(args.command, "area");
    assert!(args.copy);
}

#[test]
fn a_wayland_shelf_capture_copies_by_the_config_unless_a_flag_says() {
    let args = |flag: Option<bool>| Args {
        copy: flag.unwrap_or(true),
        copy_explicit: flag.is_some(),
        ..Args::default()
    };
    let shelf = |copy| PostCapture::Shelf { copy };
    // No flag: the config decides on Wayland.
    assert_eq!(
        decide_post_capture(&args(None), Backend::Wayland, false),
        shelf(false)
    );
    assert_eq!(
        decide_post_capture(&args(None), Backend::Wayland, true),
        shelf(true)
    );
    // A flag goes first.
    assert_eq!(
        decide_post_capture(&args(Some(false)), Backend::Wayland, true),
        shelf(false)
    );
    assert_eq!(
        decide_post_capture(&args(Some(true)), Backend::Wayland, false),
        shelf(true)
    );
    // Windows and X11 as before.
    assert_eq!(
        decide_post_capture(&args(None), Backend::Windows, true),
        shelf(false)
    );
    assert_eq!(
        decide_post_capture(&args(None), Backend::X11, true),
        PostCapture::CopyOnly
    );
}

#[test]
fn parser_rejects_removed_editor_flags() {
    assert!(parse_args(&["boltsnap".into(), "--edit".into()]).is_err());
    assert!(
        parse_args(&[
            "boltsnap".into(),
            "--editor".into(),
            "external-editor".into()
        ])
        .is_err()
    );
}

#[test]
fn parser_handles_window_modes() {
    let args = parse_args(&["boltsnap".into(), "window".into()]).unwrap();
    assert_eq!(
        CaptureMode::parse(&args.command).unwrap(),
        CaptureMode::Window
    );
    assert_eq!(
        CaptureMode::parse("active-window").unwrap(),
        CaptureMode::ActiveWindow
    );
    assert_eq!(CaptureMode::parse("select").unwrap(), CaptureMode::Area);
}

#[test]
fn parser_handles_instant_flag() {
    let a = parse_args(&[
        "boltsnap".to_string(),
        "area".to_string(),
        "--instant".to_string(),
    ])
    .unwrap();
    assert!(a.instant);
    let d = parse_args(&["boltsnap".to_string(), "area".to_string()]).unwrap();
    assert!(!d.instant);
}
