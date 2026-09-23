# Performance and functionality fixes, 2026-09-23

Status: plan, not started. Linux only; the Windows backend stays frozen.
Each phase is one capability and lands as its own commit(s). Phases 1 and 4
change user-visible behavior and need explicit maintainer approval before code.

## Baseline (measured live on the maintainer's desktop, 2026-09-23)

Hardware: RTX 4060 (driver 610.43), Hyprland, DP-3 1920x1080@240 at x=0,
DP-1 1920x1080@200 at x=1920, scale 1. System FFmpeg 8.1.2 is built with
`-nvenc -vaapi -cuda +vulkan`, so `h264_nvenc` (the code default) does not exist
and the user config pins `record_codec = "libx264"`.

| Path | CPU | Delivered |
| --- | --- | --- |
| wf-recorder `--no-dmabuf -r 240 -c libx264`, DP-3, static desktop | ~345 % | 240 FPS |
| gsr `-k h264_vulkan -f 240 -fm cfr`, DP-3 | ~25 % | 240 FPS |
| gsr `-k h264_vulkan -f 60`, DP-3+DP-1 in one stream, `-cursor no`, merged audio | ~35 % | 3840x1080 |
| gsr region `-w 800x600+2000+100 -f 240` | ~49 % (startup sample) | 474 frames / ~2 s |
| native cursor-record (GPU composite, readback, libx264), DP-3 60 FPS | ~300 % | 60 FPS |
| Vulkan cursor fixture 1080p, 120 frames: libx264 vs h264_vulkan | 0.76 s vs 0.35 s | pixels correct |
| `boltsnap full` both outputs, capture only: libway vs libwayshot | ~46 ms vs ~45 ms | 3840x1080 |

With the configured `record_default_target = "both"`, ordinary recording runs two
libx264 processes (~7 cores) and Combined mode re-encodes 3840x1080@240 with
libx264 on save. Under game load this drops frames, which also reads as a
stuttering cursor.

Current tests: `cargo fmt --check` clean, 267 tests pass (1 ignored).

## Phase 0: secure the current state

The working tree mixes three capabilities, and required files are untracked.

1. Split into commits: (a) libway screenshot backend incl. `vendor/`,
   `tools/sync-libway.py`, CI/flake/Cargo changes, `docs/libway.md`;
   (b) replay GOP snapshot sharing (`src/replay/ring.rs`, worker ring/media,
   `tools/bench-replay-snapshot.rs`); (c) the cursor-smoothing experiment on its
   own, so Phase 4 can revert it as one unit.
2. `vendor/`, `tools/`, `worker/build.rs`, `worker/native/`, `worker/src/native/`
   and `src/record/cursor.rs` are untracked. The flake (`src = ./.`) and the CI
   `sync-libway --check` step need them tracked.
3. `vendor/libway` lags `../libway`, which is mid-way through its own review-fix
   wave (DnD, API changes). Do not resync now. Phase 2 lands in `../libway` after
   that wave and is synced once.

Acceptance: each commit builds and passes `cargo fmt --check` and `cargo test`.

## Phase 1: GPU recording through gpu-screen-recorder

Goal: ordinary recordings use the same hardware path as replay. wf-recorder stays
as fallback when gsr is not installed. Needs approval (backend change).

### Encoder selection

- Extract the inline replay encoder probe (`replay/mod.rs:330-419`) into one
  Linux function, e.g. `platform/linux/encoder.rs::select(preference) ->
  Result<GsrCodec, String>`. Replay and recording both call it. Cache the result
  per daemon lifetime; re-probe after a failed start.
- New config default `record_codec = "auto"`: nvenc, vaapi, then vulkan, the
  first that passes `check-encoder`. Explicit names map to gsr `-k`
  (`h264_nvenc` -> `h264`, `h264_vulkan` -> `h264_vulkan`, ...). `libx264`
  maps to gsr `-encoder cpu -k h264`. Unknown values fail with a clear error;
  no silent fallback.
