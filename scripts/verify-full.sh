#!/usr/bin/env bash
#
# Comprehensive pre-PUSH gate. scripts/verify.sh is the fast, dependency-light
# gate run before every commit; it deliberately omits the browser- and
# runner-heavy checks so it stays quick and cross-platform. That leaves a class
# of failures (Playwright E2E, the replay gate) that only surface in CI.
#
# This script closes that gap: it runs verify.sh AND the remaining *blocking*
# lanes of the CI `verify` job (.github/workflows/ci.yml), in the same order,
# so a green run here predicts a green CI run. Run it before pushing.
#
#   verify.sh            fmt, clippy, cargo test, wasm (web), vitest, builds, tauri
#   replay gate          node-target wasm + golden state-hash fixtures
#   web E2E (chromium)   Playwright — the one blocking browser lane
#
# Deliberately NOT covered (none can fail the push, or none can run here):
#   - web E2E webkit / visual goldens: CI marks them continue-on-error
#     (non-blocking), and the visual goldens are runner-GPU-specific — they
#     would false-fail off CI's pinned runner. Run webkit yourself when
#     touching the render path:
#       pnpm --dir app exec playwright test --project=webkit
#   - Desktop E2E (.github/workflows/desktop-e2e.yml): tauri-driver cannot
#     drive the macOS WKWebView, so this lane cannot run on macOS at all. It
#     runs in CI (Linux) and can be run on the Windows VM.
#   - Release (.github/workflows/release.yml): only fires on a v* tag.
#
# Env:
#   SKIP_VERIFY=1   skip the verify.sh leg (e.g. re-running only the E2E lanes
#                   after verify.sh already passed).
#
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

# Change to repo root
cd "$(dirname "$0")/.."

if [ "${SKIP_VERIFY:-0}" = "1" ]; then
  echo "=== SKIP_VERIFY=1 — skipping scripts/verify.sh ==="
else
  echo "=== scripts/verify.sh (standard gate) ==="
  scripts/verify.sh
fi

# Replay gate — mirrors ci.yml. The runner consumes `--target nodejs`
# (CommonJS) bindings, a build distinct from verify.sh's `--target web` app
# build. NB: wasm-pack's --out-dir is relative to the crate dir.
echo "=== build kernel for Node (replay runner) ==="
wasm-pack build crates/wasm-api --target nodejs --out-dir ../../tools/replay-runner/pkg-node

echo "=== replay gate (golden state-hash fixtures) ==="
node tools/replay-runner/run.mjs

# Web E2E smoke — the blocking chromium lane. Playwright starts its own vite
# on HEW_E2E_PORT (default 4173, so it will not collide with a `pnpm dev`
# server on 5173) and installs the browser if missing. Local runs use
# retries=0 (stricter than CI's 1), so a pass here is a conservative predictor
# of CI.
echo "=== install Playwright chromium (idempotent) ==="
pnpm --dir app exec playwright install chromium

echo "=== web E2E smoke (chromium) ==="
pnpm --dir app exec playwright test --project=chromium

echo "verify-full: all green"
