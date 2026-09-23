# Replay snapshots and cursor smoothing, 2026-09-22

The implemented change shares immutable video GOP descriptors between the live
ring and frozen snapshots. Previously, freezing copied every video packet's
entry and cloned its payload reference while holding the ingest mutex. Now it
clones one `Arc` per GOP. Audio descriptors are still copied. On the first append
to a shared active GOP, `Arc::make_mut` copies that GOP's entries once. Completed
GOPs never need copying. Encoded payloads stay shared `Arc<Packet>` values.

The ring's time limit, byte accounting, closed-GOP rules, export exclusion and
no-overwrite behavior are retained. The existing conservative descriptor budget
is retained too. This reduces work under the ingest mutex during freeze/save;
it does not change the capture encoder or promise improved game frame times.
A snapshot is still O(GOP count + audio packet count), not constant time.

## Measurement

The standalone harness uses the actual before/after ring sources, optimized with
`rustc -O`, five warmups and 100 timed snapshots. It uses shared synthetic payloads,
one-second GOPs and 47 audio entries per second. Ring construction and snapshot
destruction are outside snapshot timing. It excludes FFmpeg parameter copying,
preview decoding, live mutex contention and real capture/encoding. The host was
not reserved exclusively for measurement.

| History | Before median / p95 | After median / p95 |
| --- | --- | --- |
| 60 s, 60 FPS | 101.50 / 131.30 µs | 18.35 / 25.58 µs |
| 600 s, 240 FPS | 4121.55 / 4527.65 µs | 199.91 / 244.08 µs |

An earlier run measured 77.31 → 18.35 µs and 3394.65 → 192.78 µs respectively.
These are local microbenchmarks, not an end-to-end clip latency comparison.
The harness also measures the first append after freeze, including expiration
of the oldest GOP at the window boundary. Medians were 0.82 → 0.96 µs for the
small case and 2.52 → 2.48 µs for the large case; this is not a claim that
copy-on-write has no cost or a worst-case scheduling bound.

Reproduce from the repository root:

```sh
git show d0686a038e2f2cc9cdae42770e1bb91c1d6271c5:src/replay/ring.rs > /tmp/boltsnap-ring-before.rs
BOLTSNAP_BASELINE_RING=/tmp/boltsnap-ring-before.rs \
  rustc -O --edition 2024 tools/bench-replay-snapshot.rs -o /tmp/boltsnap-ring-bench
/tmp/boltsnap-ring-bench
```

Verification: main fmt/check and 261 tests passed (one existing ignored FFmpeg
thumbnail test). Worker fmt, release build and 22 unit tests passed. Synthetic
`probe.py` and `live.py` passed packet/frame identity, audio, window/budget bounds,
freeze, concurrent ingest/status, crop/remux, stale selections and decoding.
Regression tests verify GOP sharing, one-time active-GOP detachment, immutable
frozen audio/video and eviction. The worker still reports the existing unused
`process::output_setup` warning. No running binary or user settings were replaced.

The accelerated 2400-loop soak processed 3,081,600 packets (about eight hours
of media time) in 13.73 s. Peak accounted packet bytes were 1,581,723 against an
8,388,608-byte budget. RSS peaked at 25,244 KiB with zero median growth after
warmup, and the resulting clip decoded successfully. This probe exercises
sustained ingest/eviction, not repeated live freezes or eight hours of real-time
recording. Artifacts: `/tmp/boltsnap-replay-soak.1ifwv4vg/`,
`/tmp/boltsnap-replay-test.jwh_rxqg/`, `/tmp/boltsnap-replay-live.m__0ht54/`.

## Optional cursor smoothing (experimental)

The tray has **Smooth cursor (fullscreen, experimental)**, off by default.
It affects the next ordinary recording. The selected backend is retained across
pause/resume; changing the preference does not rewrite existing recordings.
Disabling it keeps the existing wf-recorder path and allocates no cursor GPU
resources. Enabling it runs a bounded asynchronous capability check first.

The initial supported path requires all of:

- One unrotated output with even pixel dimensions, EXT output image capture and
  separate pointer capture. Regions and combined/multiple outputs are rejected.
- One DRM render node, matching Vulkan/OpenGL device UUID, GBM DMA-BUF import,
  OpenGL external memory/semaphores, and a worker built with `native-cursor`.
- Explicit `record_codec = "libx264"` and `record_profile = "quiet"` (60 FPS).
  No codec change, automatic FPS reduction or hidden CPU fallback occurs.

