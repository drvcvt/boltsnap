# Screen recording

```sh
boltsnap record                         # select a region, or show controls when active
boltsnap record full                    # record the configured fullscreen target
boltsnap recording status --json       # one machine-readable state snapshot
boltsnap recording watch --json        # newline-delimited state stream
```

When idle, `boltsnap record` opens the region selector. Draw a region and click
**Record** to start (**Clip** saves the region from the replay buffer instead).
The **Frame** and **Audio** buttons beside them toggle a thin, click-through
frame around the captured area (Linux) and audio; both choices are saved.
When a recording is already running, paused, or being saved, the same command
opens the centered recording controls instead of starting another recording.
This makes an `Alt+Print` binding a state-aware recording toggle.

The native tray icon is available whenever the daemon is running. On Windows,
its menu provides area/fullscreen screenshots, area/fullscreen recordings,
recording controls, and a daemon quit action.
On Linux the icon's dot turns red while a recording runs or is being saved and
amber while it is paused. It comes in 16 to 64 px, the small sizes fitted to
whole pixels so a 16 px tray shows it sharp. For light surfaces there is a
dark version of the mark, `assets/logo/boltsnap-icon-dark.svg`; all icons are
built from `assets/logo/construct.py` by `assets/logo/icons.py`. During a recording the menu starts with Pause or
Resume, Save to shelf and Save to disk; Discard stays in the recording controls.
"Clip fullscreen to shelf" is only enabled while the replay buffer runs.
Default monitor offers Focused display, each monitor and Both displays; a
configured monitor that is unplugged stays listed as disconnected.

The recording controls offer:

- **Pause / Resume** — pause closes the current segment; resume starts the next.
  At save time compatible segments are joined with FFmpeg stream copy, so the
  normal pause path does not re-encode or reduce quality. Paused time is not
  included in the displayed duration.
- **Save to shelf** — finalize into Boltsnap's disk-backed cache and add a temporary
  video card. Dismissing the card deletes that cached recording. Recordings of
  at least `record_shelf_to_disk_after_seconds` (default 60) are saved
  permanently to `record_dir` instead and still get a shelf card, so long clips
  survive dismissing and daemon restarts. Once a recording reaches that length
  it no longer counts toward the 2 GiB temporary cache quota, neither for
  itself nor for replay clips or imported videos; only the free disk reserve
  can pause it. Segments a crashed daemon left behind never count.
- **Save to disk** — finalize permanently in `record_dir`. With the tray toggle
  enabled, the shelf card references that same permanent file; it does not make
  a second copy, and dismissing the card never deletes the disk file.
- **Discard** — stop the recorder, remove its cache segments, and create no card
  or permanent file. On Linux the button first reads **Confirm**; a second press
  within three seconds discards.

On Linux opening the controls during a recording pauses it first and shows
them once the recorder has stopped, so they never end up in the video; each
opening adds one lossless segment boundary. Esc or Resume hides them and
continues. The open controls take keys: Space resumes, Enter saves to the
shelf, D saves to disk, Delete discards (twice).

