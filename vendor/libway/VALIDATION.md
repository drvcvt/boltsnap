# Validation

## Multi-output desktop capture speed (Boltsnap): 2026-11-01

Plan: [capture speed](docs/plans/phase-a/2026-11-01-capture-desktop-speed.md).
Status: done.

**What changed.** Desktop capture now uses one connection for all outputs
(L1). Each frame is composed straight into the canvas as soon as it is ready
(L1c). `capture_desktop_tiles` adds raw per-output tiles (L2). Commits:

| Commit | Content |
| --- | --- |
| `117bd98` | plan and timing example |
| `7ff1fbb` | split of `capture.rs` |
| `bfc19a5` | one connection for all outputs |
| `dcaa2f6` | tiles |
| `d9ac043` | pipelined direct composition |

**Synthetic tests.**

- `tests/protocol.rs`, rewritten and new:
  - every output on one connection, with the server holding readies until
    both captures are pending and answering in reverse order;
  - one output's failure fails the call and releases every frame, buffer,
    session and source; Auto then succeeds over WLR;
  - a held-back ready times out with the same cleanup.
- `tests/tiles.rs`:
  - positions, untouched XRGB8888 bytes and `stride == width * 4`;
  - mixed scales or a rotated output fall back before capturing;
  - a y-inverted frame falls back and its buffers are released;
  - no outputs returns `NoOutputs`.
- Unit tests:
  - the shared row converter against whole-frame conversion for every
    format;
  - the gap fill when a frame is one pixel short.
- All existing composition tests stay byte-identical.

The final `tools/check.sh full` passed with these library test counts:

| Feature set | Tests |
| --- | ---: |
| no default features | 261 |
| default | 265 |
| `capture` | 261 |
| `dnd` | 288 |
| `foreign-display` | 288 |
| `owned-native-system` | 281 |
| all features | 313 (1 ignored) |

**Live, on the owner's desktop with their okay.** Two 1920x1080 outputs at
scale 1, Hyprland 0.56.2, NVIDIA RTX 4060, release build, 20 runs, with a
YouTube video playing:

| Build | warm median |
| --- | --- |
| before (`117bd98`) | 18-20 ms |
| L1 alone | 26-27 ms (regression, fixed by L1c) |
| after L1c | about 15 ms |
| tiles | about 13 ms |

The P4 dmabuf probe measured:

- the GPU copy at 4-5.4 ms per output, against 8-9.6 ms for shm;
- NVIDIA GBM cannot map the tiled dmabuf for CPU readback, so there is no
  follow-up.

The full table and phase timings are in the plan, section 6.

## Toplevel requests (R3): 2026-11-01

Plan: [R3](docs/plans/done/2026-11-01-r3-toplevel-requests.md). Status: done. The
plan covers size limits, attention and idle inhibit. Bell, parent and modal
moved to the roadmap's "later" list (D1). Regions on toplevels no longer commit
(D3).

`tools/check.sh full` passed as one local invocation (Gentoo, rustc/Cargo 1.98.1,
locked dependencies, niced, two Cargo jobs). It ran library tests for each
feature set:

| Feature set | Tests |
| --- | ---: |
| no default features | 260 |
| default | 264 |
| `capture` | 260 |
| `dnd` | 287 |
| `foreign-display` | 287 |
| `owned-native-system` | 280 |
| all features | 312 (1 ignored as before) |

The same run also passed the GPU all-target check, formatting, strict Clippy,
Rustdoc with `-D missing-docs`, and the `integration/dnd-winit` and
`benchmarks` manifests.

New synthetic tests are in `src/window/tests/shell/toplevel.rs` (12 tests):

- Size limits:
  - only changed requests are sent, with no commit;
  - the safe order covers growth, shrinkage, a mixed-axis change (three
    requests), a min-only change and dropping the maximum;
  - errors leave the ledger empty;
  - the limits ride the next presented frame.
- Attention:
  - the token sequence carries no serial and no app id;
  - `activate` is sent before the token destroy;
  - a request made while one is pending is coalesced;
  - the attention calls work on layers;
  - a closed window only gets its token destroyed;
  - the call is unsupported without the global, and a pending token survives
    global removal;
  - shutdown cleanup.
