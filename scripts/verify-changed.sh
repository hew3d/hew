#!/usr/bin/env bash
#
# The quick check for work in progress: run only what the current change can
# reach. scripts/verify.sh is still the gate a change must pass before it
# merges; this is the check to run after each fix, in a few minutes or less,
# so the gate itself is saved for a finished batch.
#
#   scripts/verify-changed.sh [base]      (also: scripts/verify.sh --changed)
#
# "The change" is everything that differs from the merge base with `base`
# (default: main): committed, staged, unstaged and untracked. What runs:
#
#   crates/**, Cargo.*, toolchain,  cargo fmt; clippy and tests for the
#   clippy.toml, shared fixtures    changed crates and every crate that
#                                   depends on them (scripts/affected-crates.mjs).
#                                   In the preview build: clippy for every
#                                   affected crate the Preview flag changes
#                                   (so preview-only code still compiles
#                                   against an edited kernel), and their
#                                   tests when one of them was itself edited.
#                                   A kernel edit also checks the curves
#                                   scoreboard, which only that build has.
#   corpus/curves/**                the same, as a change to crates/api
#   shells/tauri/**                 the gate's whole tauri lane
#   app/**, the JS lock file, or    typecheck, lint, the unit tests related to
#   the kernel's API surface        the changed files (all of them when the
#   (crates/api, wasm-api)          surface, the dependencies or the app's
#                                   config changed), and the catalogs when
#                                   app/src changed
#   anything the browser loads      the smoke specs plus every changed spec
#   (app/src, e2e, public, ...)     (not the visual goldens, which only CI's
#                                   pinned runner can judge), in chromium,
#                                   preview build
#   site/**, packaging/**,          that area's own checks
#   workers/**, crates/hew-relay
#   *.sh                            bash -n
#
# What it deliberately does NOT run, and the gate does: tests of crates the
# change does not reach, the production half of the unit suites, the unit
# tests against a kernel rebuilt from a kernel-only change, the app and site
# builds, and the rest of the E2E suite. A green run here is a good sign, not
# a pass.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

# Production steps run with the flag absent; preview steps set it themselves
# (see scripts/verify.sh).
unset HEW_PREVIEW

# The specs every browser-reaching change runs (scripts/e2e-smoke-specs.txt).
SMOKE_SPECS="$(grep -vE '^[[:space:]]*(#|$)' scripts/e2e-smoke-specs.txt | tr '\n' ' ')"
SMOKE_SPECS="${SMOKE_SPECS% }"
for spec in $SMOKE_SPECS; do
  if [ ! -f "app/$spec" ]; then
    echo "verify-changed: scripts/e2e-smoke-specs.txt lists app/$spec, which does not exist" >&2
    exit 2
  fi
done
# Spec files run in parallel here. The whole suite keeps one worker
# (app/playwright.config.ts); a handful of files on a developer machine does
# not contend the way the whole suite on a small runner does.
E2E_WORKERS="${HEW_E2E_WORKERS:-4}"

BASE="${1:-main}"
if ! MERGE_BASE="$(git merge-base HEAD "$BASE" 2>/dev/null)"; then
  echo "verify-changed: no merge base with '$BASE'. Usage: scripts/verify-changed.sh [base]" >&2
  exit 2
fi
# --no-renames: a moved file counts at both ends, so the area it left is
# checked too. core.quotePath=false: a non-ASCII name comes out as itself.
CHANGED="$( {
  git -c core.quotePath=false diff --name-only --no-renames "$MERGE_BASE"
  git -c core.quotePath=false ls-files --others --exclude-standard
} | sort -u)"

has() { grep -qE "$1" <<<"$CHANGED"; }
# The changed paths matching a pattern that still exist (a deleted file has
# nothing to run).
existing() {
  local f
  while IFS= read -r f; do
    if [ -n "$f" ] && [ -e "$f" ]; then echo "$f"; fi
  done < <(grep -E "$1" <<<"$CHANGED" || true)
}

# The root of the JS workspace: the app and the site resolve their
# dependencies from here.
JS_DEPS=0
if has '^(pnpm-lock\.yaml|pnpm-workspace\.yaml|package\.json|\.node-version)$'; then JS_DEPS=1; fi

# Playwright reuses a dev server it finds on its port, and every checkout
# defaults to the same one, so a server left running by another worktree
# would be tested in place of this one. Give each checkout its own port.
if [ -z "${HEW_E2E_PORT:-}" ]; then
  export HEW_E2E_PORT=$(( 4200 + $(cksum <<<"$PWD" | cut -d' ' -f1) % 700 ))
fi

RAN=()
step() { echo "=== $* ==="; RAN+=("$*"); }

