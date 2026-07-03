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

# ---------------------------------------------------------------------------
# skp-import (clean-room.skp reader integration).
#
# NOT a workspace member: it path-depends on the sibling../OpenSKP repo,
# which has no hosted remote yet, so CI cannot resolve the dependency and the
# workspace checks above skip it. Gate it here whenever the sibling checkout
# is present (every dev box); CI skips silently. Flip to a rev-pinned git dep
# + workspace membership once OpenSKP is hosted on GitHub.
# ---------------------------------------------------------------------------
if [ -d "../OpenSKP/crates/skp" ]; then
  SKP_MANIFEST="crates/skp-import/Cargo.toml"

  echo "=== cargo fmt --check (skp-import) ==="
  cargo fmt --check --manifest-path "$SKP_MANIFEST"

  echo "=== cargo clippy --all-targets -- -D warnings (skp-import) ==="
  cargo clippy --manifest-path "$SKP_MANIFEST" --all-targets -- -D warnings

  echo "=== cargo test (skp-import) ==="
  cargo test --manifest-path "$SKP_MANIFEST"
fi

echo "verify: all green"
