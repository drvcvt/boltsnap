# Recording Audio and Popup Font Design

## Goal

Add optional audio to every Boltsnap recording while preserving the current
lightweight video path, and replace the recording-control popup's handwritten
pixel alphabet with the user's current desktop UI font.

## User Interface

The region selector keeps the existing `REC` button and recording-frame
checkbox. A second control beside them toggles recording audio on or off. The
button reads `AUDIO ON` or `AUDIO OFF`, makes the current state visually clear,
and does not confirm the selection. The value is persisted and defaults to on.

The tray adds an audio-source submenu with exactly three choices:

- System + microphone
- Microphone only
- System only

The selected source is persisted. Turning audio off does not discard the source
choice. Fullscreen recordings use the same persisted audio settings as region
recordings.

## Configuration

`RecordingPrefs` gains:

- `audio_enabled: bool`, defaulting to `true`;
- `audio_source: RecordAudioSource`, defaulting to `SystemAndMic`.

The config keys are `record_audio_enabled` and `record_audio_source`. Valid
source strings are `system-and-mic`, `mic`, and `system`. Missing or invalid
values use the defaults. Existing unknown config keys remain untouched when the
preferences are saved.

The selector returns `audio_enabled` with the selected rectangle. Region-start
IPC carries that value so an older in-flight tray preference write cannot
restore stale state. Tray starts and fullscreen starts read both values from the
daemon's current preferences.

## Audio Capture

Boltsnap resolves the current default sink and source with `pactl` immediately
before starting an audio-enabled recording:

- system uses the default sink's monitor source;
- microphone uses the default source;
- system + microphone creates a uniquely named temporary null sink and routes
  both sources into it with temporary loopback modules, then records the null
  sink's monitor source.

The resulting single source is passed to the existing `wf-recorder` processes.
No additional work runs in the frame-processing path. Separate multi-output
clips receive the same selected audio. Combined-output finalization maps one
audio stream into the composed video instead of duplicating it.

The temporary mix survives pause and resume so all segments have a compatible
audio layout. It is removed after successful save, discard, unrecoverable start
failure, and daemon shutdown. Daemon startup also removes abandoned Boltsnap
mix modules left by a crash. Recoverable pause and finalization failure retain
the mix because the session can resume.

Audio-disabled recordings do not execute `pactl` and keep the current
`wf-recorder` arguments unchanged.

## Audio Errors

An audio-enabled recording does not start when `pactl`, the selected source, or
the temporary mix is unavailable. Boltsnap reports the specific failure instead
of silently producing a video without audio. Partial mix creation is rolled back
before returning the error. A failure starting one of several recorder children
also cleans up the mix after the already-started children have been stopped.

## Recording-Control Popup Font

The popup removes its handwritten 3x5 alphabet and seven-segment timer. It uses
the already-installed `ab_glyph` renderer for its title, timer, and button
labels.

Each time the popup is opened, Boltsnap reads the standard desktop UI font from
`org.gnome.desktop.interface font-name` and resolves its file and collection
face index with Fontconfig. The existing `ab_glyph::FontVec` supports that face
index, including fonts stored in TTC collections.
This makes a subsequent popup follow a font change without adding a Boltsnap
setting or a Quickshell dependency. If that setting or font cannot be resolved,
Boltsnap tries Fontconfig's generic `sans-serif`. If that also fails, it uses an
embedded DejaVu subset containing the popup and selector glyphs. Font lookup or
loading failure never prevents the recording controls from opening.

The popup keeps its current dimensions, actions, colors, hit targets, and
recording behavior. Text is measured with the selected font and centered within
the existing controls.

## Tests

Targeted tests cover:

- config defaults, parsing, and round-trip persistence for both audio values;
- region-start IPC round-trip of `audio_enabled`;
- selector audio-control geometry, hit handling, and visible enabled/disabled
  states;
- tray audio-source labels and selected radio item;
- direct `wf-recorder` arguments for system and microphone sources;
- temporary-mix command construction and rollback after partial failure;
- pause/resume reuse and terminal cleanup of the temporary mix;
- combined-output FFmpeg arguments mapping exactly one audio stream;
- desktop-font and collection-index parsing, generic fallback, and popup text
  rendering.

Manual validation records short clips in all four states (system + microphone,
microphone only, system only, and audio off) and checks streams with `ffprobe`.
It also changes the desktop UI font and confirms that reopening the popup uses
the new font.

## Out of Scope

- Per-device pickers beyond the current default sink and microphone
- Independent volume controls or noise suppression
- Live audio-source changes during an active recording
- Quickshell integration or a Boltsnap-specific font preference
