# Recording Audio and Desktop Popup Font Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Record optional system and/or microphone audio in every Boltsnap video, expose the requested selector and tray controls, and render the Alt+Print recording popup with the current desktop UI font.

**Architecture:** Persist the audio toggle and source beside the existing recording preferences, carry the region toggle through IPC, and resolve one PulseAudio-compatible source immediately before a recording starts. Direct modes use the default source or default sink monitor; the combined mode owns a temporary null-sink mix for the lifetime of the recording session. The popup resolves the desktop font through GSettings and Fontconfig whenever it opens and renders it through the existing `ab_glyph` dependency, with an embedded DejaVu fallback.

**Tech Stack:** Rust 2024, `wf-recorder`, PipeWire-Pulse/`pactl`, FFmpeg/`ffprobe`, Smithay Client Toolkit, tiny-skia, `ab_glyph`, Fontconfig, GSettings, Cargo tests.

## Global Constraints

- Preserve all pre-existing dirty worktree changes. Inspect `git diff -- <file>` before each edit and stage only the files named by the current task; never use `git add -A` or `git add .`.
- Follow strict RED → GREEN: add the named test first, run the exact targeted command, observe the stated failure, then write the minimum implementation.
- Keep the video frame path unchanged. Audio setup and cleanup run only at recording lifecycle boundaries.
- Do not add a Rust crate or a QuickShell dependency. Reuse `ab_glyph`; invoke the already-required desktop commands directly.
- Audio-disabled recordings must not invoke `pactl` and must produce the current `wf-recorder` argument list byte-for-byte.
- An enabled audio setup failure aborts recording with a specific error; it must never silently fall back to video-only.
- Keep the temporary system+microphone mix alive across pause/resume and finalization retry. Remove it only on successful terminal save, discard, unrecoverable start failure, daemon shutdown, or stale-module cleanup at the next daemon start.
- Run `cargo fmt --check` after formatting each task and `cargo test` before the final handoff.

---

## Task 1: Persist audio preferences and carry the region toggle through IPC

**Files:**

- Modify: `src/config.rs` (`RecordBothMode`, `RecordingPrefs`, `Config::parse`, `Config::recording_prefs`, `save_recording_prefs_at`, config tests)
- Modify: `src/ipc.rs` (`Request::StartRecording`, encoder, decoder, IPC tests)
- Modify: `src/main.rs` (`record_flow`)
- Modify: `src/shelf/mod.rs` (`Request::StartRecording` handler only)
- Modify: `src/tray.rs` and any other existing `RecordingPrefs` literals only to add the new required fields or `..RecordingPrefs::default()`

**Interfaces:**

- Consumes: existing TOML preference table and `record` IPC frame.
- Produces: `RecordAudioSource`, `RecordingPrefs::{audio_enabled,audio_source}`, TOML keys `record_audio_enabled` and `record_audio_source`, and `Request::StartRecording::audio_enabled`.

### Step 1.1: Add failing config contract tests

- [ ] Add these tests to `src/config.rs`'s existing `#[cfg(test)]` module:

```rust
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
    assert_eq!(Config::parse(&saved).recording_prefs().audio_source, RecordAudioSource::Mic);
    let _ = std::fs::remove_file(path);
}

#[test]
fn invalid_audio_source_uses_default() {
    let prefs = Config::parse("record_audio_source = \"bluetooth\"\n").recording_prefs();
    assert_eq!(prefs.audio_source, RecordAudioSource::SystemAndMic);
}
```

- [ ] Run `cargo test config::tests::audio_preferences -- --nocapture`.
- [ ] Expected RED result: compilation fails because `RecordAudioSource`, `audio_enabled`, and `audio_source` do not exist.

### Step 1.2: Implement the preference model and TOML mapping

