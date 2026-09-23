# Validation

## Review fixes: 2026-09-23

The seven runtime/input findings in [the review](docs/reviews/2026-09-23.md)
are fixed, with permanent regression tests. The
[implementation plan](docs/plans/2026-09-23-review-fixes.md) tracks completion.
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

Implementation work: 2026-09-22, from `docs/plans/2026-09-22-drag-and-drop.md`.

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

