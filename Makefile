build:
	cargo build
.PHONY: build

run:
	cargo run
.PHONY: run

console:
	RUSTFLAGS="--cfg tokio_unstable --cfg nbsh_tokio_console" cargo run
.PHONY: console

release:
	cargo build --release
.PHONY: release

run-release:
	cargo run --release
.PHONY: run-release

console-release:
	RUSTFLAGS="--cfg tokio_unstable --cfg nbsh_tokio_console" cargo run --release
.PHONY: console-release
