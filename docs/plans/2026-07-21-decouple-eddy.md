# Eddy Decoupling Implementation Plan

> **For agentic workers:** Implement this plan task-by-task — one task per commit, run each task's test before moving on. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Boltsnap independent of Eddy while keeping stdout piping as the external editor handoff and matching Linux/Windows shelf behavior.

**Architecture:** Remove the Eddy-specific runtime path instead of replacing it with another editor abstraction. Keep capture, shelf, clipboard, save, drag-and-drop, and `-o -` in Boltsnap; external programs consume stdout themselves.

**Tech Stack:** Rust, Cargo tests, PowerShell packaging, NSIS, WiX, GitHub Actions.

## Global Constraints

- Preserve native Linux capture behavior and the existing platform boundary.
- Do not add dependencies or a replacement editor integration.
- Windows installers contain only Boltsnap.
- Verify with `cargo fmt --check`, `cargo test`, and `cargo check --target x86_64-pc-windows-msvc`.

---

### Task 1: Remove the runtime editor contract

**Files:**
- Modify: `src/main.rs`
- Modify: `src/config.rs`
- Modify: `src/protocol.rs`
- Modify: `src/platform/linux/ipc.rs`
- Modify: `src/platform/linux/paths.rs`
- Modify: `src/platform/windows/paths.rs`
- Modify: `src/platform/windows/ipc.rs`
- Modify: `src/shelf/model.rs`
- Delete: `src/editor.rs`
- Test: `src/main.rs`, `src/protocol.rs`

**Interfaces:**
- Consumes: existing capture output selection and `-o -` stdout path.
- Produces: CLI parsing without `edit`, `--edit`, or `--editor`; capture always continues to file, clipboard, shelf, or stdout; editor backchannel commands are rejected.

- [x] **Step 1: Write the failing test**

```rust
#[test]
fn parser_rejects_removed_editor_flags() {
    for flag in ["--edit", "--editor"] {
        let result = parse_args(&["boltsnap".into(), flag.into()]);
        assert!(result.is_err(), "{flag} must not remain an integration hook");
    }
}
```

- [x] **Step 2: Run test to verify it fails**

Run: `cargo test --bin boltsnap parser_rejects_removed_editor_flags`
Expected: FAIL because `--edit` is currently accepted.

- [x] **Step 3: Write minimal implementation**

Delete the editor-only fields, parser branches, commands, post-capture branch, resolver functions, last-edited tracking, and `src/editor.rs`. Keep `target_path`, `normalize_path`, `ensure_file`, and stdout capture because clipboard and external piping still use them.

- [x] **Step 4: Run test to verify it passes**

Run: `cargo test --bin boltsnap parser_rejects_removed_editor_flags`
Expected: PASS.

- [x] **Step 5: Reject the removed editor backchannel**

```rust
for header in [
    br#"{"cmd":"add_video"}"#.as_slice(),
    br#"{"cmd":"replace","media":"image"}"#.as_slice(),
    br#"{"cmd":"reload"}"#.as_slice(),
] {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, header, &[]).unwrap();
    assert_eq!(
        Request::read(&mut Cursor::new(bytes)).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
}
```

Run before implementation: `cargo test --lib removed_editor_backchannel_commands_are_rejected`
Expected: FAIL because `add_video`, `replace`, and `reload` are accepted.

Delete those request variants, handlers, tests, and now-unused shelf model helpers.

Run after implementation: `cargo test --lib removed_editor_backchannel_commands_are_rejected`
Expected: PASS.

### Task 2: Make shelf actions editor-independent on both platforms

**Files:**
- Modify: `src/platform/linux/shelf/mod.rs`
- Modify: `src/platform/windows/shelf.rs`

**Interfaces:**
- Consumes: existing `copy_card` / clipboard functions.
- Produces: image body clicks copy PNG and video body clicks copy a file reference; no mouse action launches an editor.

- [x] **Step 1: Apply the existing copy behavior**

```rust
Hit::Body(id) => self.copy_card(id),
```

Delete both `open_in_eddy` helpers and route the Windows right-click through its existing copy action.

- [x] **Step 2: Verify shared behavior**

Run: `cargo test`
Expected: PASS with no editor module references.

### Task 3: Remove Eddy from Windows distribution and user documentation

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `packaging/windows/Boltsnap.nsi`
- Modify: `packaging/windows/Boltsnap.wxs`
- Modify: `packaging/windows/build-msi.ps1`
- Modify: `packaging/windows/build-nsis.ps1`
- Modify: `README.md`
- Modify: `PORTING_PLAN.md`

**Interfaces:**
- Consumes: the two Boltsnap release executables.
- Produces: ZIP, MSI, and NSIS packages with no Eddy/Qt checkout, build, files, registry entries, or shortcuts.

- [x] **Step 1: Delete bundle inputs and components**

Remove Eddy/Qt parameters, staging, build steps, WiX/NSIS features, workflow checkout, and documentation that describes bundled or automatic editor integration. Keep one external pipeline example:

```sh
boltsnap area --no-copy -o - | eddy -f -
```

- [x] **Step 2: Verify the repository contract**

Run: `rg --hidden -n -i '\beddy\b' .github packaging src README.md PORTING_PLAN.md`
Expected: only the explicit external-pipeline example and NSIS cleanup for
upgrades from previously bundled releases remain.

- [x] **Step 3: Run required checks**

Run: `cargo fmt --check && cargo test && cargo check --target x86_64-pc-windows-msvc`
Expected: all available checks pass; real Windows smoke testing remains required before claiming support.