- [ ] Add the enum and fields directly after `RecordBothMode`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordAudioSource {
    SystemAndMic,
    Mic,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordingPrefs {
    pub default_target: RecordDefaultTarget,
    pub both_mode: RecordBothMode,
    pub show_frame: bool,
    pub disk_add_to_shelf: bool,
    pub audio_enabled: bool,
    pub audio_source: RecordAudioSource,
}
```

- [ ] Set these defaults in `RecordingPrefs::default()`:

```rust
audio_enabled: true,
audio_source: RecordAudioSource::SystemAndMic,
```

- [ ] Add private optional fields to `Config`:

```rust
record_audio_enabled: Option<bool>,
record_audio_source: Option<String>,
```

- [ ] Parse them in `Config::parse`:

```rust
record_audio_enabled: v.get("record_audio_enabled").and_then(|x| x.as_bool()),
record_audio_source: v
    .get("record_audio_source")
    .and_then(|x| x.as_str())
    .map(String::from),
```

- [ ] Resolve them in `Config::recording_prefs`:

```rust
audio_enabled: self.record_audio_enabled.unwrap_or(defaults.audio_enabled),
audio_source: match self.record_audio_source.as_deref() {
    Some("system-and-mic") => RecordAudioSource::SystemAndMic,
    Some("mic") => RecordAudioSource::Mic,
    Some("system") => RecordAudioSource::System,
    _ => defaults.audio_source,
},
```

- [ ] Save both values in `save_recording_prefs_at` without rebuilding the TOML table:

```rust
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
```

- [ ] Update every existing explicit `RecordingPrefs { ... }` literal reported by `rg -n 'RecordingPrefs \{' src` with `audio_enabled: true` and `audio_source: RecordAudioSource::SystemAndMic`, or use `..RecordingPrefs::default()` when the test does not care about the other defaults.
- [ ] Run `cargo test config::tests -- --nocapture`.
- [ ] Expected GREEN result: all config tests pass, including unknown-key preservation.

### Step 1.3: Add failing IPC compatibility tests

- [ ] Extend `request_start_recording_roundtrip` in `src/ipc.rs` with `audio_enabled: false`, bind it in the decoded pattern, and assert `!audio_enabled`.
- [ ] Add this compatibility test beside `start_recording_missing_frame_field_defaults_true`:

```rust
#[test]
fn start_recording_missing_audio_field_defaults_true() {
    let mut bytes = Vec::new();
    write_frame(
        &mut bytes,
        br#"{"cmd":"record","x":1,"y":2,"w":3,"h":4}"#,
        &[],
    )
    .unwrap();
    assert!(matches!(
        Request::read(&mut Cursor::new(bytes)).unwrap(),
        Request::StartRecording {
            audio_enabled: true,
            ..
        }
    ));
}
```

- [ ] Run `cargo test ipc::tests::request_start_recording_roundtrip -- --nocapture`.
- [ ] Run `cargo test ipc::tests::start_recording_missing_audio_field_defaults_true -- --nocapture`.
- [ ] Expected RED result: compilation fails because `Request::StartRecording` has no `audio_enabled` field.

### Step 1.4: Implement IPC and current-preference propagation

- [ ] Add `audio_enabled: bool` to `Request::StartRecording`.
- [ ] Add `"audio_enabled": audio_enabled` to the encoded JSON object.
- [ ] Decode it compatibly with:

```rust
audio_enabled: v
    .get("audio_enabled")
    .and_then(Value::as_bool)
    .unwrap_or(true),
```

- [ ] In `main.rs::record_flow`, keep using the currently persisted value until Task 2 adds the selector toggle, and send it in the request:

```rust
audio_enabled: prefs.audio_enabled,
```

- [ ] In `shelf/mod.rs`'s `Request::StartRecording` arm, bind `audio_enabled`, update both ordered preference copies before calling `persist_tray_prefs`, and leave the start call unchanged for now:

```rust
self.persisted_recording_prefs.audio_enabled = audio_enabled;
let mut prefs = self.recording_prefs.clone();
prefs.show_frame = show_frame;
prefs.audio_enabled = audio_enabled;
self.persist_tray_prefs(prefs);
```

- [ ] Update remaining `Request::StartRecording` constructors and patterns reported by `rg -n 'StartRecording \{' src`.
- [ ] Run `cargo test ipc::tests -- --nocapture`.
- [ ] Run `cargo test config::tests -- --nocapture`.
- [ ] Expected GREEN result: IPC round-trip and missing-field compatibility tests pass.

### Step 1.5: Verify and commit

- [ ] Run `cargo fmt`.
- [ ] Run `cargo fmt --check`.
- [ ] Run `cargo test config::tests -- --nocapture`.
- [ ] Run `cargo test ipc::tests -- --nocapture`.
- [ ] Inspect `git diff -- src/config.rs src/ipc.rs src/main.rs src/shelf/mod.rs src/tray.rs` and confirm unrelated dirty hunks remain intact.
- [ ] Stage only touched task files: `git add src/config.rs src/ipc.rs src/main.rs src/shelf/mod.rs src/tray.rs`.
- [ ] Commit: `git commit -m "feat: persist recording audio preferences"`.

---

## Task 2: Add selector audio toggle and tray audio-source submenu

**Files:**

- Modify: `src/select_skia/mod.rs` (`RecordSelectionResult`, selector state, draw path, record-control hit handling)
- Modify: `src/select_skia/render.rs` (audio button geometry and painting, render tests)
- Modify: `src/main.rs` (`record_flow` selector call and persistence)
- Modify: `src/tray.rs` (action, menu model, submenu, tests)
- Modify: `src/shelf/mod.rs` (`TrayAction::SetAudioSource` handler)

**Interfaces:**

- Consumes: `RecordingPrefs::audio_enabled` and `RecordingPrefs::audio_source`.
- Produces: `RecordSelectionResult::audio_enabled`, selector button labels `AUDIO ON`/`AUDIO OFF`, and `TrayAction::SetAudioSource`.

### Step 2.1: Add failing selector geometry and paint tests

- [ ] Add the following helpers and tests to `src/select_skia/render.rs`'s test module. Reuse its existing rectangle overlap helper if present; otherwise add the shown local one:

```rust
fn overlaps(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
    a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
}

#[test]
fn record_audio_button_stays_visible_and_clear_of_other_controls() {
    for sel in [
        (8.0, 8.0, 160.0, 100.0),
        (232.0, 8.0, 160.0, 100.0),
        (8.0, 192.0, 160.0, 100.0),
        (232.0, 192.0, 160.0, 100.0),
    ] {
        let rec = rec_pill_rect(sel, 400, 300).unwrap();
        let frame = record_frame_checkbox_rect(sel, 400, 300).unwrap();
        let audio = record_audio_button_rect(sel, 400, 300).unwrap();
        assert!(!overlaps(audio, rec));
        assert!(!overlaps(audio, frame));
        assert!(audio.0 >= 0.0 && audio.1 >= 0.0);
        assert!(audio.0 + audio.2 <= 400.0 && audio.1 + audio.3 <= 300.0);
    }
}

#[test]
fn record_audio_button_has_distinct_on_and_off_rendering() {
    let (w, h) = (400, 300);
    let sel = (80.0, 80.0, 200.0, 120.0);
    let mut on = Pixmap::new(w, h).unwrap();
    let mut off = Pixmap::new(w, h).unwrap();
    draw_record_audio_button(&mut on, sel, w, h, true);
    draw_record_audio_button(&mut off, sel, w, h, false);
    assert_ne!(on.data(), off.data());
    assert!(on.data().iter().any(|byte| *byte != 0));
    assert!(off.data().iter().any(|byte| *byte != 0));
}
```

- [ ] Run `cargo test select_skia::render::tests::record_audio_button -- --nocapture`.
- [ ] Expected RED result: compilation fails because `record_audio_button_rect` and `draw_record_audio_button` do not exist.

### Step 2.2: Implement deterministic control layout and rendering

- [ ] Add constants next to the frame-control constants:

```rust
const AUDIO_GAP: f64 = 6.0;
const AUDIO_LABEL_ON: &str = "AUDIO ON";
const AUDIO_LABEL_OFF: &str = "AUDIO OFF";
const AUDIO_PX: f32 = 13.0;
const AUDIO_PAD: f64 = 7.0;
const AUDIO_DOT_R: f64 = 4.0;
const AUDIO_DOT_GAP: f64 = 6.0;
```

- [ ] Implement `record_audio_button_rect` with this exact placement policy: measure the wider `AUDIO OFF` label; try immediately after the frame control when it is right of REC; try immediately before the frame control when it is left of REC; then try the same x as REC directly below the REC row; finally try directly above it. Clamp the fallback x to `[0, surf_w - width]`; return `None` only when the surface is smaller than the control itself.
- [ ] Use this complete function body after calculating `width` and `height` from `badge_font`, `AUDIO_PX`, and `AUDIO_PAD`:

```rust
let rec = rec_pill_rect(sel, surf_w, surf_h)?;
let frame = record_frame_checkbox_rect(sel, surf_w, surf_h)?;
let candidates = if frame.0 >= rec.0 + rec.2 {
    [
        (frame.0 + frame.2 + AUDIO_GAP, frame.1),
        (frame.0 - AUDIO_GAP - width, frame.1),
    ]
} else {
    [
        (frame.0 - AUDIO_GAP - width, frame.1),
        (frame.0 + frame.2 + AUDIO_GAP, frame.1),
    ]
};
for (x, y) in candidates {
    let candidate = (x, y, width, height);
    let overlaps = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| {
        a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
    };
    if x >= 0.0
        && y >= 0.0
        && x + width <= surf_w as f64
        && y + height <= surf_h as f64
        && !overlaps(candidate, rec)
        && !overlaps(candidate, frame)
    {
        return Some((x, y, width, height));
    }
}
let x = rec.0.min((surf_w as f64 - width).max(0.0));
for y in [rec.1 + rec.3 + AUDIO_GAP, rec.1 - AUDIO_GAP - height] {
    if y >= 0.0 && y + height <= surf_h as f64 && width <= surf_w as f64 {
        return Some((x, y, width, height));
    }
}
None
```

- [ ] Implement `draw_record_audio_button` using the existing rounded-rectangle and `draw_text_aa` helpers: background `(0x12,0x12,0x12,230)`, label chosen from the two constants, red dot `(0xff,0x3b,0x30)` and bright label when enabled, gray dot `(0x70,0x70,0x70)` and label `(0xa0,0xa0,0xa0)` when disabled.
- [ ] Run `cargo test select_skia::render::tests::record_audio_button -- --nocapture`.
- [ ] Expected GREEN result: both new render tests pass.

### Step 2.3: Add failing selector hit-state test, then wire the toggle

- [ ] Add a `#[cfg(test)] mod tests` to `src/select_skia/mod.rs` (or extend it if present) with a test named `audio_control_hit_does_not_confirm`. The test obtains the audio rectangle, clicks its center, and asserts `record_control_hit(...) == Some(RecordControlHit::Audio)`.
- [ ] Run `cargo test select_skia::tests::audio_control_hit_does_not_confirm -- --nocapture`.
- [ ] Expected RED result: compilation fails because `record_control_hit` and `RecordControlHit` do not exist.
- [ ] Implement pure hit testing in `src/select_skia/mod.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecordControlHit {
    Audio,
    Frame,
    Record,
}

fn contains(rect: (f64, f64, f64, f64), point: (f64, f64)) -> bool {
    point.0 >= rect.0
        && point.0 < rect.0 + rect.2
        && point.1 >= rect.1
        && point.1 < rect.1 + rect.3
}

fn record_control_hit(
    sel: (f32, f32, f32, f32),
    surf_w: u32,
    surf_h: u32,
    point: (f64, f64),
) -> Option<RecordControlHit> {
    if render::record_audio_button_rect(sel, surf_w, surf_h).is_some_and(|r| contains(r, point)) {
        Some(RecordControlHit::Audio)
    } else if render::record_frame_checkbox_rect(sel, surf_w, surf_h).is_some_and(|r| contains(r, point)) {
        Some(RecordControlHit::Frame)
    } else if render::rec_pill_rect(sel, surf_w, surf_h).is_some_and(|r| contains(r, point)) {
        Some(RecordControlHit::Record)
    } else {
        None
    }
}
```

- [ ] Add `audio_enabled: bool` to `Selector` and `RecordSelectionResult`.
- [ ] Change the public signature to:

```rust
pub fn run_select_record(
    initial_show_frame: bool,
    initial_audio_enabled: bool,
) -> DynResult<RecordSelectionResult>
```

- [ ] Add the audio boolean to `run_selector`; screenshot mode passes `true` but ignores it, recording mode passes the persisted value.
- [ ] Draw `draw_record_audio_button` immediately after `draw_record_frame_checkbox`.
- [ ] Replace the duplicated record-control pointer checks with this match before selection editing begins:

```rust
match record_control_hit(sel, self.surf_w, self.surf_h, (x, y)) {
    Some(RecordControlHit::Audio) => {
        self.audio_enabled = !self.audio_enabled;
        self.interaction = None;
        self.request_redraw();
        return;
    }
    Some(RecordControlHit::Frame) => {
        self.show_frame = !self.show_frame;
        self.interaction = None;
        self.request_redraw();
        return;
    }
    Some(RecordControlHit::Record) => {
        self.confirm_rect(rect);
        return;
    }
    None => {}
}
```

- [ ] In `main.rs::record_flow`, call `run_select_record(prefs.show_frame, prefs.audio_enabled)`, persist once when either returned boolean differs, and send `selection.audio_enabled` in IPC.
- [ ] Run `cargo test select_skia:: -- --nocapture`.
- [ ] Expected GREEN result: selector geometry, rendering, and hit tests pass.

### Step 2.4: Add failing tray model test, then add the submenu

- [ ] In the tray test snapshot, set `audio_source: RecordAudioSource::Mic` and add:

```rust
#[test]
fn menu_model_selects_microphone_audio_source() {
    let model = menu_model(&snapshot(PublicRecordingState::Idle));
    assert_eq!(model.audio_source_selected, 1);
    assert_eq!(AUDIO_SOURCE_LABELS, ["System + microphone", "Microphone only", "System only"]);
}
```

- [ ] Run `cargo test tray::tests::menu_model_selects_microphone_audio_source -- --nocapture`.
- [ ] Expected RED result: `audio_source_selected` and `AUDIO_SOURCE_LABELS` do not exist.
- [ ] Import `RecordAudioSource`, add `SetAudioSource(RecordAudioSource)` to `TrayAction`, and add:

```rust
const AUDIO_SOURCE_LABELS: [&str; 3] = [
    "System + microphone",
    "Microphone only",
    "System only",
];

fn audio_source_index(source: RecordAudioSource) -> usize {
    match source {
        RecordAudioSource::SystemAndMic => 0,
        RecordAudioSource::Mic => 1,
        RecordAudioSource::System => 2,
    }
}

fn audio_source_at(index: usize) -> RecordAudioSource {
    match index {
        1 => RecordAudioSource::Mic,
        2 => RecordAudioSource::System,
        _ => RecordAudioSource::SystemAndMic,
    }
}
```

- [ ] Add `audio_source_selected: audio_source_index(snapshot.prefs.audio_source)` to `TrayMenuModel`.
- [ ] Insert an `Audio source` `SubMenu` immediately after `Both displays mode`. Its one `RadioGroup` uses the three labels, `model.audio_source_selected`, and this callback:

```rust
select: Box::new(|tray: &mut Self, index| {
    let source = audio_source_at(index);
    tray.snapshot.prefs.audio_source = source;
    tray.send(TrayAction::SetAudioSource(source));
}),
```

- [ ] Handle `TrayAction::SetAudioSource(source)` in `shelf/mod.rs` by cloning current preferences, assigning `prefs.audio_source = source`, and calling `persist_tray_prefs(prefs)`.
- [ ] Run `cargo test tray::tests -- --nocapture`.
- [ ] Expected GREEN result: all menu model and action tests pass.

### Step 2.5: Verify and commit

- [ ] Run `cargo fmt` and `cargo fmt --check`.
- [ ] Run `cargo test select_skia:: -- --nocapture`.
- [ ] Run `cargo test tray::tests -- --nocapture`.
- [ ] Run `cargo check`.
- [ ] Inspect the five task-file diffs and verify that clicking audio does not call `confirm_rect`.
- [ ] Stage only: `git add src/select_skia/mod.rs src/select_skia/render.rs src/main.rs src/tray.rs src/shelf/mod.rs`.
- [ ] Commit: `git commit -m "feat: add recording audio controls"`.

---

## Task 3: Resolve direct sources, build a temporary mix, and pass one source to wf-recorder

**Files:**

- Create: `src/record/audio.rs`
- Modify: `src/record.rs` (module export, `wf_recorder_args`, `wf_recorder_output_args`, tests)
- Modify: `src/record/session.rs` (`spawn_segment`, `spawn_segment_with`, recorder tests; no session ownership yet)

**Interfaces:**

- Consumes: `RecordAudioSource`, current PipeWire-Pulse defaults, and the `pactl` command.
- Produces: `AudioCapture::{source,cleanup}`, `prepare_audio`, `cleanup_stale_mixes`, and optional `--audio=<source>` wf-recorder arguments.

### Step 3.1: Add failing wf-recorder argument tests

- [ ] Change the existing argument tests to pass `None`, assert their vectors remain unchanged, and add:

```rust
#[test]
fn wf_recorder_area_adds_selected_audio_source() {
    let args = wf_recorder_args(
        &Geometry { x: 1, y: 2, w: 3, h: 4 },
        "libx264",
        Some("desk.monitor"),
        Path::new("/tmp/out.mp4"),
    );
    assert!(args.iter().any(|arg| arg == "--audio=desk.monitor"));
}

#[test]
fn wf_recorder_output_without_audio_keeps_previous_arguments() {
    let args = wf_recorder_output_args("DP-3", "libx264", None, Path::new("/tmp/out.mp4"));
    assert!(!args.iter().any(|arg| arg.starts_with("--audio")));
}
```

- [ ] Run `cargo test record::tests::wf_recorder_ -- --nocapture`.
- [ ] Expected RED result: function arity mismatches because the optional source parameter does not exist.

### Step 3.2: Add the optional recorder argument without changing video-only output

- [ ] Change both builders to accept `audio_source: Option<&str>` immediately before `out: &Path`.
- [ ] Insert this after `capture_profile_args(codec)` and before `-f`:

```rust
if let Some(source) = audio_source {
    args.push(format!("--audio={source}"));
}
```

- [ ] Change `spawn_segment` and `spawn_segment_with` to accept `audio_source: Option<&str>` immediately after `codec`, and forward it to both argument builders.
- [ ] Update every existing call reported by `rg -n 'spawn_segment(_with)?\(' src` to pass `None` for now, including fake-spawn tests.
- [ ] Run `cargo test record::tests::wf_recorder_ -- --nocapture`.
- [ ] Run `cargo test record::session::tests -- --nocapture`.
- [ ] Expected GREEN result: argument and recorder-spawn tests pass; video-only expected vectors are unchanged.

### Step 3.3: Add failing pure pactl tests

- [ ] Create `src/record/audio.rs` with imports and the test module first. The fake runner records `args.join(" ")` and pops these queued outputs in order.
- [ ] Add these three tests:

```rust
#[test]
fn system_source_uses_default_sink_monitor() {
    let mut fake = FakePactl::new(["speakers\n", "1\talsa_output.monitor\n2\tspeakers.monitor\n"]);
    let capture = prepare_audio_with(RecordAudioSource::System, "unused", |a| fake.run(a)).unwrap();
    assert_eq!(capture.source(), "speakers.monitor");
    assert_eq!(fake.commands, ["get-default-sink", "list short sources"]);
}

#[test]
fn mic_source_uses_default_source() {
    let mut fake = FakePactl::new(["studio_mic\n", "1\tstudio_mic\n"]);
    let capture = prepare_audio_with(RecordAudioSource::Mic, "unused", |a| fake.run(a)).unwrap();
    assert_eq!(capture.source(), "studio_mic");
    assert_eq!(fake.commands, ["get-default-source", "list short sources"]);
}

#[test]
fn combined_source_builds_mix_and_rolls_back_partial_failure() {
    let mut fake = FakePactl::with_results([
        Ok("speakers\n"),
        Ok("studio_mic\n"),
        Ok("1\tspeakers.monitor\n2\tstudio_mic\n"),
        Ok("41\n"),
        Ok("42\n"),
        Err("second loopback failed"),
        Ok(""),
        Ok(""),
    ]);
    let error = prepare_audio_with(RecordAudioSource::SystemAndMic, "boltsnap_mix_test", |a| fake.run(a))
        .unwrap_err();
    assert!(error.contains("second loopback failed"));
    assert!(fake.commands.contains(&"unload-module 42".to_string()));
    assert!(fake.commands.contains(&"unload-module 41".to_string()));
    let unloads = fake.commands.iter().filter(|c| c.starts_with("unload-module")).cloned().collect::<Vec<_>>();
    assert_eq!(unloads, ["unload-module 42", "unload-module 41"]);
}
```

- [ ] Implement `FakePactl` entirely inside the test module with `VecDeque<Result<String,String>>`; `run` pushes the joined command then returns the next queued result.
- [ ] Export the module with `pub mod audio;` in `src/record.rs`.
- [ ] Run `cargo test record::audio::tests -- --nocapture`.
- [ ] Expected RED result: `AudioCapture` and `prepare_audio_with` are missing.

### Step 3.4: Implement the direct and mixed source resolver

- [ ] Add this public type and production runner:

```rust
#[derive(Debug)]
pub struct AudioCapture {
    source: String,
    modules: Vec<u32>,
}

impl AudioCapture {
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn cleanup(self) -> Result<(), String> {
        unload_modules(&self.modules, run_pactl)
    }
}

fn run_pactl(args: &[String]) -> Result<String, String> {
    let output = std::process::Command::new("pactl")
        .args(args)
        .output()
        .map_err(|error| format!("run pactl: {error}"))?;
    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|error| format!("read pactl output: {error}"))
    } else {
        Err(format!(
            "pactl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
```

- [ ] Implement helpers with these contracts:

```rust
fn one_line(output: String, what: &str) -> Result<String, String>;
fn source_names(output: &str) -> Vec<&str>;
fn require_source(names: &[&str], source: &str) -> Result<(), String>;
fn load_module(args: Vec<String>, run: &mut impl FnMut(&[String]) -> Result<String, String>) -> Result<u32, String>;
fn unload_modules(
    modules: &[u32],
    mut run: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<(), String>;
```

- [ ] `unload_modules` must iterate `modules.iter().rev()`, attempt every unload even after one fails, and return the first error.
- [ ] Implement `prepare_audio_with` so direct modes query one default plus `list short sources`, validate the selected name, and return an empty module list.
- [ ] For `SystemAndMic`, execute exactly this sequence and push each returned module ID immediately:

```text
pactl get-default-sink
pactl get-default-source
pactl list short sources
pactl load-module module-null-sink sink_name=<mix> sink_properties=device.description=Boltsnap
pactl load-module module-loopback source=<sink>.monitor sink=<mix>
pactl load-module module-loopback source=<mic> sink=<mix>
```

- [ ] Implement the resolver with the following complete control flow. `call` converts string slices to owned arguments for the injected runner; `load_module` parses its trimmed output as a module ID.

```rust
fn call(
    run: &mut impl FnMut(&[String]) -> Result<String, String>,
    args: &[&str],
) -> Result<String, String> {
    run(&args.iter().map(|arg| (*arg).to_string()).collect::<Vec<_>>())
}

fn prepare_audio_with(
    mode: RecordAudioSource,
    mix_name: &str,
    mut run: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<AudioCapture, String> {
    let direct = match mode {
        RecordAudioSource::System => {
            let sink = one_line(call(&mut run, &["get-default-sink"])?, "default sink")?;
            Some(format!("{sink}.monitor"))
        }
        RecordAudioSource::Mic => {
            Some(one_line(call(&mut run, &["get-default-source"])?, "default source")?)
        }
        RecordAudioSource::SystemAndMic => None,
    };
    if let Some(source) = direct {
        let output = call(&mut run, &["list", "short", "sources"])?;
        require_source(&source_names(&output), &source)?;
        return Ok(AudioCapture { source, modules: Vec::new() });
    }

    let sink = one_line(call(&mut run, &["get-default-sink"])?, "default sink")?;
    let mic = one_line(call(&mut run, &["get-default-source"])?, "default source")?;
    let system = format!("{sink}.monitor");
    let output = call(&mut run, &["list", "short", "sources"])?;
    let names = source_names(&output);
    require_source(&names, &system)?;
    require_source(&names, &mic)?;

    let mut modules = Vec::with_capacity(3);
    let setup = (|| -> Result<(), String> {
        modules.push(load_module(
            vec![
                "load-module".into(),
                "module-null-sink".into(),
                format!("sink_name={mix_name}"),
                "sink_properties=device.description=Boltsnap".into(),
            ],
            &mut run,
        )?);
        modules.push(load_module(
            vec![
                "load-module".into(),
                "module-loopback".into(),
                format!("source={system}"),
                format!("sink={mix_name}"),
            ],
            &mut run,
        )?);
        modules.push(load_module(
            vec![
                "load-module".into(),
                "module-loopback".into(),
                format!("source={mic}"),
                format!("sink={mix_name}"),
            ],
            &mut run,
        )?);
        Ok(())
    })();

    if let Err(error) = setup {
        return match unload_modules(&modules, &mut run) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; roll back audio mix: {cleanup}")),
        };
    }
    Ok(AudioCapture {
        source: format!("{mix_name}.monitor"),
        modules,
    })
}
```

- [ ] On any error after the first load, `unload_modules` therefore receives all successfully collected IDs and returns the original setup error with cleanup detail only when cleanup also fails.
- [ ] Implement the public wrapper with a collision-resistant name:

```rust
static MIX_ID: AtomicU64 = AtomicU64::new(0);

pub fn prepare_audio(mode: RecordAudioSource) -> Result<AudioCapture, String> {
    let id = MIX_ID.fetch_add(1, Ordering::Relaxed);
    let mix = format!("boltsnap_mix_{}_{}", std::process::id(), id);
    prepare_audio_with(mode, &mix, run_pactl)
}
```

- [ ] Run `cargo test record::audio::tests -- --nocapture`.
- [ ] Expected GREEN result: direct resolution, exact command order, and reverse rollback pass.

### Step 3.5: Test and implement stale mix cleanup

- [ ] Add a fake-runner test with module-list output containing one unrelated module, a Boltsnap null sink, and two Boltsnap loopbacks; assert only the three matching IDs are unloaded in reverse list order.
- [ ] Run the new test and observe RED because `cleanup_stale_mixes_with` is absent.
- [ ] Implement:

```rust
fn cleanup_stale_mixes_with(
    mut run: impl FnMut(&[String]) -> Result<String, String>,
) -> Result<(), String> {
    let output = run(&["list".into(), "short".into(), "modules".into()])?;
    let modules = output
        .lines()
        .filter(|line| line.contains("boltsnap_mix_"))
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .collect::<Vec<_>>();
    unload_modules(&modules, run)
}

pub fn cleanup_stale_mixes() -> Result<(), String> {
    cleanup_stale_mixes_with(run_pactl)
}
```

- [ ] Run `cargo test record::audio::tests -- --nocapture`.
- [ ] Expected GREEN result: unrelated modules are preserved and all matching modules are attempted in reverse order.

### Step 3.6: Verify and commit

- [ ] Run `cargo fmt` and `cargo fmt --check`.
- [ ] Run `cargo test record::tests -- --nocapture`.
- [ ] Run `cargo test record::audio::tests -- --nocapture`.
- [ ] Run `cargo test record::session::tests -- --nocapture`.
- [ ] Run `cargo check`.
- [ ] Run `rg -n 'spawn_segment(_with)?\([^\n]*&tools' src/record src/shelf/mod.rs` and inspect every call for the explicit `None` placeholder added in this task.
- [ ] Stage only: `git add src/record.rs src/record/audio.rs src/record/session.rs`.
- [ ] Commit: `git commit -m "feat: prepare recording audio sources"`.

---

## Task 4: Own audio for the complete recording lifecycle and preserve it in final output

**Files:**

- Modify: `src/record/session.rs` (`RecordingSession::audio`, constructor/tests)
- Modify: `src/record/finalize.rs` (`build_combined_args`, test)
- Modify: `src/shelf/mod.rs` (startup cleanup, starts, resume, terminal cleanup, shutdown)
- Modify: `README.md` (runtime requirement and configuration keys)

**Interfaces:**

- Consumes: `prepare_audio`, `AudioCapture`, `cleanup_stale_mixes`, and persisted audio preferences.
- Produces: one stable audio source for all segments, lifecycle cleanup, and a combined FFmpeg output with at most one audio stream.

### Step 4.1: Add failing session ownership and source-selection tests

- [ ] Add a test-only constructor to `AudioCapture`:

```rust
#[cfg(test)]
pub(crate) fn for_test(source: &str) -> Self {
    Self { source: source.into(), modules: Vec::new() }
}
```

- [ ] Add `requested_audio` near the shelf recording start helpers, then add these unit tests in `shelf/mod.rs`'s test module:

```rust
#[test]
fn requested_audio_respects_toggle_and_keeps_source_choice() {
    let mut prefs = RecordingPrefs {
        audio_enabled: false,
        audio_source: RecordAudioSource::System,
        ..RecordingPrefs::default()
    };
    assert_eq!(requested_audio(&prefs), None);
    prefs.audio_enabled = true;
    assert_eq!(requested_audio(&prefs), Some(RecordAudioSource::System));
}
```

- [ ] In `record/session.rs`, add a test constructing a session with `Some(AudioCapture::for_test("boltsnap_mix_test.monitor"))`, simulate pause bookkeeping, and assert the session still reports the same source before resume.
- [ ] Run `cargo test shelf::tests::requested_audio_respects_toggle_and_keeps_source_choice -- --nocapture`.
- [ ] Run `cargo test record::session::tests::audio_source_survives_pause -- --nocapture`.
- [ ] Expected RED result: helper, session field, and constructor argument do not exist.

### Step 4.2: Add session ownership and pass the source on resume

- [ ] Add `pub audio: Option<AudioCapture>` to `RecordingSession` and import it from `crate::record::audio`.
- [ ] Add `audio: Option<AudioCapture>` immediately before `active` in `RecordingSession::new`, assign it, and pass `None` in existing test constructors that are unrelated to audio.
- [ ] Implement the shelf helper exactly as:

```rust
fn requested_audio(prefs: &RecordingPrefs) -> Option<RecordAudioSource> {
    prefs.audio_enabled.then_some(prefs.audio_source)
}

fn cleanup_audio_async(audio: Option<AudioCapture>) {
    if let Some(audio) = audio {
        std::thread::spawn(move || {
            if let Err(error) = audio.cleanup() {
                eprintln!("boltsnap daemon: clean up recording audio: {error}");
            }
        });
    }
}
```

- [ ] In the resume branch, derive `let audio_source = session.audio.as_ref().map(AudioCapture::source);` and call `spawn_segment(&scope, &codec, audio_source, &RecorderTools::default())`.
- [ ] Run the two targeted tests again.
- [ ] Expected GREEN result: disabled returns `None`; pause keeps the same source.

### Step 4.3: Add failing combined-audio mapping test, then map one optional stream

- [ ] Add this test beside the existing `build_combined_args` tests:

```rust
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
```

- [ ] Run `cargo test record::finalize::tests::combined_output_maps_only_first_optional_audio_stream -- --nocapture`.
- [ ] Expected RED result: no audio map is present.
- [ ] Extend `build_combined_args` immediately after `-c:v <codec>` with:

```rust
"-map".into(),
"0:a?".into(),
"-c:a".into(),
"copy".into(),
```

- [ ] Run `cargo test record::finalize::tests -- --nocapture`.
- [ ] Expected GREEN result: exactly one optional audio stream is mapped, while video-only inputs remain valid because of `?`.

### Step 4.4: Integrate audio setup into both start paths

- [ ] In both `start_recording` (area) and `start_named_recording_outputs` (fullscreen), prepare before spawning:

```rust
let mut audio = requested_audio(&self.recording_prefs)
    .map(crate::record::audio::prepare_audio)
    .transpose()?;
let audio_source = audio.as_ref().map(AudioCapture::source);
let active = match spawn_segment(&scope, &codec, audio_source, &tools) {
    Ok(active) => active,
    Err(error) => {
        if let Some(audio) = audio.take() {
            let cleanup = audio.cleanup();
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; clean up recording audio: {cleanup}"),
            });
        }
        return Err(error);
    }
};
```

- [ ] Pass `audio` into `RecordingSession::new` in both paths.
- [ ] Confirm the area-start handler has already copied selector `audio_enabled` into `self.recording_prefs` before `start_recording` runs.
- [ ] Confirm `StartDefaultRecording`, tray fullscreen start, named output start, and both-output start all reach `start_named_recording_outputs` and therefore use the same current preferences.
- [ ] Add a test around a small extracted start-preparation helper, or extend the existing fake recorder test, proving `audio_enabled = false` passes `None` and never calls the fake pactl setup closure.
- [ ] Run the new targeted test and all shelf tests: `cargo test shelf::tests -- --nocapture`.
- [ ] Expected GREEN result: disabled setup is never called; enabled sessions own the returned capture.

### Step 4.5: Integrate terminal cleanup without breaking retries

- [ ] At daemon startup, immediately after existing orphan recording-file cleanup and before Wayland setup, call:

```rust
if let Err(error) = crate::record::audio::cleanup_stale_mixes() {
    eprintln!("boltsnap daemon: clean up stale recording audio: {error}");
}
```

- [ ] In successful save finalization, take `session.audio` only after the final artifact and thumbnail/shelf actions have succeeded, remove the session from `self.recording`, and pass the audio to `cleanup_audio_async`.
- [ ] On finalization error, leave `session.audio` in place so the user can retry save or discard.
- [ ] In discard, move `session.audio` into the existing discard worker or call `cleanup_audio_async` after the session is removed; retain the existing segment-file removal behavior.
- [ ] On an unrecoverable recorder start failure, use the synchronous cleanup block from Step 4.4 before returning the error.
- [ ] On pause and recoverable unexpected recorder exit, do not take or clean `session.audio`.
- [ ] After the Wayland event loop exits, take any remaining session and synchronously call `audio.cleanup()` before returning from `run_daemon`; log failure without hiding the original daemon result.
- [ ] Add/extend lifecycle tests to assert: pause retains `Some(audio)`, finalization failure retains it, successful terminal completion takes it, and discard takes it.
- [ ] Run `cargo test record::session::tests -- --nocapture`.
- [ ] Run `cargo test shelf::tests -- --nocapture`.
- [ ] Expected GREEN result: ownership moves exactly once on terminal paths and remains on retryable paths.

### Step 4.6: Document runtime behavior and verify real argument structure

- [ ] In `README.md`'s recording requirements, add `pactl` from PipeWire-Pulse/PulseAudio for audio-enabled recording.
- [ ] In the recording configuration section, document:

```toml
record_audio_enabled = true
record_audio_source = "system-and-mic" # "system-and-mic", "mic", or "system"
```

- [ ] State that source modes use the current default sink/source and that per-device pickers and volume controls are not provided.
- [ ] Run `cargo fmt` and `cargo fmt --check`.
- [ ] Run `cargo test record:: -- --nocapture`.
- [ ] Run `cargo test shelf::tests -- --nocapture`.
- [ ] Run `cargo check`.
- [ ] Inspect `rg -n 'spawn_segment\(' src` and verify every call passes either the active session source or an explicit `None` in tests.
- [ ] Inspect `rg -n 'prepare_audio|cleanup_stale_mixes|cleanup_audio_async' src/shelf/mod.rs` and account for start, startup, success, discard, and shutdown paths.

### Step 4.7: Commit

- [ ] Inspect `git diff -- src/record/session.rs src/record/finalize.rs src/shelf/mod.rs README.md` and preserve unrelated user hunks.
- [ ] Stage only: `git add src/record/session.rs src/record/finalize.rs src/shelf/mod.rs README.md`.
- [ ] Commit: `git commit -m "feat: record and finalize session audio"`.

---

## Task 5: Replace the popup pixel font with the current desktop UI font

**Files:**

- Create: `src/shelf/font.rs`
- Modify: `src/shelf/mod.rs` (font module, daemon field, refresh on popup open, draw call)
- Modify: `src/shelf/paint.rs` (ab_glyph text renderer; remove popup pixel alphabet and seven-segment timer)
- Modify: `assets/fonts/dejavu-badge.ttf` (printable ASCII + multiplication sign fallback subset)

**Interfaces:**

- Consumes: `org.gnome.desktop.interface font-name`, Fontconfig match output `<path>\t<face-index>`, and the embedded fallback font.
- Produces: `load_popup_font() -> FontVec` and `draw_recording_popup(..., font: &FontVec)`.

### Step 5.1: Add failing font parsing and fallback tests

- [ ] Create `src/shelf/font.rs` and add these tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ab_glyph::Font;

    #[test]
    fn parses_gsettings_family_without_trailing_size() {
        assert_eq!(
            parse_gsettings_font("'GeistMono Nerd Font 11'\n"),
            Some("GeistMono Nerd Font".into())
        );
        assert_eq!(parse_gsettings_font("'Inter Variable 10.5'"), Some("Inter Variable".into()));
    }

    #[test]
    fn parses_fontconfig_collection_index() {
        assert_eq!(
            parse_fc_match("/usr/share/fonts/inter/Inter.ttc\t2\n"),
            Some((PathBuf::from("/usr/share/fonts/inter/Inter.ttc"), 2))
        );
    }

    #[test]
    fn embedded_fallback_contains_popup_and_selector_glyphs() {
        let font = fallback_popup_font();
        for ch in "RECORDING PAUSED SAVING... SHELF DISK DISCARD AUDIO ON OFF 01:23 ×".chars() {
            if !ch.is_whitespace() {
                assert_ne!(font.glyph_id(ch), ab_glyph::GlyphId(0), "missing {ch:?}");
            }
        }
    }
}
```

- [ ] Add `mod font;` beside the shelf's existing module declarations.
- [ ] Run `cargo test shelf::font::tests -- --nocapture`.
- [ ] Expected RED result: the parser and fallback loader functions do not exist.

### Step 5.2: Expand the embedded fallback and implement resolution

- [ ] Regenerate the existing licensed subset from the installed DejaVu source:

```bash
pyftsubset /usr/share/fonts/TTF/DejaVuSans.ttf \
  --unicodes='U+0020-007E,U+00D7' \
  --output-file=assets/fonts/dejavu-badge.ttf
