# Replay worker

Experimental Linux media worker, launched by the shelf replay supervisor.
The main Boltsnap binary does not link FFmpeg. User controls and installation
are documented in [replay setup](../../../../../docs/replay.md).

The probe reads a finite compressed stream into a time- and byte-limited GOP
ring, then remuxes its retained history into Matroska. Video and audio remain
encoded. Input must have closed GOPs, no frame reordering, positive packet
durations, one video stream and at most one audio stream. `--closed-gop` is an
explicit assertion about the test source, not bitstream validation.

`--seconds` sets the maximum retained video window, defaulting to 60 seconds.
Old GOPs are discarded as soon as their start falls outside that window.
The next keyframe can make a clip shorter; an expired active GOP is discarded
until another keyframe arrives. Compressed audio packets may overlap the
video boundaries slightly.

## Build

Linux, Rust, pkg-config, clang/libclang and development packages for
libavformat, libavcodec and libavutil are required. Tests also need Python 3
and an FFmpeg CLI with libx264 and AAC. `ffmpeg-next` is pinned in the separate
manifest and lockfile. The local verified library version is FFmpeg 8.1.2;
the Ubuntu 22.04/24.04 CI matrix is configured separately.

From the repository root:

```sh
cargo build --release --locked --manifest-path src/platform/linux/replay/worker/Cargo.toml
cargo test --locked --manifest-path src/platform/linux/replay/worker/Cargo.toml
cargo fmt --manifest-path src/platform/linux/replay/worker/Cargo.toml --check
```

## Probe

```sh
src/platform/linux/replay/worker/target/release/boltsnap-replay-worker capabilities
src/platform/linux/replay/worker/target/release/boltsnap-replay-worker probe \
  --input source.nut --output clip.mkv --closed-gop --seconds 30 --memory-mib 128
```

`--input -` reads stdin. The finite probe ends at EOF. The separate
`live SOCKET DIRECTORY SECONDS MEMORY_MIB ENCODER` mode accepts status, freeze,
cancel, save and stop requests over a same-user abstract Unix socket. A freeze
reply contains bounded JSON followed by a length-declared PNG preview. The
supervisor owns capture and worker processes, autostart and shelf integration.
`capabilities` lists encoders in the linked library. It does not claim that
their hardware is present or usable.

Output is created privately and published without replacing another file.
A failed write leaves a named `.partial` file for inspection. The probe caps
output size at twice its packet budget. Packet accounting includes native
buffer lengths, side data and a conservative container overhead allowance;
it is not a cap on total process RSS or on the demuxer's internal allocations.

For the continuous NUT transport, the current candidate options are
`write_index=0`, `syncpoints=none`, `strict=experimental` and `flush_packets=1`.
Disabling only the index does **not** prevent growing demuxer syncpoint
storage. The probe can read ordinary finite NUT fixtures, but such a stream
must not be used for an unlimited replay session.

## Verification

```sh
python3 src/platform/linux/replay/worker/tests/probe.py \
  --worker src/platform/linux/replay/worker/target/release/boltsnap-replay-worker
```

```sh
python3 src/platform/linux/replay/worker/tests/live.py \
  --worker src/platform/linux/replay/worker/target/release/boltsnap-replay-worker
# Optional hardware export test:
python3 src/platform/linux/replay/worker/tests/live.py \
  --worker src/platform/linux/replay/worker/target/release/boltsnap-replay-worker \
  --encoder h264_vulkan
```

The live test exercises frozen selection, last-frame preview pixels, cropped and
full exports, system-tone audio, stale IDs and concurrent status/ingest. libx264
is used only for the default synthetic fixture and CPU export test.

The finite test generates synthetic video and audio. It checks complete decoding,
frame hashes, encoded audio hashes, clipping boundaries, byte pressure,
existing-file protection, rejection of reordered input and a real pipe.
Artifacts remain in the printed temporary directory.

`--vulkan` additionally tests hardware crop pixels against a CPU reference
and performs a hardware encode. It is a capability gate, not a required test
on machines without Vulkan. The local FFmpeg 8.1.2 crop path currently fails
the pixel comparison for a padded H.264 fixture; it is not approved for use.

```sh
python3 src/platform/linux/replay/worker/tests/soak.py \
  --worker src/platform/linux/replay/worker/target/release/boltsnap-replay-worker \
  --source source.nut --loops 2400
```

The soak measures packet processing and Linux RSS across an accelerated
media timeline. With enough samples it rejects RSS growth over 4 MiB after
warmup. It does not measure game frame times or replace real-time
capture tests. `--syncpoints default` reproduces the rejected NUT transport.
Results and remaining P0 gates are recorded in
[the benchmark log](../../../../../docs/replay-benchmarks.md).

## Optional native cursor producer

`--features native-cursor` adds the experimental ordinary-recording producer;
it does not replace replay capture. It additionally needs FFmpeg 8.1.2 libavfilter,
EGL/GLES 3, GBM and Vulkan development libraries plus a C compiler. See
[cursor setup, tests and limitations](../../../../../docs/recording-smoothing.md#optional-cursor-smoothing-experimental).
The `cursor-fixture` and `cursor-record-fixture` commands use synthetic pixels
or a caller-selected test compositor/audio tone and are diagnostic entry points.
Live `cursor-record` rejects hardware codecs and non-60-FPS rates explicitly.
