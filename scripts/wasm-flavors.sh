#!/usr/bin/env bash
#
# The kernel's two WASM flavors (docs/dev/DEVELOPMENT.md "Preview features").
#
# The module embeds crates/api's live API, whose Preview wall is fixed at
# compile time (HEW_PREVIEW=1 -> cfg(hew_preview)), so a production module and
# a preview module are different builds. Each is built ONCE per gate run into
# its own directory, then copied into the slot a consumer reads:
#
#   build <production|preview> <web|nodejs> [--no-opt]
#       wasm-pack into target/wasm-flavors/<flavor>/<target>/, with HEW_PREVIEW
#       set for preview and REMOVED from the environment for production, and
#       a `preview-build` stamp ("1" or "0") beside the package. A --no-opt
#       build also leaves a `no-opt` stamp.
#   use <production|preview>
#       copy the web package into app/src/wasm/pkg (what Vite, Vitest and
#       Playwright load) and freshen its mtimes so app/scripts/build-wasm.mjs
#       does not rebuild over it. A --no-opt package gets that script's
#       `dev-build` mark, so a later plain `pnpm --dir app build` rebuilds
#       rather than bundle an unoptimized kernel (the gate's own app build
#       opts in with HEW_UNOPTIMIZED_WASM=1).
#   use-node <production|preview>
#       copy the Node package into tools/replay-runner/pkg-node.
#
# A missing flavor is an error, never a fallback to whatever is in the slot.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

STASH="target/wasm-flavors"

# A relative CARGO_TARGET_DIR would resolve against wasm-pack's crate
# directory, not this one; make the preview directory absolute from here, as
# scripts/verify.sh does for its cargo steps.
case "${CARGO_TARGET_DIR:-}" in
  "") PREVIEW_TARGET="$PWD/target/preview" ;;
  /*) PREVIEW_TARGET="$CARGO_TARGET_DIR/preview" ;;
  *) PREVIEW_TARGET="$PWD/$CARGO_TARGET_DIR/preview" ;;
esac

die() { echo "wasm-flavors: $*" >&2; exit 1; }

flavor_ok() {
  case "$1" in production|preview) ;; *) die "flavor must be production or preview, not '$1'" ;; esac
}

copy_into() { # <from> <to>
  [ -f "$1/preview-build" ] || die "$1 is not built; run: scripts/wasm-flavors.sh build ..."
  rm -rf "$2"
  mkdir -p "$(dirname "$2")"
  cp -R "$1" "$2"
  find "$2" -type f -exec touch {} +
}

cmd="${1:-}"
case "$cmd" in
  build)
    flavor="${2:-}"; target="${3:-}"; shift 3 || die "usage: build <flavor> <web|nodejs> [--no-opt]"
    flavor_ok "$flavor"
    case "$target" in web|nodejs) ;; *) die "target must be web or nodejs, not '$target'" ;; esac
    out="$STASH/$flavor/$target"
    rm -rf "$out"
    # NB: wasm-pack's --out-dir is relative to the crate dir.
    if [ "$flavor" = preview ]; then
      # Its own cargo target directory, so the two flavors do not overwrite
      # each other's artifacts and an unchanged kernel is not recompiled.
      HEW_PREVIEW=1 CARGO_TARGET_DIR="$PREVIEW_TARGET" wasm-pack build crates/wasm-api --target "$target" "$@" --out-dir "../../$out"
      echo 1 > "$out/preview-build"
    else
      env -u HEW_PREVIEW wasm-pack build crates/wasm-api --target "$target" "$@" --out-dir "../../$out"
      echo 0 > "$out/preview-build"
    fi
    case " $* " in *" --no-opt "*) : > "$out/no-opt" ;; esac
    echo "wasm-flavors: built $flavor/$target"
    ;;
  use)
    flavor="${2:-}"; flavor_ok "$flavor"
    copy_into "$STASH/$flavor/web" app/src/wasm/pkg
    # The mark names the module by its mtime, as app/scripts/build-wasm.mjs
    # reads it, so it is written after copy_into has freshened the files.
    if [ -f app/src/wasm/pkg/no-opt ]; then
      node -e 'const fs = require("fs"); fs.writeFileSync(process.argv[2], String(fs.statSync(process.argv[1]).mtimeMs))' \
        app/src/wasm/pkg/wasm_api_bg.wasm app/src/wasm/pkg/dev-build
    fi
    echo "wasm-flavors: app/src/wasm/pkg is the $flavor module"
    ;;
  use-node)
    flavor="${2:-}"; flavor_ok "$flavor"
    copy_into "$STASH/$flavor/nodejs" tools/replay-runner/pkg-node
    echo "wasm-flavors: tools/replay-runner/pkg-node is the $flavor module"
    ;;
  *)
    die "usage: scripts/wasm-flavors.sh build <production|preview> <web|nodejs> [--no-opt] | use <flavor> | use-node <flavor>"
    ;;
esac
