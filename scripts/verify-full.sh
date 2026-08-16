#!/usr/bin/env bash
#
# Comprehensive pre-PUSH gate. scripts/verify.sh is the fast, dependency-light
# gate run before every commit; it deliberately omits the browser- and
# runner-heavy checks so it stays quick and cross-platform. That leaves a class
# of failures (Playwright E2E, the replay gate) that only surface in CI.
#
# This script closes most of that gap: it runs verify.sh AND the blocking lanes
# of the CI `verify` job (.github/workflows/ci.yml) that CAN run off the runner,
# in the same order. Run it before pushing. A green run here predicts a green CI
# run except for the visual goldens — see below, and take that exception
# seriously if you touched the render path.
#
#   verify.sh            fmt, clippy, cargo test, wasm (web), vitest, builds, tauri
#   replay gate          node-target wasm + golden state-hash fixtures
#   web E2E (chromium)   Playwright — the blocking functional browser lane
#   relay contract       scripts/relay-contract.sh — the share-relay Worker's
#                        unit suite plus the black-box conformance suite
#                        against wrangler dev AND the hew-relay binary (CI's
#                        `relay-contract` job)
#
# Deliberately NOT covered (none can fail the push, or none can run here):
#   - web E2E visual goldens: BLOCKING in CI, but genuinely unrunnable here.
#     The goldens are rendered by the ANGLE-SwiftShader inside CI's pinned
#     Chromium on CI's runner; off that host they false-fail on antialiasing
#     alone. This is therefore the one blocking CI lane a green run here does
#     NOT predict — if you touched the render path, or bumped three.js or
#     Playwright, expect CI to have the last word, and regenerate through
#     Actions ▸ Regen Visual Goldens rather than locally.
#   - web E2E webkit: CI marks it continue-on-error (non-blocking), because
#     its WebKitGTK/llvmpipe/Xvfb stack is not pinned by this repository. Run
#     it yourself when touching the render path:
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
# `--forbid-only` fails the run outright if a `.only` was committed, which
# would otherwise silently reduce the suite to a single test while still
# reporting green.
#
# The chromium suite must also finish with ZERO skips, enforced below. A
# skipped test reports green while asserting nothing, and there is no louder
# signal that it did so: `test.skip()` inside a body turns a failing
# precondition into a pass, and a `test.skip(browserName === 'chromium', …)`
# removes the test entirely, because chromium is the ONLY project this gate
# runs. Both shapes were present in this suite, and the second one hid a real
# Chromium/WebView2 double-click defect behind a passing gate for as long as it
# existed. A genuinely unrunnable case belongs behind a capability check that
# FAILS when the capability is missing, or it belongs deleted — either way it
# gets argued for here rather than added quietly.
E2E_LOG="$(mktemp -t hew-e2e-XXXXXX)"
trap 'rm -f "$E2E_LOG"' EXIT
pnpm --dir app exec playwright test --project=chromium --forbid-only | tee "$E2E_LOG"
if grep -qE '[0-9]+ skipped' "$E2E_LOG"; then
  echo
  echo "verify-full: FAILED — the chromium E2E run skipped tests:"
  grep -E '[0-9]+ skipped' "$E2E_LOG"
  echo "A skipped test reports green while testing nothing. Fix the test, or"
  echo "state the case for the skip in scripts/verify-full.sh."
  exit 1
fi

# Relay contract — mirrors ci.yml's `relay-contract` job exactly (same
# script). Starts and stops its own local servers on the 187xx/1879x ports.
echo "=== relay contract (Worker + hew-relay) ==="
scripts/relay-contract.sh

echo "verify-full: all green"
