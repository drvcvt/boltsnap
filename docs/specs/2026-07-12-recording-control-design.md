# Recording Control, Tray, and Quickshell Design

## Goal

Replace Boltsnap's permanent recording overlays with a state-aware recording workflow:

- the region outline is optional and its choice persists;
- Alt+Print starts a recording when idle and opens recording controls when active;
- a native tray icon starts region or fullscreen recordings and stores recording defaults;
- pause/resume produces a seamless final video without re-encoding ordinary segments;
- recordings can be kept temporarily in the shelf, saved permanently to disk, or discarded;
- a small Quickshell widget consumes public Boltsnap IPC and is visible only while a recording is active, paused, or finalizing.

Boltsnap remains independent of Quickshell. The shell is an IPC client, not a dependency of the recorder.

## User Experience

### Region selector

The record-mode selector keeps the existing draggable region and `REC` button. A checkbox beside `REC` controls whether Boltsnap shows the thin black outline around the captured region while recording.

The checkbox value persists in Boltsnap's existing configuration as `record_show_frame`. Changing it in either the selector or tray updates the same setting. The default remains enabled to preserve current behavior.

The permanent black elapsed-time/Stop pill is removed.

### Alt+Print behavior

`boltsnap record` becomes state-aware:

- `idle`: open the region selector;
- `recording` or `paused`: ask the daemon to open the centered recording-control popup;
- `finalizing`: show the popup in its disabled `Saving…` state rather than starting another recording.

The popup opens on the currently focused monitor. Escape closes only the popup and never changes recording state.

While recording, the popup offers:

- Pause
- Shelf Save
- Disk Save
- Discard

While paused, Pause becomes Resume. Shelf Save, Disk Save, and Discard remain available.

### Native tray

The Boltsnap daemon owns an always-visible StatusNotifierItem tray icon. Its right-click menu contains:

- Start region recording
- Start fullscreen recording using the configured default target
- Default target submenu populated from connected outputs:
  - each individual output, such as `BenQ (DP-3)` and `AOC (DP-1)`;
  - Both displays
- Both-displays mode:
  - Separate clips
  - Combined clip
- Show recording frame checkbox
- `Video: Move to shelf after Disk Save` checkbox

The settings persist in Boltsnap's existing config as `record_default_target`, `record_both_mode`, `record_show_frame`, and `record_disk_add_to_shelf`. First-run defaults are the focused output, separate clips, visible frame, and adding disk saves to the shelf. If a configured individual output is disconnected, fullscreen start falls back to the focused output and notifies the user. If `both` is selected but only one output remains, Boltsnap records that remaining output and reports the fallback.

The tray is a Boltsnap surface. It does not depend on Quickshell being installed or running.

### Quickshell widget

Quickshell adds a `BoltsnapRecordingService` that starts one long-lived public client:

```text
boltsnap recording watch --json
```

The service parses newline-delimited JSON events and exposes state, active elapsed time, recording scope, and outputs to the bar. Control actions invoke the public `boltsnap recording ...` commands.

The bar widget is placed immediately left of the existing VPN module:

- `recording`: red dot/button and `MM:SS` active elapsed time;
- `paused`: dimmed amber dot/button and frozen elapsed time;
- `finalizing`: `Saving…` with controls disabled;
- `idle`: hidden.

Clicking the widget opens a small Quickshell popout with the same Pause/Resume, Shelf Save, Disk Save, and Discard actions. Quickshell owns only its presentation; all decisions and lifecycle state remain in Boltsnap.

If the watch process exits, Quickshell restarts it with a short delay. This also reconnects after a Boltsnap daemon restart.

## Recording State and IPC

The daemon is the single source of truth and permits one logical recording session at a time.

Public states are:

- `idle`
- `recording`
- `paused`
- `finalizing`

Internal short-lived transitions may be more detailed, but public clients need only these four states and an action-enabled flag.

The existing framed Unix-socket protocol gains request/response commands for:

- status
- watch
- show controls
- pause
- resume
- save to shelf
- save to disk
- discard

`watch` sends an initial snapshot and then one JSON line whenever state changes or the displayed whole second changes. A representative event is:

```json
{"state":"recording","elapsed_ms":83000,"scope":"area","outputs":["DP-3"],"actions_enabled":true}
```

The CLI wraps this protocol so clients do not need to implement Boltsnap's binary frame envelope. Control commands return a non-zero exit code and a concise error when the requested transition is invalid.

Disconnected watchers are removed without affecting the recording. Status publication must never block the Wayland event loop; slow or broken watchers are dropped.

## Recording Lifecycle

### Single region or output

A session begins with one `wf-recorder` child writing a segment under Boltsnap's disk-backed recording cache. The in-memory session tracks:

- capture geometry or output;
- completed segment paths;
- current child and current segment path;
- accumulated active duration;
- start instant of the current active segment;
- destination choice and saved preferences.

Elapsed time is active time only. Paused wall-clock time is excluded.

The default capture profile records the selected native resolution at a constant 240 FPS. On the default `h264_nvenc` path it uses NVENC's `p5` high-quality preset with VBR constant-quality target 16. The same profile is reused for every pause segment so stream-copy concatenation remains valid. Codec overrides remain supported; codec-specific parameters are added only for encoders Boltsnap recognizes.

### Pause and resume

`wf-recorder` has no native pause operation. Pause therefore sends SIGINT so the current segment is finalized correctly, waits for it off the Wayland thread, then enters `paused`. Resume starts a new segment with the same geometry, outputs, codec, and dimensions.

At finalization:

