# Replay clips (experimental, Linux / Hyprland)

The replay service continuously captures one display and system output audio.
It keeps encoded packets in memory. Full-display exports copy these packets to
MP4 without re-encoding. Region exports decode and crop on the CPU, then
use the selected hardware encoder. The buffer continues during export.

## Controls

- Tray menu → **Replay buffer** (a checkbox: start or stop).
- Tray menu → **Clip fullscreen to shelf**, or `boltsnap replay save`.
- `boltsnap record` opens the existing area selector. With a ready replay buffer,
  it shows the last buffered frame on the recorded display. Select a rectangle
  and press **Clip** to save that historical region to the shelf. **Record** still
  starts a new recording. Escape releases the frozen replay selection.
- `boltsnap replay status` prints readiness, duration, encoder and export state,
  plus `bytes` held and the live `budget_bytes`. Bytes at the budget with a
  duration under `duration_seconds` means the bitrate bounds the history.

Suggested binds alongside the existing Alt+Print recording bind:

```ini
bind = ALT SHIFT, Print, exec, boltsnap replay save
```

The local Hyprland Lua configuration uses Alt+Shift+Print for this command.
The Clip button is disabled when the replay buffer is stopped, warming up,
exporting, or already held by another selection; it then says why. Replay always contains system
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

