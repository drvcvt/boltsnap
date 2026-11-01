#!/usr/bin/env python3
"""Draw the README's benchmark chart from a benchmark report's raw times.

  python3 tools/benchmark_chart.py benchmarks/full-capture-2026-11-02.md

Reads the JSON block of raw wall-clock times (seconds per run, keyed
`tool_version`) and writes `assets/benchmark.svg`: one bar per tool at its
median, on a linear axis, Boltsnap first.
"""
import json
import re
import statistics
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "assets" / "benchmark.svg"
NAMES = {"boltsnap": "Boltsnap", "flameshot": "Flameshot", "grim": "grim", "wayshot": "Wayshot"}

WIDTH = 680
LABEL_W = 150
VALUE_W = 70
BAR_H = 20
GAP = 12
PAD = 8


def runs(report):
    block = re.search(r"```json\n(.*?)```", report.read_text(), re.S)
    if not block:
        sys.exit(f"{report}: no JSON block of raw times")
    return json.loads(block.group(1))


def label(key):
    tool, _, version = key.partition("_")
    name = NAMES.get(tool, tool)
    # A commit instead of a version is the current development build.
    return f"{name} {version}" if "." in version else f"{name} (main)"


def svg(medians):
    longest = max(ms for _, ms in medians)
    scale = (WIDTH - LABEL_W - VALUE_W) / longest
    height = PAD * 2 + len(medians) * (BAR_H + GAP) - GAP
    rows = []
    for i, (name, ms) in enumerate(medians):
        y = PAD + i * (BAR_H + GAP)
        width = max(ms * scale, 2)
        bar = "bar own" if name.startswith("Boltsnap (main)") else "bar"
        rows.append(
            f'<text class="label" x="{LABEL_W - 12}" y="{y + BAR_H / 2}">{name}</text>'
            f'<rect class="{bar}" x="{LABEL_W}" y="{y}" width="{width:.1f}" height="{BAR_H}" rx="3"/>'
            f'<text class="value" x="{LABEL_W + width + 8:.1f}" y="{y + BAR_H / 2}">{ms:.0f} ms</text>'
        )
    style = (
        "text{font:13px system-ui,-apple-system,'Segoe UI',sans-serif;dominant-baseline:central;"
        "font-variant-numeric:tabular-nums}"
        ".label{fill:#3b3f45;text-anchor:end}.value{fill:#3b3f45}"
        ".bar{fill:#b9bec6}.own{fill:#2f6fde}"
        "@media (prefers-color-scheme:dark){.label,.value{fill:#d5d8dd}"
        ".bar{fill:#5a6069}.own{fill:#5b93f0}}"
    )
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{WIDTH}" height="{height}" '
        f'viewBox="0 0 {WIDTH} {height}" role="img">'
        f"<style>{style}</style>{''.join(rows)}</svg>\n"
    )


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    times = runs(Path(sys.argv[1]))
    medians = [(label(key), statistics.median(values) * 1000) for key, values in times.items()]
    medians.sort(key=lambda item: (not item[0].startswith("Boltsnap"), item[1]))
    OUT.write_text(svg(medians))
    for name, ms in medians:
        print(f"{name}: {ms:.1f} ms")


if __name__ == "__main__":
    main()
