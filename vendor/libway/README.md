# libway

A small Rust library for Linux Wayland clients. The first capability is capture:
Boltsnap screenshots today, owned frames for future recording and clipping.
Rust 1.88 or newer. MIT licensed. This is an unpublished 0.1 API.

## Features

- Dedicated connection, output discovery and runtime capture capabilities.
- EXT image-copy-capture with output capture sources, or WLR screencopy v1–v3.
- CPU frames backed by sealed-size SHM, with checked packed-pixel conversion.
- Optional GBM DMA-BUF frames with owned plane descriptors and explicit
  format/modifier negotiation. No EGL context or GPU readback dependency.
- Output capture, upright images, logical regions and mixed-scale desktop images.
- Persistent EXT sessions, presentation timestamps, damage, deadlines and cancellation.

`default-features = false` builds the protocol/CPU frame core. The default `image`
feature adds `image::RgbaImage`, orientation and composition. `gpu` adds GBM and
requires the system GBM library and headers (for example `libgbm-dev`). The CPU
build has no GBM dependency. PNG support is only enabled for examples/tests;
applications choose their own image codecs.

## CPU capture

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

## Layout and development

- `capture.rs`: connection pump, sequencing, deadlines, stream/resource ownership.
- `capture/events.rs`: protocol event handlers and bounded constraint batches.
- `buffer.rs`: SHM lifecycle, owned frames, packed format conversion.
- `gpu.rs`: optional GBM allocation and DMA-BUF plane ownership.
- `compose.rs`: image orientation, scaling and desktop/region composition.
- `types.rs`, `error.rs`: portable capture data and errors.
- `tests/support`: isolated Wayland server with fault injection.

The initial research is in `RESEARCH.md` in the standalone project. The checked
results and remaining compositor tests are in `VALIDATION.md`.

```sh
cargo fmt --check
cargo test --locked
cargo test --locked --no-default-features
cargo test --locked --all-features
cargo clippy --locked --all-features --all-targets -- -D warnings
```

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

See [VALIDATION.md](VALIDATION.md) for regression coverage, local comparison with
libwayshot 0.7.3 and the limits of synthetic measurements. The standalone
`benchmarks/` crate keeps comparison dependencies out of library consumers.