- Idle inhibit:
  - enabling and disabling are idempotent;
  - the inhibitor is destroyed before the surface on close, also while a lease
    holds the surface, and on shutdown;
  - the calls work on layers;
  - without the global, enabling is unsupported and disabling is fine.

`src/window/tests/shell/region.rs` now checks that toplevel regions send no
commit and that layer input and opaque regions still do. The peer compositor
gained `src/window/tests/peer/toplevel.rs` for the activation and idle-inhibit
globals.

Before the final check, a fresh review found a mixed-axis case where two
size-limit requests in either order briefly had max below min. That is now
fixed with a three-step order. An audit of the tests against the plan found
eight gaps, which are now closed. Details are in the plan's section 10.

The live check ran twice on Hyprland 0.56.2 with one owned toplevel, using
`examples/attention.rs` (debug build). Both runs logged:

```
configured 950x1032
shown
idle inhibit on
configured 320x200
attention requested
idle inhibit off
done
```

The second run also had a read-only listener on Hyprland's event socket
(`.socket2.sock`):

```
14:38:17 openwindow>>564352489500,7,libway.attention,libway attention
14:38:19 urgent>>564352489500
14:38:29 closewindow>>564352489500
```

- **Attention:** `request_attention` after focus loss made Hyprland mark
  exactly this window urgent. It did not take focus.
- **Size limits:** the first configure tiles the window at 950x1032, above the
  640x400 maximum. Hyprland does not apply size limits to tiled windows, and
  the protocol allows that.
- **The 320x200 configure:** it arrived in both runs, even though in the second
  run the owner only clicked into the window and away. Hyprland chose it on its
  own, and the cause was not investigated. So this check does not show whether
  an interactive resize respects the minimum.
- **Idle inhibit:** created and destroyed without errors. Its screen effect was
  not observed, because that would need changes to the session's idle
  timeouts.

## Layer surfaces (R7): 2026-11-01

Plan: [R7](docs/plans/done/2026-11-01-r7-layer-shell.md). Status: done, including the
live check.

`tools/check.sh full` passed as one local invocation (Gentoo, rustc/Cargo 1.98.1,
locked dependencies, niced, two Cargo jobs): library tests for no default
features (248), default (252), `capture` (248), `dnd` (275), `foreign-display`
(275), `owned-native-system` (268) and all features (300, 1 ignored as before),
the GPU all-target check, formatting, strict Clippy, Rustdoc with
`-D missing-docs`, and the `integration/dnd-winit` and `benchmarks` manifests.

New synthetic tests against literal wire traces:

- `src/window/tests/shell/layer.rs` covers creation:
  - absent global and `NotReady`;
  - exact creation order with a null and a chosen output, every field's raw wire
    value;
  - eleven invalid combinations with an empty ledger;
  - the version gates v1 to v5, with nothing sent on refusal, and teardown per
    version.
- `src/window/tests/shell/layer_runtime.rs` covers runtime behaviour:
  - configure/ack, with the ack before the observation and before the present
    commit, 0 replaced by the requested size, two configures in order;
  - a 0 configure on a stretched axis, which fails without an ack;
  - state changes that send only the changed requests and one commit;
  - `closed`, which retires the identity once, cancels keyboard and pointer
    focus, and destroys the role before the surface;
  - an own close followed by `closed`;
  - keys and leave on a layer;
  - role checks with a real seat;
  - CPU presentation;
  - global removal.
- `src/window/tests/shell/region.rs` covers input and opaque regions.
- `src/window/tests/scaling.rs` and `src/window/scale/tests.rs` cover output
  identity: announced only after the first `done`, removal, no reuse, bounded
  names.
- `src/window/tests/native/admission.rs` covers raw handles and native frames on
  a layer.

A fresh review of the diff and an audit of the tests against the plan's 13 test
items ran before the final check. Their findings are listed in the plan's
section 11.

Live check on Hyprland 0.56.2 (DP-3 and DP-1, 1920x1080, scale 1), owned test
surfaces only, `cargo run --no-default-features --example layer` (debug
build):