The capture codec comes from `gpu-screen-recorder --info`, like ordinary
recordings: `auto` takes hardware H.264 (NVENC or VA-API, else Vulkan video).
GPU Screen Recorder brings its own FFmpeg, so a system FFmpeg without NVENC no
longer moves replay to Vulkan. The system FFmpeg only encodes region clips: its
encoder matching the capture is tried first, then `h264_nvenc`, `h264_vaapi`,
`h264_vulkan`, each by actually encoding a frame. Without one, replay still
runs and region clips say they need a hardware encoder. Manual H.264, HEVC and
AV1 variants of these families are accepted when available and usable. This
covers candidate NVIDIA, AMD and Intel paths, but is not hardware validation for
all three vendors. There is no CPU encoding in the capture service. The test
harness can explicitly use libx264 for synthetic fixtures.

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
# bitrate_kbps = 20000
```

Duration accepts 1–600 seconds, FPS 1–240, packet memory 64–4096 MiB and
`bitrate_kbps` 500–200000. Capture uses a constant bitrate (`-bm cbr`), so RAM
and history do not depend on how much the picture moves. Without
`bitrate_kbps` it is sized to the live third of `memory_mib`: `duration_seconds`
plus one second of video and 128 kbps audio, with a tenth to spare, clamped to
2–80 Mbps. The defaults give about 21 Mbps, which fits 60 seconds in 512 MiB.
A larger explicit bitrate shortens the history instead.
The default video history never exceeds 60 seconds. A different configured
value becomes the hard maximum. GOP boundaries, warmup or memory pressure can
make the saved history shorter. Audio packets can overlap video boundaries by
one packet. The capture uses one-second closed GOPs without B-frames.
The capture pipe is enlarged to 1 MiB (about 100 ms of video), so a worker held
up by a busy CPU does not stall the encoder.

An `output` that is not connected stops replay from starting. Without
`output`, replay uses the recording preference's named display, or the focused
display at startup; when that named display is disconnected it takes the
focused one and says so, like a recording does. A preference to record both
displays still starts one focused replay display. Restart replay after changing
configuration.

The top-level `record_cursor` applies to replay too: `mellow` and `quick` run
GPU Screen Recorder with `-cursor no` and the smooth-cursor plugin
`libboltsnap_gsr_cursor.so` (installed beside Boltsnap, see the README's
"Smooth cursor"), so replay clips contain the smoothed arrow. The mode is read
when replay starts. Replay clips have no `X.cursor.json`.

One third of the packet budget belongs to the live ring, one to a frozen export,
and one to its anonymous in-memory source file. Packet buffers are reference
counted during snapshot creation. Preview decoding, encoder surfaces, libraries
and process overhead are additional memory, so `memory_mib` is not a total RSS
limit. The preview decodes only the final GOP and transfers a bounded PNG.

Only one selection/export is accepted at a time. A selection expires after
120 seconds. Region export uses limited CPU threads and reduced scheduling
priority but runs as fast as the decoder and encoder allow. It is no longer
paced to the clip duration. The shelf receives the completed file immediately
after export. Full-display remux still avoids encoding. Region export uses
QP/CRF 18 to limit the additional encoding loss. Exports, the region encoder
and probes run at nice 19 with the lowest best-effort IO priority; only GPU
Screen Recorder and the selector's preview keep normal priority. Helpers get
`oom_score_adj` 500, so under memory pressure the kernel ends them before a
game. Stop
cancels the capture and any unfinished export; a partial file can remain, but
an MP4 cut off before its index is not playable. Completed clips are privately
created in the recording cache and published without replacing existing files.
The shelf is RAM-only, so the daemon removes leftover `clip-*.mp4` files (and `.mkv` from older workers) with the
other orphaned recordings when it starts. Existing cache and disk
reserve checks apply; concurrent ordinary recording is not a shared disk
reservation. Do not treat that preflight check as a hard system-wide disk cap.

A failed export removes its partial file at once, so it cannot keep a full disk
full. A finished clip that only could not take its name stays, and the error
names it; startup cleanup removes such `clip-*.mp4.<pid>.<seq>.partial` files
with the other orphans.

A buffer whose input failed or stalled stays saveable: `replay save` writes
what it holds, the clip ends where capture stopped, and a notification says so.
The watchdog replaces the session after about 15 seconds. It ends only the
dead capture and keeps the old buffer until the new session runs; an export
started on the old buffer meanwhile finishes first. If the restart fails, the
old buffer stays saveable, one notification names the reason, and the watchdog
tries again after 30 s, 60 s and then every 5 minutes.

An open selection survives a failed input: the worker reports it as
`selection_active` until Save, Cancel or its 120-second expiry, and neither the
watchdog nor a manual Start replaces the session meanwhile. Start answers that
a selection or export is still open. A worker older than the daemon lacks the
field and is treated as holding no selection.

Replay clips are MP4. The capture stream carries a `1/fps` time base and MP4
keeps it, so every frame sits exactly on its slot; Matroska would round frame
times to whole milliseconds (at 120 FPS 0, 8, 17, 25 ms), which makes fast
motion judder. Region exports read a NUT copy of the buffer for the same
reason. Saving a replay card moves its bytes unchanged and keeps the `.mp4`
name; dragging it offers `video/mp4` next to the file reference. The daemon
still accepts `.mkv` clips from an older worker.

## Logs and load

GPU Screen Recorder's output goes to `replay-capture.log` in Boltsnap's cache,
every line with its local time, up to 4 MiB per session; the previous
session's log stays as `replay-capture.log.old`. The worker's lines go to
`daemon.log` with the time. A capture below 90 % of its frame rate for three
reports in a row is noted in `daemon.log` ("replay capture slowed to …", then
"back at …"). The settings window's Replay page shows under **Load** the CPU
(percent of one core) and memory of the buffer's recorder, its helper and the
worker, measured once a second.

## Verification and limits

Synthetic live tests cover retained duration, frozen selection, preview pixels,
crop dimensions, audio, complete decoding, concurrent buffer/status processing,
export exclusion, stale selections and exact frame times of full and region
clips (every video packet on the `1/fps` grid). Hardware encoding was first
tested locally with H.264 Vulkan on an RTX 4060 / FFmpeg 8.1.2; the machine has
used NVENC since. Constant-bitrate capture is not yet checked live.

The supervisor-to-shelf path also passed with a synthetic capture executable.
Native KMS capture then ran locally on GPU Screen Recorder 6.1.2 with
`cap_sys_admin` on `gsr-kms-server`: the buffer reached `ready`, and
`boltsnap replay save` wrote a 22 s 1920x1080 H.264/AAC clip to the shelf that
decodes without errors. Region export from the selector is not covered by that
run.

The native fullscreen run above is limited live evidence. Interactive region
export, gaming frame times, long-running real-time stability, monitor changes
and AMD/Intel drivers still need validation. The GPU-only crop
prototype failed a pixel comparison on padded video surfaces and is not enabled.
No claim of imperceptible capture overhead follows from the synthetic tests.
See [benchmark results](replay-benchmarks.md) and the
[worker instructions](../src/platform/linux/replay/worker/README.md).

Snapshot preparation now shares immutable video GOPs and only copies the active
GOP when capture next appends to it. See [snapshot measurements](recording-smoothing.md).
