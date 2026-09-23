# Optional cursor smoothing: implementation plan

Status: implementation started. Separate cursor capture is implemented and tested;
GPU composition/encoder integration and the user-facing toggles remain unfinished.
User requirement: an explicit toggle, default off, with planning before code.
Scope: Linux recording and replay. Screenshots are unaffected. Windows remains
frozen. Existing libway and replay optimizations are independent of this feature.

## Product behavior

Two independently persisted checkboxes use the same implementation:

- Recording settings: **Smooth cursor** (`record_cursor_smoothing = false`).
- Replay settings: **Smooth cursor** (`[replay].cursor_smoothing = false`).

Separate preferences avoid enabling a continuous background cost just because
smoothing was selected for one ordinary recording. Both default to false when
absent. Explicit non-boolean values fail validation; they are not treated as true
or silently coerced. Existing config keys survive writes.

A setting is copied into the immutable session configuration at start. Pause and
resume keep that session's setting. Changes while recording affect the next
recording. For running replay, show **Applies after replay restart**; do not
restart automatically, discard buffered history or rewrite old clips. Status
reports requested preference and effective running mode separately.

The disabled path continues to use the existing recorder adapters and does not
open a cursor session, allocate smoothing buffers or introduce lookahead. If the
user explicitly requests smoothing but the selected capture target cannot support
it, report why before starting. Do not silently record without the requested
effect, drop audio, lower FPS, switch codecs or fall back to CPU encoding.

The UI is wired last, after a functional backend exists. Unsupported/unknown
capability is visibly unavailable with a short reason. Capability discovery is
asynchronous and tied to the actual output/seat/device/encoder combination;
registry presence alone is not proof. A saved preference survives temporary
unavailability and is rechecked at start.

## Current code and boundaries

| Area | Existing entry points | Planned responsibility |
| --- | --- | --- |
| Preferences | `src/config.rs`, `src/replay/settings.rs` | Strict default-off settings; preserve unknown config; distinguish desired/effective state |
| Linux UI | `src/platform/linux/tray.rs`, `src/platform/linux/shelf/mod.rs` | Checkboxes, asynchronous capability state, persistence/error reconciliation |
| Recording session | `src/record/session.rs` and Linux shelf start/resume callers | Snapshot option per session and select an OS backend through a narrow platform API |
| Replay supervisor | `src/platform/linux/replay/mod.rs` | Select legacy/native producer, bounded startup, status and child ownership |
| Capture library | `../libway/src`, synchronized `vendor/libway` | Cursor capture protocol and owned metadata/images; no encoding or smoothing policy |
| Media worker | `src/platform/linux/replay/worker/` | Native capture adapter, cursor resampling/composition, encoder/audio timing and output transport |
| Portable policy | A small module under `src/record/`, shared with the worker by the existing module pattern | Sample interpolation, discontinuity handling and timeline arithmetic; no native types |

Inspect all start, resume, stop, freeze, save and preference-write callers before
editing. Existing Unix-specific recording launch code is already in a shared
path; extract only the launcher capability that this work actually changes into
`src/platform/linux/`. Do not refactor unrelated recording policy or Windows.
FFmpeg/GPU dependencies remain in the separate Linux worker, not the main binary
or libway's default CPU feature graph.

## Chosen architecture and feasibility gate

The current adapters deliver already composited cursor pixels. Relocating them
cannot restore the obscured background. Smoothing therefore needs a native
cursor-free capture source and composition **before** encoding. This preserves
remux-only fullscreen replay export; cursor motion is fixed when recorded.
Post-export editable cursor tracks are outside this feature.

Use one optional native producer shared by ordinary recording and replay, hosted
in the existing standalone worker rather than a second recorder implementation.
For replay it feeds the existing bounded encoded-packet ring/transport contract.
For ordinary recording it writes the existing segment/container contract and
participates in current pause/finalize/cache safeguards.

First target the locally available Vulkan encoder path. The installed FFmpeg
8.1.2 build has Vulkan/libdrm/libpulse enabled and NVENC/VA-API disabled. Do not
select NVENC merely because the machine has an NVIDIA GPU. Device capabilities,
formats and synchronization must be proven with synthetic frames before choosing
a production import/composition implementation.

The feasibility spike must demonstrate DMA-BUF import, a movable alpha cursor,
color conversion, hardware encode and decoded pixel correctness at native
resolution. Prefer supported FFmpeg hardware-frame/filter APIs. Verify runtime
cursor positioning and device/queue interoperability explicitly; do not assume a
static overlay filter supports efficient per-frame movement. If those public
APIs cannot implement it, document that result before introducing a small isolated
GPU compositor. Do not build against FFmpeg private structs or add an FFmpeg/GSR
fork as an implicit dependency. No high-throughput/zero-copy claim follows just
from successful GBM export/import.