```
hud configured 240x120
hud shown
hud pointer events: 0
prompt configured 360x160
prompt shown
prompt keyboard enter
prompt closed by Escape
```

The HUD was checked from a screenshot cropped to the corner, which the owner
confirmed. It sits top right, 24 px from both edges and below the panel
(`Respect`), translucent with a 2 px border. The owner moved the pointer across
it and it saw no pointer events (`InputRegion::None`).

The first two runs used `Exclusive` for the prompt. Both logged
`prompt keyboard enter` and then `prompt timeout`, because the owner pressed
nothing: the prompt had landed on the other monitor and locked the keyboard.
Reading the Hyprland source explains this. With a null output, Hyprland picks
its focused monitor, and `Exclusive` is a hard lock on keyboard and pointer.
The prompt now uses `OnDemand` (normal window focus, it closes on focus loss),
and the rustdoc and README say which mode fits which surface.

Unrelated to R7, one parallel full run hit a single
`dnd_incoming::leave_after_drop_does_not_abort_and_limits_apply` failure. That
test passed 30 times in a row on its own, and the final full run was green.

## Primary selection (R2): 2026-11-01

Plan: [R2](docs/plans/phase-a/2026-10-20-r2-primary-selection.md). Status: built; the
owner-run live check is still open, so R2 counts as synthetic-only.

`tools/check.sh full` passed as one local invocation (Gentoo, rustc/Cargo 1.98.1,
locked dependencies, niced, two Cargo jobs): library tests for no default
features (225), default (229), `capture` (225), `dnd` (252), `foreign-display`
(252), `owned-native-system` (244) and all features (276, 1 ignored as before),
the GPU all-target check, formatting, strict Clippy, Rustdoc with
`-D missing-docs`, and the `integration/dnd-winit` and `benchmarks` manifests.

New synthetic tests: 12 cases in `src/window/tests/clipboard/primary.rs`: absent
global, one bind and one device per seat (including a seat added after the bind),
offers published separately from the clipboard, null selection, reads from the
primary offer, a primary read superseding a clipboard read, the press/serial
rule, exact sent bytes with independent clipboard and primary sources,
compositor cancel, global removal, seat removal and shutdown.