- Replace `h264_nvenc` as the hard default in `config.rs:404`.
- Remove `record_codec = "libx264"` from the user config only with approval.

### Argument builder

New Linux module `platform/linux/recorder.rs` with a pure argv builder
(unit-tested like `wf_recorder_args`), selected in `spawn_segment`:

| Scope | gsr `-w` |
| --- | --- |
| One output | `DP-3` |
| Area | `WxH+X+Y` (logical, same values as today's `-g`) |
| Both, Separate | one gsr per output (today's model) |
| Both, Combined | one gsr: `A;x=..;y=..|B;x=..;y=..` from monitor layout |

Common: `-f 240|60 -fm cfr -k <codec> -q very_high -bm qp -keyint 2
-fallback-cpu-encoding no -o <segment>.mp4`. Combined in one stream removes
the xstack re-encode in `finalize.rs:69-77` for gsr sessions; finalize becomes
concat-copy only. Mixed-scale layouts need x/y/width/height in physical pixels
of the canvas; verify with a scaled test layout before enabling, otherwise keep
the xstack path for mixed scales.

### Audio

- system: `-a device:<default-sink>.monitor`; mic: `-a device:<source>`;
  system+mic: `-a "device:A|device:B"`. gsr merges internally, so the
  null-sink/loopback modules in `audio.rs:126-174` are only needed for the
  wf-recorder fallback.
- `-ac aac` so segments stay concat-compatible with the current finalize.
- Open issue from the live tests: gsr logs `failed to write frame index 1 to
  muxer (-22)` once at start with audio. Reproduce, compare `-ac opus`/mkv,
  and confirm no audio gap before shipping (count audio packets vs duration).

### Lifecycle

Keep the segment model: SIGINT stops (gsr saves on SIGINT), resume starts a new
segment with identical codec/size/audio so `concat -c copy` still works.
PDEATHSIG, reaping, auto-pause on child exit and cache limits stay unchanged.
gsr's in-process pause (SIGUSR2) would avoid concat but changes the lifecycle;
out of scope.

Update `require_recorder` (`main.rs:376`) and `shelf/mod.rs:2208,2289,2495` to
accept gsr or wf-recorder; error strings name the actual recorder. Fake-recorder
test scripts parse `-o` in addition to `-f`.

### Acceptance

- argv unit tests for all four scopes, audio variants and codec mapping.
- Live, per scope: CPU below ~50 % at 240 FPS single output, frame count within
  1 % of fps x duration, A/V offset check with a click impulse, pause/resume
  concat, save to shelf and disk, gsr killed mid-recording -> auto-pause.
- Combined save no longer re-encodes (finalize time near file copy time).
- README support matrix and prerequisites list gpu-screen-recorder.

## Phase 2: screenshot path

Work in `../libway` after its current fix wave, then sync the snapshot once.

1. **Parallel output capture.** `capture_desktop` captures outputs one after
   another with one pending capture per connection. Capture each output on its
   own connection in `thread::scope`, compose afterwards. Target: 2x1080p capture
   stage below ~25 ms (`BOLTSNAP_TIMINGS=1 boltsnap full`), today ~46 ms.
2. **Scale rounding** (`compose.rs:108,175`): round instead of truncate; skip
   the Gaussian resize when the size differs by at most 1 px (crop/pad instead);
   round the placement offsets too. Test with 1.2/1.5/1.75 scales.
3. **EXT -> WLR retry**: in `Backend::Auto`, retry WLR on EXT
   `CaptureFailed`/`SessionStopped`/`Timeout` within the remaining deadline.
   One shared deadline per boltsnap request instead of a fresh 5 s per call.
4. **Format preference**: prefer 8888 SHM formats when offered, before 10-bit.
5. **Canvas**: fill alpha only where no output covers the canvas; copy rows with
   `copy_from_slice` instead of per-pixel `imageops::replace`.
6. **boltsnap adapter** (`capture.rs:322,353`): after a portal fallback verify
   the layout on a fresh connection and keep the original libway error in the
   message; let a failing `outputs()` in Area mode also reach the portal path.

Acceptance: libway unit tests for each fix; boltsnap full/area/window timings
recorded before and after; synthetic EXT/WLR fallback tests in libway.

## Phase 3: replay start cleanup

Drop the duplicated cursor check in `serve()` (`replay/mod.rs:568-571`); the
check in `start_capture` covers all callers and no longer skips stalled-session
cleanup. Superseded if Phase 4 removes the replay cursor setting.

## Phase 4: cursor smoothing, Screen Studio style

Needs approval. The current experiment smooths live with an 8 ms lookahead at
60 FPS and burns ~3 cores; the effect is barely visible and it cannot be enabled
from the tray with the current config. Screen Studio instead records the cursor
separately and re-renders it at export with a spring model (tension, friction,
mass; presets Slow/Mellow/Quick/Rapid), using the full recorded path.

### Remove

Worker `native-cursor` feature, `native/gpu.c|h`, `test_gpu.c`, `native/gpu.rs`,
`native/mod.rs`, `build.rs`, `tests/native.py`, the worker libway dependency,
`platform/linux/cursor_recording.rs`, the tray probe state machine and the
libx264/quiet/single-output gates. Keep libway's cursor session API (CPU only)
and `src/record/cursor.rs` as the place for the new motion model.

### Capture

With the toggle on, gsr runs with `-cursor no -write-first-frame-ts yes`. The
daemon runs one cursor-track thread per segment: a libway `cursor_stream` per
captured output, writing a sidecar next to the segment:
monotonic receipt time, enter/leave, position, cursor image id; images stored
once, deduplicated by hash. Cost is event handling only. The `.ts` file gives the
monotonic time of the first video frame, which aligns both clocks.

### Render at save

- Motion (portable, `src/record/cursor.rs`): non-causal spring simulation over
  the whole track, evaluated at output frame times. Presets as named constants;
  jumps across outputs, leave/enter and long gaps snap instead of animating.
  Optional later: shake removal (dead zone), hide when idle, cursor scale.
- Compositing, lazy first: FFmpeg filtergraph with a cursor atlas, `crop` +
  `overlay` driven by a generated `sendcmd` file (position, sprite cell,
  visibility), encoded with the Phase 1 encoder. Measure export speed; only if it
  is too slow, move compositing to the GPU.
- Coordinates: per-output positions mapped into region/combined canvas space.
- Pause/resume: one track per segment, each aligned to its own `.ts`.
- Data safety: keep the cursor-free segments and tracks until export succeeded.
  On render failure, retry with raw positions; never deliver a cursorless video
  silently. Missing track -> notify and keep the segment.

### Replay

Later step: replay gsr with `-cursor no` plus a bounded in-memory cursor ring in
the daemon (same window as the video ring); clip export renders the cursor.
Until then replay records the system cursor and has no smoothing option.

### UI

One tray checkbox "Smooth cursor". Availability is checked once at daemon start
(gsr present, EXT cursor session offered). No per-target probe, no codec/profile
prerequisites. Preset choice via config first, tray submenu later.

### Acceptance

Deterministic motion tests (constant velocity, stop without overshoot beyond the
preset, reversal, warp, leave/enter, gaps); a synthetic render test with known
track -> decoded cursor position within 1 px; live recordings at 60 and 240 FPS
on one output, Combined and region; export time measured; toggle off leaves
Phase 1 output byte-for-byte unchanged in argv.

## Order and open decisions

Order: 0 -> 1 -> 2 -> 3 -> 4. Phase 2 can run in parallel with 1 once `../libway`
is ready.

Maintainer decisions needed:
1. Approve gsr as the primary recorder (Phase 1) and dropping `libx264` from the
   personal config.
2. Approve deleting the native cursor pipeline and rebuilding it as described in
   Phase 4. Default preset and whether region recording is in the first version.
3. Commit split in Phase 0 (commits only on request).
