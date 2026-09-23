# libway

A small Rust library for Linux Wayland clients. Capabilities: capture (Boltsnap
screenshots today, owned frames for future recording and clipping) and
drag-and-drop over `wl_data_device`, including a guest mode for winit.
Rust 1.88 or newer. MIT licensed. This is an unpublished 0.1 API.

## Features

- Dedicated connection, output discovery and runtime capture capabilities.
- EXT image-copy-capture with output capture sources, or WLR screencopy v1–v3.
- CPU frames backed by sealed-size SHM, with checked packed-pixel conversion.
- Optional GBM DMA-BUF frames with owned plane descriptors and explicit
  format/modifier negotiation. No EGL context or GPU readback dependency.
- Output capture, upright images, logical regions and mixed-scale desktop images.
- Persistent EXT sessions, presentation timestamps, damage, deadlines and cancellation.

`default-features = false` builds shared types and the `Display`/`SurfaceHandle` connection layer; enable `capture` for the
protocol/CPU frame core, `dnd` for drag-and-drop, and `foreign-display` to attach to a
`wl_display` owned by another library such as winit. The default `image`
feature adds `image::RgbaImage`, orientation and composition. `gpu` adds GBM and
requires the system GBM library and headers (for example `libgbm-dev`). The CPU
build has no GBM dependency. PNG support is only enabled for examples/tests;
applications choose their own image codecs.

## CPU capture

This snippet is compiled as a `no_run` doctest on
[`Connection::capture_desktop`](src/compose.rs).

```rust,no_run
use libway::{CaptureOptions, Connection};
# fn main() -> libway::Result<()> {
let options = CaptureOptions::default();
let mut connection = Connection::connect(&options)?;
let desktop = connection.capture_desktop(&options)?;
println!("{}x{}", desktop.image.width(), desktop.image.height());
# Ok(())
# }
```

`Connection::connect` uses `WAYLAND_DISPLAY`/`WAYLAND_SOCKET` in the caller's
Wayland environment. `Connection::from_socket` uses a caller-owned Unix socket,
which also makes tests independent of the user's compositor. Connections never
map surfaces, open portals or select a DRM device automatically.

`outputs()` returns connection-specific `OutputId` handles, logical rectangles,
current untransformed mode sizes and transforms. Handles from another connection
or removed outputs fail. `capabilities()` reflects globals already dispatched;
call `outputs()` to synchronize discovery before inspecting it.

`capture(id, &options)` returns an owned CPU `Frame`. `rgba8()` copies raw pixels
to tightly packed, premultiplied RGBA8 in buffer orientation. `to_image()` also
undoes Y inversion and the compositor transform. Alpha remains premultiplied.
ARGB/XRGB and ABGR/XBGR 8888/2101010 plus RGB/BGR888 are supported. Ten-bit
conversion reduces channel precision; it is **not HDR tone mapping**. Neither
capture protocol provides sufficient color metadata to promise a color-managed
result.

`capture_region(Rect, &options)` uses compositor-global logical coordinates;
negative origins are valid. Composition uses the highest output scale, an opaque
black background for gaps, Gaussian resizing, and integer truncation at pixel
boundaries. Outputs are captured sequentially, not as an atomic multi-monitor
snapshot. Layout changes abort capture instead of returning mismatched geometry.

## Separate cursor capture

`connection.cursor_stream(output_id, options)` creates an EXT pointer-cursor
session. Use a dedicated connection/thread so waiting for a cursor image never
blocks desktop video. Ordinary screenshots do not bind a seat or open this session.
The initial API requires exactly one advertised seat with a pointer; WLR-only and
ambiguous multi-seat setups fail explicitly.

`stream.poll_event(timeout)` returns ordered `CursorEvent::{Enter, Leave,
Position, Image}` observations. An idle timeout returns `Ok(None)` and retains an
in-flight image request. Supply a positive timeout; zero does not dispatch events.
Cancellation and other errors close the session; dropping it releases pending
buffers and protocol resources. Returned image frames remain owned by the caller.

Positions use transformed output-buffer pixels. Image pixels and hotspots use
raw cursor-buffer coordinates; consumers must account for the frame transform.
Images preserve premultiplied alpha. A hotspot is paired with the image whose
`ready` event publishes it, including when another hotspot arrives in the same
Wayland dispatch. `generation` starts at one per session. `received_at` is local
monotonic receipt time, **not** an input-device timestamp. Visibility events and
positions are delivered even while a cursor image request is idle.

