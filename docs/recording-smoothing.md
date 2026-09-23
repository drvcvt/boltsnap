# Replay snapshots, 2026-09-22

The implemented change shares immutable video GOP descriptors between the live
ring and frozen snapshots. Previously, freezing copied every video packet's
entry and cloned its payload reference while holding the ingest mutex. Now it
clones one `Arc` per GOP. Audio descriptors are still copied. On the first append
to a shared active GOP, `Arc::make_mut` copies that GOP's entries once. Completed
GOPs never need copying. Encoded payloads stay shared `Arc<Packet>` values.

The ring's time limit, byte accounting, closed-GOP rules, export exclusion and
no-overwrite behavior are retained. The existing conservative descriptor budget
is retained too. This reduces work under the ingest mutex during freeze/save;
it does not change the capture encoder or promise improved game frame times.
A snapshot is still O(GOP count + audio packet count), not constant time.

## Measurement

The standalone harness uses the actual before/after ring sources, optimized with
`rustc -O`, five warmups and 100 timed snapshots. It uses shared synthetic payloads,
one-second GOPs and 47 audio entries per second. Ring construction and snapshot
destruction are outside snapshot timing. It excludes FFmpeg parameter copying,
preview decoding, live mutex contention and real capture/encoding. The host was
not reserved exclusively for measurement.

| History | Before median / p95 | After median / p95 |
| --- | --- | --- |
| 60 s, 60 FPS | 101.50 / 131.30 µs | 18.35 / 25.58 µs |
| 600 s, 240 FPS | 4121.55 / 4527.65 µs | 199.91 / 244.08 µs |

An earlier run measured 77.31 → 18.35 µs and 3394.65 → 192.78 µs respectively.
These are local microbenchmarks, not an end-to-end clip latency comparison.
The harness also measures the first append after freeze, including expiration
of the oldest GOP at the window boundary. Medians were 0.82 → 0.96 µs for the
small case and 2.52 → 2.48 µs for the large case; this is not a claim that
copy-on-write has no cost or a worst-case scheduling bound.

Reproduce from the repository root:

```sh
git show d0686a038e2f2cc9cdae42770e1bb91c1d6271c5:src/replay/ring.rs > /tmp/boltsnap-ring-before.rs
BOLTSNAP_BASELINE_RING=/tmp/boltsnap-ring-before.rs \
  rustc -O --edition 2024 tools/bench-replay-snapshot.rs -o /tmp/boltsnap-ring-bench
/tmp/boltsnap-ring-bench
```

Verification: main fmt/check and 261 tests passed (one existing ignored FFmpeg
thumbnail test). Worker fmt, release build and 22 unit tests passed. Synthetic
`probe.py` and `live.py` passed packet/frame identity, audio, window/budget bounds,
freeze, concurrent ingest/status, crop/remux, stale selections and decoding.
Regression tests verify GOP sharing, one-time active-GOP detachment, immutable
frozen audio/video and eviction. The worker still reports the existing unused
`process::output_setup` warning. No running binary or user settings were replaced.

The accelerated 2400-loop soak processed 3,081,600 packets (about eight hours
of media time) in 13.73 s. Peak accounted packet bytes were 1,581,723 against an
8,388,608-byte budget. RSS peaked at 25,244 KiB with zero median growth after
warmup, and the resulting clip decoded successfully. This probe exercises
sustained ingest/eviction, not repeated live freezes or eight hours of real-time
recording. Artifacts: `/tmp/boltsnap-replay-soak.1ifwv4vg/`,
`/tmp/boltsnap-replay-test.jwh_rxqg/`, `/tmp/boltsnap-replay-live.m__0ht54/`.
