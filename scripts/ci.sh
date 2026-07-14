#!/usr/bin/env bash
# Local mirror of .github/workflows/ci.yml. Zero reliance on the CI provider.
set -euo pipefail

echo "==> fmt"
cargo fmt --check

echo "==> build"
cargo build --workspace

echo "==> clippy"
cargo clippy --all-targets --workspace -- -D warnings

echo "==> test"
cargo test --workspace

echo "==> docs"
cargo doc --no-deps --workspace

echo "==> bench (no-run)"
cargo bench --no-run

echo "==> fuzz build (if cargo-fuzz is installed)"
if command -v cargo-fuzz >/dev/null 2>&1; then
  (cd fuzz && cargo fuzz build)
else
  echo "    cargo-fuzz not found — skipping (install with: cargo install cargo-fuzz --locked)"
fi

echo "==> all local CI steps passed"