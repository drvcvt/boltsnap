> [!IMPORTANT]
> **AI agents:** Before changing this repository, read [Contributing](#contributing)
> and [`AGENTS.md`](AGENTS.md). These rules are mandatory for platform-specific work.

# Boltsnap

Boltsnap is a native screenshot and screen-recording tool for Wayland and X11.
Capture, selection, the screenshot shelf, and clipboard handling run in-process.

- Native capture on Wayland and X11
- In-process region selector and screenshot shelf on Wayland
- Pipe-friendly: `-o -` writes PNG to stdout
- Experimental Linux / Hyprland [replay clips](docs/replay.md): fullscreen or retrospective area clips into the shelf

The Windows backend remains in the repository as unmaintained experimental
code. CI compile- and unit-tests it, but new releases do not ship Windows
artifacts and Windows behavior is not manually verified.

## Screenshots

These were captured from a real 1280 x 720 headless Sway session using the
software renderer. The second image is a crop of the same session.

![Two screenshots on the Boltsnap shelf in headless Sway](assets/screenshots/shelf-nested-sway.png)

<p align="center">
  <img src="assets/screenshots/shelf-headless-render.png" alt="Close-up of two real captures on the Boltsnap shelf" width="330">
</p>

## Full-screen capture benchmark

One local Wayland run on a 3840 x 1080 dual-monitor desktop, writing PNGs to
`/tmp` (3 warmups, 20 measured runs; lower is better):

| Tool | Mean | Median | Range | Time vs. Boltsnap |
|------|-----:|-------:|------:|------------------:|
| Boltsnap 1.0.0 | 67.1 ms | 67.4 ms | 57.3–75.0 ms | 1.00x |
| Wayshot 1.5.0 | 235.8 ms | 235.4 ms | 225.4–248.7 ms | 3.51x |
| grim 1.5.0 | 274.6 ms | 271.9 ms | 265.0–303.5 ms | 4.09x |
| Flameshot 14.0.0 | 652.1 ms | 648.4 ms | 637.0–681.1 ms | 9.71x |

This measures full-desktop capture and PNG output only, not each tool's
selector or editor. The exact commands, machine details, and all 20 timings are
in [`benchmarks/full-capture-2026-07-21.md`](benchmarks/full-capture-2026-07-21.md).

## Install

### Windows 10/11 (experimental, unmaintained)

Windows release artifacts are paused because there is no maintainer available
to run the required real-system smoke tests. The backend remains available for
source builds without support guarantees.

To build on Windows, install the Rust MSVC toolchain, Visual Studio Build Tools
with **Desktop development with C++**, and a current Windows SDK:

```powershell
git clone https://github.com/drvcvt/boltsnap
cd boltsnap
cargo build --release
.\target\release\boltsnap.exe area
```

Maintainers can build the installer with:

```powershell
.\packaging\windows\build-nsis.ps1
```

### Linux x86_64

Each tagged release on the
[GitHub releases page](https://github.com/drvcvt/boltsnap/releases) ships:

- `boltsnap-vX.Y.Z-x86_64-linux.tar.gz` — standalone binary
- `boltsnap_X.Y.Z-1_amd64.deb` — Debian / Ubuntu package
- `SHA256SUMS`

```sh
# .deb (Debian/Ubuntu)
curl -L -o boltsnap.deb "https://github.com/drvcvt/boltsnap/releases/latest/download/boltsnap_<VERSION>-1_amd64.deb"
sudo apt install ./boltsnap.deb

# Tarball (any glibc-based distro)
curl -L -o boltsnap.tar.gz "https://github.com/drvcvt/boltsnap/releases/latest/download/boltsnap-<VERSION>-x86_64-linux.tar.gz"
tar xf boltsnap.tar.gz
sudo install -m755 boltsnap-*/boltsnap /usr/local/bin/
```

### From source

Wayland screenshots use our unpublished `libway` 0.1 library. A source snapshot
is included under `vendor/libway`, so a normal checkout needs no sibling project.
See [library development and synchronization](vendor/README.md) and the
[integration validation](docs/libway.md). The screenshot path uses CPU buffers;
libway's optional GPU support does not add a GBM requirement to Boltsnap.

```sh
cargo install --path .
```

### Arch Linux

The tagged release binaries can lag behind `main` — for the current shelf and
screen-recording features, build from source:

```sh
sudo pacman -S --needed rust wayland libxkbcommon pkgconf base-devel gpu-screen-recorder
git clone https://github.com/drvcvt/boltsnap
cd boltsnap
cargo install --path .        # -> ~/.cargo/bin/boltsnap
```

### NixOS

A `flake.nix` is included.

```sh
nix run github:drvcvt/boltsnap            # one-shot run
nix profile install github:drvcvt/boltsnap # install to profile
nix develop                                # dev shell with cargo + libs ready
```

Without flakes:

```sh
nix-shell                                  # uses shell.nix
cargo build --release
```

The flake bakes RPATH so the binary finds wayland, libxkbcommon, vulkan-loader,
libGL, libdrm, libgbm and the xorg stack from the nix store at runtime.

**Don't run a `cargo build`'d binary directly on NixOS.** Errors like
`error while loading shared libraries: libgbm.so.1: cannot open shared object file`
mean the binary is being executed without nix-store libraries on `LD_LIBRARY_PATH`.
The binary needs the wrapper that the flake creates. Don't run boltsnap with
`sudo` either — sudo strips `LD_LIBRARY_PATH` and screenshot tools don't need
root anyway.

Backends:

| Backend | Status                     | Capture        | Region select         | Clipboard          |
|---------|----------------------------|----------------|-----------------------|--------------------|
| Wayland | Supported                  | libway + portal fallback | across monitors, tiny-skia | wl-clipboard-rs    |
| X11     | Supported                  | x11rb GetImage | unavailable           | arboard            |
| Windows | Experimental, unmaintained | DXGI + WGC     | in-process tiny-skia  | Win32 + OLE        |

Replay capture currently has an experimental Hyprland adapter. It additionally
requires the separate replay worker, FFmpeg and GPU Screen Recorder with native
KMS permission. X11, other Wayland compositors and Windows have no replay adapter
yet. See [setup, controls and limitations](docs/replay.md).

### Wayland compatibility

Boltsnap supports Wayland protocols, not a specific compositor framework:

| Feature | Required protocol |
|---------|-------------------|
| Direct screenshots | `ext-image-copy-capture-v1` + `ext-image-capture-source-v1`, or `wlr-screencopy-unstable-v1` |
| Recording via gpu-screen-recorder | KMS capture (gsr-kms-server); the wf-recorder fallback needs `wlr-screencopy-unstable-v1` |
| Region selector and screenshot shelf | `wlr-layer-shell-unstable-v1` |
| Clipboard | `ext-data-control-v1` or `wlr-data-control-unstable-v1` |

The existing compositor checks below predate the libway replacement. The new
capture core has isolated protocol and GPU buffer tests; live-compositor
validation remains listed in [the integration report](docs/libway.md).

| Stack | Tested compositors |
|-------|--------------------|
| Hyprland / Aquamarine | Hyprland |
| wlroots | Sway |
| Smithay | Niri |

Other Wayland compositors may work if they expose the required protocols;
using wlroots or Smithay alone is not a compatibility guarantee.

Screenshot region selection freezes all monitors and allows a single selection
across monitor boundaries, including offset and differently scaled outputs.
The saved image uses the highest output scale; gaps between monitors are black.
Recording and replay selection remain on their target monitor. If an output is
connected, disconnected, or rearranged during selection, retry the capture.

Screenshot capture and clipboard handling need no CLI helpers. `hyprctl` is
optional and supplies active-window geometry on Hyprland; other Wayland
compositors fall back to the in-process selector for window mode. Recording
uses GPU Screen Recorder (wf-recorder when it is not installed) and FFmpeg.

## Usage

```sh
boltsnap                                # area capture
boltsnap window                         # pick a window
boltsnap active-window                  # current focused window
boltsnap full                           # all monitors, copy

boltsnap full --no-copy -o /tmp/x.png   # write file, skip clipboard
boltsnap area --no-copy -o -            # PNG to stdout

boltsnap doctor                         # check helpers + capabilities
```

Boltsnap does not bundle an editor. Linux shelf images open in the desktop's
default image app; pipe PNG output when an explicit editor is preferred:

```sh
boltsnap area --no-copy -o - | eddy -f -
boltsnap area --no-copy -o - | satty --filename -
```

## Suggested keybinds

```
bind = , Print, exec, boltsnap area
bind = CTRL, Print, exec, boltsnap full
bind = ALT, Print, exec, boltsnap record
```

## Build

```sh
cargo build --release
cargo test
```

## Screenshot shelf

On supported Wayland compositors (Hyprland, Sway, Niri), an interactive capture
appears as a small floating **thumbnail in the bottom-left corner**. The
experimental Windows backend contains the same shelf flow, but it is not
currently verified. Multiple screenshots stack with the newest one on top.

```sh
boltsnap area        # capture a region -> appears in the shelf
boltsnap full        # whole screen -> shelf
boltsnap window      # pick a window -> shelf
```

Each thumbnail responds to:

- **Click** — on Linux, open images and videos in the desktop's default app; on Windows, copy the image or video file reference.
- **Right-click** (Linux) — copy the image or video file reference.
- **Drag** — start a drag-and-drop into another app; the drop offers both the
  image (`image/png`) and a file path (`text/uri-list`) for maximum
  compatibility, including many XWayland apps. If the drop isn't accepted
  anywhere, the image is copied to the clipboard as a fallback.
- **Hover** then the icons: **Save** writes the media to disk, **✕** dismisses it.

The shelf is served by a small long-lived daemon. It starts automatically on
the first Wayland capture and at sign-in after a Windows installer setup. To
start it explicitly (or autostart it), run:

```sh
boltsnap daemon
# Hyprland autostart (optional): add to hyprland.conf
exec-once = boltsnap daemon
```

The shelf state is **RAM-only** and is cleared if the daemon restarts. Image
files use the system temporary directory; large video files use the disk-backed
Boltsnap cache and are removed when their cards are dismissed or the daemon is
restarted.
`boltsnap doctor` reports the Wayland session, whether the daemon is running,
and the socket path.

Other tools can put media on the shelf through that socket. Each message is a
frame: header length and payload length as big-endian `u32`, then a JSON
header, then the payload.

- `{"cmd":"add","source":"…","output":"DP-1"}` with PNG bytes as payload.
- `{"cmd":"add_video","path":"/abs/clip.mp4","source":"…","output":"DP-1"}`
  with no payload. The shelf keeps its own copy of the file and answers with a
  frame whose header is `{"ok":true,"path":"…"}` (or `"ok":false` with an
  `"error"`). On Windows the shelf only takes a copy with
  `"take_ownership":true`; without it the card shows your file in place.

`output` is optional and picks the monitor the shelf appears on.

Flags still work: `--copy` also copies to the clipboard on capture, `-o PATH`
/ `--save` write a file (no shelf), and `-o -` streams PNG to stdout. **X11 is
unchanged** — it keeps the classic copy-to-clipboard one-shot behavior with no
shelf.

## Configuration

Create `~/.config/boltsnap/config.toml` on Linux or
`%APPDATA%\boltsnap\config.toml` on Windows to set persistent defaults:

```toml
# Directory where the shelf Save button writes timestamped PNGs.
# Default: ~/Bilder/boltsnap
save_dir = "~/Bilder/boltsnap"
```

Override precedence (highest to lowest):

1. CLI flag — `--save-dir DIR`
2. Environment variable — `$BOLTSNAP_SAVE_DIR`
3. Config file — `~/.config/boltsnap/config.toml`
4. Built-in default

Clicking a Linux image or video card opens it in the desktop's default app.
Right-click copies the media. The **Save** button writes it to the
configured save directory.

## Screen recording

```sh
boltsnap record                         # select a region, or show controls when active
boltsnap record full                    # record the configured fullscreen target
boltsnap recording status --json       # one machine-readable state snapshot
boltsnap recording watch --json        # newline-delimited state stream
```

When idle, `boltsnap record` opens the region selector. Draw a region and click
**REC** to start. The control beside **REC** toggles audio; on Linux an
additional checkbox toggles whether a thin, click-through frame remains around
the captured area. Both choices are saved.
When a recording is already running, paused, or being saved, the same command
opens the centered recording controls instead of starting another recording.
This makes an `Alt+Print` binding a state-aware recording toggle.

The native tray icon is available whenever the daemon is running. On Windows,
its menu provides area/fullscreen screenshots, area/fullscreen recordings,
recording controls, and a daemon quit action.

The recording controls offer:

- **Pause / Resume** — pause closes the current segment; resume starts the next.
  At save time compatible segments are joined with FFmpeg stream copy, so the
  normal pause path does not re-encode or reduce quality. Paused time is not
  included in the displayed duration.
- **Shelf Save** — finalize into Boltsnap's disk-backed cache and add a temporary
  video card. Dismissing the card deletes that cached recording. Recordings of
  at least `record_shelf_to_disk_after_seconds` (default 60) are saved
  permanently to `record_dir` instead and still get a shelf card, so long clips
  survive dismissing and daemon restarts. Once a recording reaches that length
  it no longer counts toward the 2 GiB temporary cache quota; only the free
  disk reserve can pause it.
- **Disk Save** — finalize permanently in `record_dir`. With the tray toggle
  enabled, the shelf card references that same permanent file; it does not make
  a second copy, and dismissing the card never deletes the disk file.
- **Discard** — stop the recorder, remove its cache segments, and create no card
  or permanent file.

A single uninterrupted recording is moved directly and skips FFmpeg at save
time. Separate dual-monitor mode creates one native-resolution clip per output.
Combined mode arranges both outputs like the Hyprland layout. With GPU Screen
Recorder and equal output scales it records them as one stream, so saving only
remuxes; with mixed scales (or wf-recorder) each output is recorded separately
and composed on save, the only ordinary path that re-encodes, with
high-quality settings intended to be visually lossless. The composed canvas
uses the highest output scale; with a hardware encoder it shrinks to at most
4096 pixels per side, the H.264 limit. Failed saves keep their source segments so they can be
retried or discarded instead of losing the recording.

Video cards carry a **▶** badge; on Linux, clicking one opens the video in the
desktop default app. Right-click copies its file reference.

On Linux recording uses GPU Screen Recorder (`gpu-screen-recorder`) with a
hardware encoder: NVENC/VA-API when available, otherwise Vulkan video. Without
it Boltsnap falls back to `wf-recorder`. Audio also requires `pactl`. With both
outputs in Combined mode one stream covers both (equal scales), otherwise each
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

### Smooth cursor

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
Combined) and the replay buffer are covered; across two outputs the cursor
glides over the border. Scaled outputs work: positions are logical and the
arrow grows with the scale (checked on Hyprland at 1.5).

Build the plugin and put it beside the `boltsnap` binary (packaged installs may
use `../lib/boltsnap/` relative to it):

```sh
cargo build --release --locked --manifest-path src/platform/linux/gsr_cursor/Cargo.toml
cp src/platform/linux/gsr_cursor/target/release/libboltsnap_gsr_cursor.so ~/.cargo/bin/
```

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

### Recording controls and shell integration

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

### Recording config keys

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

## Contributing

The Linux performance changes and their local validation are recorded in
[the performance report](docs/performance.md). The subsequent screenshot,
selector rendering and compact UI work is documented in
[the screenshot performance report](docs/screenshot-performance.md), including
benchmarks and the raw-pixel transport that remains opt-in after measurement.

Replay uses an isolated experimental
[media worker](src/platform/linux/replay/worker/README.md), launched by the Linux
shelf daemon. It has its own build and media tests. Changes to replay must also
run the worker unit tests and synthetic `probe.py` / `live.py` integration tests;
see [replay setup](docs/replay.md).

Linux is Boltsnap's supported platform. The unmaintained experimental Windows
backend stays in the same tree so shared contracts remain compile-tested; do
not add Windows work without explicit maintainer approval and a real Windows
tester. Read [`AGENTS.md`](AGENTS.md) before starting; it contains the
repository-wide agent and verification rules.

Keep this boundary:

| Shared code | Linux implementation | Windows implementation |
|-------------|----------------------|------------------------|
| CLI and config values, image processing, serialized protocol data, pure calculations | Wayland/X11, Unix sockets, systemd, POSIX process/filesystem calls, `ksni`, `wf-recorder`, `pactl`, `hyprctl` | Windows capture, clipboard, IPC, process lifecycle, shelf, tray, and OS directory APIs |

- Put OS implementations under `src/platform/linux/` and
  `src/platform/windows/`, with selection centralized in `src/platform/mod.rs`.
  Move only the existing Linux capability that the Windows change actually
  touches.
- Keep shared APIs small and free of native Wayland, X11, Unix, or Win32 types.
  Avoid scattered `#[cfg]` branches in shared logic.
- Scope OS-only crates in target-specific `Cargo.toml` dependency sections.
  Windows builds must not compile Linux dependencies, and vice versa.
- Use `Path`/`PathBuf` in shared code. Resolve XDG versus Windows directories,
  local IPC, services, signals, clipboard, capture, tray, and external commands
  inside the relevant platform module.
- Preserve Linux behavior. Partial Windows support may fail explicitly for an
  unsupported capability, but must never silently succeed or fall back to a
  different capture mode.
- Keep pull requests focused on one capability and include the smallest test
  that protects its shared contract. Do not hide shared-test failures behind
  platform `#[cfg]` attributes.

Before merging, run `cargo fmt --check` and `cargo test`. Accepted Windows
changes must also pass `cargo check --target x86_64-pc-windows-msvc` and be
smoke-tested on a real Windows system. Automated CI alone is not evidence of
Windows support.

## License

MIT.