Example top-level settings in the existing Boltsnap config:

```toml
record_codec = "libx264"
record_profile = "quiet"
record_cursor_smoothing = true
```

The toggle persists only `record_cursor_smoothing`. Select the codec/profile
explicitly before enabling it. A failed probe explains why it cannot enable the
feature. Source changes or recording failures stop the session with an error.
The output segment remains available for the existing recovery path.

libway supplies cursor-free DMA-BUF frames and separate cursor observations.
Boltsnap interpolates observed positions with an 8 ms lookahead, without
extrapolation. Visibility changes, large warps and long sample gaps reset motion.
The system cursor itself is unchanged. Cursor samples have receipt timestamps
because the protocol supplies no position timestamps; video uses the protocol's
monotonic presentation timestamps. Scheduling and compositor delivery still
limit apparent smoothness.

EGL imports the background, composites the premultiplied-alpha cursor at
fractional coordinates, and shares a Vulkan RGBA allocation through OPAQUE_FD
memory plus external semaphores. FFmpeg performs GPU NV12 conversion, then reads
back for the explicitly selected software encoder. This is **not** zero-copy
hardware encoding. Audio retains the existing source selection and is muxed into
fragmented MP4 on a common timestamp timeline. Queues and image history are
bounded; excessive encoder backlog stops recording instead of silently changing
its requested rate.

Replay smoothing is unavailable. `[replay].cursor_smoothing = true` fails before
replacing an existing replay session/history. The local FFmpeg 8.1.2 Vulkan
encoder produces video-profile validation errors even in a baseline FFmpeg
command, so that route is not enabled for live capture. The diagnostic fixture
can exercise it independently. HDR, rotated outputs, multiple seats/devices,
region recording and hardware encoding are outside this initial supported path.

### Build and verification

Build the companion beside the main executable:

```sh
cargo build --release --locked
cargo build --release --locked --features native-cursor \
  --manifest-path src/platform/linux/replay/worker/Cargo.toml
```

Additional development dependencies: FFmpeg 8.1.2 (libavfilter, libavutil,
libavcodec, libavformat), EGL, GLES 3, GBM, Vulkan, pkg-config and a C compiler.
The feature is optional; normal worker builds retain their existing dependencies.
No compatibility claim is made for older FFmpeg development headers.

Headless regression, with an explicitly selected GPU and no desktop capture:

```sh
python3 src/platform/linux/replay/worker/tests/native.py \
  --worker /path/to/boltsnap-replay-worker --node /dev/dri/renderD128
LIBWAY_TEST_NATIVE_WORKER=/path/to/boltsnap-replay-worker \
  cargo test --manifest-path vendor/libway/Cargo.toml --features gpu \
  --test native_consumer -- --ignored --nocapture
```

The tests check real DMA-BUF import after producer handles close, GPU fences,
80 frames of exact background/alpha/hidden/edge pixels, and 120 decoded frames
with interpolated cursor positions. The private synthetic Wayland server checks
independent video/cursor connections, continued output on a static background,
AAC audio, packet cadence including the first frame, stream duration agreement,
SIGINT finalization and released protocol resources. Pure motion tests cover
reversals, gaps, warps, bounded history and integer frame-clock timing.

Local RTX 4060 synthetic throughput (120 simple frames, libx264, no live capture):
1440p took 1.12 s (~107 FPS), 4K took 1.92 s (~63 FPS). These are favorable
synthetic workloads and not supported game-load FPS guarantees. 4K has very
little reserve. A separate 1080p run with Vulkan validation took 1.39 s (~86 FPS);
those timings are not directly comparable. The 240-FPS profile is deliberately
rejected. Real compositor motion, game-load smoothness, long A/V drift and live
pause/resume still need user testing. The synthetic tests are not a substitute.

Final verification for this checkpoint: 267 main tests passed (one existing
ignored thumbnail test); 26 worker tests passed with and without `native-cursor`;
38 libway tests passed including opt-in GPU, native recording and actual Boltsnap
consumer tests. Minimal libway features also passed. Main/worker formatting,
Linux check, release builds, strict C compilation, worker Clippy, snapshot hashes,
`native.py`, `probe.py` and `live.py` passed. Clippy retains the pre-existing unused
`process::output_setup` warning. Installed binaries have `before-cursor-20260922-175414`
backups; the running daemon and user settings were not changed. The new tray
entry requires starting the updated daemon after preserving any RAM shelf items.
