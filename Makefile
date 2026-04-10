.PHONY: lint fmt test build install uninstall ml-setup ml-model ml-clean

VENV := ml_venv
PYTHON := $(VENV)/bin/python
PIP := $(VENV)/bin/pip

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

## Setup Python virtual environment and install ML tools
ml-setup:
	python3 -m venv $(VENV)
	$(PIP) install "optimum[onnxruntime]" transformers onnx

## Download and export CodeBERT model to models/ (requires python3)
ml-model: ml-setup
	$(PYTHON) ml_research/fetch_base_model.py
	rm -rf temp_model

## Clean up ML artifacts
ml-clean:
	rm -rf $(VENV) temp_model

## Install to ~/.cargo/bin for local testing
install:
	cargo install --path .

## Uninstall from ~/.cargo/bin
uninstall:
	cargo uninstall arcanon
