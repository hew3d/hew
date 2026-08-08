#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

# Change to repo root
cd "$(dirname "$0")/.."

# ---------------------------------------------------------------------------
# Toolchain drift guard.
#
# rustup enforces rust-toolchain.toml by itself, and pnpm enforces the
# `packageManager` field by itself, but nothing enforces wasm-pack — and it
# builds the kernel this gate then tests. A local wasm-pack that differs from
# the pinned one means this gate is not testing what CI will. Offline: a
# version string comparison, no network.
# ---------------------------------------------------------------------------
echo "=== wasm-pack version (pinned) ==="
WANT_WASM_PACK="$(sed -n 's/^WASM_PACK_VERSION="\(.*\)"$/\1/p' scripts/install-wasm-pack.sh)"
HAVE_WASM_PACK="$(wasm-pack --version 2>/dev/null | awk '{print $2}' || true)"
if [ "$HAVE_WASM_PACK" != "$WANT_WASM_PACK" ]; then
  echo "verify: FAILED — wasm-pack is '${HAVE_WASM_PACK:-not installed}', pinned is '$WANT_WASM_PACK'."
  echo "A different wasm-pack builds a different kernel than CI will. Fix with:"
  echo "  scripts/install-wasm-pack.sh"
  exit 1
fi
echo "wasm-pack $HAVE_WASM_PACK"

echo "=== cargo fmt --check ==="
cargo fmt --check

echo "=== cargo clippy --workspace --all-targets -- -D warnings ==="
cargo clippy --workspace --all-targets -- -D warnings

echo "=== cargo test --workspace ==="
cargo test --workspace

echo "=== wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg ==="
wasm-pack build crates/wasm-api --target web --out-dir ../../app/src/wasm/pkg

# One command per line, no && lists: a && list's failure was observed once
# being carried past under set -e (typecheck errors logged, gate still
# printed "all green" — cause not reproduced). Separate simple commands
# leave errexit nothing to misjudge, and each step gets its own header.
echo "=== pnpm --dir app typecheck ==="
pnpm --dir app typecheck

echo "=== pnpm --dir app typecheck:test ==="
pnpm --dir app typecheck:test

echo "=== pnpm --dir app test ==="
pnpm --dir app test

echo "=== pnpm --dir app build ==="
pnpm --dir app build

echo "=== pnpm --dir site check ==="
pnpm --dir site check

echo "=== pnpm --dir site build ==="
pnpm --dir site build

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

# tauri.conf.json declares bundle.externalBin (the hew-cli sidecar), which
# makes tauri-build's build script hard-fail the moment this crate is
# compiled — not just bundled — if the matching binaries/hew-cli-<host
# triple> file isn't already on disk. Stage it before the fmt/clippy below.
echo "=== Stage hew-cli sidecar ==="
node shells/tauri/scripts/stage-cli-sidecar.mjs

echo "=== cargo fmt --check (tauri shell) ==="
cargo fmt --check --manifest-path "$TAURI_MANIFEST"

echo "=== cargo clippy --all-targets -- -D warnings (tauri shell) ==="
cargo clippy --manifest-path "$TAURI_MANIFEST" --all-targets -- -D warnings

echo "verify: all green"
