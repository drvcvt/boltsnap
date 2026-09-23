# Validation

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
| `--no-default-features` tests | 26 passed |
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
