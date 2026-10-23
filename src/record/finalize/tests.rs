use super::*;
use std::fs::OpenOptions;
use std::os::unix::fs::PermissionsExt;

fn temp_dir(label: &str) -> PathBuf {
    loop {
        let path = std::env::temp_dir().join(format!(
            "boltsnap-finalize-{label}-{}-{}",
            std::process::id(),
            WORK_ID.fetch_add(1, Ordering::Relaxed)
        ));
        match fs::create_dir(&path) {
            Ok(()) => return path,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("create unique test directory: {error}"),
        }
    }
}

fn file(path: &Path, bytes: &[u8]) -> PathBuf {
    fs::write(path, bytes).unwrap();
    path.to_path_buf()
}

fn tools(dir: &Path, ffmpeg: &Path) -> RecorderTools {
    RecorderTools {
        gsr: None,
        wf_recorder: "unused".into(),
        ffmpeg: ffmpeg.into(),
        segment_dir: dir.into(),
    }
}

fn fake_ffmpeg(dir: &Path, succeeds: bool) -> PathBuf {
    let body = if succeeds {
        "#!/bin/sh\nfor last do :; done\nprintf output > \"$last\"\n"
    } else {
        "#!/bin/sh\nexit 7\n"
    };
    executable(dir.join("ffmpeg"), body)
}

fn fake_partial_ffmpeg(dir: &Path) -> PathBuf {
    executable(
        dir.join("ffmpeg-partial"),
        "#!/bin/sh\nfor last do :; done\nprintf partial > \"$last\"\nexit 7\n",
    )
}