This is a dependency gate, not a user approval gate: a failed spike means the
backend remains unavailable while the technical approach is revised. A UI-only
switch or unconnected motion helper does not complete the feature.

## Cursor and video acquisition

Implement `ext_image_copy_capture_manager_v1.create_pointer_cursor_session` and
`wl_seat`/`wl_pointer` lifecycle in libway. Expose connection-independent owned
metadata: visibility, source/seat generation, position, image, hotspot and sample
receipt time with an explicit clock basis. Keep proxy types internal.

- Main frames request no painted cursor. The cursor session supplies its image.
- Publish a changed hotspot together with the cursor image's `ready`, never at
  hotspot-event receipt alone. Retain the previous image until the new one is ready.
- Preserve negative/partially off-output coordinates and clip at composition.
- Map buffer transform, output scale, crop and hotspot in one defined coordinate
  chain. Keep floating-point positions until sampling the GPU texture.
- Reset motion history on leave/enter, source/seat replacement, transform/scale
  changes and discontinuities. Bound cursor dimensions, image bytes, sample count
  and outstanding buffers independently of full-screen limits.
- Cursor position events have no source timestamp. Record monotonic receipt time;
  do not pretend it is synchronized hardware input time. Measure resulting skew.
- WLR alone does not provide this separate cursor session. No global input hook,
  fabricated cursor bitmap or compositor polling loop is substituted silently.

libway currently has one pending capture state per connection and blocking
`Stream::next_frame`. Preserve that screenshot API. A dedicated bounded cursor
connection/event loop is the initial implementation, matched to the selected
output and seat. Video capture runs separately from the encoder cadence. This
lets cursor motion continue over a static desktop while EXT waits for new video
content. Match output identity again after discovery and reject ambiguous mappings;
name matching alone is insufficient across reconnects.

## Motion and timing policy

First version: fixed-lag linear interpolation only, with an initial 8 ms maximum
lookahead to be validated. No default extrapolation and no hidden strength slider.
The toggle changes the recorded cursor, not the user's actual pointer/input.

Use a monotonic output timeline derived from session origin and integer frame
index/FPS; do not repeatedly sleep one rounded frame period. For output time T,
select cursor observations bracketing T and interpolate only within that interval.
Allow bounded waiting until T plus lookahead; never interpolate across a visibility
change, warp, long gap or clock discontinuity. If no valid bracket arrives, hold
the newest valid position at or before T. On leave, hide the cursor immediately
at its event boundary. The stationary case settles without an oscillating tail.

Receipt timestamps are imperfect, so interpolation is not guaranteed to improve
already regular, high-rate samples. Synthetic and real tests must show whether
the chosen lookahead reduces visible stepping without unacceptable cursor/content
misalignment. If a motion filter is needed beyond resampling, evaluate it against
the same delay/overshoot tests before adding it; do not quietly change semantics.

Extrapolation is deferred, rather than bundled into the toggle. A later opt-in
variant would need a short hard prediction horizon, bounded speed and displacement,
and reset on reversals/stops. No unbounded continuation of the last velocity.

Wall-clock buffering must not become an A/V timestamp offset. Video and audio
keep their common capture timeline and are released with corresponding bounded
lookahead. Timestamp audio from the audio source with its measured latency;
do not assign PTS solely from packet arrival. Test drift, startup and pause/resume.
At a static desktop, reuse the latest completed background and composite the
new cursor at output cadence. Never reuse a buffer still accessed by capture,
composition or encoding.

## GPU ownership, pressure and failure

Keep capture, composition and encoder lifetimes explicit with owned frame leases
and completion fences. Extend libway reuse only after a consumer-completion
contract exists; dropping a Wayland frame object is not GPU completion.

Negotiate a bounded pool/queue against the configured byte budget and encoder
surface requirements. Start by measuring four surfaces per output, with a hard
negotiated maximum rather than allocating more under load. Driver allocations
are additional to the replay packet budget and must be documented separately.

Bound cursor history initially to 256 samples/125 ms; prune by both count and age.
Overflow creates a discontinuity rather than allowing interpolation across missing
history. Keep only a bounded set of cursor image generations. Bound video queue
age as well as length; obsolete pending source frames may be skipped before
encoding, but already encoded interdependent packets must not be discarded
arbitrarily. Preserve output timestamps and expose dropped/repeated-frame counts.

Measure frame age, capture/composition/encode latency, queue depth and maximum
retained surfaces using aggregated counters, not per-frame disk logs. Gate results
on matching resolution/FPS/codec/quality/audio and active versus static content.
Do not trade away image quality or FPS to make a benchmark look faster.

On output/device removal or capability loss, stop cleanly and preserve recoverable
segments. Do not continue with a missing/double cursor or restart the legacy
recorder midway through a file. Stop/cancellation drains or cancels bounded work,
releases FDs/leases and reaps only children owned by this session.