case "${CARGO_TARGET_DIR:-}" in
  "") PREVIEW_TARGET="$PWD/target/preview" ;;
  /*) PREVIEW_TARGET="$CARGO_TARGET_DIR/preview" ;;
  *) PREVIEW_TARGET="$PWD/$CARGO_TARGET_DIR/preview" ;;
esac

if [ -z "$CHANGED" ]; then
  echo "verify-changed: nothing differs from $BASE"
  exit 0
fi
echo "verify-changed: $(wc -l <<<"$CHANGED" | tr -d ' ') changed file(s) against $BASE (${MERGE_BASE:0:8})"

# ---------------------------------------------------------------------------
# Rust workspace
# ---------------------------------------------------------------------------
# The app-side paths and corpus/ are fixtures that Rust tests read across
# crate lines (scripts/affected-crates.mjs has the list).
if has '^(crates/|Cargo\.toml$|Cargo\.lock$|rust-toolchain\.toml$|clippy\.toml$|\.?rustfmt\.toml$|\.cargo/|preview-features\.json$|app/e2e/fixtures/|app/public/samples/|corpus/)'; then
  step "cargo fmt --check"
  cargo fmt --check

  AFFECTED="$(node scripts/affected-crates.mjs <<<"$CHANGED")"
  if [ -n "$AFFECTED" ]; then
    PKGS=()
    while IFS= read -r crate; do PKGS+=(-p "$crate"); done <<<"$AFFECTED"

    step "cargo clippy ($(wc -l <<<"$AFFECTED" | tr -d ' ') affected crates) --all-targets -- -D warnings"
    cargo clippy "${PKGS[@]}" --all-targets -- -D warnings

    step "cargo test (affected crates: $(tr '\n' ' ' <<<"$AFFECTED" | sed 's/ $//'))"
    cargo test "${PKGS[@]}"

    # The preview half, for the crates the flag changes (crates/api and what
    # links it). Clippy runs for every affected one: code behind
    # cfg(hew_preview) is not compiled by the production steps above, so an
    # edit below it (a kernel signature, say) can break it unseen. Their
    # tests run only when one of them was itself edited; a crate that is
    # merely downstream of the edit runs the same tests in both builds.
    FLAGGED="$(cargo tree -i api --workspace -e normal,build,dev --prefix none | sed -E 's/ v[0-9].*//' | LC_ALL=C sort -u)"
    OWNERS="$(node scripts/affected-crates.mjs --owners <<<"$CHANGED")"
    PREVIEW_AFFECTED="$(comm -12 <(echo "$FLAGGED") <(LC_ALL=C sort <<<"$AFFECTED"))"
    PREVIEW_EDITED="$(comm -12 <(echo "$FLAGGED") <(LC_ALL=C sort <<<"$OWNERS"))"
    if [ -n "$PREVIEW_AFFECTED" ]; then
      PPKGS=()
      while IFS= read -r crate; do PPKGS+=(-p "$crate"); done <<<"$PREVIEW_AFFECTED"
      step "HEW_PREVIEW=1 cargo clippy ($(tr '\n' ' ' <<<"$PREVIEW_AFFECTED" | sed 's/ $//'))"
      HEW_PREVIEW=1 CARGO_TARGET_DIR="$PREVIEW_TARGET" cargo clippy "${PPKGS[@]}" --all-targets -- -D warnings
      if [ -n "$PREVIEW_EDITED" ]; then
        step "HEW_PREVIEW=1 cargo test ($(tr '\n' ' ' <<<"$PREVIEW_AFFECTED" | sed 's/ $//'))"
        HEW_PREVIEW=1 CARGO_TARGET_DIR="$PREVIEW_TARGET" cargo test "${PPKGS[@]}"
      elif has '^crates/kernel/'; then
        # The one preview-only test a kernel edit can change the answer of:
        # the curves scoreboard exists in no production build, so the
        # production tests above do not stand in for it.
        step "scripts/scoreboard.sh --check"
        scripts/scoreboard.sh --check
      fi
    fi
  fi
fi

# ---------------------------------------------------------------------------
# Web app
# ---------------------------------------------------------------------------
# The kernel's API surface: a change here can change the generated types and
# what every unit test sees through the module.
SURFACE=0
if has '^(crates/(api|wasm-api)/|preview-features\.json$)'; then SURFACE=1; fi

if has '^(crates/api/|sdk/plugin/)'; then
  step "pnpm --dir sdk/plugin check (boot script current; fix: pnpm --dir sdk/plugin build)"
  pnpm --dir sdk/plugin check
fi

