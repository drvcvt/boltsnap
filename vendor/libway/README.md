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
  format/modifier negotiation. No EGL context.
- Optional fast desktop tiles read back from DMA-BUFs through Vulkan (loaded at
  runtime) or a LINEAR mapping, falling back to SHM with a queryable cause.
- Output capture, upright images, logical regions and mixed-scale desktop images.
- Persistent EXT sessions, presentation timestamps, damage, deadlines and cancellation.

`default-features = false` builds shared types and the `Display`/`SurfaceHandle` connection layer; enable `capture` for the
protocol/CPU frame core, `dnd` for drag-and-drop, `foreign-display` to attach to a
`wl_display` owned by another library such as winit, and `window` for caller-owned
windows (`owned-native-system` implies `window`). The default `image`
feature adds `image::RgbaImage`, orientation and composition. `gpu` adds GBM and
requires the system GBM library and headers (for example `libgbm-dev`). `vulkan`
implies `gpu` and adds `capture_desktop_tiles_fast`; it links no Vulkan library
and loads `libvulkan.so.1` at runtime. The CPU build has no GBM dependency. PNG support is only enabled for examples/tests;
applications choose their own image codecs.

## Caller-owned windows

Everything in this section needs the `window` feature (not a default). The
smallest example is `cargo run --no-default-features --features window --example
window`: one CPU window with a solid fill that prints `configured WxH`, `shown`
and, once the compositor closes it, `closed`. Every window example
(`attention`, `colors`, `layer`, `presentation`, `primary`, `window`) requires
`--features window`.

`window::Host` owns an independent connection and every participating native
queue. Call `pump` on the owner thread, drain owned `WindowEvent` values and
apply application close policy explicitly. It starts no background reader and
invokes no application callback during protocol dispatch. Worker threads use
the bounded send-only `WindowProxy`; retained leases keep native dependencies
alive after logical close or host shutdown. CPU presentation owns bounded SHM
buffers and waits for real release/frame events before reuse.

`Host::present_cpu` damages the whole frame. `Host::present_cpu_damaged` takes the
same complete RGBA frame plus `PixelRect` buffer-pixel regions that changed since
the last `Presented` frame (wrong regions leave stale pixels on screen). Rects are
clamped, empty ones ignored, and a list with none left is `InvalidInput`. More than
16 collapse to their bounding box. Reused slots rewrite only the regions changed
since the frame they hold and are fully rewritten when that is not provable (fresh,
older than the last three frames, or another layout in between). The compositor gets
one `damage_buffer` per rect on `wl_surface` version 4 and later, otherwise whole-
surface damage; the first frame and any resize or scale change damage everything.

`window::Host` negotiates decorations through the optional
`zxdg_decoration_manager_v1`. When it was advertised before discovery completed,
every new window requests server-side mode before its first commit and the decoration
object is destroyed before its toplevel. `Configuration::decorations` is
`Decorations::Server` only after the compositor's decoration configure said
server-side, and `Client` without the manager, after a client-side choice, or before
any decoration configure; a mode change arrives as an ordinary new `Configured`
event. A manager that appears after discovery is not bound, and windows are never
retrofitted. `Configuration` also carries `maximized`, `fullscreen`, `activated` and
`tiled` (`Tiled { left, right, top, bottom }`) from the toplevel state array, and
keeps them through scale-only reconfiguration.

`move_window`, `resize_window` (`ResizeEdge`) and `show_window_menu` take a window
and an `InputSeatId` and use that seat's latest pointer button press on that window.
Without one they return `NotReady`, and a retired window or unknown seat returns
`InvalidInput`; nothing is sent in either case. The press is cleared when the seat
loses its pointer or is removed. `set_maximized`, `set_minimized` and
`set_fullscreen` (no output preference) need no press. All six are requests: the
compositor may ignore them, and maximize and fullscreen are confirmed only by a later
configuration.

`Host::set_cursor_shape(window, CursorShape)` uses the optional
`wp_cursor_shape_manager_v1`, bound whenever it is advertised. Each `wl_pointer` gets
a shape device right after `get_pointer`, destroyed before the pointer is released
(capability loss, seat removal, shutdown). The shape is stored per window until it
closes, applied at once to every seat whose pointer is inside the window using that
seat's latest enter serial, and applied again on every later enter. Until a shape is
set nothing is requested. Without the manager (absent or removed) the call returns
`Unsupported("wp_cursor_shape_v1")`; there is no xcursor fallback. `CursorShape` is
a non-exhaustive mirror of the version 1 shapes.

