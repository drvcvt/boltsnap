# Agent instructions

> [!IMPORTANT]
> AI agents must read this file and the [Contributing](README.md#contributing)
> section before changing the repository.

These rules apply to the whole repository.

## Before editing

- Inspect the full call path and all callers of code being changed.
- Preserve existing Linux behavior unless the change explicitly replaces it.
- Keep each change limited to one capability; do not perform unrelated platform
  refactors.

## Platform boundary

Boltsnap ships native Linux and Windows backends. Wayland/X11 capture, Unix
sockets, systemd startup, POSIX signals, `ksni`, and the
`wf-recorder`/`pactl`/`hyprctl` integrations are Linux implementation details;
Win32/DXGI/WGC capture, named pipes, Media Foundation, WASAPI, and the Task
Scheduler autostart are Windows implementation details.

- Keep portable policy and data code outside OS backends: CLI parsing, config
  values, image processing, protocol serialization, and pure calculations.
- Put OS implementations behind one narrow `platform` API. New platform code
  belongs in `src/platform/linux/` or `src/platform/windows/`, selected in
  `src/platform/mod.rs` with module-level `#[cfg(target_os = ...)]` declarations.
- When a Windows change touches an existing Linux-only implementation, move
  that capability behind the platform boundary. Do not move unrelated modules.
- Do not scatter OS checks through shared business logic. A tiny local `#[cfg]`
  is acceptable only when extracting a module would be less clear.
- Shared types must not expose Wayland, X11, Unix, or Win32 library types.
- Put OS-only crates in target-specific `Cargo.toml` dependency sections. A
  Linux build must not compile Windows dependencies, and a Windows build must
  not compile Wayland/X11/Unix dependencies.
- Keep paths as `Path`/`PathBuf`. Resolve config, cache, runtime, and temporary
  directories inside the platform layer; never add `/tmp`, `HOME`, XDG, or
  Windows environment assumptions to shared code.
- Isolate native IPC, process detachment, signals, service startup, clipboard,
  tray, capture, selection, and recording per OS. Do not emulate Unix behavior
  on Windows by shelling out to Linux-oriented commands.
- Partial Windows support is acceptable when unsupported commands fail clearly
  and are documented. Never report success for a no-op fallback.
- Known Windows gaps and open decisions are listed under "Noch offen" in
  [PORTING_PLAN.md](PORTING_PLAN.md). Do not implement items from that list —
  or any other new feature — without explicit maintainer approval: propose
  first, implement after agreement.

## Verification

- Run `cargo fmt --check` and `cargo test` for every change.
- Platform-neutral tests must remain platform-neutral; do not hide failures
  with `#[cfg]`.
- Linux backend changes require a Linux check. Windows backend changes require
  `cargo check --target x86_64-pc-windows-msvc` and functional tests on Windows
  following [docs/windows-smoke-test.md](docs/windows-smoke-test.md).
- Do not claim Windows support until capture, clipboard, output paths, and error
  handling have been smoke-tested on Windows.
- Update the README support matrix and platform prerequisites when capabilities
  change.