Cursor images use SHM with at most 1024 pixels per axis and honor stricter caller
allocation limits. Metadata is bounded to 256 queued observations; overflow is
an explicit error, not silent motion loss. This is a capture primitive, not a
smoothing implementation or an end-to-end recording backend.

## Frames for recording

This loop is compiled as a `no_run` doctest on [`Connection::stream`](src/capture.rs);
[`examples/frames.rs`](examples/frames.rs) is the executable version.

```rust,no_run
use libway::{BufferKind, CaptureOptions, Connection};
# fn main() -> libway::Result<()> {
let options = CaptureOptions::default();
let mut connection = Connection::connect(&options)?;
let output = connection.outputs(&options)?.into_iter().next().ok_or(libway::Error::NoOutputs)?;
let mut stream = connection.stream(output.id, options, BufferKind::Cpu)?;
for _ in 0..3 {
    let frame = stream.next_frame()?;
    // Send frame.storage to your consumer and retain it until consumption ends.
    println!("{:?}", frame.presentation_time);
}
# Ok(())
# }
```

One stream exclusively borrows its connection. Only one protocol frame is active
at a time; the library has no background capture thread or unbounded queue. EXT
sessions persist, while WLR creates one request per frame. EXT can wait for new
content indefinitely after the first frame; the configured timeout bounds that
wait. A timeout, cancellation or other error closes the stream. Start a new
stream to retry. `Cancellation` is cloneable and can be triggered from another
thread. Polling checks it at intervals of at most 20 ms, subject to scheduling.

Each returned frame owns a fresh allocation, so holding an older frame cannot
make it change under a consumer. The library keeps no frame history. This is a
correctness-first foundation, not a measured high-FPS recorder: no buffer pool,
encoder, audio clock, replay ring, PipeWire connection or GPU-to-CPU conversion
is included. Consumers must bound their own retained frames and encode queues.

## GPU capture

Enable `features = ["gpu"]`, open a render node explicitly using
`gpu::GpuAllocator::open`, then use
`capture_with(id, &options, BufferKind::Gpu(allocator))` or `stream`.

The library intersects capture formats/modifiers with linux-dmabuf v3
advertisements, verifies the EXT source device, allocates with GBM and waits for
the asynchronous Wayland import result. INVALID (implicit layout) is used only
when advertised. A rejected import is an error, never an empty successful frame.
GPU selection never silently falls back to a CPU buffer. `Backend::Auto` may
switch EXT to WLR on an unsupported format/capability; forced backends do not.

`FrameStorage::Gpu` exposes format, dimensions, modifier, plane offsets/strides
and borrowed FDs. To import asynchronously, keep the frame alive until the
consumer GPU/encoder finishes, or duplicate descriptors into your own owned
import object. Frames retain the allocation and device after the allocator or
capture connection is dropped. GPU objects are deliberately thread-local (`Rc`);
an encoder integration should design its ownership boundary explicitly.

`ready` signals capture completion; `wl_buffer.release` is not the completion
mechanism for these protocols. This implementation follows their implicit
DMA-BUF synchronization contract. It does not offer explicit sync fences.

## Bounds and errors

The default operation timeout is 5 seconds. Setup and protocol waits are bounded;
CPU conversion/resizing and driver calls are synchronous and cannot be preempted
inside a call. Cancellation is checked between protocol operations. The desktop
helper carries one deadline through its constituent captures and final check.

Defaults allow 32 outputs, 64 million pixels and 768 MiB of payload/scratch.
Dimensions, stride, multiplication, event-list sizes and SHM protocol integer
limits are checked before allocation. Composition conservatively budgets image
and resize scratch. DMA-BUF accounting estimates plane extents; driver-internal
allocations, retained caller frames and allocator overhead are not a process RSS
limit. Allocation failures return errors where Rust/dependency APIs permit;
`image` and GPU drivers may still use infallible internal allocations.

Use typed `Error` variants for recovery. Capability/format absence, timeouts,
cancellation, invalid dimensions, removed outputs, changed layouts and stopped
sessions are distinct. Portal consent and fallback policy belong to the caller.

## Drag and drop

For DnD without capture, image or GPU code:

```toml
[dependencies]
libway = { path = "../libway", default-features = false, features = ["dnd"] }
```

A `Dnd` session binds its own registry, seats and data devices on a `Display`.
Pass surfaces from that same connection and register targets in surface-local
logical coordinates. The compiled [module example](src/dnd/mod.rs) shows a shared
wayland-rs connection, reader wakeups and completion for a copy-only consumer.
Safe handles retain weak connection provenance; foreign or destroyed handles
are rejected before registering a target or starting a drag.