`Host::request_presentation(window, tag)` asks the optional `wp_presentation` (bound
at version 2 or lower whenever advertised) for feedback on that window's next content
commit: the commit of a `present_cpu` call that returns `Presented`, or the driver
commit after `NativeFrame::stage`. Results that commit nothing keep the tag armed;
arming again before the commit replaces it, and a replaced tag gets no event. Every
consumed tag produces exactly one `WindowEvent::Presentation { window, tag, outcome }`
unless the window closes or the host stops first. `Presented` carries the raw
`clock_id`, the timestamp since that clock's epoch, the predicted refresh (`None` for
0), the MSC sequence and the raw flags; `Discarded` means the update was never shown;
`Unavailable` means the manager was gone at commit time or the timing was malformed
(`tv_nsec` out of range, no clock yet). Timestamps are never guessed, and frame
callbacks stay the redraw gate. At most eight armed and unanswered requests per window,
each holding one slot of the 1024-event cap; more return `LimitExceeded`. Without the
global the call returns `Unsupported("wp_presentation")`. A removed manager is
destroyed and its outstanding feedback still arrives. `sync_output` is not reported.
[`examples/presentation.rs`](examples/presentation.rs) is a live check.

For native GPU interop, enable `owned-native-system`. This selects the system
Wayland transport and exposes a single `NativePresenter` claim implementing
standard native handle traits. Keep that claim alive for the driver surface,
for example through an `Arc`; it retains the connection, globals, role and
surface. It cannot be used after logical retirement. Creating an additional
foreign surface or reader remains the cooperating driver's responsibility.

Begin each GPU attempt with `Host::begin_native_frame(&claim, revision)` before
driver configure or acquire. `NeedsFrame` waits for the preceding real callback;
`Suspended` forbids configure/acquire/present until a new configuration resumes
the window. A `Ready` guard exclusively borrows both host and claim and provides
the validated logical/physical configuration. Configure, acquire, encode and
submit using that snapshot. Then `stage` with the acquired texture's physical
extent, immediately make the driver's sole present/commit call, and consume the
guard with `scheduled`. Libway stages scale, viewport and one typed frame callback;
it never attaches or commits this GPU frame. `scheduled` records the caller's
acknowledgment, not successful display or GPU completion.

Cancel or drop before staging to retry without native requests. Cancel, error
or unwind after staging logically closes the window, since a Wayland frame
callback has no cancellation request. There is no extra commit or fabricated
callback. Keep driver resources alive until safely released. A forgotten guard
blocks pump and window mutation; explicitly close the attempt's own window or
shut down the host to recover. Admission of a separate capability does not service
native events or mutate that staged window. Driver calls must remain outside
connection read reservations.

The synthetic guard fixtures and borrow checks do not establish real GPU WSI
ownership, driver-internal queue/read behavior, physical presentation, recovery
or idle/pacing performance. Those native acceptance gates remain pending.

### Toplevel requests

```rust
host.set_size_limits(window, SizeLimits {
    min_width: 320,
    min_height: 200,
    max_width: 640,
    max_height: 400,
})?;
host.set_idle_inhibit(window, true)?; // while a video plays
host.request_attention(window)?;      // a job finished while unfocused
```