fn executable(path: PathBuf, body: &str) -> PathBuf {
    let mut script = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    script.write_all(body.as_bytes()).unwrap();
    script.sync_all().unwrap();
    drop(script);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn request(
    segments: BTreeMap<Option<String>, Vec<PathBuf>>,
    mode: RecordBothMode,
    destination: SaveDestination,
) -> FinalizeRequest {
    FinalizeRequest {
        segments,
        monitors: Vec::new(),
        both_mode: mode,
        codec: "h264_nvenc".into(),
        fps: 120,
        destination,
        cursor: RecordCursor::System,
        region: None,
    }
}

fn monitor(name: &str, x: i32) -> Monitor {
    Monitor {
        name: name.into(),
        description: String::new(),
        x,
        y: 0,
        width: 1920,
        height: 1080,
        scale: 1.0,
        focused: x == 0,
    }
}

#[test]
fn promotion_keeps_the_container_extension() {
    let dir = temp_dir("promote-extension");
    for (name, extension) in [
        ("clip-1.mkv", Some("mkv")),
        ("rec.mp4", Some("mp4")),
        ("in.WebM", Some("webm")),
        ("odd.not an ext", None),
        ("bare", None),
    ] {
        let source = file(&dir.join(name), name.as_bytes());
        let saved = promote_recording(&source, &dir.join("saved"), None).unwrap();
        assert_eq!(fs::read(&saved).unwrap(), name.as_bytes(), "{name}");
        assert_eq!(
            saved.extension().and_then(|e| e.to_str()),
            extension,
            "{name}"
        );
        assert!(!source.exists(), "{name}");
    }
    let _ = fs::remove_dir_all(dir);
}

/// E1: a Combined disk save whose directory cannot be created keeps every
/// segment untouched, composes nothing, and saving again (no resume in
/// between) works.
#[test]
fn a_failed_combined_disk_save_can_be_saved_again() {
    let dir = temp_dir("combined-retry");
    let ffmpeg = fake_ffmpeg(&dir, true);
    let blocked = file(&dir.join("not-a-directory"), b"blocked");
    let segments = BTreeMap::from([
        (Some("A".into()), vec![file(&dir.join("a.mp4"), b"a")]),
        (Some("B".into()), vec![file(&dir.join("b.mp4"), b"b")]),
    ]);
    let mut first = request(
        segments.clone(),
        RecordBothMode::Combined,
        SaveDestination::Disk(blocked),
    );
    first.monitors = vec![monitor("A", 0), monitor("B", 1920)];
    let failure = finalize_recording(first, &tools(&dir, &ffmpeg)).unwrap_err();
    assert_eq!(failure.recoverable_segments, segments);
    assert_eq!(fs::read(dir.join("a.mp4")).unwrap(), b"a");
    let composed = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .any(|entry| entry.file_name().to_string_lossy().contains("combined"));
    assert!(!composed);

    let saved = dir.join("saved");
    let mut retry = request(
        failure.recoverable_segments,
        RecordBothMode::Combined,
        SaveDestination::Disk(saved.clone()),
    );
    retry.monitors = vec![monitor("A", 0), monitor("B", 1920)];
    let clips = finalize_recording(retry, &tools(&dir, &ffmpeg)).unwrap();
    assert_eq!(clips.len(), 1);
    assert!(clips[0].permanent);
    assert!(clips[0].path.starts_with(&saved));
    assert_eq!(fs::read(&clips[0].path).unwrap(), b"output");
    assert!(!dir.join("a.mp4").exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn one_stream_combined_clips_map_the_cursor_over_both_outputs() {
    let mut req = request(
        BTreeMap::new(),
        RecordBothMode::Combined,
        SaveDestination::Shelf,
    );
    req.monitors = vec![monitor("DP-1", 1920), monitor("DP-3", 0)];
    // One stream: no output key, no region, but the whole layout.
    assert_eq!(cursor_frame(&req, None, false), Some(((0.0, 0.0), 3840.0)));
    assert_eq!(
        cursor_frame(&req, Some("DP-1"), false),
        Some(((1920.0, 0.0), 1920.0))
    );
    req.region = Some(super::super::Geometry {
        x: 10,
        y: 20,
        w: 300,
        h: 200,
    });
    assert_eq!(cursor_frame(&req, None, false), Some(((10.0, 20.0), 300.0)));
}

#[test]
fn one_segment_fast_path_does_not_invoke_ffmpeg() {
    let dir = temp_dir("single");
    let segment = file(&dir.join("one.mp4"), b"video");
    let clips = finalize_recording(
        request(
            BTreeMap::from([(None, vec![segment.clone()])]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, Path::new("/definitely/missing/ffmpeg")),
    )
    .unwrap();
    assert_eq!(clips[0].path, segment);
    assert!(!clips[0].permanent);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn concat_uses_demuxer_and_stream_copy() {
    let args = build_concat_args(
        Path::new("/tmp/list.txt"),
        Path::new("/tmp/out.mp4"),
        Some(120),
    );
    assert_eq!(
        args,
        vec![
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            "/tmp/list.txt",
            "-c",
            "copy",
            "-video_track_timescale",
            "120",
            "/tmp/out.mp4"
        ]
    );
    let variable = build_concat_args(Path::new("/tmp/l.txt"), Path::new("/tmp/o.mp4"), None);
    assert!(!variable.iter().any(|arg| arg == "-video_track_timescale"));
}

#[test]
#[ignore = "requires the FFmpeg CLI with libx264"]
fn remuxed_gsr_segments_keep_exact_frame_times() {
    let dir = temp_dir("frame-grid");
    // Matroska stores whole milliseconds, like gsr's segments.
    let segment = dir.join("segment.mkv");
    let status = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x36:rate=120",
        ])
        .args(["-t", "0.5", "-c:v", "libx264", "-bf", "0", "-f", "matroska"])
        .arg(&segment)
        .status()
        .unwrap();
    assert!(status.success());
    let tools = tools(&dir, Path::new("ffmpeg"));
    let clip = finalize_group(
        None,
        std::slice::from_ref(&segment),
        Some(120),
        &dir,
        &tools,
    )
    .unwrap();

    let probe = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "packet=pts_time", "-of", "csv=p=0"])
        .arg(&clip)
        .output()
        .unwrap();
    let times: Vec<f64> = String::from_utf8(probe.stdout)
        .unwrap()
        .lines()
        .map(|line| line.parse().unwrap())
        .collect();
    assert_eq!(times.len(), 60);
    for time in times {
        let slot = (time * 120.0).round() / 120.0;
        assert!(
            (time - slot).abs() < 1e-5,
            "frame at {time} s is off its slot"
        );
    }
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn concat_list_escapes_single_quotes() {
    assert_eq!(
        escape_concat_path(Path::new("/tmp/it's.mp4")),
        "/tmp/it'\\''s.mp4"
    );
}

#[test]
fn combined_output_maps_only_first_optional_audio_stream() {
    let args = build_combined_args(
        &[PathBuf::from("left.mp4"), PathBuf::from("right.mp4")],
        "[0:v][1:v]xstack=inputs=2[v]",
        "libx264",
        Path::new("combined.mp4"),
    );
    let audio_maps = args
        .windows(2)
        .filter(|pair| pair[0] == "-map" && pair[1].contains(":a"))
        .collect::<Vec<_>>();
    assert_eq!(audio_maps.len(), 1);
    assert_eq!(audio_maps[0][1], "0:a?");
    assert!(args.windows(2).any(|pair| pair == ["-c:a", "copy"]));
}

#[test]
fn gpu_combined_encode_uploads_after_xstack() {
    let args = build_combined_args(
        &[PathBuf::from("left.mkv"), PathBuf::from("right.mkv")],
        "[0:v][1:v]xstack=inputs=2[v]",
        "h264_vulkan",
        Path::new("combined.mp4"),
    );
    assert_eq!(
        args[..5],
        [
            "-y",
            "-init_hw_device",
            "vulkan=hw",
            "-filter_hw_device",
            "hw"
        ]
    );
    assert!(args.contains(&"[0:v][1:v]xstack=inputs=2,format=nv12,hwupload[v]".to_owned()));
    assert!(args.windows(2).any(|pair| pair == ["-rc_mode", "cqp"]));
    let software = build_combined_args(
        &[PathBuf::from("left.mkv"), PathBuf::from("right.mkv")],
        "[0:v][1:v]xstack=inputs=2[v]",
        "h264_nvenc",
        Path::new("combined.mp4"),
    );
    assert!(
        !software
            .iter()
            .any(|arg| arg.contains("hwupload") || arg == "-init_hw_device")
    );
}

#[test]
fn separate_mode_keeps_one_clip_per_output() {
    let dir = temp_dir("separate");
    let a = file(&dir.join("a.mp4"), b"a");
    let b = file(&dir.join("b.mp4"), b"b");
    let clips = finalize_recording(
        request(
            BTreeMap::from([
                (Some("DP-3".into()), vec![a]),
                (Some("DP-1".into()), vec![b]),
            ]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, Path::new("unused")),
    )
    .unwrap();
    assert_eq!(clips.len(), 2);
    assert_eq!(
        clips
            .iter()
            .filter_map(|clip| clip.output.as_deref())
            .collect::<Vec<_>>(),
        vec!["DP-1", "DP-3"]
    );
    let _ = fs::remove_dir_all(dir);
}

/// Fake ffprobe beside `ffmpeg` that logs one line per call to `calls`.
fn fake_ffprobe(dir: &Path) -> PathBuf {
    executable(
        dir.join("ffprobe"),
        &format!(
            "#!/bin/sh\necho call >> '{}'\nprintf 'width=1920\\nheight=1080\\nduration=5\\n'\n",
            dir.join("calls").display()
        ),
    )
}

fn smooth_segment(path: &Path) -> PathBuf {
    let segment = file(path, b"segment");
    file(
        &PathBuf::from(format!("{}.ts", segment.display())),
        b"monotonic_microsec realtime_microsec\n1000 0\n",
    );
    file(
        &PathBuf::from(format!("{}.cursor", segment.display())),
        b"boltsnap-cursor 1\noutput 0 0\n2000 p 10 10\n",
    );
    segment
}

#[test]
fn cursor_json_reuses_the_segment_probe_for_the_clip_size() {
    let dir = temp_dir("cursor-probe");
    let segment = smooth_segment(&dir.join("a.mkv"));
    let ffmpeg = fake_ffmpeg(&dir, true);
    fake_ffprobe(&dir);
    let mut req = request(
        BTreeMap::from([(Some("DP-3".into()), vec![segment])]),
        RecordBothMode::Separate,
        SaveDestination::Shelf,
    );
    req.monitors = vec![monitor("DP-3", 0)];
    req.cursor = RecordCursor::Quick;
    let clips = finalize_recording(req, &tools(&dir, &ffmpeg)).unwrap();
    let json: serde_json::Value = serde_json::from_slice(
        &fs::read(super::super::cursor::sidecar_path(&clips[0].path)).unwrap(),
    )
    .unwrap();
    assert_eq!(json["width"], 1920);
    let calls = fs::read_to_string(dir.join("calls")).unwrap();
    assert_eq!(
        calls.lines().count(),
        1,
        "one ffprobe per segment, none for the clip"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn composed_combined_clips_still_probe_the_canvas() {
    let dir = temp_dir("cursor-compose");
    let a = smooth_segment(&dir.join("a.mkv"));
    let b = smooth_segment(&dir.join("b.mkv"));
    let ffmpeg = fake_ffmpeg(&dir, true);
    fake_ffprobe(&dir);
    let mut req = request(
        BTreeMap::from([
            (Some("DP-3".into()), vec![a]),
            (Some("DP-1".into()), vec![b]),
        ]),
        RecordBothMode::Combined,
        SaveDestination::Shelf,
    );
    req.monitors = vec![monitor("DP-3", 0), monitor("DP-1", 1920)];
    req.cursor = RecordCursor::Quick;
    let clips = finalize_recording(req, &tools(&dir, &ffmpeg)).unwrap();
    assert!(super::super::cursor::sidecar_path(&clips[0].path).is_file());
    // Two segment probes plus one for the composed canvas.
    let calls = fs::read_to_string(dir.join("calls")).unwrap();
    assert_eq!(calls.lines().count(), 3);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn combined_mode_composes_one_clip_and_removes_inputs_after_success() {
    let dir = temp_dir("combined");
    let a = file(&dir.join("a.mp4"), b"a");
    let b = file(&dir.join("b.mp4"), b"b");
    let ffmpeg = fake_ffmpeg(&dir, true);
    let mut req = request(
        BTreeMap::from([
            (Some("DP-3".into()), vec![a.clone()]),
            (Some("DP-1".into()), vec![b.clone()]),
        ]),
        RecordBothMode::Combined,
        SaveDestination::Shelf,
    );
    req.monitors = vec![monitor("DP-3", 0), monitor("DP-1", 1920)];
    let clips = finalize_recording(req, &tools(&dir, &ffmpeg)).unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(clips[0].output, None);
    assert!(clips[0].path.is_file());
    assert!(!a.exists());
    assert!(!b.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn failed_combined_encode_removes_partial_output_and_keeps_inputs() {
    let dir = temp_dir("partial-combined");
    let a = file(&dir.join("a.mp4"), b"a");
    let b = file(&dir.join("b.mp4"), b"b");
    let ffmpeg = fake_partial_ffmpeg(&dir);
    let mut req = request(
        BTreeMap::from([
            (Some("DP-3".into()), vec![a.clone()]),
            (Some("DP-1".into()), vec![b.clone()]),
        ]),
        RecordBothMode::Combined,
        SaveDestination::Shelf,
    );
    req.monitors = vec![monitor("DP-3", 0), monitor("DP-1", 1920)];
    assert!(finalize_recording(req, &tools(&dir, &ffmpeg)).is_err());
    assert!(a.is_file());
    assert!(b.is_file());
    assert!(
        !fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains("-combined-"))
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn combined_layout_uses_hyprland_positions() {
    let filter = build_xstack_filter(&[
        Monitor {
            name: "DP-3".into(),
            description: "BenQ".into(),
            x: 0,
            y: 0,
            width: 2560,
            height: 1440,
            scale: 1.0,
            focused: true,
        },
        Monitor {
            name: "DP-1".into(),
            description: "AOC".into(),
            x: 2560,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: false,
        },
    ])
    .unwrap();
    assert!(filter.contains("xstack=inputs=2:layout=0_0|2560_0:fill=black"));
}

#[test]
fn combined_layout_uses_largest_scale_without_downscaling() {
    let filter = build_xstack_filter(&[
        Monitor {
            name: "A".into(),
            description: String::new(),
            x: 0,
            y: 0,
            width: 3840,
            height: 2160,
            scale: 2.0,
            focused: true,
        },
        Monitor {
            name: "B".into(),
            description: String::new(),
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            focused: false,
        },
    ])
    .unwrap();
    assert!(filter.contains("[0:v]scale=3840:2160"));
    assert!(filter.contains("[1:v]scale=3840:2160"));
    assert!(filter.contains("layout=0_0|3840_0"));
}

#[test]
fn combined_layout_fits_hardware_h264_canvas() {
    let monitor = |name: &str, x, scale| Monitor {
        name: name.into(),
        description: String::new(),
        x,
        y: 0,
        width: 1920,
        height: 1080,
        scale,
        focused: false,
    };
    let (a, b) = (monitor("DP-3", 0, 1.0), monitor("DP-1", 1920, 1.5));
    // 1.5 would need a 4800x1620 canvas; NVENC H.264 stops at 4096.
    let filter = build_xstack_filter_for(&[&a, &b], canvas_limit("h264_nvenc")).unwrap();
    assert!(filter.contains("[0:v]scale=2458:1382"));
    assert!(filter.contains("[1:v]scale=1638:922"));
    assert!(filter.contains("layout=0_0|2458_0"));
    assert_eq!(canvas_limit("libx264"), f64::INFINITY);
}

#[test]
fn nvenc_combined_quality_is_visually_lossless() {
    assert_eq!(
        quality_args("h264_nvenc"),
        vec![
            "-preset", "p1", "-tune", "hq", "-rc", "vbr", "-cq", "16", "-b:v", "0"
        ]
    );
    assert_eq!(
        quality_args("libx264"),
        vec!["-preset", "veryfast", "-crf", "16"]
    );
    assert_eq!(quality_args("vp9"), vec!["-q:v", "2"]);
}

#[test]
fn insufficient_space_is_rejected() {
    let dir = temp_dir("space");
    let error = ensure_free_space(&dir, u64::MAX / 2).unwrap_err();
    assert!(error.contains("insufficient disk space"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn available_space_reports_real_filesystem_capacity() {
    let dir = temp_dir("available-space");
    assert!(available_space(&dir).unwrap() > 0);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recording_reserve_requires_more_than_two_gibibytes() {
    assert!(require_recording_reserve(RECORDING_DISK_RESERVE_BYTES).is_err());
    assert!(require_recording_reserve(RECORDING_DISK_RESERVE_BYTES + 1).is_ok());
}

#[test]
fn recording_cache_counts_regular_files() {
    let dir = temp_dir("recording-cache");
    File::create(dir.join("clip.mp4"))
        .unwrap()
        .set_len(RECORDING_CACHE_LIMIT_BYTES)
        .unwrap();

    let error = check_recording_cache_limit(&dir).unwrap_err();

    assert!(error.contains("temporary recording cache reached 2 GiB"));
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn recording_cache_requires_less_than_two_gibibytes() {
    assert!(require_recording_cache_limit(RECORDING_CACHE_LIMIT_BYTES).is_err());
    assert!(require_recording_cache_limit(RECORDING_CACHE_LIMIT_BYTES - 1).is_ok());
}

#[test]
fn single_matroska_segment_is_remuxed_to_mp4() {
    let dir = temp_dir("single-mkv");
    let segment = file(&dir.join("one.mkv"), b"video");
    let ffmpeg = fake_ffmpeg(&dir, true);
    let clips = finalize_recording(
        request(
            BTreeMap::from([(None, vec![segment.clone()])]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, &ffmpeg),
    )
    .unwrap();
    assert_eq!(clips[0].path.extension().unwrap(), "mp4");
    assert!(clips[0].path.is_file());
    assert!(!segment.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn failed_ffmpeg_preserves_all_segments() {
    let dir = temp_dir("failure");
    let first = file(&dir.join("one.mp4"), b"one");
    let second = file(&dir.join("two.mp4"), b"two");
    let ffmpeg = fake_ffmpeg(&dir, false);
    let result = finalize_recording(
        request(
            BTreeMap::from([(None, vec![first.clone(), second.clone()])]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, &ffmpeg),
    );
    let failure = result.unwrap_err();
    assert!(failure.completed.is_empty());
    assert!(first.is_file());
    assert!(second.is_file());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn ffmpeg_failure_includes_stderr() {
    let dir = temp_dir("ffmpeg-stderr");
    let ffmpeg = executable(
        dir.join("ffmpeg-error"),
        "#!/bin/sh\nprintf 'nvenc exploded' >&2\nexit 7\n",
    );
    let error = run_ffmpeg(&ffmpeg, &[]).unwrap_err();
    assert!(error.contains("nvenc exploded"), "{error}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn failed_concat_removes_partial_output_and_list_but_keeps_inputs() {
    let dir = temp_dir("partial-concat");
    let first = file(&dir.join("one.mp4"), b"one");
    let second = file(&dir.join("two.mp4"), b"two");
    let ffmpeg = fake_partial_ffmpeg(&dir);
    let result = finalize_recording(
        request(
            BTreeMap::from([(None, vec![first.clone(), second.clone()])]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, &ffmpeg),
    );
    assert!(result.is_err());
    assert!(first.is_file());
    assert!(second.is_file());
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.contains("-concat-") || name.contains("-final-")
        })
        .collect();
    assert!(leftovers.is_empty(), "leftover work files: {leftovers:?}");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn successful_concat_removes_segments_after_output_exists() {
    let dir = temp_dir("concat-success");
    let first = file(&dir.join("one.mp4"), b"one");
    let second = file(&dir.join("two.mp4"), b"two");
    let ffmpeg = fake_ffmpeg(&dir, true);
    let clips = finalize_recording(
        request(
            BTreeMap::from([(None, vec![first.clone(), second.clone()])]),
            RecordBothMode::Separate,
            SaveDestination::Shelf,
        ),
        &tools(&dir, &ffmpeg),
    )
    .unwrap();
    assert!(clips[0].path.is_file());
    assert!(!first.exists());
    assert!(!second.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn same_filesystem_save_renames_source() {
    let dir = temp_dir("rename");
    let source = file(&dir.join("source.mp4"), b"video");
    let destination = dir.join("saved.mp4");
    move_final_file(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), b"video");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn same_filesystem_move_never_overwrites_an_existing_destination() {
    let dir = temp_dir("rename-no-replace");
    let source = file(&dir.join("source.mp4"), b"source");
    let destination = file(&dir.join("saved.mp4"), b"existing");
    let error = move_final_file(&source, &destination).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&source).unwrap(), b"source");
    assert_eq!(fs::read(&destination).unwrap(), b"existing");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn partial_disk_save_reports_completed_and_only_unmoved_recoverable_segments() {
    let dir = temp_dir("partial-disk");
    let cache = dir.join("cache");
    let saved = dir.join("saved");
    fs::create_dir_all(&cache).unwrap();
    let first = file(&cache.join("first.mp4"), b"first");
    let second = file(&cache.join("second.mp4"), b"second");
    let ready = BTreeMap::from([
        (Some("DP-1".into()), vec![first.clone()]),
        (Some("DP-3".into()), vec![second.clone()]),
    ]);
    let mut calls = 0;
    let failure = move_to_disk_with(ready, &saved, |source, destination| {
        calls += 1;
        if calls == 2 {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected"))
        } else {
            move_final_file(source, destination)
        }
    })
    .unwrap_err();
    assert_eq!(failure.completed.len(), 1);
    assert!(failure.completed[0].permanent);
    assert_eq!(failure.recoverable_segments.len(), 1);
    assert!(
        failure
            .recoverable_segments
            .contains_key(&Some("DP-3".into()))
    );
    assert!(!first.exists());
    assert!(second.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn disk_save_retries_a_raced_name_without_overwriting_it() {
    let dir = temp_dir("disk-race");
    let cache = dir.join("cache");
    let saved = dir.join("saved");
    fs::create_dir_all(&cache).unwrap();
    let source = file(&cache.join("source.mp4"), b"source");
    let ready = BTreeMap::from([(None, vec![source])]);
    let mut raced = None;
    let mut calls = 0;
    let clips = move_to_disk_with(ready, &saved, |source, destination| {
        calls += 1;
        if calls == 1 {
            fs::write(destination, b"raced")?;
            raced = Some(destination.to_path_buf());
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        } else {
            move_final_file(source, destination)
        }
    })
    .unwrap();
    assert_eq!(fs::read(raced.unwrap()).unwrap(), b"raced");
    assert_eq!(fs::read(&clips[0].path).unwrap(), b"source");
    assert_eq!(calls, 2);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn shelf_promotion_never_overwrites_and_removes_temporary_sources() {
    let dir = temp_dir("promote");
    let destination = dir.join("saved");
    let first = file(&dir.join("first.mp4"), b"first");
    let second = file(&dir.join("second.mp4"), b"second");

    let first_saved = promote_recording(&first, &destination, None).unwrap();
    let second_saved = promote_recording(&second, &destination, None).unwrap();

    assert_ne!(first_saved, second_saved);
    assert_eq!(fs::read(first_saved).unwrap(), b"first");
    assert_eq!(fs::read(second_saved).unwrap(), b"second");
    assert!(!first.exists());
    assert!(!second.exists());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn disk_save_moves_one_segment_to_a_permanent_unique_name() {
    let dir = temp_dir("disk-save");
    let cache = dir.join("cache");
    let saved = dir.join("saved");
    fs::create_dir_all(&cache).unwrap();
    let source = file(&cache.join("source.mp4"), b"video");
    let clips = finalize_recording(
        request(
            BTreeMap::from([(Some("DP 1".into()), vec![source.clone()])]),
            RecordBothMode::Separate,
            SaveDestination::Disk(saved),
        ),
        &tools(&cache, Path::new("unused")),
    )
    .unwrap();
    assert!(clips[0].permanent);
    assert!(clips[0].path.is_file());
    assert!(!source.exists());
    assert!(
        clips[0]
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("-DP-1.mp4")
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn copy_preserves_another_operations_partial_file() {
    let dir = temp_dir("foreign-part");
    let source = file(&dir.join("source.mp4"), b"video");
    let destination = dir.join("saved.mp4");
    let part = file(
        &dir.join(format!("saved.mp4.part-{}", std::process::id())),
        b"another save",
    );

    let result = copy_then_remove(&source, &destination);
    assert_eq!(fs::read(part).unwrap(), b"another save");
    result.unwrap();
    assert_eq!(fs::read(destination).unwrap(), b"video");
    assert!(!source.exists());
}

#[test]
fn completed_copy_never_overwrites_target_and_preserves_source_on_commit_failure() {
    let dir = temp_dir("copy-no-replace");
    let source = file(&dir.join("source.mp4"), b"video");
    let destination = file(&dir.join("saved.mp4"), b"existing");
    let error = copy_then_remove(&source, &destination).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(source).unwrap(), b"video");
    assert_eq!(fs::read(destination).unwrap(), b"existing");
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn committed_cross_filesystem_copy_survives_source_cleanup_failure() {
    let dir = temp_dir("copy-cleanup");
    let source_dir = dir.join("source");
    let destination_dir = dir.join("destination");
    fs::create_dir_all(&source_dir).unwrap();
    fs::create_dir_all(&destination_dir).unwrap();
    let source = file(&source_dir.join("source.mp4"), b"video");
    let destination = destination_dir.join("saved.mp4");
    fs::set_permissions(&source_dir, fs::Permissions::from_mode(0o555)).unwrap();
    let result = copy_then_remove(&source, &destination);
    fs::set_permissions(&source_dir, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(result.is_ok());
    assert_eq!(fs::read(&destination).unwrap(), b"video");
    assert!(source.is_file());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn real_cross_filesystem_move_dispatches_when_available() {
    let source_root = temp_dir("real-cross-source");
    let Some(other_root) = [Path::new("/dev/shm"), Path::new("/run/user")]
        .into_iter()
        .find(|path| {
            path.is_dir()
                && fs::metadata(path).map(|m| m.dev()).ok()
                    != fs::metadata(&source_root).map(|m| m.dev()).ok()
        })
    else {
        let _ = fs::remove_dir_all(source_root);
        return;
    };
    let destination_root = other_root.join(format!(
        "boltsnap-cross-test-{}-{}",
        std::process::id(),
        WORK_ID.fetch_add(1, Ordering::Relaxed)
    ));
    if fs::create_dir(&destination_root).is_err() {
        let _ = fs::remove_dir_all(source_root);
        return;
    }
    let source = file(&source_root.join("source.mp4"), b"video");
    let destination = destination_root.join("saved.mp4");
    move_final_file(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(fs::read(destination).unwrap(), b"video");
    let _ = fs::remove_dir_all(source_root);
    let _ = fs::remove_dir_all(destination_root);
}

mod recoverable;