## Implementation order

1. **Feasibility and fixtures:** headless cursor/video server, synthetic GPU import,
   movable cursor composition and hardware encoding proof; record constraints.
2. **libway cursor API:** protocol state machine, owned images/metadata, dedicated
   connection lifecycle; fake-server tests including cursor-only motion and removal.
3. **Timing policy:** portable interpolation and scheduler with deterministic clock
   tests, bounded histories and discontinuity rules; CPU reference compositor for
   pixel comparison, not an automatic production fallback.
4. **Native worker:** cursor-free capture, GPU composition and encoder integration,
   audio clock, pool/fences, stop and backpressure. Prove pixel and A/V correctness.
5. **Recording/replay integration:** backend selection, immutable session option,
   segment lifecycle, encoded-ring ingestion and status. Preserve existing modes;
   report unsupported multi-output/crop combinations explicitly until tested.
6. **Config and UI:** strict default-off fields, persisted checkboxes, effective
   state and restart notice. Enable only proven target/backend combinations.
7. **Validation and documentation:** end-to-end synthetic tests, feature matrix,
   performance comparisons, then deliberate live compositor checks. Complete
   only after the toggle changes the actual recorded pixels and off preserves
   existing recording behavior.

## Acceptance checks

- Missing/false settings exercise the legacy path with zero smoothing resources;
  true settings select the proven backend. Persistence failures reconcile the UI.
- Test checkbox and config changes before start, during recording, while paused,
  during replay/export and after restart; previously buffered clips remain intact.
- Deterministic movement fixtures: constant velocity, jittered delivery, stopping,
  reversal, warp, gaps, visibility boundaries, animated cursor/hotspot changes,
  every transform, fractional DPI, negative origins and crop edges.
- Validate exactly one cursor, correct hotspot/alpha, preserved background,
  static-desktop animation, and expected output timestamps. Use decoded reference
  pixels with codec-appropriate tolerances; use lossless GPU readback for exact
  composition tests. Readback is test-only.
- Exercise resource exhaustion, failed imports, encoder backpressure, disconnect,
  source changes, cancellation and repeated starts/stops; no unbounded queues,
  accidental CPU fallback or buffer reuse before completion.
- A/V impulses and long drift tests, pause/resume and gap-free segment handling.
- Main fmt/test/Linux check; worker fmt/unit tests and `probe.py`/`live.py`; libway
  minimal/default/GPU tests, strict library lints, snapshot synchronization; soak
  with concurrent repeated freeze/export and measured RSS plateau.
- Compare off/on at 1080p and 4K, 60/120/240 FPS where supported. Record actual
  delivered FPS, p50/p95/p99 latency, drops/repeats, CPU/GPU load and memory.
  Synthetic results alone do not establish smoothness under game load.

No live capture, user-config change, recorder replacement or restart is required
for planning. Hardware/compositor support is a measured matrix, not a blanket
Wayland or NVIDIA/AMD/Intel claim.

## Primary references

- [Wayland EXT capture and cursor protocol](https://raw.githubusercontent.com/wayland-mirror/wayland-protocols/main/staging/ext-image-copy-capture/ext-image-copy-capture-v1.xml): separate cursor session, cursor-free capture, position and hotspot semantics.
- [FFmpeg 8.1.2 Vulkan hardware context](https://raw.githubusercontent.com/FFmpeg/FFmpeg/n8.1.2/libavutil/hwcontext_vulkan.c): reference for the import/device feasibility investigation, not proof of compatibility with the exported allocation.
- Local source: libway capture state/stream ownership, Boltsnap preference/session
  callers, replay supervisor and worker. [Previous performance work](../recording-smoothing.md).

## Implementation checkpoint, 2026-09-22

The optional ordinary-recording path is implemented and headlessly tested:
libway EXT video/cursor capture, portable bounded interpolation, EGL/Vulkan
composition, explicit libx264 encoding, audio muxing, default-off persisted tray
toggle and session-scoped selection. See [implementation and limitations](../recording-smoothing.md#optional-cursor-smoothing-experimental).

The successful GPU bridge imports FFmpeg's Vulkan allocation into GL through
OPAQUE_FD external memory, with binary external semaphore and timeline handoffs.
This supersedes the failed direct GBM-to-Vulkan import experiment. GPU image
pixels and decoded cursor positions pass synthetic regression tests.

This is a limited experimental implementation, not completion of every acceptance
item above. Hardware encoder validation remains blocked in the local FFmpeg
baseline. Replay, area/multiple-output capture and rotated displays are rejected.
The initial recording profile is explicit libx264 at 60 FPS; 240 FPS is rejected
rather than silently reduced. Live compositor A/B performance, long audio drift,
animated cursor/DPI coverage and real pause/resume remain to be validated.
