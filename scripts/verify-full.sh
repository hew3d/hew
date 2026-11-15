#!/usr/bin/env bash
#
# Comprehensive pre-PUSH gate. scripts/verify.sh is the fast, dependency-light
# gate run before every commit; it deliberately omits the browser- and
# runner-heavy checks so it stays quick and cross-platform. That leaves a class
# of failures (Playwright E2E, the replay gate) that only surface in CI.
#
# This script closes most of that gap: it runs verify.sh AND the blocking lanes
# of the CI lanes (.github/workflows/ci.yml) that CAN run off the runner,
# in the same order. Run it before pushing. A green run here predicts a green CI
# run except for the visual goldens — see below, and take that exception
# seriously if you touched the render path.
#
#   verify.sh            fmt, clippy, cargo test, wasm (web), vitest, builds, tauri
#   cargo-deny           advisories (incl. yanked crates) for both cargo trees
#   replay gate          node-target wasm + golden state-hash fixtures, on
#                        the production and the preview module
#   web E2E (chromium)   Playwright — the blocking functional browser lane,
#                        in the preview build (flag on, the curve specs
#                        included) and in the production-shape build (flag
#                        off, the Preview wall spec included)
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
#                   after verify.sh already passed). The production build's
#                   wall tests still run (verify.sh wall), and nothing here
#                   skips a production-build lane: the Preview wall's
#                   production half has no knob (docs/dev/DEVELOPMENT.md
#                   "Preview features").
#
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

# Change to repo root
cd "$(dirname "$0")/.."

# Production steps run with the flag absent, whatever the caller's shell
# says; preview steps set it explicitly (see scripts/verify.sh).
unset HEW_PREVIEW

if [ "${SKIP_VERIFY:-0}" = "1" ]; then
  echo "=== SKIP_VERIFY=1 — skipping scripts/verify.sh except its production wall tests ==="
  scripts/verify.sh wall
  # The E2E lanes below need both WASM flavors; build them if verify.sh's
  # wasm lane has not.
  [ -f target/wasm-flavors/production/web/preview-build ] || scripts/wasm-flavors.sh build production web --no-opt
  [ -f target/wasm-flavors/preview/web/preview-build ] || scripts/wasm-flavors.sh build preview web --no-opt
else
  echo "=== scripts/verify.sh (standard gate) ==="
  scripts/verify.sh
fi

# Dependency advisories — mirrors audit.yml's cargo-deny matrix, advisories
# section only (the job also runs bans, licenses and sources, which are not
# gated here). Advisories include yanked crates, and a yank can land on a lock
# that was green last week, so this must run before push, not only in the
# daily Audit workflow.
#
# cargo-deny reads yank status from cargo's sparse-index cache and never
# revalidates it, so a cached entry from before a yank passes locally while
# CI fails. There is no cargo-deny flag to force a refresh. Removing the index
# cache makes cargo refetch the entries the check needs: slower, but correct.
# The cache is only a cache, and cargo rebuilds it.
echo "=== cargo-deny (advisories, both trees) ==="
DENY_VERSION="0.20.2"
HAVE_DENY="$(cargo-deny --version 2>/dev/null | awk '{print $2}' || true)"
if [ "$HAVE_DENY" != "$DENY_VERSION" ]; then
  echo "verify-full: FAILED — cargo-deny is '${HAVE_DENY:-not installed}', pinned is '$DENY_VERSION'."
  echo "Install the pinned version with:"
  echo "  cargo install cargo-deny --version $DENY_VERSION --locked"
  exit 1
fi
rm -rf "${CARGO_HOME:-$HOME/.cargo}"/registry/index/*/.cache
cargo deny --config deny.toml --all-features check advisories
cargo deny --manifest-path shells/tauri/src-tauri/Cargo.toml --config deny.toml --all-features check advisories

# Replay gate — mirrors ci.yml. The runner consumes `--target nodejs`
# (CommonJS) bindings, a build distinct from verify.sh's `--target web` app
# build, unoptimized as in CI (the replay checks state hashes, not size).
# Both modules: a recording replays the same in either (the kernel is never
# gated), except a recorded live-API frame of a preview command, which the
# production module refuses.
for flavor in production preview; do
  echo "=== build kernel for Node (replay runner, $flavor) ==="
  scripts/wasm-flavors.sh build "$flavor" nodejs --no-opt
  scripts/wasm-flavors.sh use-node "$flavor"
  echo "=== replay gate (golden state-hash fixtures, $flavor) ==="
  node tools/replay-runner/run.mjs
done

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
#
# The Preview wall adds the one argued-for exception, in both builds: a spec
# that drives a Preview feature skips in the production-shape run, and each
# half of the wall spec skips in the other build. Those skips come only from
# the app's registry-driven helpers (app/e2e/helpers/preview.ts), whose
# reasons carry the "preview-wall:" mark; scripts/check-e2e-skips.mjs fails
# any other skip, and fails the run if the wall spec passed nothing.
#
# Two runs, each against its own build and WASM flavor:
#   preview      HEW_PREVIEW=1: the Vite dev server, flag on; the curve specs
#   production   HEW_PREVIEW=0: `vite build` with the flag off, served as a
#                release is (app/playwright.config.ts); the wall spec. This
#                run has no skip knob.
E2E_REPORT="$(mktemp -t hew-e2e-XXXXXX)"
trap 'rm -f "$E2E_REPORT"' EXIT
for build in preview production; do
  echo "=== web E2E smoke (chromium, $build build) ==="
  scripts/wasm-flavors.sh use "$build"
  if [ "$build" = preview ]; then flag=1; else flag=0; fi
  HEW_PREVIEW="$flag" PLAYWRIGHT_JSON_OUTPUT_FILE="$E2E_REPORT" \
    pnpm --dir app exec playwright test --project=chromium --forbid-only --reporter=list,json
  node scripts/check-e2e-skips.mjs "$E2E_REPORT" "$build" --wall
done
# Leave the slot as the preview module, the dev server's default, as
# verify.sh does.
scripts/wasm-flavors.sh use preview

# Relay contract — mirrors ci.yml's `relay-contract` job exactly (same
# script). Starts and stops its own local servers on the 187xx/1879x ports.
echo "=== relay contract (Worker + hew-relay) ==="
scripts/relay-contract.sh

echo "verify-full: all green"