```

- [ ] Keep `assets/fonts/LICENSE-DejaVu.txt` unchanged.
- [ ] Implement the pure parsers:

```rust
fn parse_gsettings_font(output: &str) -> Option<String> {
    let value = output.trim().trim_matches('\'');
    let (family, size) = value.rsplit_once(' ')?;
    size.parse::<f32>().ok()?;
    (!family.is_empty()).then(|| family.to_string())
}

fn parse_fc_match(output: &str) -> Option<(PathBuf, u32)> {
    let line = output.lines().next()?;
    let (path, index) = line.split_once('\t')?;
    Some((PathBuf::from(path), index.trim().parse().ok()?))
}
```

- [ ] Implement `fontconfig_match(query)` by running:

```text
fc-match -f %{file}\t%{index}\n <query>
```

- [ ] Implement the loader completely as follows; unsuccessful commands, empty paths, unreadable files, invalid face indices, and invalid fonts all return `None`:

```rust
fn fontconfig_match(query: &str) -> Option<(PathBuf, u32)> {
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{file}\t%{index}\n", query])
        .output()
        .ok()?;
    output.status.success().then_some(())?;
    parse_fc_match(&String::from_utf8(output.stdout).ok()?)
}

fn load_fontconfig_font(query: &str) -> Option<FontVec> {
    let (path, index) = fontconfig_match(query)?;
    if path.as_os_str().is_empty() {
        return None;
    }
    FontVec::try_from_vec_and_index(std::fs::read(path).ok()?, index).ok()
}
```
- [ ] Implement:

```rust
pub fn fallback_popup_font() -> FontVec {
    FontVec::try_from_vec(include_bytes!("../../assets/fonts/dejavu-badge.ttf").to_vec())
        .expect("embedded DejaVu popup font must be valid")
}

