# Contributing

> [!IMPORTANT]
> **AI agents:** Before changing this repository, read this file and
> [`AGENTS.md`](../AGENTS.md). These rules are mandatory for platform-specific work.

Start with the [codebase inventory](../docs/codebase.md) for runtime entry points,
separate builds, current verification and open review findings. Build and test:

```sh
cargo build --release
cargo test
```

Dated performance reports preserve measurements from their stated revisions.

The Linux performance changes and their local validation are recorded in
[the performance report](../docs/performance.md). The subsequent screenshot,
selector rendering and compact UI work is documented in
[the screenshot performance report](../docs/screenshot-performance.md), including
benchmarks and the raw-pixel transport that remains opt-in after measurement.

Replay uses an isolated experimental
[media worker](../src/platform/linux/replay/worker/README.md), launched by the Linux
shelf daemon. It has its own build and media tests. Changes to replay must also
run the worker unit tests and synthetic `probe.py` / `live.py` integration tests;
see [replay setup](../docs/replay.md).

Linux is Boltsnap's supported platform. The unmaintained experimental Windows
backend stays in the same tree so shared contracts remain compile-tested; do
not add Windows work without explicit maintainer approval and a real Windows
tester. Read [`AGENTS.md`](../AGENTS.md) before starting; it contains the
repository-wide agent and verification rules.

Keep this boundary:

| Shared code | Linux implementation | Windows implementation |
|-------------|----------------------|------------------------|
| CLI and config values, image processing, serialized protocol data, pure calculations | Wayland/X11, Unix sockets, systemd, POSIX process/filesystem calls, `ksni`, `wf-recorder`, `pactl`, `hyprctl` | Windows capture, clipboard, IPC, process lifecycle, shelf, tray, and OS directory APIs |

- Put OS implementations under `src/platform/linux/` and
  `src/platform/windows/`, with selection centralized in `src/platform/mod.rs`.
  Move only the existing Linux capability that the Windows change actually
  touches.
- Keep shared APIs small and free of native Wayland, X11, Unix, or Win32 types.
  Avoid scattered `#[cfg]` branches in shared logic.
- Scope OS-only crates in target-specific `Cargo.toml` dependency sections.
  Windows builds must not compile Linux dependencies, and vice versa.
- Use `Path`/`PathBuf` in shared code. Resolve XDG versus Windows directories,
  local IPC, services, signals, clipboard, capture, tray, and external commands
  inside the relevant platform module.
- Preserve Linux behavior. Partial Windows support may fail explicitly for an
  unsupported capability, but must never silently succeed or fall back to a
  different capture mode.
- Keep pull requests focused on one capability and include the smallest test
  that protects its shared contract. Do not hide shared-test failures behind
  platform `#[cfg]` attributes.

Before merging, run `cargo fmt --check` and `cargo test`. Accepted Windows
changes must also pass `cargo check --target x86_64-pc-windows-msvc` and be
smoke-tested on a real Windows system. Automated CI alone is not evidence of
Windows support.
