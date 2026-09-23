# Replay clips (experimental, Linux / Hyprland)

The replay service continuously captures one display and system output audio.
It keeps encoded packets in memory. Full-display exports copy these packets to
Matroska without re-encoding. Region exports decode and crop on the CPU, then
use the selected hardware encoder. The buffer continues during export.

## Controls

- Tray menu → **Replay buffer → Start** / **Stop**.
- Tray menu → **Clip fullscreen to shelf**, or `boltsnap replay save`.
- `boltsnap record` opens the existing area selector. With a ready replay buffer,
  it shows the last buffered frame on the recorded display. Select a rectangle
  and press **Clip** to save that historical region to the shelf. **REC** still
  starts a new recording. Escape releases the frozen replay selection.
- `boltsnap replay status` prints readiness, duration, encoder and export state.

Suggested binds alongside the existing Alt+Print recording bind:

```ini
bind = ALT SHIFT, Print, exec, boltsnap replay save
```

The local Hyprland Lua configuration uses Alt+Shift+Print for this command.
The Clip button is disabled when the replay buffer is stopped, warming up,
exporting, or already held by another selection. Replay always contains system
audio; the selector's recording audio toggle applies to new recordings.

## Prerequisites and build

The main binary does not link FFmpeg. Install the separate Linux worker next to
Boltsnap or on its PATH. Building the worker requires pkg-config, clang/libclang
and FFmpeg development libraries for libavformat, libavcodec and libavutil.

```sh
cargo build --release --locked
cargo build --release --locked --manifest-path src/platform/linux/replay/worker/Cargo.toml
install -m755 src/platform/linux/replay/worker/target/release/boltsnap-replay-worker target/release/
```

Runtime dependencies are `hyprctl`, FFmpeg CLI, GPU Screen Recorder and its KMS
helper with working native capture permission. GPU Screen Recorder must support
NUT output, `-ffmpeg-opts`, `-ffmpeg-video-opts` and the selected encoder. The local
adapter targets GPU Screen Recorder 6.1.2. Its installation and KMS permission
are not managed by Boltsnap. Companion binaries are searched beside Boltsnap,
then on PATH. Packaged releases do not yet bundle the replay worker.

Auto selection probes `h264_nvenc`, `h264_vaapi`, then `h264_vulkan` by actually
encoding a frame. Manual H.264, HEVC and AV1 variants of these families are
accepted when available and usable. This covers candidate NVIDIA, AMD and Intel
paths, but is not hardware validation for all three vendors. Other installed
encoders receive an explicit unsupported-adapter error. There is no automatic
CPU encoding fallback in the capture service. The test harness can explicitly
use libx264 for synthetic fixtures.

## Configuration

Add to Boltsnap's existing `config.toml`:

```toml
[replay]
autostart = false
duration_seconds = 60
memory_mib = 512
fps = 60
encoder = "auto"
# output = "DP-1"
```

Duration accepts 1–600 seconds, FPS 1–240 and packet memory 64–4096 MiB.
The default video history never exceeds 60 seconds. A different configured
value becomes the hard maximum. GOP boundaries, warmup or memory pressure can
make the saved history shorter. Audio packets can overlap video boundaries by
one packet. The capture uses one-second closed GOPs without B-frames.

Without `output`, replay uses the recording preference's named display, or the
focused display at startup. A preference to record both displays still starts
one focused replay display. Restart replay after changing configuration.

One third of the packet budget belongs to the live ring, one to a frozen export,
and one to its anonymous in-memory source file. Packet buffers are reference
counted during snapshot creation. Preview decoding, encoder surfaces, libraries
and process overhead are additional memory, so `memory_mib` is not a total RSS
limit. The preview decodes only the final GOP and transfers a bounded PNG.

Only one selection/export is accepted at a time. A selection expires after
120 seconds. Region export uses limited CPU threads and reduced scheduling
priority but runs as fast as the decoder and encoder allow. It is no longer
paced to the clip duration. The shelf receives the completed file immediately
after export. Full-display remux still avoids encoding. Capture uses the
very-high quality preset because the selector also displays a decoded buffer
frame; region export uses QP/CRF 18 to limit the additional encoding loss. Stop cancels the capture and any unfinished export; a partial file
can remain for recovery. Completed clips are privately created in the recording
cache and published without replacing existing files. Existing cache and disk
reserve checks apply; concurrent ordinary recording is not a shared disk
reservation. Do not treat that preflight check as a hard system-wide disk cap.

## Verification and limits

Synthetic live tests cover retained duration, frozen selection, preview pixels,
crop dimensions, audio, complete decoding, concurrent buffer/status processing,
export exclusion and stale selections. Hardware encoding has been tested locally
with H.264 Vulkan on an RTX 4060 / FFmpeg 8.1.2.

The supervisor-to-shelf path also passed with a synthetic capture executable.
Native KMS capture then ran locally on GPU Screen Recorder 6.1.2 with
`cap_sys_admin` on `gsr-kms-server`: the buffer reached `ready`, and
`boltsnap replay save` wrote a 22 s 1920x1080 H.264/AAC clip to the shelf that
decodes without errors. Region export from the selector is not covered by that
run.

Real desktop capture, gaming frame times, long-running real-time stability,
monitor changes and AMD/Intel drivers still need validation. The GPU-only crop
prototype failed a pixel comparison on padded video surfaces and is not enabled.
No claim of imperceptible capture overhead follows from the synthetic tests.
See [benchmark results](replay-benchmarks.md) and the
[worker instructions](../src/platform/linux/replay/worker/README.md).

Snapshot preparation now shares immutable video GOPs and only copies the active
GOP when capture next appends to it. See [snapshot measurements and cursor-smoothing
constraints](recording-smoothing.md) for the latest performance work. Cursor
smoothing is not enabled by this change.
