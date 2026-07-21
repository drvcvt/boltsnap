#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/../../.." && pwd)

python3 - "$root" <<'PY'
from pathlib import Path
import sys
import tomllib

root = Path(sys.argv[1])
plugin = root / "contrib/hyprland-xdnd-fix"

required = [
    root / "hyprpm.toml",
    plugin / "Makefile",
    plugin / "main.cpp",
    plugin / "README.md",
]
missing = [str(path.relative_to(root)) for path in required if not path.is_file()]
assert not missing, f"missing plugin files: {', '.join(missing)}"

manifest = tomllib.loads((root / "hyprpm.toml").read_text())
entry = manifest["boltsnap-xdnd-fix"]
assert entry["output"] == "contrib/hyprland-xdnd-fix/boltsnap-xdnd-fix.so"
assert entry["build"] == ["make -C contrib/hyprland-xdnd-fix all"]

readme = (plugin / "README.md").read_text().lower()
for warning in ("experimental", "crash", "x86_64", "function hook", "disable"):
    assert warning in readme, f"README must explicitly mention {warning!r}"

source = (plugin / "main.cpp").read_text()
assert "HASH != CLIENT_HASH" in source, "plugin must reject Hyprland ABI mismatches"
assert "METHODS.size() != 1" in source, "plugin must reject missing or ambiguous hooks"
assert "CX11DataDevice::sendEnter" in source, "hook lookup must target the exact member"
assert "#if !defined(__x86_64__)" in source, "plugin must reject unsupported architectures"

enter = source.index("g_sendEnterHook->m_original")
position = source.index("thisptr->sendMotion")
assert enter < position, "XdndEnter must be sent before the initial XdndPosition"
PY
