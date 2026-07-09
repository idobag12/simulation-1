#!/usr/bin/env bash
# The CI-equivalent local gate (SPEC §3, §14): format, lints as errors,
# every test including the determinism suite. A phase is not done — and a
# change is not landable — unless this script exits 0.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> cargo fmt --check"
cargo fmt --all -- --check

echo "==> cargo clippy (-D warnings)"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> cargo test (workspace, includes determinism suite)"
cargo test --workspace --quiet

echo "==> check.sh: all green"
