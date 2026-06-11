#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

# Change to repo root
cd "$(dirname "$0")/.."

echo "=== cargo fmt --check ==="
cargo fmt --check

echo "=== cargo clippy --workspace --all-targets -- -D warnings ==="
cargo clippy --workspace --all-targets -- -D warnings

echo "=== cargo test --workspace ==="
cargo test --workspace

echo "=== wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg ==="
wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg

echo "=== pnpm --dir app typecheck && pnpm --dir app test && pnpm --dir app build ==="
pnpm --dir app typecheck && pnpm --dir app test && pnpm --dir app build

echo "verify: all green"