`DndEvent::Ready` arrives once the compositor knows the session's data devices (and
pointers, with `track_input`). Hover events are coalesced per seat; `target` is
`None` when the topmost enabled target under the pointer accepts no offered type.
A drop emits `Leave` at once and later `Dropped` or `TransferFailed`; a new drag
never aborts a transfer that is still reading.

`Payload::Files` are lossless local paths parsed from `text/uri-list` (RFC 8089
`file:` URIs with empty, `localhost` or local-hostname authority; malformed lists
fail that representation). `Payload::Text` accepts `text/plain;charset=utf-8` and the X11 aliases
`UTF8_STRING`, `TEXT` and Latin-1 `STRING` that XWayland bridges expose;
`Accept::Mime` yields raw bytes. When the preferred type fails to decode, the same offer is
read again in the next accepted type: a browser link arrives as a `text/uri-list` of `https`
URIs, which is not a file list, and falls back to its `text/plain`. Defaults are
1 MiB per incoming representation, 1024 file entries, 64 retained MIME announcements,
4 unanswered drops, 5 s without progress and 15 s total across MIME fallbacks.
Payload/entry/transfer failures produce `TransferFailed`, never partial payloads.
Only the first 64 incoming MIME announcements are stored; later announcements
are ignored, so a supported type outside that prefix is unavailable.

Call `complete` once after processing a delivered drop. For v3 Copy/Move, the
accepted action must match negotiation; for Ask, select Copy or Move offered by
the source. None cannot finish a v3 drop. On v1/v2, None/Copy/Move acknowledge
locally without `finish`. Ask is never a final answer. Invalid answers return
`InvalidInput` and leave the drop available for correction or rejection.
Successful Move completion can authorize the source to delete its originals.

Outgoing drags describe intent: `start_drag(DragRequest { data: DragData::Files(..),
.. })`. The serial must come from the button press or touch down that began the
implicit grab; with `DndOptions::track_input` libway records it from its own
`wl_pointer`/`wl_touch` and keeps it only while a button or touch point is down, so a stale
serial is never sent; otherwise pass `serial: Some(..)`. With a reader thread, call
`dnd.dispatch()` right before `start_drag` to take in a press the reader has read but not yet
dispatched. Files also go out as
plain text and text also as `UTF8_STRING`, `TEXT` and `STRING`, so XDND targets
behind a bridge that only take text still get something. `DragData::Lazy` produces
bytes per requested announced type. `DragData::FilesAndText` serves a `text/uri-list` to file
targets and the given text verbatim to text targets. Sends are written without blocking and finish
even after the drag ended, within their transfer deadlines. A Rust `main` already ignores `SIGPIPE`; C hosts must do
the same before dragging.

Outgoing MIME names must be nonempty, NUL-free and at most 1024 UTF-8 bytes;
announcements are bounded by `max_mime_types`, including convenience types.
X11 aliases remain supported and outgoing payload size is not capped by
`max_payload`. Icons require dimensions 1..=1024 and a hotspot inside the image.
Validation happens before replacing an existing drag or consuming its serial.

Migration from the reviewed snapshot: contradictory completion actions, zero or
unrepresentable transfer deadlines, invalid MIME/icon inputs and foreign safe
handles now return errors. `run(Duration::ZERO)` performs at most one nonblocking
read cycle; `wait_ready(Duration::ZERO)` only checks current readiness.
`TransferError` now implements Display and the standard Error trait, converts
into `libway::Error`, and is exposed through `source()`.

### Attaching to winit

`features = ["foreign-display"]` links `wayland-client/system`, the same
libwayland winit uses, and unlocks `Display::from_raw` and `SurfaceHandle::from_raw`
from `raw-window-handle`. winit's Wayland loop skips `about_to_wait` when only
foreign queues received events, so run libway's reader thread and wake the loop
through an `EventLoopProxy`. The following excerpt is implemented in the compiled
[receiver example](integration/dnd-winit/src/bin/receiver.rs):

```rust,ignore
let display = unsafe { Display::from_raw(wayland_display_handle.display) };
let surface = unsafe { SurfaceHandle::from_raw(wayland_window_handle.surface) }?;
let dnd = Dnd::new(&display, DndOptions::default())?;
let proxy = event_loop.create_proxy();
let reader = dnd.spawn_reader(move || { let _ = proxy.send_event(UserEvent::Wake); })?;
// ApplicationHandler::user_event: for event in dnd.events() { .. }
// ApplicationHandler::exiting: drop(reader); drop(dnd); drop(display);
```

