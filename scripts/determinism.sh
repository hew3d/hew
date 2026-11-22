#!/usr/bin/env bash
# The determinism golden (docs/dev/DEVELOPMENT.md, "The determinism golden").
#
# Rebuilds every model under corpus/curves/ and corpus/determinism/ through
# the Hew API, hashes each document's state and tessellation, and compares
# with corpus/determinism/golden.tsv.
#
#   scripts/determinism.sh                  rewrite the golden from this run
#   scripts/determinism.sh --check          fail, listing every differing row,
#                                           if this run differs from the golden
#   scripts/determinism.sh --out <file>     write this run's table to <file>
#                                           and exit 0, to diff another
#                                           platform's table against the golden
#
# The blend commands are Preview features, so this is a preview build. It
# uses the gate's preview target directory, so the two share a cache.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
# A relative --out path is the caller's, so remember where they stood.
CALLER_DIR="$PWD"
cd "$(dirname "$0")/.."

case "${CARGO_TARGET_DIR:-}" in
  "") PREVIEW_TARGET="$PWD/target/preview" ;;
  /*) PREVIEW_TARGET="$CARGO_TARGET_DIR/preview" ;;
  *) PREVIEW_TARGET="$PWD/$CARGO_TARGET_DIR/preview" ;;
esac

update=1
out=""
case "${1:-}" in
  "") ;;
  --check) update=0 ;;
  --out)
    [ -n "${2:-}" ] || { echo "usage: scripts/determinism.sh --out <file>" >&2; exit 2; }
    update=0
    case "$2" in /*) out="$2" ;; *) out="$CALLER_DIR/$2" ;; esac
    ;;
  *) echo "usage: scripts/determinism.sh [--check | --out <file>]" >&2; exit 2 ;;
esac

# The modes are exclusive: an inherited variable must not turn a check into a
# rewrite or into a run that passes whatever the golden says.
unset HEW_DETERMINISM_OUT UPDATE_DETERMINISM

if [ -n "$out" ]; then
  HEW_PREVIEW=1 CARGO_TARGET_DIR="$PREVIEW_TARGET" HEW_DETERMINISM_OUT="$out" \
    cargo test -p api --test determinism_golden -- --nocapture the_determinism_golden_matches_the_checked_in_table
else
  HEW_PREVIEW=1 CARGO_TARGET_DIR="$PREVIEW_TARGET" UPDATE_DETERMINISM="$update" \
    cargo test -p api --test determinism_golden -- --nocapture the_determinism_golden_matches_the_checked_in_table
fi