`Host::set_size_limits` replaces a toplevel's logical size limits; 0 on an axis
(the `SizeLimits::default()`) means no limit there. Set them right after
`create_window`, before the first frame. Libway sends only the changed
`set_min_size`/`set_max_size` requests and no commit of its own, so they apply with
the window's next presented frame, CPU or native. The compositor may ignore them
(Hyprland treats 1 or less as unset). `Host::size_limits` reads the stored values
back. Values above `i32::MAX` return `InvalidDimensions`; max below min on an axis
where both are set, a layer, or a retired or foreign window return `InvalidInput`;
nothing is sent on error. Input and opaque regions follow the same rule on
toplevels (no commit, next frame), while on layers they commit at once (see
[Layer surfaces](#layer-surfaces)).

`Host::request_attention` asks the compositor to draw the user's attention to a
toplevel or layer. It sends an `xdg_activation_v1` token without a seat serial
and activates the window with it once the token arrives; the protocol lets
compositors refuse focus for such a token. It never moves keyboard focus on its own;
Hyprland marks the window urgent and focuses it only with `misc:focus_on_activate`.
While a request for the window is still waiting for its token, another call returns
`Ok` and sends nothing. Without the global it returns `Unsupported`.

`Host::set_idle_inhibit(window, true)` keeps the screen from blanking, dimming or
locking while that toplevel or layer is visible; `false` stops it. Both are
idempotent, and the effect ends when the window closes. Enabling without
`zwp_idle_inhibit_manager_v1` returns `Unsupported`; disabling always works.
Retired or foreign windows return `InvalidInput`. None of the three adds a
`WindowEvent`. [`examples/attention.rs`](examples/attention.rs) is a live check.

### Layer surfaces

`Host::create_layer(LayerOptions)` gives a surface the `zwlr_layer_shell_v1` role
instead of `xdg_toplevel`, for overlays, HUDs, panels and selectors. It returns an
ordinary `WindowId`: configurations, `enable_cpu`/`present_cpu`, `claim_native` and
its raw handles, presentation feedback, input and `close` work unchanged. Wait for
`Configured` before presenting, as for a toplevel.

```rust
let hud = host.create_layer(LayerOptions {
    namespace: "app.hud".into(),
    output: None, // compositor's choice; or Some(OutputId) from host.outputs()
    state: LayerState {
        layer: Layer::Overlay,
        anchor: Anchor::TOP | Anchor::RIGHT,
        width: 240,
        height: 120,
        margin: [24, 24, 0, 0], // top, right, bottom, left
        exclusive_zone: ExclusiveZone::Respect,
        exclusive_edge: None,
        keyboard: KeyboardInteractivity::None,
    },
})?;
host.set_input_region(hud, InputRegion::None)?; // click-through
```

`LayerState::default()` is a full-output overlay (all anchors, 0x0, `Ignore`). A 0
width or height stretches between both opposite anchors, which must be set.
`ExclusiveZone` maps to the wire values 0 (`Respect`), -1 (`Ignore`) and n
(`Reserve`). Keyboard: `None` (the default) for HUDs, OSDs and panels; `OnDemand`
for prompts and dialogs, which then behave like a window (focused when mapped on
Hyprland, a click elsewhere takes focus and sends `KeyboardObservation::Leave`);
`Exclusive` only for short full-screen selectors with an Escape path, one overlay
per output, because it locks the keyboard (on Hyprland the pointer too) until the
surface goes away. With `output: None` the compositor picks the monitor (Hyprland:
its focused monitor, not necessarily the one under the user's eyes). `Host::set_layer_state` replaces
the state, sends only changed requests and commits once without a buffer, so the
change applies without a new frame; a new size arrives as a configuration. Fields
the bound version lacks return `Unsupported` (`OnDemand` below v4, an exclusive edge
below v5, a later layer change below v2); invalid combinations return
`InvalidInput`; nothing is sent on error. The compositor's `closed` (for example its
output went away) retires the window like `Host::close` and publishes `Closed`.
Toplevel requests (`set_title`, `move_window`, `set_maximized`, ...) on a layer
return `InvalidInput`. There is no hide/show: close and create again, or present a
transparent frame with an empty input region. Layers can carry popups (below).

`Host::set_input_region` (`All`, `None` for click-through, or up to 64 `Rects`) and
`Host::set_opaque_region` work on every window, toplevel or layer, in surface-local
logical coordinates. On a layer they commit once without a buffer, so the change
applies without a new frame; on a toplevel they send no commit and apply with the
next presented frame, like size limits.

`Host::outputs()` lists announced outputs as `OutputInfo { id, name, description }`
(`name` is the connector, such as `DP-3`, from `wl_output` v4). `OutputAdded` and
`OutputRemoved` events follow changes; outputs may be announced after `Ready`.
[`examples/layer.rs`](examples/layer.rs) is a live check.

### Popups

`Host::create_popup(PopupOptions)` places an `xdg_popup` (menu, dropdown,
tooltip) on a toplevel, a layer or another popup of the same host and returns an
ordinary `WindowId`: configurations, CPU and native presentation, input,
regions and `close` work as for any window.

```rust
let menu = host.create_popup(PopupOptions {
    size: (200, 160),
    anchor_rect: LogicalRect { x: click_x, y: click_y, width: 1, height: 1 },
    anchor: PopupAnchor::TopLeft,
    gravity: PopupGravity::BottomRight,
    constraint_adjustment: ConstraintAdjustment::FLIP_X | ConstraintAdjustment::FLIP_Y,
    grab: Some(seat), // None for tooltips
    ..PopupOptions::new(parent)
})?;
```

- **Building options.** Always start from `PopupOptions::new(parent)` and
  override fields, so fields added later do not break the literal.
- **Parent.** The parent must have presented a frame first. Hyprland never
  configures a popup on an unmapped toplevel or popup parent. The anchor
  rectangle must lie inside the parent's current size.
- **Configurations.** A popup's `Configured` carries
  `placement: Some(PopupPlacement { x, y, width, height, repositioned })`
  relative to the parent.
- **Reposition.** `reposition_popup` (xdg_wm_base v3) returns a token, and the
  completing configuration reports it in `placement.repositioned`.
- **Grabs.** `grab: Some(seat)` takes the seat's latest pointer or key press
  anywhere in the parent's popup tree, so menus opened from the keyboard and
  submenus work. A grabbing popup sits on a toplevel, a layer or the topmost
  grabbing popup, with one grab chain per host.
- **Dismissal.** The compositor's dismissal (`popup_done`) arrives as `Closed`.
  `WindowEvent::PopupGrabRefused` comes right before it when a grabbing popup
  is dismissed before its first configuration. Closing any window closes the
  popups above it first, topmost first, with one `Closed` each.
- **Hyprland behavior.** Hyprland 0.56.2 ignores the grab serial and the
  `reactive` hint. A submenu that joins a grab gets no keyboard focus, so route
  keys from any popup of the tree to the topmost one yourself. A click on a
  layer dismisses that layer's grabbing popup. A popup without a grab on a
  layer only gets keys if the layer's keyboard interactivity is not `None`.
  Popups of toplevels are hit-tested by their rectangle, ignoring the input
  region (`ViewHitTester.cpp:264`), so a tooltip there is never click-through.
  Layer popups honour the region.
- **Live check.** [`examples/popup.rs`](examples/popup.rs) runs all of this
  live.

## Window input methods

`window::Host::enable_ime` requires matching keyboard and text-input-v3 focus
for the exact window/seat. Supply current configuration, optional UTF-8
surrounding state (at most 4000 bytes) and a logical candidate rectangle.
`update_ime` publishes complete state; `disable_ime` retires the exact activation.
The existing caller-owned event queue publishes `ImeEvent::Focus`, complete
`Done` transactions and `Ended`. No extra connection, reader, surface commit,
feature flag or input-method service is created.

Commit counts belong to each native protocol object; activation identities remain
unique across its replacement. Mismatched same-activation Done blocks still reach
the app, while only the newest outbound state waits for synchronization. Old
activations cannot acquire current work. Pending strings replace rather than
concatenate. Native payload admission caps commit/preedit at 4096 bytes each;
escaped `ImeText::shared()` Arc clones retain their charge against 128 KiB and
1024 live allocation records until their final drop. This is payload accounting,
not an RSS bound. The pinned default Rust transport additionally limits a complete
wire message to 4096 bytes including headers, so it cannot deliver the full
4096-byte string cap. The approved system transport remains a separate build path.
Malformed data (text over 4096 bytes, NUL, bad preedit cursor, cursor on a null
preedit, or a payload ledger limit hit by the pending batch) drops the whole pending
batch and retires that activation with `ImeEvent::Ended` at the next `Done`; the
host keeps running and `enable_ime` may be called again. Protocol-order violations
(Enter before Leave, a future `Done` count), transport errors and event-queue
exhaustion still stop the host.
Applications own document edits, rendering support and producer arbitration.
Synthetic verification does not establish desktop IME interoperability.

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
boundaries. All outputs are captured on the calling connection with every capture
request in flight together; the compositor may still copy them one after another,
and the result is not an atomic multi-monitor snapshot. One output's failure fails
the whole call. Layout changes abort capture instead of returning mismatched geometry.

`capture_desktop_tiles(&options)` needs only `capture`, not `image`. It returns one
`DesktopTile { frame, x, y }` per output, untouched: the server's pixel format,
`stride == width * 4`, no RGBA conversion, orientation, resize or canvas copy,
plus the canvas size, logical bounds and scale. It returns `Ok(None)` when tiles
cannot represent the desktop as is (mixed scales, a rotated or y-inverted output,
a size that would need resampling, a padded stride); call `capture_desktop` then.
Gaps between outputs belong to no tile. `examples/desktop_timing.rs` times both
paths, and captures the live desktop when run.

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
capture connection is dropped. GPU objects are `Send`: buffers share their GBM
device behind a lock, and every GBM call holds it, so a connection with GPU state
can still live in a `static Mutex`.

`ready` signals capture completion; `wl_buffer.release` is not the completion
mechanism for these protocols. This implementation follows their implicit
DMA-BUF synchronization contract. It does not offer explicit sync fences.

### Fast desktop tiles

`features = ["vulkan"]` adds `capture_desktop_tiles_fast(&options)`: the same
tiles, layout and `Ok(None)` rules as `capture_desktop_tiles`, read back from
DMA-BUFs when the system allows it. Tiers, fastest first:

1. Vulkan: GBM buffers with a vendor modifier that Vulkan can import
   (`VK_EXT_image_drm_format_modifier`, `VK_EXT_external_memory_dma_buf`), on
   the physical device matching the compositor's DMA-BUF device
   (`VK_EXT_physical_device_drm`), copied to host memory.
2. LINEAR: a LINEAR GBM buffer read through `mmap` inside a
   `DMA_BUF_IOCTL_SYNC` window, for systems without a usable Vulkan. This tier
   is **not checked on real hardware**: no driver on the development system
   allocates LINEAR (NVIDIA's GBM reports a zero stride). It is tested on real
   DMA-BUFs from `/dev/udmabuf` against the test server only.
3. SHM, as `capture_desktop_tiles`.

`DesktopTiles::readback` names the tier used; `tile_readback()` returns it with
the first reason a faster tier was not used (`ReadbackFallback`). A cause that
will not change (no loader, extension, device, DMA-BUF support or modifier) turns
its tier off until `reset_readback()`; a device mismatch is retried once the
compositor names another device; timeouts and refused imports are retried on the
next call. Every failure falls through within the same deadline; none panics.

The Vulkan instance, device, queue and host buffers stay on the connection, so
keep the connection for repeated captures: the first call pays about 70 ms of
driver setup on the development system, later calls none. Only ARGB8888 and
XRGB8888 use the GPU tiers; other formats go to SHM. The LINEAR tier needs the
compositor's device, which a connection learns from its first capture.

On the Vulkan tier with cached host memory (`HOST_CACHED`, as on NVIDIA), a
tile's `CpuBuffer` is the mapped host buffer itself rather than a copy. The
tile owns that buffer until it is dropped, even after the connection is
dropped or `reset_readback()` ran, so tiles can go to another thread and the
next capture can run meanwhile. At most eight buffers are leased at once;
past that, tiles are copied to the heap. Two idle buffers are kept for the
next capture. Without cached host memory, tiles are always copied.

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
- `runtime/`: private caller-owned coordinator, bounded commands and durable wake.
- `window/`: owned roles/configuration/leases, CPU SHM output and guarded native
  driver presentation boundary.
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

Ordinary clipboard on the owned window Host now provides explicit eager source,
current offer, asynchronous receive and cancellation methods. The
[clipboard result](docs/test-results/coordination/clipboard/README.md) records
limits, synthetic verification and remaining cross-client acceptance. It uses
this host's existing eventloop without requiring DnD or a new feature.

Primary selection (middle-click paste) uses the optional
`zwp_primary_selection_device_manager_v1`, bound at version 1 whenever advertised,
with one primary device per seat. `Host::set_primary_selection` and
`Host::primary_selection_offer` follow `set_clipboard` and `clipboard_offer`: same
focus and press rules, limits and shared budget. The seat's clipboard and primary
sources are independent, so replacing one never cancels the other. Primary offers
arrive as `ClipboardEvent::PrimarySelection` and are read with the ordinary
`receive_clipboard` (the offer id says which selection it is). A new read on a
seat cancels that seat's previous read of either kind. Without the global, or after
its removal, both calls return
`Unsupported("zwp_primary_selection_device_manager_v1")` until a new Host.
`ClipboardEvent` is non-exhaustive. `cargo run --features window --example primary` checks it live: a
left click sets primary, a middle click prints it.

## Review and benchmarks

The [2026-09-23 review](docs/reviews/2026-09-23.md) records confirmed issues and
API maintenance gaps. The [implementation plan](docs/plans/done/2026-09-23-review-fixes.md)
tracks the implemented fixes, regression tests and remaining acceptance criteria.
The current check results and any unverified environments are in `VALIDATION.md`.

See [VALIDATION.md](VALIDATION.md) for regression coverage, local comparison with
libwayshot 0.7.3 and the limits of synthetic measurements. The standalone
`benchmarks/` crate keeps comparison dependencies out of library consumers.
