# Live smooth cursor through a gpu-screen-recorder plugin, 2026-09-24

Status: implemented; live recording check pending (needs the user's approval).
Linux only; the Windows backend stays frozen.

Replaces the save-time cursor renderer (Phase 4 of
[2026-09-23-performance-functionality.md](2026-09-23-performance-functionality.md)).
Product decisions are settled (user, 2026-09-24): the cursor is burned in while
recording, saving only remuxes, no `X.clean.mp4` anymore, the look stays
(vector arrow at `XCURSOR_SIZE` x scale, MELLOW 60/15.5 and QUICK 600/49,
`SHAKE_PX` 2, 8 blur taps over 8 ms, modes `system|mellow|quick`, tray submenu).

## 1. What gsr actually does with a plugin

Read in gsr 396526a (`plugin/plugin.h`, `src/plugins.c`,
`src/recorder/recorder.c`, `src/recorder/screenshot.c`, `src/egl.c`,
`src/color_conversion.c`) and compared file by file with the installed build
(Gentoo `gpu-screen-recorder-9999`, git ab9cf69 = 6.1.2 plus two commits): all
plugin-relevant files are identical, `/usr/include/gsr/plugin.h` too.

- **The plugin draws over the captured frame, not into an empty overlay.** With
  at least one plugin gsr sets `output_color_conversion = &plugins.color_conversion`,
  so the capture itself is drawn into the plugin texture (RGB destination).
  `gsr_plugins_draw` then binds that texture's FBO, sets the viewport to the
  video size and calls every `draw`. Afterwards the texture is converted into the
  encoder's YUV textures. The plugin therefore must **not** clear anything.
  (The session handoff assumed a cleared overlay composited afterwards; that is
  wrong.)
- **Alpha of the plugin texture reaches the encoder.** The RGB->YUV shader
  writes `FragColor.w = pixel.a` and blending is globally on with
  `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)` (`egl.c`). The captured
  frame is opaque. If the plugin blends with gsr's state, the texture alpha
  becomes `a*a + (1-a)` < 1 at cursor edges and the YUV conversion mixes those
  pixels with the previous frame's YUV data (ghosting). The plugin must blend
  colour with `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` and keep the destination alpha
  (`glBlendFuncSeparate(..., GL_ZERO, GL_ONE)`), then restore gsr's blend func.
- **Context:** EGL with `EGL_OPENGL_ES_API`, `EGL_CONTEXT_CLIENT_VERSION 3`
  -> GLES 3.x, `#version 300 es` shaders, `texelFetch`. GLX only for X11 +
  NVIDIA monitor capture (`graphics_api` = GLX); the smooth cursor needs the
  Wayland cursor session anyway, so the plugin refuses GLX in init.
- **GL entry points:** gsr loads GL functions with `dlsym` from
  `dlopen("libGL.so.1")` (GLVND dispatch to the current EGL context). The
  plugin does the same; no link-time GL dependency.
- **Texture format:** `GL_RGBA8` for 8-bit codecs, `GL_RGBA16` for 10-bit
  (`hevc_10bit` etc.). Not sRGB: values are the captured, already encoded
  colours, so the arrow's sRGB bytes are written as is and blending happens in
  gamma space, like the old CPU blend. 10-bit needs nothing special (normalized
  texture, same shader); Boltsnap never selects 10-bit codecs.
- **Orientation:** texture row 0 is the top image row (the encoder copies
  texture rows in memory order; the `hello_triangle` example flips y for the
  same reason). `floor(gl_FragCoord.y)` is the image row.
- **Interface version:** gsr has no version negotiation. It checks only that
  `name` is set and `version != 0`; the ABI is the struct layout of
  `plugin.h`, identical in the installed header. Nothing to test beyond
  loading.
- `init` runs with the context current, once, after capture setup; `draw` runs
  once per encoded frame, only when not paused; `deinit` runs before the EGL
  context is destroyed. `is_damaged` matters only for `-fm content`; Boltsnap
  uses `cfr`.

## 2. Design

```
daemon                                           gpu-screen-recorder (one per output / region)
cursor_track thread per output ── pipe ─────────▶ plugin (libboltsnap_gsr_cursor.so)
   │  (libway cursor_positions)   (fan-out:          reads new lines each draw(),
   │                               every tracker     runs the spring to "now",
   └─ <segment>.cursor file        writes to every   draws the blurred arrow over
      (for X.cursor.json)          plugin's pipe)    the captured frame
```

### Position delivery: daemon tracker + inherited pipe

The daemon keeps its tracker threads (`cursor_track.rs`). Each tracker writes
every event to its segment track file (unchanged format, for `X.cursor.json`)
and to the write ends of the session's plugin pipes. gsr inherits the read end
(CLOEXEC cleared only in the forked child, `pre_exec`); the plugin gets its
number through the environment, sets it `O_NONBLOCK | FD_CLOEXEC` and drains it
at every `draw`.

Chosen over a Wayland connection inside the plugin because it keeps the plugin
free of Wayland/libway (tiny cdylib: `libc` + `tiny-skia`), keeps the "compositor
has no cursor session -> start fails with a message" check in the daemon, needs
no second cursor session for `X.cursor.json`, and gives Combined recordings one
continuous pointer path (below).

Writes are non-blocking; a full pipe (plugin stalled, > ~8 s of events) drops
lines instead of blocking the tracker. `SIGPIPE` is ignored in Rust binaries, a
dead gsr only makes writes fail.

Feed protocol, one ASCII line per event (shared code, round-trip tested):
`SOURCE US p X Y` (global logical position) and `SOURCE US l` (leave). `SOURCE`
is the tracker index within the segment, `US` CLOCK_MONOTONIC microseconds of
receipt (same clock as the plugin's `clock_gettime`). Enter carries no position
and is not sent; visibility follows positions as in `cursor::timeline`.

### Plugin configuration

One variable, `BOLTSNAP_CURSOR`, set on the gsr command:
`fd=5 preset=mellow size=24 origin=1920,0 width=1280`

- `preset`: `mellow` | `quick` (the `RecordCursor` keys).
- `size`: nominal arrow size in logical pixels (`XCURSOR_SIZE`, default 24,
  read by the daemon).
- `origin`: global logical coordinate of the video's top-left pixel.
- `width`: logical width the video covers.

The plugin derives `scale = video_width / width` (video pixels per logical
unit, `init_params.width`) and draws `arrow(size * scale)`. Missing or invalid
config makes `gsr_plugin_init` fail, so gsr exits at start with the plugin's
message instead of recording without a cursor.

### Coordinate mapping

`pixel = (global - origin) * scale`, per plugin:

| Scope | origin | width |
| --- | --- | --- |
| One output | output's logical position | output's logical width |
| Area | region x, y | region w |
| Combined / both | each gsr: its own output | its own output's logical width |

For more than one output every plugin receives the positions of all trackers
of the segment. The spring runs on one continuous path in each plugin, so a
cursor gliding across the border keeps gliding and is drawn partly on both
outputs where the arrow overlaps the edge; positions outside the video clip
away. A leave hides the cursor only when no tracker still sees it (same rule as
`cursor::merge`). Output geometry comes from the tracker's libway output
(logical rect), region geometry from the selection.

### Motion (portable, shared by #[path])

New `src/record/cursor_motion.rs`, included by the plugin crate like the replay
worker includes `src/replay`: `Preset`, `MELLOW`, `QUICK`, `SHAKE_PX`,
`Sample`, `Spring`, `SHUTTER_MS`, `BLUR_SAMPLES`, the arrow, the feed protocol,
the plugin config and a live stepper:

- `Motion::push(sample)` queues samples (clip pixels, ms).
- `Motion::advance_to(ms)` simulates 1 ms steps exactly like the old
  `trajectory` (apply samples due at the tick, record position, advance), and
  keeps the last `SHUTTER_MS + 1` positions. Late samples apply at the next
  tick. Gaps longer than 2 s are skipped (the spring has settled).
- `Motion::taps(hotspot)` returns the 8 exposure taps ending at the newest tick
  (rounded sprite top-left or hidden), the old `exposure` rule. At rest all 8
  coincide: a sharp arrow. A fresh plugin starts simulating `SHUTTER_MS` before
  its first frame, so the first frame is not faded in.

`cursor.rs` keeps the track file format, `timeline`, `merge` and
`sidecar_json`; the save-time `trajectory`, `exposure`, YUV `Sprite` and
`blend_*` go away.

### GL drawing path (only inside `draw`)

- First `draw`: load entry points from `libGL.so.1`, compile one program, upload
  the arrow (straight-alpha RGBA8, NEAREST), create an empty VAO. Any failure is
  fatal: message on stderr (the daemon log), then `_exit(1)`; the daemon sees the
  recorder die and pauses with an error, so no cursor-less video is delivered
  silently.
- Every `draw`: drain the pipe, `advance_to(now)`, compute the taps. Hidden
  (all taps hidden): return without GL calls. Otherwise draw one quad over the
  union of the tapped sprite rectangles. The fragment shader fetches the texel
  under each of the 8 taps, sums premultiplied colour and alpha, and outputs
  `rgb = sum.rgb / sum.a`, `a = sum.a / 8` (discard at 0): straight alpha of the
  averaged exposure. Blending `SRC_ALPHA, ONE_MINUS_SRC_ALPHA` for colour and
  `ZERO, ONE` for alpha yields `frame * (1 - coverage) + avg_premultiplied`,
  exactly the mean of the 8 single-arrow composites (not 8 overdraws).
- State touched and restored: program, VAO, active texture unit and its 2D
  binding, blend func (back to gsr's `SRC_ALPHA, ONE_MINUS_SRC_ALPHA`). No
  clear, no viewport change, no FBO change.
- `deinit` frees Rust memory only; the GL objects die with gsr's context.
- Cost per frame: one `read`, a few spring steps, one small draw call.

### Recording integration

- `gsr::args(..., cursor: Option<&Path>)`: with a plugin path it adds
  `-cursor no -write-first-frame-ts yes -p <plugin>`; without, `-cursor yes`
  (unchanged system-cursor argv).
- `spawn_segment(..., cursor: RecordCursor, ...)`: for smooth modes it resolves
  the plugin, creates one pipe per gsr, starts one tracker per output/region
  (before capture, so the initial position is known), then spawns each gsr with
  `BOLTSNAP_CURSOR` and its read end. Trackers stay in `ActiveRecorder.cursor`
  and stop when the segment stops (unchanged). Streamed-MKV stop/kill rules are
  untouched.
- Plugin lookup: `libboltsnap_gsr_cursor.so` beside the running binary, then
  `../lib/boltsnap/` relative to it. Missing -> the recording start fails with
  "smooth cursor needs libboltsnap_gsr_cursor.so beside boltsnap".
- Resume spawns the same way; each segment gets fresh pipes and trackers.

### Replay

`start_capture` reads `record_cursor` like it reads the default target. For
smooth modes it starts one tracker for the replay monitor (no track file), and
runs gsr with `-cursor no -p <plugin>` plus `BOLTSNAP_CURSOR`. The tracker lives
in the replay `Session` and stops with it. Replay clips have no `cursor.json`.
The mode is fixed per replay start; changing it applies on the next start.

### `X.cursor.json`: kept

Eddy's studio plan uses the raw samples for "follow cursor" and zoom
suggestions, and they stay cheap: the tracker already runs for the plugin, the
track file is a few KB per minute, and finalize needs one ffprobe per segment
(durations, as before) plus one for the clip size. Format `boltsnap.cursor` v1
unchanged, `cursor_in_video: true`, no `clean_video` key. Writing it is
best-effort: a failure is logged and the clip (which already contains the
cursor) is still delivered. It moves and is deleted with the clip.

### Removed

`src/record/cursor_render.rs` render path (`Job`, `render`, `pipe`,
`encode_args`, `probe_color`, `enlarge_pipe`, libx264 retry), `finalize.rs`
`render_cursor`, `render_combined` and the cursor branches around them, the
YUV sprite/blend code and `trajectory`/`exposure` in `cursor.rs`,
`clean_path` and all `X.clean.mp4` handling (`move_sidecars` rewrite, shelf
dismissal), the ignored synthetic render test and the save-time
`live_smooth_cursor_recording` render step. What remains of `cursor_render.rs`
(track loading, ffprobe, sidecar JSON, moving the JSON) becomes
`src/record/cursor_sidecar.rs`.

### Failure behaviour

| Situation | Result |
| --- | --- |
| Plugin .so missing | start fails with a message naming the file |
| No EXT cursor session | start fails (tracker check, unchanged) |
| Bad config / GLX context | `gsr_plugin_init` fails, gsr exits, start fails |
| Shader/GL setup fails | plugin logs and exits gsr on the first frame |
| Pipe full | tracker drops lines (cursor may lag), recording continues |
| Tracker ends early | plugin keeps the last state; `cursor.json` covers what was seen |
| `cursor.json` write fails | logged, clip delivered |

### Packaging

- New crate `src/platform/linux/gsr_cursor/` (`cdylib`, own `Cargo.toml` and
  lockfile, Linux-only dependencies in a target section: `libc`, `tiny-skia`).
- CI: a job for fmt, clippy, test and build of the crate.
- `flake.nix`: build the plugin and install it to `$out/lib/boltsnap/`.
- README: build/install step, Smooth cursor section, config comment, replay note.

## 3. Tests and acceptance

Unit tests (no GPU):
- config round trip daemon -> plugin, and rejection of bad values;
- mapping for output, region and Combined, with scale 1.5;
- feed protocol round trip and merge-style visibility across sources;
- `Motion` equals the old per-ms spring (existing spring tests ported), taps at
  rest coincide, moving taps spread, appearing fades, first frame not faded;
- plugin init/deinit through the exported C functions without any GL context;
- gsr argv with and without plugin; plugin lookup; `cursor.json` without
  `clean_video`; sidecar moves.

GL check (plugin crate, ignored by default, run locally): surfaceless EGL
(GLES 3) with a 64x64 RGBA8 FBO filled opaque grey, run the real `draw`; read
back and assert the arrow position/orientation, that alpha stays 1, and that a
two-position exposure averages to `frame*(1-c) + colour*c`.

Done gates: `cargo fmt --check`, `nice -n 19 cargo test --locked -j 4`,
`cargo clippy --locked --all-targets` with no new warnings, the same for the
plugin crate, `git grep` shows no save-time render code.

Live (only after the user approves): one short recording per scope with mellow,
check cursor visibility, smoothness and placement; confirm saving takes about as
long as a normal recording.

## 4. Implementation plan (file by file)

Commit A, "Draw the smooth cursor live through a gsr plugin" (one capability:
replace the renderer):

1. `src/record/cursor_motion.rs` (new, portable): move `Preset`, `MELLOW`,
   `QUICK`, `Sample`, `Spring`, `SHAKE_PX`, `SHUTTER_MS`, `BLUR_SAMPLES`,
   `arrow`/`Arrow` from `cursor.rs`; add `preset(name)`, `cursor_size()`,
   `Motion`, `Mapping` (+ `apply`), feed `line`/`parse_line`, `PluginConfig`
   (`to_env`/`parse`), `ENV`. Tests: spring ports, taps, feed, config.
2. `src/record/cursor.rs`: keep track format, `timeline`, `merge`,
   `sidecar_json` (drop `clean_video` parameter), `sidecar_path`; re-export
   `Sample`/`Mapping` from `cursor_motion`; drop `clean_path`, `trajectory`,
   `smooth`, `exposure`, `Sprite`, `yuv420_len`, `blend_*`, `frame_ms`.
3. `src/record/cursor_render.rs` -> `src/record/cursor_sidecar.rs`: keep
   `load`, `remove_segment_files`, first-frame parsing, `probe`,
   `packet_duration`, `to_pixels`, `base64`, `write_atomic`; add
   `write(clip, samples, size, scale, mode)`; `move_sidecars` only moves the
   JSON. Drop everything else and the render tests.
4. `src/record.rs`: module list.
5. `src/record/finalize.rs`: tracks still loaded before concat (errors logged,
   not fatal); Combined always `compose_outputs`; after that write
   `cursor.json` per clip with `cursor_frame` + ffprobe width; delete
   `render_cursor`, `render_combined`; keep `cursor_frame`.
6. `src/platform/linux/cursor_track.rs`: `pipes(n)`, `start(source, index,
   track: Option<PathBuf>, feeds)` returning the output's logical rect, writes
   to file and pipes; `plugin_path()` lookup.
7. `src/platform/linux/gsr.rs`: `args(..., plugin: Option<&Path>)`; tests.
8. `src/record/session.rs`: `spawn_segment(..., cursor: RecordCursor, ...)`,
   `spawn_segment_with` builds a `Command` (env + inherited fd) and hands it to
   the spawn closure; update tests and the ignored live test (no render step).
9. `src/platform/linux/shelf/mod.rs`: pass the mode; discard cleanup unchanged.
10. `src/shelf/model.rs`: drop the clean video removal.
11. `src/platform/linux/gsr_cursor/` (new crate): `Cargo.toml`, `Cargo.lock`,
    `src/lib.rs` (FFI, config, feed reader, plugin state), `src/gl.rs` (loader,
    shader, draw), GL check test.
12. `.github/workflows/ci.yml`, `flake.nix`, `README.md`, this plan, the
    2026-09-23 plan's Phase 4 note.

Commit B, "Smooth cursor in the replay buffer": `replay/mod.rs`
(`start_capture`, `Session.cursor`), README replay note, `docs/replay.md`.

## 5. Implementation notes

- Baseline before the change: `cargo test --locked` 271 + 22 passed, 4 ignored;
  clippy 32 warnings. After: 273 + 22 passed (two new tracker tests), 2 ignored (the synthetic render
  test and the arrow preview are gone); clippy 31 (no new ones). Plugin crate:
  14 tests plus the GL check, clippy clean.
- GL check (`cargo test -- --ignored` in the plugin crate) passes locally on
  Mesa's surfaceless EGL (GLES 3): the real shader compiles, opaque arrow pixels
  land unflipped at the mapped position, transparent ones leave the frame, the
  frame's alpha stays 255 everywhere, and 4 + 3 half-alpha taps give
  `grey*(1-c) + white*c` (not repeated overdraw). CI runs it on llvmpipe.
- The release `.so` exports only `gsr_plugin_init`/`gsr_plugin_deinit`, links
  no GL library, and loads through the C ABI (ctypes): init succeeds with a
  valid `BOLTSNAP_CURSOR`, fails with a message for a bad preset or GLX.
- Track file errors no longer stop the tracker; they only cost `cursor.json`.
- `FinalizeRequest.fps` and the xstack canvas size were only needed by the
  renderer and are gone.
- Not verified yet (needs a real recording, only with the user's approval):
  that gsr loads the plugin and draws at the right place live, the row
  orientation inside gsr (the GL check assumes row 0 = top, as gsr's encoder
  copy and `hello_triangle` imply), and scale != 1: libway documents cursor
  positions as output-buffer pixels while the Hyprland source sends logical
  ones; the mapping follows the handoff (logical), verified only at scale 1.
- `flake.nix` was not built locally (no Nix on this machine).
- Replay: `smooth_cursor()` in `replay/mod.rs` adds `-cursor no -p <plugin>`,
  the env and the inherited feed to the capture command; the tracker lives in
  `Session.cursor`. Not live-tested (starting replay records the desktop).