pub fn load_popup_font() -> FontVec {
    let desktop = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "font-name"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| parse_gsettings_font(&output));

    desktop
        .as_deref()
        .and_then(load_fontconfig_font)
        .or_else(|| load_fontconfig_font("sans-serif"))
        .unwrap_or_else(fallback_popup_font)
}
```

- [ ] Run `cargo test shelf::font::tests -- --nocapture`.
- [ ] Expected GREEN result: family, TTC index, and fallback glyph tests pass.

### Step 5.3: Add failing ab_glyph popup rendering test

- [ ] Change the existing popup paint test to create `let font = crate::shelf::font::fallback_popup_font();` and pass `&font` to `draw_recording_popup`.
- [ ] Add this focused test:

```rust
#[test]
fn popup_font_renderer_paints_text_pixels() {
    let (w, h) = (180, 48);
    let mut buf = vec![0u8; (w * h * 4) as usize];
    let font = crate::shelf::font::fallback_popup_font();
    draw_font_text(&mut buf, w, h, &font, 8.0, 30.0, "RECORDING 01:23", 18.0, GLYPH_RGB);
    assert!(buf.chunks_exact(4).any(|pixel| pixel[3] != 0));
}
```

- [ ] Run `cargo test shelf::paint::tests::popup_font_renderer_paints_text_pixels -- --nocapture`.
- [ ] Expected RED result: `draw_font_text` and the popup font argument do not exist.

### Step 5.4: Implement measured antialiased text and remove handwritten glyphs

- [ ] Import `ab_glyph::{Font, FontVec, PxScale, ScaleFont, point}` in `shelf/paint.rs`.
- [ ] Implement measurement with kerning:

```rust
fn text_width(font: &FontVec, text: &str, px: f32) -> f32 {
    let scaled = font.as_scaled(PxScale::from(px));
    let mut width = 0.0;
    let mut previous = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(prev) = previous {
            width += scaled.kern(prev, id);
        }
        width += scaled.h_advance(id);
        previous = Some(id);
    }
    width
}
```

- [ ] Implement the renderer exactly as:

```rust
#[allow(clippy::too_many_arguments)]
fn draw_font_text(
    canvas: &mut [u8],
    w: u32,
    h: u32,
    font: &FontVec,
    x: f32,
    baseline: f32,
    text: &str,
    px: f32,
    color: (u8, u8, u8),
) {
    let scale = PxScale::from(px);
    let scaled = font.as_scaled(scale);
    let mut caret = x;
    let mut previous = None;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(prev) = previous {
            caret += scaled.kern(prev, id);
        }
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        if let Some(outlined) = font.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|gx, gy, coverage| {
                blend_px(
                    canvas,
                    w,
                    h,
                    bounds.min.x as i32 + gx as i32,
                    bounds.min.y as i32 + gy as i32,
                    color.0,
                    color.1,
                    color.2,
                    coverage,
                );
            });
        }
        caret += scaled.h_advance(id);
        previous = Some(id);
    }
}
```
- [ ] Change `draw_recording_popup` to accept `font: &FontVec` after `elapsed`.
- [ ] Render title at 18 px after the red dot, vertically centered in the existing header; render elapsed at 18 px right-aligned to x=382; render button labels at 14 px centered from `text_width` and scaled ascent/descent.
- [ ] Remove `draw_pixel_text`, `pixel_glyph`, `draw_time`, `draw_seg_digit`, and only the segment helpers/constants no longer referenced elsewhere. Keep all popup dimensions, colors, rounded rectangles, hit targets, and state labels unchanged.
- [ ] Run `cargo test shelf::paint::tests -- --nocapture`.
- [ ] Expected GREEN result: background/control tests and glyph rasterization test pass.
- [ ] Run `rg -n 'draw_pixel_text|pixel_glyph|draw_time|draw_seg_digit' src/shelf/paint.rs`.
- [ ] Expected output: no matches.

### Step 5.5: Refresh the desktop font each time Alt+Print opens controls

- [ ] Add `popup_font: ab_glyph::FontVec` beside the daemon's popup fields and initialize it with `font::fallback_popup_font()`.
- [ ] At the beginning of `create_popup`, before allocating the pool, assign:

```rust
self.popup_font = font::load_popup_font();
```

- [ ] Pass `&self.popup_font` from `draw_popup` into `draw_recording_popup`.
- [ ] Do not resolve the font on timer ticks or ordinary redraws; the `create_popup` assignment is the only refresh point.
- [ ] Run `cargo fmt` and `cargo fmt --check`.
- [ ] Run `cargo test shelf::font::tests -- --nocapture`.
- [ ] Run `cargo test shelf::paint::tests -- --nocapture`.
- [ ] Run `cargo test shelf::tests -- --nocapture`.
- [ ] Run `cargo check`.

### Step 5.6: Full automated and manual verification

- [ ] Run the complete suite: `cargo test`.
- [ ] Run `cargo clippy --all-targets -- -D warnings` and fix only warnings introduced by these five tasks.
- [ ] Run `cargo build`.
- [ ] Start the locally built daemon and record four short clips: `System + microphone`, `Microphone only`, `System only`, and selector `AUDIO OFF`.
- [ ] For each output run `ffprobe -v error -show_entries stream=index,codec_type,codec_name -of compact <clip>`.
- [ ] Expected: the first three clips contain one video stream and one audio stream; the audio-off clip contains one video stream and no audio stream.
- [ ] Start a system+microphone recording, pause, resume, save, and confirm one audio stream and continuous audible content on both sides of the pause.
- [ ] Start a combined two-display recording and confirm `ffprobe` reports exactly one audio stream.
- [ ] While no recording is active, run `pactl list short modules | rg boltsnap_mix_`.
- [ ] Expected output: no matches after successful save or discard.
- [ ] Open controls with Alt+Print, change `org.gnome.desktop.interface font-name`, close and reopen controls, and confirm the new popup uses the new desktop font without restarting Boltsnap.
- [ ] Temporarily run with an invalid desktop font setting and confirm the popup still opens using Fontconfig `sans-serif` or embedded DejaVu.

### Step 5.7: Commit and final worktree audit

- [ ] Inspect `git diff -- src/shelf/font.rs src/shelf/mod.rs src/shelf/paint.rs assets/fonts/dejavu-badge.ttf`.
- [ ] Stage only: `git add src/shelf/font.rs src/shelf/mod.rs src/shelf/paint.rs assets/fonts/dejavu-badge.ttf`.
- [ ] Commit: `git commit -m "feat: follow desktop font in recording popup"`.
- [ ] Run `git status --short` and confirm only the user's original unrelated changes/untracked plans remain.
- [ ] Run `git log -5 --oneline` and confirm exactly one implementation commit per task.
