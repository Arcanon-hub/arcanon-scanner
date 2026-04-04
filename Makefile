.PHONY: lint fmt test build install uninstall

## Run clippy with denied warnings (BLDG-01)
lint:
	cargo clippy -- -D warnings

## Check code formatting (BLDG-02)
fmt:
	cargo fmt --check

## Run all tests (BLDG-03)
test:
	cargo test

## Build debug and release binaries (BLDG-04)
build:
	cargo build
	cargo build --release

## Install to ~/.cargo/bin for local testing
install:
	cargo install --path .

## Uninstall from ~/.cargo/bin
uninstall:
	cargo uninstall arcanon
