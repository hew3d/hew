#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

# Change to repo root
cd "$(dirname "$0")/.."

# ---------------------------------------------------------------------------
# The Preview wall (docs/dev/DEVELOPMENT.md "Preview features"): every lane
# that depends on the build flag runs in BOTH configurations. A production
# build is the one that protects regular users, so a production step is
# never inferred from the caller's shell: HEW_PREVIEW is removed from the
# environment here, and only the preview steps set it, explicitly, as
# `HEW_PREVIEW=1 <command>`. Nothing below skips the production half.
# ---------------------------------------------------------------------------
unset HEW_PREVIEW
# The crates whose code the flag changes: crates/api (where cfg(hew_preview)
# is set) and everything that links it. Every other crate builds the same
# either way, so its tests run once.
preview_crates() {
  cargo tree -i api --workspace -e normal,build,dev --prefix none \
    | sed -E 's/ v[0-9].*//' | sort -u | sed 's/^/-p /' | tr '\n' ' '
}

# Lanes. With no argument this runs the whole gate in order, as it always has.
# CI runs the same lanes as separate parallel jobs (.github/workflows/ci.yml),
# so each lane here is the single source of truth for what that job checks:
#
#   rust    packaging tests, cargo fmt, clippy, cargo test; the production
#           build for the workspace, then the preview build for the crates
#           the flag changes
#   wasm    build the kernel's two WASM flavors (scripts/wasm-flavors.sh):
#           the production module optimized, as it ships (CI builds that as
#           the `wasm-release` job; its test lanes use unoptimized builds of
#           both flavors from the `wasm` job instead), and the preview module
#           unoptimized, for the tests only
#   web     typecheck, lint, i18n, vitest in both builds (each held to
#           app/scripts/check-unit-skips.mjs), then the production app build
#           and the site (needs both flavors in target/wasm-flavors; leaves
#           app/src/wasm/pkg as the preview module, the dev default)
#   tauri   Tauri host crate fmt, then clippy and tests in both builds
#           (needs app/dist from the web lane)
#   wall    the production build's wall tests alone (Rust and Tauri): the
#           cheap part of the production half, for a caller that skips the
#           rest (scripts/verify-full.sh SKIP_VERIFY=1)
LANE="${1:-all}"
case "$LANE" in
  all|rust|wasm|web|tauri|wall) ;;
  *) echo "usage: scripts/verify.sh [rust|wasm|web|tauri|wall]" >&2; exit 2 ;;
esac
lane() { [ "$LANE" = "all" ] || [ "$LANE" = "$1" ]; }

# ---------------------------------------------------------------------------
# Toolchain drift guard.
#
# rustup enforces rust-toolchain.toml by itself, and pnpm enforces the
# `packageManager` field by itself, but nothing enforces wasm-pack — and it
# builds the kernel this gate then tests. A local wasm-pack that differs from
# the pinned one means this gate is not testing what CI will. Offline: a
# version string comparison, no network.
# ---------------------------------------------------------------------------
if lane wasm; then
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
fi

if lane rust; then
  echo "=== Flatpak packaging command tests ==="
  python3 -B -m unittest discover -s packaging/tests -v

  echo "=== cargo fmt --check ==="
  cargo fmt --check

  echo "=== cargo clippy --workspace --all-targets -- -D warnings (production) ==="
  cargo clippy --workspace --all-targets -- -D warnings

  echo "=== cargo test --workspace (production) ==="
  cargo test --workspace

  PREVIEW_CRATES="$(preview_crates)"
  echo "=== HEW_PREVIEW=1 cargo clippy $PREVIEW_CRATES--all-targets -- -D warnings (preview) ==="
  # shellcheck disable=SC2086 # one -p per crate
  HEW_PREVIEW=1 cargo clippy $PREVIEW_CRATES --all-targets -- -D warnings

  echo "=== HEW_PREVIEW=1 cargo test $PREVIEW_CRATES(preview) ==="
  # shellcheck disable=SC2086
  HEW_PREVIEW=1 cargo test $PREVIEW_CRATES
fi

# Only when asked for by name: the rust and tauri lanes already run these.
if [ "$LANE" = "wall" ]; then
  echo "=== production wall tests (Rust) ==="
  cargo test -p api --test preview_wall
  cargo test -p hew-cli --test preview_wall
  cargo test -p wasm-api --lib preview_wall
  echo "=== production wall tests (Tauri native menu) ==="
  node shells/tauri/scripts/stage-cli-sidecar.mjs
  cargo test --manifest-path shells/tauri/src-tauri/Cargo.toml preview_menu
fi