While recording, the daemon pauses the recording (segments kept, a
notification and the controls' title say why) when the recorder exits, when a
GPU Screen Recorder segment stops growing or stops reporting encoded frames
for 6 seconds, or when free space in
the recording cache falls to what saving still needs: at least 2 GiB, and
room to remux everything recorded so far when that remux happens on the cache's
disk. It warns once 2 GiB before that point. A disk save remuxes straight into
`record_dir`, so a recording only needs room there once; when `record_dir`
is on another disk, a long recording also pauses (and warns 2 GiB before)
while that disk still has room for the finished file. If the daemon dies
mid-recording (crash, logout, a lost output) or before a failed save was
retried or discarded, the next start saves the segments it left as
`boltsnap-<time>-<output>-recovered.mp4` in `record_dir` and says so;
segments that cannot be joined are moved there unchanged. FFmpeg work at save
time and thumbnails run at nice 19 with the lowest best-effort IO priority.

A single MP4 segment is moved directly and skips FFmpeg at save time.
GPU Screen Recorder produces Matroska segments, so even an uninterrupted
recording is remuxed to MP4 with FFmpeg without re-encoding. Matroska keeps
frame times in whole milliseconds; the remux puts each frame back on its exact
slot so playback does not judder.
Separate dual-monitor mode creates one native-resolution clip per output.
Combined mode arranges both outputs like the Hyprland layout. With GPU Screen
Recorder, equal output scales and a layout up to 4096 pixels per side it records
them as one stream, so saving only remuxes; otherwise (or with wf-recorder) each
output is recorded separately
and composed on save, the only ordinary path that re-encodes, with
high-quality settings intended to be visually lossless. The composed canvas
uses the highest output scale; with a hardware encoder it shrinks to at most
4096 pixels per side, the H.264 limit. Failed finalization retains recoverable
files, which may already be remuxed or composed. The popup then reads
**Save failed**: Save to shelf, Save to disk and Discard still work, Resume stays off
because the recovered files no longer match new segments. A recorder that
merely failed to stop cleanly keeps its segments and can still resume.

Video cards carry a **▶** badge; on Linux, clicking one opens the video in the
desktop default app. Right-click copies its file reference.

On Linux recording uses GPU Screen Recorder (`gpu-screen-recorder`) with a
hardware encoder: NVENC/VA-API when available, otherwise Vulkan video. Without
it Boltsnap falls back to `wf-recorder`. Audio also requires `pactl`. With both
outputs in Combined mode one stream covers both (equal scales, up to 4096 px), otherwise each
output is recorded separately and composed on save with the same encoder. On
NVIDIA, FFmpeg needs NVENC: GPU Screen Recorder's Vulkan H.264 encoder froze
here at 1080p 240 FPS after a few seconds. For demo videos `record_profile =
"quiet"` (60 FPS) keeps files light enough to play smoothly; 240 FPS over two
outputs is about a gigapixel per second to decode. The
unmaintained Windows backend contains native Windows Graphics Capture, Media
Foundation H.264/AAC, and WASAPI recording code, but it is not currently
manually verified or shipped.

```sh
# Arch / Manjaro
pacman -S gpu-screen-recorder libpulse
```

## Smooth cursor

Inspired by Screen Studio: with `record_cursor = "mellow"` or `"quick"`
gpu-screen-recorder leaves out the system cursor and a small gsr plugin
(`libboltsnap_gsr_cursor.so`) draws a spring-smoothed arrow into every captured
frame instead. Boltsnap follows the pointer through the
`ext-image-copy-capture-v1` cursor session and feeds it to the plugin while
recording; the critically damped spring (no overshoot) removes small jitter and
lets fast moves glide, with a short motion blur. Mellow trails more (90 % of a
move after about 0.5 s), quick follows closely (about 0.11 s). The arrow is
rasterized at four times the video resolution and area-filtered per pixel, so
it stays sharp at subpixel positions and in motion. The cursor is part of the video, so saving only remuxes, as
with the system cursor. Single outputs, areas, both outputs (Separate and
Combined) and the replay buffer are covered. At a monitor boundary the cursor
can snap if the old tracker reports Leave before the new tracker reports Enter.
Scaled outputs work: positions are logical and the
arrow grows with the scale (checked on Hyprland at 1.5).

Build the plugin and put it beside the `boltsnap` binary (packaged installs may
use `../lib/boltsnap/` relative to it):

```sh
cargo build --release --locked --manifest-path src/platform/linux/gsr_cursor/Cargo.toml
cp src/platform/linux/gsr_cursor/target/release/libboltsnap_gsr_cursor.so ~/.cargo/bin/
```

The Nix package includes the plugin. The current tarball and Debian packaging
do not; those installations need the separate plugin build for smooth cursor modes.

Beside each recorded clip `X.mp4` Boltsnap keeps `X.cursor.json` (raw pointer
samples in clip pixels, format `boltsnap.cursor` v1, `cursor_in_video: true`,
at most 120 per second) for editors, e.g. to follow the cursor when zooming. It moves and is deleted
with the clip. Replay clips have none. There is no cursor-free copy of the
video, so editors cannot re-render or drop the cursor.

Limits: the drawn cursor is always Boltsnap's arrow at `XCURSOR_SIZE`. Hyprland
0.56 can only copy SHM cursor images and stops that capture for other cursors,
so shape changes (I-beam, hand) and clicks are not shown. The compositor must
offer the EXT cursor session and the plugin must be installed; otherwise
starting a smooth recording or replay fails with a message instead of recording
without a cursor. The mode is fixed when a recording or the replay buffer starts.

## Recording controls and shell integration

The public control commands are suitable for scripts and shell widgets:

```sh
boltsnap recording show-controls
boltsnap recording pause
boltsnap recording resume
boltsnap recording save-shelf
boltsnap recording save-disk
boltsnap recording discard
```

`boltsnap stop` remains a compatibility alias for `recording save-shelf`.
Quickshell can consume the long-lived `recording watch --json` stream to show a
red running timer, an amber paused timer, and `Saving…` while finalizing. It calls
the public commands above for controls; Boltsnap does not depend on Quickshell,
and no video data or paths are sent through IPC.

## Recording config keys

```toml
# ~/.config/boltsnap/config.toml

# Recording encoder (Linux; Windows always encodes H.264 through Media
# Foundation). Default "auto": hardware H.264 through gpu-screen-recorder
# (NVENC or VA-API, else Vulkan video). Explicit FFmpeg names such as
# "h264_vulkan", "hevc_nvenc" or "libx264" (CPU) select one encoder without
# fallback. wf-recorder, used only without gpu-screen-recorder, maps "auto" to
# h264_nvenc.
record_codec = "auto"

# Live recording profile (Linux): "quality" = 240 fps (default), "quiet" = 60 fps.
# record_fps (1-240) overrides the profile's frame rate, e.g. 120: smooth
# motion at half the data of 240. A frame rate that divides the monitor's
# refresh rate (240 Hz: 60/120/240) captures it without judder.
# Encoder quality settings stay the same. Takes effect on the next recording;
# pause/resume keeps its original profile. Replay has its own fps setting.
record_profile = "quality"

# Directory where finished .mp4 files are saved.
# Default: same as save_dir
record_dir = "~/Videos/boltsnap"

# Fullscreen recording target: "focused", "output:<name>", or "both".
# "output:<name>" and "both" are Linux-only; Windows records the focused monitor.
# Default: focused
record_default_target = "focused"

# When the target is "both": "separate" or "combined" (Linux).
# Default: separate
record_both_mode = "separate"

# Pointer in recordings and replay (Linux, gpu-screen-recorder): "system"
# records the compositor's cursor as is. "mellow" and "quick" draw a
# spring-smoothed arrow (XCURSOR_SIZE) live through the gsr plugin
# libboltsnap_gsr_cursor.so, see "Smooth cursor". Tray: Recording > Cursor.
# Default: system
record_cursor = "system"

# Show the outline around a recorded region (Linux). Default: true
record_show_frame = true

# Add permanently saved recordings to the shelf without copying them. Default: true
record_disk_add_to_shelf = true

# Shelf saves of recordings at least this long go to record_dir (with a shelf
# card) instead of the temporary cache. 0 keeps every shelf save temporary.
# Default: 60
record_shelf_to_disk_after_seconds = 60

# Include audio in recordings. Default: true
record_audio_enabled = true

# "system-and-mic", "mic", or "system". Default: system-and-mic
record_audio_source = "system-and-mic"
```

`$BOLTSNAP_RECORD_CODEC` overrides `record_codec` from the environment.

Audio sources follow the current default sink and microphone. Per-device
pickers and volume controls are intentionally left to the desktop audio mixer.
