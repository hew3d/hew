#!/usr/bin/env bash
# The curves scoreboard (docs/dev/DEVELOPMENT.md, "The curves scoreboard").
#
# Rebuilds every model under corpus/curves/ through the Hew API, runs every
# case at every size, rewrites corpus/curves/SCOREBOARD.tsv, and prints what
# changed since the last run, cases that got worse first.
#
#   scripts/scoreboard.sh           run, rewrite the table, print the comparison
#   scripts/scoreboard.sh --check   run and fail if the table would change
#
# The blend commands are Preview features, so this is a preview build. It
# uses the gate's preview target directory, so the two share a cache.
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"
cd "$(dirname "$0")/.."

case "${CARGO_TARGET_DIR:-}" in
  "") PREVIEW_TARGET="$PWD/target/preview" ;;
  /*) PREVIEW_TARGET="$CARGO_TARGET_DIR/preview" ;;
  *) PREVIEW_TARGET="$PWD/$CARGO_TARGET_DIR/preview" ;;
esac

update=1
case "${1:-}" in
  "") ;;
  --check) update=0 ;;
  *) echo "usage: scripts/scoreboard.sh [--check]" >&2; exit 2 ;;
esac

HEW_PREVIEW=1 CARGO_TARGET_DIR="$PREVIEW_TARGET" UPDATE_SCOREBOARD="$update" \
  cargo test -p api --test curves_scoreboard -- --nocapture the_scoreboard_matches_the_checked_in_table
