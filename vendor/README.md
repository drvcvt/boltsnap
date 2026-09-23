# libway snapshot

Boltsnap uses the unpublished `libway` 0.1.0 source snapshot in `libway/`.
The independent development project is `../libway`. Keeping this snapshot in the
checkout preserves standalone Cargo, CI, release and Nix builds without a local
sibling directory or an unpublished remote dependency. No generated build output
is included. The Linux dependency uses CPU capture and the `image` feature;
GBM/GPU support is optional for other library consumers.

Edit the standalone project, test it, then synchronize deliberately:

```sh
python3 tools/sync-libway.py ../libway
python3 tools/sync-libway.py --check ../libway
cargo test --locked
cargo test --locked --manifest-path vendor/libway/Cargo.toml --all-features
```

CI verifies the snapshot hashes and tests the embedded library. `--check` without
a source path checks only the committed snapshot, so CI needs no sibling checkout.
The sync command refuses to remove obsolete files; review such removals explicitly.
`libway.sha256` records the exact payload, including its lockfile and validation
notes. Research history stays in the standalone project. This is a temporary
source-distribution arrangement until a published libway version can replace the
path dependency; do not maintain an independent fork here.