if lane wasm; then
  # Each flavor is built once and copied into place by the lanes that use
  # it. The production module is optimized because the web lane's app build
  # ships it; the preview module only serves the tests, so it skips wasm-opt
  # (most of a wasm-pack build's time).
  echo "=== WASM, production flavor (optimized) ==="
  scripts/wasm-flavors.sh build production web
  echo "=== WASM, preview flavor (HEW_PREVIEW=1, no wasm-opt) ==="
  scripts/wasm-flavors.sh build preview web --no-opt
  # The dev server's default is the preview build.
  scripts/wasm-flavors.sh use preview
fi

if lane web; then
  # One command per line, no && lists: a && list's failure was observed once
  # being carried past under set -e (typecheck errors logged, gate still
  # printed "all green" — cause not reproduced). Separate simple commands
  # leave errexit nothing to misjudge, and each step gets its own header.
  echo "=== pnpm --dir app typecheck ==="
  pnpm --dir app typecheck

  echo "=== pnpm --dir app typecheck:test ==="
  pnpm --dir app typecheck:test

  echo "=== pnpm --dir app lint ==="
  pnpm --dir app lint

  # Localization (docs/LOCALIZATION.md): the committed catalogs must match the
  # source — a wrapped string with no catalog entry, or a stale entry, fails
  # here rather than reaching a translator — and every SHIPPED locale's
  # catalog must be complete.
  echo "=== pnpm --dir app i18n:extract (catalogs in sync) ==="
  pnpm --dir app i18n:extract
  # `git diff` sees modified tracked files; the second test catches a catalog
  # extract just CREATED (a new locale's .po), which `git diff` never reports.
  if ! git diff --quiet --exit-code -- app/src/locales || git status --porcelain --untracked-files=all -- app/src/locales | grep -q '^??'; then
    echo "verify: FAILED — message catalogs are out of date. Commit the result of:"
    echo "  pnpm --dir app i18n:extract"
    git --no-pager diff --stat -- app/src/locales
    exit 1
  fi

  echo "=== pnpm --dir app i18n:check (shipped catalogs complete) ==="
  pnpm --dir app i18n:check

  echo "=== pnpm --dir app i18n:menus --check (native menu labels current) ==="
  pnpm --dir app i18n:menus --check

  # The unit suites in both builds, each against its own WASM flavor (the
  # app's kernel loader refuses a module of the other flavor). Each run is
  # held to the skip rule by app/scripts/check-unit-skips.mjs: the preview
  # run skips nothing, the production run skips only what the Preview wall
  # marks ("preview-wall:"), and neither may fail. The production run goes
  # last so the app build below, a production build on the production
  # module, starts from it.
  UNIT_REPORT="$(mktemp -t hew-unit-XXXXXX)"
  for build in preview production; do
    if [ "$build" = preview ]; then flag=1; else flag=0; fi
    echo "=== HEW_PREVIEW=$flag vitest run ($build) ==="
    scripts/wasm-flavors.sh use "$build"
    HEW_PREVIEW="$flag" pnpm --dir app exec vitest run --reporter=default --reporter=json --outputFile="$UNIT_REPORT"
    node app/scripts/check-unit-skips.mjs "$UNIT_REPORT" "$build"
  done
  rm -f "$UNIT_REPORT"

  echo "=== pnpm --dir app build (production) ==="
  pnpm --dir app build
  # Back to the preview module, the dev server's default build.
  scripts/wasm-flavors.sh use preview

  echo "=== pnpm --dir site check ==="
  pnpm --dir site check

  echo "=== pnpm --dir site build ==="
  pnpm --dir site build
fi

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
if lane tauri; then
  TAURI_MANIFEST="shells/tauri/src-tauri/Cargo.toml"

  # tauri.conf.json declares bundle.externalBin (the hew-cli sidecar), which
  # makes tauri-build's build script hard-fail the moment this crate is
  # compiled — not just bundled — if the matching binaries/hew-cli-<host
  # triple> file isn't already on disk. Stage it before the fmt/clippy below.
  echo "=== Stage hew-cli sidecar ==="
  node shells/tauri/scripts/stage-cli-sidecar.mjs

  echo "=== cargo fmt --check (tauri shell) ==="
  cargo fmt --check --manifest-path "$TAURI_MANIFEST"

  echo "=== cargo clippy --all-targets -- -D warnings (tauri shell, production) ==="
  cargo clippy --manifest-path "$TAURI_MANIFEST" --all-targets -- -D warnings

  echo "=== cargo test (tauri shell, production) ==="
  cargo test --manifest-path "$TAURI_MANIFEST"

  echo "=== HEW_PREVIEW=1 cargo clippy --all-targets -- -D warnings (tauri shell, preview) ==="
  HEW_PREVIEW=1 cargo clippy --manifest-path "$TAURI_MANIFEST" --all-targets -- -D warnings

  echo "=== HEW_PREVIEW=1 cargo test (tauri shell, preview) ==="
  HEW_PREVIEW=1 cargo test --manifest-path "$TAURI_MANIFEST"
fi

echo "verify: all green ($LANE)"
