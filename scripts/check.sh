#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-protected-server.py
PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-server-launcher.py
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release