- one segment is moved directly; no merge is performed;
- multiple compatible segments are concatenated with FFmpeg's concat demuxer and `-c copy`;
- no ordinary pause/resume path decodes or re-encodes video, so it has no quality loss.

### Both displays

Both-displays mode starts one synchronized `wf-recorder` child per selected output. Pause and resume operate on all current children as one logical session.

Separate mode produces one final clip per output. Each output's segments use stream-copy concatenation independently. When added to the shelf, separate mode produces one card per clip.

Combined mode first finalizes each output's segments, then composes the output videos according to their Hyprland monitor arrangement. Side-by-side monitors remain side by side; other layouts use their normalized positions, with empty canvas areas filled black. Composition requires one final encode. It uses the configured codec—hardware-accelerated by default—with high-quality settings intended to be visually lossless. The separate source clips are deleted only after the combined output has succeeded.

### Shelf Save

Shelf Save finalizes into Boltsnap's disk-backed cache and adds the resulting file or files as video cards. These files remain temporary and follow the shelf daemon's existing orphan-cleanup policy.

Saving a temporary shelf card to disk is an asynchronous promotion, not an additional permanent copy. Boltsnap chooses a collision-safe destination, moves the file when both paths share a filesystem, and otherwise uses the existing checked copy-then-remove path. Only after success does the card switch to the permanent path and lifetime. Repeated saves therefore cannot overwrite or duplicate a clip.

### Disk Save

Disk Save finalizes into the configured `record_dir`. When source and destination share a filesystem, Boltsnap renames rather than copies. A cross-filesystem destination uses a safe copy followed by source removal only after the copy completes successfully.

When `Video: Move to shelf after Disk Save` is enabled, the shelf card references the permanent saved file. It does not create a second video copy. Separate dual-monitor output creates two permanent files and, when enabled, two shelf cards.

### Discard

Discard stops all active recorder children with SIGINT, waits for clean finalization off the Wayland thread, deletes current and completed segments, removes recording overlays and popups, and returns to `idle` without adding cards.

## Performance and Storage

- Video bytes never travel through IPC.
- The daemon's Wayland/calloop thread never waits for `wf-recorder`, FFmpeg, file copies, or merges.
- One-segment recordings avoid FFmpeg entirely at save time.
- Pause segments use stream copy, not re-encoding.
- Disk-saved shelf cards reference the permanent path instead of duplicating files.
- Combined dual-monitor mode is the only path that necessarily re-encodes video.
- Segment and composition inputs are deleted immediately after verified output success.
- Before a merge or composition that temporarily needs a second full-size output, Boltsnap checks available disk space. If space is insufficient, it keeps all segments and reports the problem instead of risking data loss.
- Starting a recording requires more than 2 GiB available on the recording-cache filesystem. While recording, the existing daemon tick checks available space once per second. At or below the 2 GiB reserve it stops all recorder children cleanly, preserves completed segments, enters the recoverable paused state, and reports the disk-space reason over IPC.
- Daemon startup cleanup removes abandoned cache recordings from crashes, while permanent `record_dir` files are never treated as cache.

The unavoidable temporary peak for a paused or combined recording is the source segments plus the new final file. Without a pause, single-output Shelf Save or Disk Save does not create this duplicate peak.

## Failure Handling

- Invalid state transitions are rejected without mutating the session.
- A failed child spawn leaves the session idle and removes any empty output file.
- Recorder shutdown is bounded: send SIGINT and wait up to 10 seconds, then SIGTERM for up to 2 seconds, then SIGKILL. Any non-empty segment remains recoverable even when the child exits unsuccessfully.
- Unexpected recorder exit stops the logical session and preserves any readable segments for recovery.
- Failed concat, composition, move, or copy never deletes source segments.
- The UI remains in `finalizing` until the worker reports success or failure.
- On success, the daemon publishes the final idle event after cards/files are ready.
- On failure, Boltsnap notifies the user, returns to a recoverable paused state where saving can be retried or discarded, and includes the error in status output.
- Only Boltsnap-created cache paths are automatically deleted.
- Recorder and FFmpeg failures retain concise stderr diagnostics instead of discarding them.
- IPC rejects oversized frames and out-of-range or zero-sized recording geometry before allocating payload buffers or starting a child.

## Daemon Ownership

IPC startup first asks systemd to start an optional user-provided unit, then falls
back to the built-in detached launch when no unit is installed. Recorder children
also receive a parent-death signal so the fallback cannot leave a hidden capture
writing after the daemon dies. This keeps systemd integration available without
shipping or requiring a unit.

## Testing

Rust tests cover:

- legal and illegal recording-state transitions;
- elapsed active-time accounting across multiple pauses;
- segment grouping for one and two outputs;
- stream-copy concat and combined-layout FFmpeg arguments;
- one-segment fast paths;
- IPC request/response and newline-delimited watch events;
- config parsing and persistence for all tray settings;
- output fallback when the configured monitor is absent;
- cleanup rules for success, failure, discard, cache, and permanent files.

Lifecycle tests use fake recorder/FFmpeg commands so they verify process and cleanup behavior without recording the desktop. Existing capture and shelf tests remain part of the regression suite.

Quickshell validation includes `qmllint`, a real `recording watch` stream, visibility/state checks, and manual confirmation that the module sits directly left of VPN and opens its controls. Tray validation confirms dynamic monitor entries and persisted check/radio items in a real StatusNotifier host.

## Out of Scope

- Audio recording remains unchanged.
- Timeline editing belongs in Eddy, not the recording controller.
- Quickshell does not receive video paths or bytes and does not manage recorder processes.
- Mathematical lossless encoding for combined displays is not provided; combined output targets visually lossless high-quality hardware encoding.