The reader holds a prepared read only while polling and resolves it before anything
else, so winit's own reads never wait on libway longer than one poll wakeup. It
wakes for compositor messages, pipe readiness, pending socket writes and explicit
control changes (no timers unless a transfer has a deadline); the
consumer is woken at most once per `events()` drain. On a connection shared through
`Display::from_connection` with wayland-rs's pure Rust backend, `prepare_read` does not check a
queue for events another thread already read, so an owner blocking in
`EventQueue::blocking_dispatch` or `roundtrip` next to the reader can sleep on them; poll with a
timeout and dispatch again instead. The system backend (`foreign-display`, winit) checks its
queue and is not affected. `Dnd::run` and `wait_ready`
are refused on guest displays, whose owner reads the socket.

XDND: Wayland clients do not speak XDND. The compositor's X window manager bridges
XDND to `wl_data_device`; libway contributes the MIME aliases, the action mapping
(XDND `Link`/`Private` have no Wayland equivalent and are never claimed) and the
harness in `integration/dnd-winit/`, which drives real X11 clients under XWayland.
Which directions a compositor bridges is recorded per compositor in
`VALIDATION.md`, not assumed.

## Layout and development

- `capture.rs`: connection pump, sequencing, deadlines, stream/resource ownership.
- `capture/events.rs`: protocol event handlers and bounded constraint batches.
- `buffer.rs`: SHM lifecycle, owned frames, packed format conversion.
- `gpu.rs`: optional GBM allocation and DMA-BUF plane ownership.
- `compose.rs`: image orientation, scaling and desktop/region composition.
- `types.rs`, `error.rs`: portable capture data and errors.
- `display.rs`: owned, shared or guest connection; surface handles.
- `dnd/`: engine (state machine, targets, negotiation), drag (outgoing drags and
  icons), transfer (pipe reactor), uri (URI lists), interop (XDND aliases), reader
  (cooperative reader thread).
- `tests/support`: isolated Wayland server with fault injection and a scriptable
  data device.
- `integration/dnd-winit`: private-Weston harness with winit, wayland-client, x11rb
  and GTK3 clients (`run.py`).

The initial research is in `RESEARCH.md` in the standalone project. The checked
results and remaining compositor tests are in `VALIDATION.md`.

```sh
./tools/check.sh full
LIBWAY_CARGO=/path/to/rust-1.88/bin/cargo LIBWAY_RUSTC=/path/to/rust-1.88/bin/rustc ./tools/check.sh msrv
```

[`tools/check.sh`](tools/check.sh) runs locked library tests across the feature
matrix. `full` also checks formatting, strict Clippy, Rustdoc and both development
subprojects. Variables select executable paths, not command strings; `RUSTC` is
also honored. The MSRV mode requires an actual Rust 1.88.0 compiler and fails
clearly if another compiler is selected. Neither mode installs toolchains or
launches capture examples, compositors or input helpers.

Prerequisites: Linux with Unix sockets, Cargo/rustc, and development libraries
for GBM and libwayland-client (for example `libgbm-dev` and `libwayland-dev`).
`full` also requires rustfmt and Clippy. Dependencies must be available in Cargo's
cache or downloadable; no system packages are installed by the script.

Tests use socketpairs and synthetic pixels. Hardware tests are opt-in and never
connect to the desktop:

```sh
LIBWAY_TEST_RENDER_NODE=/dev/dri/renderD128 cargo test --locked --all-features -- --include-ignored
```

Explicitly running the following examples **does capture the Wayland session
selected by your environment**:

```sh
cargo run --example snapshot -- capture.png
cargo run --example frames
cargo run --features gpu --example frames -- /dev/dri/renderD128
```

The snapshot example refuses to overwrite an existing file. The frame example
prints metadata for up to three frames, then exits (or reports an idle timeout).

Boltsnap integration can also be tested without a live session by setting
`LIBWAY_TEST_BOLTSNAP` to a separately built executable and running
`cargo test --test consumer -- --ignored`. See `VALIDATION.md` for exact coverage.

## Review and benchmarks

The [2026-09-23 review](docs/reviews/2026-09-23.md) records confirmed issues and
API maintenance gaps. The [implementation plan](docs/plans/2026-09-23-review-fixes.md)
tracks the implemented fixes, regression tests and remaining acceptance criteria.
The current check results and any unverified environments are in `VALIDATION.md`.

See [VALIDATION.md](VALIDATION.md) for regression coverage, local comparison with
libwayshot 0.7.3 and the limits of synthetic measurements. The standalone
`benchmarks/` crate keeps comparison dependencies out of library consumers.
