#!/bin/sh
# Renders the README's pictures with Boltsnap's own painters, without a
# desktop, and with default settings (an empty config directory):
#   assets/screenshots/selector.png   the selector over two settings windows
#   assets/screenshots/shelf.png      the shelf with three cards, one hovered
set -eu
cd "$(dirname "$0")/.."
work="${CARGO_TARGET_DIR:-target}/readme-shots"
mkdir -p "$work/config"
export XDG_CONFIG_HOME="$PWD/$work/config"
shots=assets/screenshots

# Two floating windows, at the sizes `WINDOWS` in examples/readme_shots.rs gives.
render() {
    cargo run --quiet --release --bin boltsnap -- settings --render "$1" "$work/$1.png" --size "$2"
}
render Recording 900x760
render Screenshots 680x600
cargo run --quiet --release --example readme_shots -- \
    desktop "$work/Recording.png" "$work/Screenshots.png" "$work/desktop.png"

# The test also checks the toolbar's geometry gates afterwards; the shots
# are written before that, so a gate failure does not stop the pictures.
SEL_BASE="$work/desktop.png" SEL_OUT="$work/selector" \
    cargo test --quiet --bin boltsnap selector_shots -- --ignored >/dev/null 2>&1 ||
    echo "selector_shots: a geometry gate failed, see cargo test output" >&2
cp "$work/selector/screenshot.png" "$shots/selector.png"

# Earlier screenshots on the cards, newest first: the selector, then parts of
# the two windows.
cargo run --quiet --release --example readme_shots -- shelf "$shots/shelf.png" \
    "$shots/selector.png@420,200,1140" "$work/desktop.png@1100,240,600" "$work/desktop.png@140,140,700"
