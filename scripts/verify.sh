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

# ---------------------------------------------------------------------------
# Desktop shell (Tauri host crate).
#
# The Tauri host is deliberately NOT a cargo workspace member (it drags
# desktop-only deps into the kernel test loop), so the checks above skip it
# entirely. The desktop app is our PRIMARY testing target, yet a bug in the
# host crate — a broken menu wiring, a bad command signature, an invalid
# tauri.conf.json or capability file — only surfaces at `tauri dev`/`build`
# time unless we check it here. So hold it to the same fmt + clippy bar as the
# workspace.
#
# These run AFTER the app build so that `app/dist` exists: tauri's
# generate_context! macro reads frontendDist at compile time, and tauri-build's
# build script validates tauri.conf.json + capabilities/*.json on the way
# through. A clean clippy here therefore also means the config is well-formed.
# ---------------------------------------------------------------------------
TAURI_MANIFEST="shells/tauri/src-tauri/Cargo.toml"

echo "=== cargo fmt --check (tauri shell) ==="
cargo fmt --check --manifest-path "$TAURI_MANIFEST"

echo "=== cargo clippy --all-targets -- -D warnings (tauri shell) ==="
cargo clippy --manifest-path "$TAURI_MANIFEST" --all-targets -- -D warnings

echo "verify: all green"
