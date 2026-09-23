# libway integration

Boltsnap's Linux screenshot backend now uses our independent Rust library
`libway` 0.1.0 instead of libwayshot. The standalone project is `../libway`;
the unpublished source is included in `vendor/libway` to keep normal checkouts,
CI and Nix builds self-contained. [Snapshot workflow](../vendor/README.md).

Full screenshots, active-window regions and the frozen desktop used by the area
and window selectors use CPU SHM frames. EXT image-copy-capture with an output
capture source is preferred; WLR screencopy is selected when EXT is unavailable
or has no supported buffer format. Portal fallback remains in Boltsnap, including
its existing scope: full screenshots and desktop-capture failures, not arbitrary
active-window capture. Portal consent remains compositor/backend policy.

The selector still receives its existing image and monitor types. The adapter
converts all eight transforms and checks that layout metadata matches the captured
image. Output removal or a layout change aborts selection capture rather than
opening it with stale geometry. Portal capture also checks for layout changes.
Negative coordinates, highest-output scaling and the 64-million-pixel guard are
retained. libway adds bounded protocol waits, buffer validation and working-memory
limits. Black desktop gaps are opaque; output captures remain sequential.

The library's optional GPU feature provides owned GBM DMA-BUF frames and persistent
capture sessions as a foundation for recording consumers. Boltsnap does not enable
that feature for screenshots. This change does not replace wf-recorder, the replay
worker, audio capture or any encoder. Synthetic WLR capture measurements show lower latency than libwayshot 0.7.3;
real-compositor and recording performance remain unverified. X11, selector/shelf implementations and the frozen Windows backend were
not refactored.

## Validation, 2026-09-22

- Linux `cargo check --locked`, `cargo build --locked`, `cargo fmt --check`: passed.
- Boltsnap `cargo test --locked`: 260 passed; one pre-existing FFmpeg thumbnail test
  remains ignored. The new region parser test includes negative origins, zero and
  overflowing dimensions.
- libway default, minimal CPU and all-feature tests: passed. All-feature execution
  with explicit test binary/render node runs 30 tests with none skipped.
- Actual test-built Boltsnap produced correct mixed-scale PNG stdout against
  isolated EXT, WLR and EXT-to-WLR fallback servers. The fixture PNG was decoded,
  pixel-checked and inspected. No live compositor was connected.
- GPU tests allocated/exported/reimported a real GBM buffer on the local render
  node, checked owned descriptor lifetimes, and tested Wayland FD transport,
  asynchronous import rejection and cleanup on the synthetic server.
- Strict Clippy passes for libway. Supplemental strict Clippy for the entire
  Boltsnap tree finds existing warnings in unrelated code; all diagnostic source
  spans were present unchanged at the original HEAD. These are not suppressed.
- The Linux normal dependency graph no longer includes libwayshot, GBM or EGL from
  capture. The CPU-only libway dependency remains target-specific to Linux.

See the [library validation report](../vendor/libway/VALIDATION.md) for protocol
coverage, commands and explicit limits. CI now verifies the snapshot, tests the
library feature variants, and runs the isolated Boltsnap consumer test.

The first sandboxed Unix-socket tests were denied with EPERM; permission-enabled
runs passed. Build artifacts were directed to `/tmp/boltsnap-libway-target` and
`/tmp/libway-work/target`, preserving the user's running development binary.

Still requiring an intentional live/hardware test session: actual compositor
rendering into DMA-BUFs and consumer image verification; sustained capture and
encoder synchronization; multi-GPU and multiple vendor combinations; real mixed
DPI/rotation/hotplug; interactive selector/shelf and portal behavior. No Windows
support claim or Windows feature change is included. The active desktop was not captured or changed. No visible UI, install, commit or
publication was performed.

## Follow-up review

Fixed in-flight constraint/damage validation, resize scratch accounting and
handling of duplicate capture-manager globals. Single-output composition avoids
an extra full-frame allocation/copy; packed 8888 conversion avoids per-pixel
vector growth checks. New regression tests cover each fix and every transform
through the direct composition path.

The isolated release benchmark measured 1080p WLR capture at 6.827 ms median
versus libwayshot 0.7.3 at 12.601 ms; 4K at 26.575 versus 44.035 ms. These timings
include synthetic server work and RGBA output, not real compositor rendering or
PNG export. See the [full methodology and caveats](../vendor/libway/VALIDATION.md#follow-up-review-and-performance-comparison).
The reproducible benchmark is a separate crate and adds no production dependency.