Two fixes were needed to get the suite green. `clipboard/transfer/tests.rs`
failed intermittently in parallel runs because `signal_tests.rs` spawns a child
that briefly holds every descriptor between fork and exec; the affected
assertions now wait (bounded) for the EOF or descriptor reuse they test, and 30
consecutive clipboard runs passed (details in the R2 plan's deviations). Clippy
1.98 added `chunks_exact_to_as_chunks`; seven existing sites use `as_chunks`
now (separate commit).

Open: `cargo run --no-default-features --example primary` on Hyprland, steps
(a) to (c) in the plan's proof section, run by the owner because it touches the
desktop's real primary selection.

## Presentation feedback (R1): 2026-10-20

Plan: [R1](docs/plans/done/2026-10-20-r1-presentation-feedback.md). Rust/Cargo 1.95.0,
locked dependencies. The local machine had under 4 GB RAM available, so every check
row that needs no system Wayland/GBM headers ran on the Anvil worker `mini`
(NixOS 25.11); the `owned-native-system` rows there loaded Nix's libwayland 1.24.0
through `LD_LIBRARY_PATH`. Rows that link the system libwayland or GBM ran locally
(Gentoo, libwayland 1.24.0), niced, one Cargo job.

New synthetic tests: 15 CPU cases in `src/window/tests/output/presentation.rs` and
3 native cases in `src/window/tests/native/presentation.rs`. They cover the absent
global, the bound version, exact `presented` decoding including unknown flag bits and
refresh 0, unarmed commits, replacement, tags kept armed across `NeedsFrame`, the
eight-request window cap, event-slot reservations with a nearly full queue, close
with outstanding feedback, manager removal and re-advertisement, malformed
`tv_nsec`, `presented` before `clock_id`, shutdown, and native staging (feedback
after the frame callback, no libway commit, cancel before and after staging). All
18 pass on both the pure-Rust and the system backend.

`mini`: library tests for no default features, default, `capture`, `dnd` and
`owned-native-system` passed; strict Clippy (all targets) for default, `dnd` and
`owned-native-system` passed; Rustdoc with `-D warnings -D missing-docs` for default,
`dnd` and `owned-native-system` passed. `cargo fmt --check` passed locally.

Local: tests for `foreign-display` (240 library tests) and all features (264 library
tests, 1 ignored as before), the GPU-without-image all-target check, strict Clippy
for all features and all-feature Rustdoc with `-D missing-docs` passed. Formatting,
all-target check and strict Clippy for `integration/dnd-winit` and `benchmarks`
(default and `gpu`) passed after rebuilding libway from this source (rmeta
timestamps checked). Together these cover every row of `tools/check.sh full`; the
script itself was not run as one invocation because of the split. The new
`examples/presentation.rs` compiles.

Live check, owner-run on Hyprland 0.56.2 (window on DP-3, 1920x1080 at 239.76 Hz,
VRR off, scale 1), `cargo run --no-default-features --example presentation`
(debug build):

```
answers 240: presented 240, discarded 0, unavailable 0
clock 1 flags 0x5
median interval Some(29.687ms), median refresh Some(4.170837ms)
```

All 240 tags were answered with `Presented`; clock 1 is `CLOCK_MONOTONIC`; flags
0x5 are vsync and hw_completion (no hw_clock); the median refresh of 4.1708 ms
matches 239.76 Hz. The median interval between presented frames is the example's
own frame rate (debug build, CPU path, about 34 frames per second) and not part
of the feedback contract. The owner saw occasional stutter. Measured afterwards
with copies of the CPU swizzle and the example's frame code: a debug build spends
26.5 ms per frame swizzling a 950x1040 buffer (the window was tiled by dwindle),
release 0.44 ms. That matches the 29.7 ms interval; roadmap part R16 removes the
swizzle where the compositor allows it. MSRV stays unverified.


The seven runtime/input findings in [the review](docs/reviews/2026-09-23.md)
are fixed, with permanent regression tests. The
[implementation plan](docs/plans/done/2026-09-23-review-fixes.md) tracks completion.
Current checks ran on Linux x86_64 with Rust/Cargo 1.95.0 and locked dependencies.

`./tools/check.sh full` passed. Counts below separate ordinary tests from doctests;
ignored tests were not executed. Feature rows use `--no-default-features` except
Default and All features.

| Feature set | Ordinary passed | Doctests passed | Ignored |
| --- | ---: | ---: | ---: |
| Minimal | 6 | 0 | 0 |
| Default | 34 | 3 | 1 |
| Capture | 27 | 2 | 0 |
| DnD | 69 | 3 | 0 |
| Foreign display | 72 | 3 | 0 |
| All features | 101 | 6 | 4 |

The same full run passed:

- GPU without image: all-target compilation.
- Root formatting and strict Clippy for all targets, all features and DnD-only.
- Default, DnD-only and all-feature Rustdoc with `-D warnings -D missing-docs`.
  Public documentation is complete; the crate now warns on new missing docs.
- Formatting, all-target compilation and strict Clippy for the winit integration
  and the benchmark; benchmark checks cover both default and GPU features.
- DnD-only normal dependency inspection: no `image` or `gbm` dependency.

Regression coverage includes mismatched Copy/Move completion and retry, legacy
completion, Ask choices, confirmed socket backpressure with readers started before
and after queued writes, caller-driven polling and reader shutdown under pressure.
Deadline tests cover buffered and empty EOF at expiry, inactivity, outgoing expiry,
MIME fallback totals, zero waits and unrepresentable durations. Outgoing validation
covers Static/Lazy MIME names and limits, hotspot boundaries, preservation of an
existing drag/serial, foreign/destroyed safe surfaces, shared and guest displays,
raw/safe identity and stable hashes. Incoming MIME-prefix behavior and ordinary
Rust error propagation are also covered.

Six targeted release-profile tests also passed: public duration validation,
malformed Static/Lazy MIME inputs, icon hotspot boundaries, and the three reactor
tests. These used `--release --no-default-features --features dnd`; arithmetic
edge cases therefore pass with and without debug overflow checks.

Each confirmed bug had its new regression run against the unfixed code first.
Capture examples were compiled, not executed. The live desktop, hardware capture,
external consumer binaries, real XDND bridges and RustSec audit were not exercised
in this follow-up. Historical results below are not new validation of this patch.

**MSRV remains unverified:** Rust 1.88 is not installed locally. The `msrv` mode
correctly refused the available Rust 1.95.0 with exit status 2 and instructions to
select installed compiler/Cargo executables. No toolchain was installed. To close
this acceptance item, run the same library matrix with actual Rust 1.88.0:

```sh
LIBWAY_CARGO=/path/to/rust-1.88/bin/cargo LIBWAY_RUSTC=/path/to/rust-1.88/bin/rustc ./tools/check.sh msrv
```

The script exports the selected compiler as `RUSTC`, checks its version, and never
silently substitutes a current-toolchain run for MSRV verification. It contains no
cleanup, compositor, live-capture or input-helper commands.

## Earlier implementation validation: 2026-09-22

Implementation work: 2026-09-22. This file distinguishes protocol/driver testing
from real-compositor behavior; a synthetic server is not proof of compositor
compatibility or recording performance.

The isolated test suite covers EXT and WLR 1/2/3, padded SHM rows, pixel/alpha and
10-bit conversion, all eight inverse transforms, negative origins, mixed 1x/1.5x
scales, region cropping, layout changes, removed outputs/managers, changed buffer
constraints, old-frame immutability, bounded discovery/capture, idle streams,
cancellation, early failure, disconnects and resource destruction.

Optional hardware tests explicitly open `/dev/dri/renderD128` without connecting
to the desktop. They allocate/export GBM storage, reimport the allocation through
a second GBM device and check descriptor lifetime after allocator destruction.
The local driver successfully exported one plane with actual modifier
`0x0300000000e08012`; the advertised implicit import path uses
`0x00ffffffffffffff`. This is one local driver result, not a portability claim.

The GPU protocol fixture transports real exported DMA-BUF FDs through an isolated
Wayland socket, exercises EXT and WLR, accepts/rejects asynchronous imports, and
checks cleanup while a returned frame remains alive. It does **not** render GPU
pixels. The independent GBM reimport test verifies driver import separately.

Required additional validation before claiming broad production GPU capture:

- Actual compositor rendering into captured DMA-BUFs, consumer import/readback,
  image correctness and implicit synchronization under sustained load.
- Intel/AMD/NVIDIA and multi-GPU source/allocator combinations, explicit modifiers,
  real monitor hotplug and dynamic modes.
- Real Hyprland/Sway/Niri EXT/WLR captures, mixed DPI/rotation and portal fallback.
- End-to-end Boltsnap selector/shelf behavior on the live desktop after replacement.
- Recording throughput, latency, power, frame drops and encoder backpressure.

The active desktop was deliberately neither captured nor modified. No visible
selector, portal dialog, monitor reconfiguration or locker was launched. Existing
recording/replay implementations were not replaced. The synthetic comparison below does not establish real-compositor or zero-copy
recording performance.

## Completed checks

Local toolchain: Rust 1.95.0, Linux x86_64; system GBM 26.0.8. Commands use locked
dependencies and separate build directories. Socket/SCM_RIGHTS tests require a
sandbox that permits Unix socket traffic; the first restricted runs failed with
EPERM, and passed when run with that permission.

| Check | Result |
| --- | --- |
| `cargo fmt --check` | passed |
| Default CPU/image tests | 33 passed; opt-in consumer test run separately |
| `--no-default-features --features capture` tests | 26 passed |
| `--all-features -- --include-ignored`, explicit render node and test binary | 37 passed, none skipped |
| GPU without image, `cargo check --no-default-features --features gpu --all-targets` | passed |
| `cargo clippy --all-features --all-targets -- -D warnings` | passed |
| Actual Boltsnap against synthetic EXT, WLR and EXT-to-WLR fallback | passed |

The Boltsnap consumer test launches the separately built binary with an empty
environment, its inherited socketpair FD as `WAYLAND_SOCKET`, no session D-Bus
address, and `full --backend wayland --no-copy -o -`. It decodes the resulting PNG
and checks mixed-scale dimensions and pixels. Optional `LIBWAY_TEST_PNG` writes a
new synthetic artifact; it never captures the active desktop.

```sh
LIBWAY_TEST_BOLTSNAP=/absolute/path/to/test/boltsnap \
  cargo test --locked --test consumer -- --ignored
```

The standalone source and Boltsnap's vendored snapshot are byte-for-byte checked
by `tools/sync-libway.py --check ../libway` in the Boltsnap repository. The main
Boltsnap Linux check/build/fmt and 260 regular tests passed (one pre-existing
FFmpeg thumbnail test remains ignored). Supplemental strict Clippy on the whole
Boltsnap tree reports pre-existing warnings; every reported source span was
checked against the original Git HEAD. No unrelated lint refactor was included.

## Follow-up review and performance comparison

The review fixed three concrete correctness issues:

- In-flight damage validation now uses the dimensions of the attached buffer,
  even if the session advertises different constraints before that frame is ready.
  The regression failed with `InvalidDimensions` before the fix.
- Gaussian resize scratch uses source width times destination height, matching
  image 0.25.10's vertical-first RGBA32F intermediate. Arithmetic is checked;
  an anisotropic resize test rejects an insufficient budget before resizing.
- Duplicate manager globals no longer overwrite a selected manager or invalidate
  it when an unselected duplicate is removed.

Full-output composition transfers the converted image directly into the result,
removing one full-size allocation and copy. Packed 8888 conversion fills a sized
output slice, avoiding per-pixel vector growth checks. Transform, alpha, padding,
region and mixed-scale tests cover these paths. Allocation limits intentionally
remain conservative.

Reproduce the separate benchmark (libwayshot is **only** a benchmark dependency):

```sh
cargo run --locked --release --manifest-path benchmarks/Cargo.toml
```

Local measurement, 2026-09-22: Rust 1.95.0, one synthetic XRGB output, persistent
connections, five warmups then 40 captures. Both implementations receive identical
SHM pixels over separate socketpairs, with event-driven server polling. Timing
includes server pixel generation, capture and owned RGBA output; excludes initial
connection discovery, PNG, selectors, portals, display rendering and GPU work.
The machine was not reserved exclusively for benchmarking. These are illustrative
local samples, not stable latency guarantees or an end-to-end Boltsnap claim.

| WLR v3 / RGBA output | libway median / p95 | libwayshot 0.7.3 median / p95 |
| --- | --- | --- |
| 1920 × 1080 | 6.827 / 9.041 ms | 12.601 / 19.186 ms |
| 3840 × 2160 | 26.575 / 30.822 ms | 44.035 / 47.068 ms |

In this run, median WLR capture time was approximately 40–46% lower. libway EXT
alone measured 6.414 / 7.342 ms at 1080p and 25.023 / 28.758 ms at 4K. No EXT
speed ratio is reported: libwayshot 0.7.3's screenshot path omitted the required
initial `damage_buffer` request and failed the fixture's protocol assertion.
Its separate screencast implementation does send that request. The EXT protocol
XML explicitly requires full damage for a buffer's first capture.

The benchmark checks dimensions and RGB. libwayshot 0.7.3 preserves the unused
XRGB byte as alpha (zero in this fixture), whereas libway returns opaque alpha.
Do not interpret these timings as bit-identical RGBA results. Both common WLR
runs completed; the initial EXT comparison failure was not silently counted as
successful capture. Real compositor/GPU throughput still needs separate testing.

## Separate cursor capture follow-up

Seven isolated-server regression tests cover alpha-preserving cursor images,
matching image/hotspot generations across a resize, metadata during an idle image
request, visibility transitions, queue overflow, pointer/session errors, missing
seats, allocation limits, cancellation, dropping an in-flight image, connection
reuse and cleanup. Ordinary WLR capture continues without binding any seat.
Cursor events are receipt-timestamped; real-compositor cursor behavior and
end-to-end smoothing/encoding remain unvalidated.

## Drag-and-drop

Implementation work: 2026-09-22, from `docs/plans/done/2026-09-22-drag-and-drop.md`.

| Check | Result |
| --- | --- |
| `--no-default-features --features dnd` tests | 57 passed (no `image`/`gbm` in `cargo tree -e no-dev`) |
| `--no-default-features --features foreign-display` tests | 60 passed (system libwayland) |
| `--all-features` tests | 89 passed, 4 opt-in ignored |
| Default and `capture`-only tests | 33 and 26 passed, unchanged |
| `cargo test --doc --features dnd` | module example compiles |
| `cargo clippy --all-features --all-targets -- -D warnings`, same for `dnd` only | passed |
| Full `--all-features` suite ten times in a row | no flaky failure |

The test compositor rejects what Weston rejects (a preferred action outside the set,
`finish` with a none/ask action); any such request fails the test. Isolated protocol
tests (`tests/dnd_*.rs`, `tests/display.rs`) cover owned, shared
and guest displays, raw surface handles, data device v1–v3, readiness only after the
compositor knows the devices and pointers, target routing with priority and
enable/disable, refusal when the top target accepts nothing offered, motion
coalescing, accept/set_actions sequences, selection offers destroyed at once,
eviction of stale announcements, drop with `finish` only after consumer
completion, rejection without `finish`, raw and Latin-1 payloads, `Ask` drops answered only with an action the source offers, fallback from an undecodable
`text/uri-list` (browser `https` links) to the offer's text and failure only after every
accepted type failed, tracked serials only while a button is held, `FilesAndText` sources,
consumer dispatch next to a running reader with a 4 MiB send to a slow reader, fallbacks
that keep the drop's total deadline, the `file://HOSTNAME/` authority of `ls --hyperlink`,
one reader per session, the grab serial consumed by a started drag, invalid target specs
and relative drag paths rejected, sticky event-queue overflow, drops outside
targets, leave after drop, a new drag during a running transfer, payload/entry
limits, stalled writers, unregistering during a transfer, malformed URI lists,
closed pipe ends (checked per pipe inode, not by process fd counts), outgoing drags
with tracked and explicit serials, every offered MIME served, lazy 4 MiB payloads
over a 1 MiB pipe that outlive the drag, drag icons and invalid icons, v1 drops,
cancellation and replacement, and the reader thread's wake policy (including events produced by consumer calls), transfer
deadlines without display traffic and coexistence with an owner loop reading the
same socket.

With `foreign-display`, libwayland reads 4 KiB per call and the capture pump stops
reading while notices are queued, so the cursor flood test sees backpressure
instead of an overflow error there. The 256-notice bound holds either way.

### Private Weston

`integration/dnd-winit/run.py` against Weston 14.0.2 built privately from the
same checked release archive as Termo's (`-Dxwayland=true`, headless backend,
pixman renderer) with Termo's unchanged test-seat module. Nothing ran against the
desktop. Results were stable over three runs:

| Scenario | Weston 14 headless |
| --- | --- |
| Wayland sender (libway, tracked serial, icon) to winit receiver (libway guest + reader) | dropped; path with a space delivered exactly |
| X11 XDND source (x11rb) to winit receiver | drag starts in X; not bridged |
| GTK3 X11 source to winit receiver | drag starts in X; not bridged |
| Wayland sender to X11 XDND receiver | not bridged; sender sees `cancelled` |
| Idle winit receiver, 5 s without input | 0 `libway-dnd` thread context switches, fd and RSS unchanged |

The XDND results match Weston's source: `xwayland/dnd.c` only handles an
`XdndEnter` sent to its own `XdndAware` window, which it never maps, so toolkit
sources searching under the pointer never find it, and there is no Wayland-to-X
path at all. Under Weston, X11 clients report Weston's own pid through
`SO_PEERCRED` because Weston hands Xwayland a socketpair; the harness targets them
that way.

Hyprland 0.56.2 carries an XDND bridge in its XWM but could not be nested:
aquamarine requires `wl_compositor` v6 and Weston 14 offers v5. XDND bridging on a
compositor that implements it therefore remains unvalidated, as do touch-started
drags on real hardware, multi-seat compositors, portal file transfer offers
(`application/vnd.portal.filetransfer`) and Termo/Boltsnap end-to-end use.

