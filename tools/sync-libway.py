#!/usr/bin/env python3
"""Copy/check the unpublished libway snapshot without deleting files.

  python3 tools/sync-libway.py ../libway
  python3 tools/sync-libway.py --check [../libway]
"""
import argparse
import hashlib
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[1]
DEST = ROOT / "vendor" / "libway"
MANIFEST = ROOT / "vendor" / "libway.sha256"
TOP = ("Cargo.toml", "Cargo.lock", "LICENSE", "README.md", "VALIDATION.md", ".gitignore",
       "benchmarks/Cargo.toml", "benchmarks/Cargo.lock", "benchmarks/src/main.rs")


def files(root):
    result = {name: root / name for name in TOP}
    for directory in ("src", "tests", "examples"):
        for path in (root / directory).rglob("*"):
            if path.is_file():
                result[path.relative_to(root).as_posix()] = path
    return result


def digest(paths):
    return "".join(
        f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {name}\n"
        for name, path in sorted(paths.items())
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", nargs="?", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    if args.check:
        actual = digest(files(DEST))
        if actual != MANIFEST.read_text():
            raise SystemExit("libway snapshot differs from vendor/libway.sha256")
        if args.source and actual != digest(files(args.source.resolve())):
            raise SystemExit("libway snapshot differs from the standalone project")
        print("libway snapshot verified")
        return
    if args.source is None:
        parser.error("source project path required for synchronization")
    source = args.source.resolve()
    if source == DEST.resolve():
        parser.error("source and destination must differ")
    paths = files(source)
    contents = digest(paths)  # Validate every source before any write.
    if DEST.exists():
        extra = set(files(DEST)) - set(paths)
        if extra:
            raise SystemExit(f"Obsolete snapshot files require explicit removal: {sorted(extra)}")
    for name, path in paths.items():
        target = DEST / name
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)
    MANIFEST.write_text(contents)
    print(f"Updated libway snapshot: {len(paths)} files")


if __name__ == "__main__":
    main()