if has '^app/' || [ "$SURFACE" = 1 ] || [ "$JS_DEPS" = 1 ]; then
  # The preview module, rebuilt only when it is missing or older than the
  # kernel sources (app/scripts/build-wasm.mjs, as `pnpm --dir app dev` does).
  step "kernel module for the app (preview flavor, rebuilt only if stale)"
  HEW_PREVIEW=1 node app/scripts/build-wasm.mjs --dev

  step "pnpm --dir app typecheck"
  pnpm --dir app typecheck
  step "pnpm --dir app typecheck:test"
  pnpm --dir app typecheck:test
  step "pnpm --dir app lint"
  pnpm --dir app lint

  if has '^app/src/'; then
    # The gate compares the catalogs with what is committed. Work in
    # progress may carry catalog edits that are not committed yet, so here
    # the test is that a fresh extract moves nothing further.
    step "message catalogs in sync and complete"
    locales_sum() { find app/src/locales -type f | LC_ALL=C sort | xargs shasum | shasum; }
    LOCALES_BEFORE="$(locales_sum)"
    pnpm --dir app i18n:extract
    if [ "$(locales_sum)" != "$LOCALES_BEFORE" ]; then
      echo "verify-changed: FAILED — message catalogs were out of date; the extract just updated them."
      echo "Review and keep the result:"
      git --no-pager diff --stat -- app/src/locales
      exit 1
    fi
    pnpm --dir app i18n:check
    pnpm --dir app i18n:menus --check
  fi

  if [ "$SURFACE" = 1 ] || [ "$JS_DEPS" = 1 ] || has '^app/(package\.json|pnpm-lock\.yaml|vite\.config\.ts|vitest\.config\.ts|tsconfig[^/]*\.json|src/test/)'; then
    step "HEW_PREVIEW=1 vitest run (all unit tests, preview)"
    HEW_PREVIEW=1 pnpm --dir app exec vitest run
  else
    RELATED=()
    while IFS= read -r f; do RELATED+=("${f#app/}"); done < <(existing '^app/(src|e2e)/.*\.tsx?$' | grep -v '\.spec\.ts$' || true)
    if [ "${#RELATED[@]}" -gt 0 ]; then
      step "HEW_PREVIEW=1 vitest related (unit tests reaching ${#RELATED[@]} changed file(s), preview)"
      HEW_PREVIEW=1 pnpm --dir app exec vitest related --run "${RELATED[@]}"
    fi
  fi
fi

# ---------------------------------------------------------------------------
# Browser: the smoke specs plus every changed spec (not the visual goldens)
# ---------------------------------------------------------------------------
if has '^app/(src/|e2e/|public/|index\.html$|package\.json$|vite\.config\.ts$|playwright\.config\.ts$)' || [ "$SURFACE" = 1 ] || [ "$JS_DEPS" = 1 ]; then
  SPECS=()
  for spec in $SMOKE_SPECS; do SPECS+=("$spec"); done
  # The visual goldens only mean anything on CI's pinned runner.
  while IFS= read -r f; do
    spec="${f#app/}"
    case " $SMOKE_SPECS " in *" $spec "*) ;; *) SPECS+=("$spec") ;; esac
  done < <(existing '^app/e2e/.*\.spec\.ts$' | grep -v '^app/e2e/visual/' || true)

  step "web E2E (chromium, preview): smoke + changed specs, ${#SPECS[@]} file(s), $E2E_WORKERS workers"
  pnpm --dir app exec playwright install chromium
  HEW_PREVIEW=1 pnpm --dir app exec playwright test --project=chromium --forbid-only --workers="$E2E_WORKERS" "${SPECS[@]}"
fi

# ---------------------------------------------------------------------------
# Everything else with a check of its own
# ---------------------------------------------------------------------------
if has '^shells/tauri/'; then
  if [ ! -d app/dist ]; then
    echo "verify-changed: FAILED — the Tauri shell changed, and checking it needs a built app in app/dist."
    echo "Build one (it is reused afterward) with:"
    echo "  scripts/verify.sh wasm && scripts/verify.sh web"
    exit 1
  fi
  step "scripts/verify.sh tauri"
  scripts/verify.sh tauri
fi

if has '^site/' || [ "$JS_DEPS" = 1 ]; then
  step "pnpm --dir site check"
  pnpm --dir site check
fi

# The packaging tests also read the updater-manifest script and the shell's
# install-channel module.
if has '^(packaging/|\.github/scripts/build-updater-manifest\.py$|shells/tauri/src-tauri/src/install_channel\.rs$)'; then
  step "Flatpak packaging command tests"
  python3 -B -m unittest discover -s packaging/tests
fi

if has '^(workers/share-relay/|crates/hew-relay/|scripts/relay-contract\.sh$)'; then
  step "scripts/relay-contract.sh"
  scripts/relay-contract.sh
fi

if has '^workers/bug-intake/'; then
  step "bug-intake Worker tests"
  ( cd workers/bug-intake && { [ -d node_modules ] || npm ci; } && npm test )
fi

SCRIPTS=()
while IFS= read -r f; do SCRIPTS+=("$f"); done < <(existing '\.sh$')
if [ "${#SCRIPTS[@]}" -gt 0 ]; then
  step "bash -n (${#SCRIPTS[@]} changed script(s))"
  for f in "${SCRIPTS[@]}"; do bash -n "$f"; done
fi

echo
if [ "${#RAN[@]}" -eq 0 ]; then
  echo "verify-changed: nothing to run for this change (${SECONDS}s)"
else
  echo "verify-changed: green in ${SECONDS}s. Ran:"
  for s in "${RAN[@]}"; do echo "  - $s"; done
fi
echo "This is the quick check, not the gate: run scripts/verify.sh before merging."
