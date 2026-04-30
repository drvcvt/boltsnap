.PHONY: build release test install doctor smoke edit-last clean

build:
	cargo build

release:
	cargo build --release

test:
	cargo test

install:
	cargo install --path .

doctor:
	cargo run --quiet -- doctor

smoke:
	cargo run --release --quiet -- full --no-copy -o /tmp/boltsnap-smoke.png
	file /tmp/boltsnap-smoke.png

edit-last:
	cargo run --release --quiet -- --edit

clean:
	cargo clean
	rm -f /tmp/boltsnap-smoke.png
