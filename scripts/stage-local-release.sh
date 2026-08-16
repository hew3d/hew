#!/usr/bin/env bash
#
# Stage a release-shaped directory from the working tree, for testing the
# self-hosting path (docs/SELF_HOSTING.md, scripts/hew-web.sh) BEFORE a
# GitHub release exists — or for seeding an internal mirror. Produces, under
# dist/local-release/ (or $1):
#
#   hew-web-<tag>.tar.gz                    the web app + deploy/ (exactly what
#                                           .github/workflows/release.yml packs)
#   hew-relay-<tag>-linux-x86_64.tar.gz     static musl relay + unit + README
#   hew-relay-<tag>-linux-aarch64.tar.gz    (both architectures, like a release;
#                                           --x86_64-only skips the arm64 leg)
#   latest                                  the tag, for var_version=latest
#
# then prints the one-liner to serve it and the hew-web.sh invocation that
# installs from it (`var_release_base=`). Nothing here talks to GitHub, and
# nothing in the container-side path differs from a real release — the
# assets are laid out and named identically, so what gets tested is the
# real installer, not a special dev mode of it.
#
# The tag defaults to `v<workspace version>-local` so a staged build can
# never be mistaken for a published one; override with TAG=v0.9.0.
#
# Cross-compiling the relay from macOS or any Linux needs only the rustup
# targets (added on demand): hew-relay is pure Rust, and rustc's
# self-contained musl crt plus rust-lld link both x86_64 and aarch64
# without a musl-gcc — the RUSTFLAGS below say so.
#
set -euo pipefail
cd "$(dirname "$0")/.."

OUT=dist/local-release
ARM64=yes
for arg in "$@"; do
  case "$arg" in
    --x86_64-only) ARM64=no ;;
    --*) echo "stage-local-release: unknown option $arg" >&2; exit 2 ;;
    *) OUT="$arg" ;;
  esac
done

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
TAG="${TAG:-v${VERSION}-local}"

mkdir -p "$OUT"
OUT="$(cd "$OUT" && pwd)"

echo "=== web app → $OUT/hew-web-$TAG.tar.gz ==="
pnpm --dir shells/web build
# Same layout as release.yml: dist at the root, deploy/ alongside.
tar -czf "$OUT/hew-web-$TAG.tar.gz" -C shells/web/dist . -C ../ deploy

pack_relay() { # <rust target> <asset arch>
  local target="$1" arch="$2"
  echo "=== hew-relay ($target) → $OUT/hew-relay-$TAG-linux-$arch.tar.gz ==="
  rustup target add "$target" >/dev/null
  RUSTFLAGS="-C linker=rust-lld -C link-self-contained=yes" \
    cargo build --release -p hew-relay --target "$target"
  local bin="target/$target/release/hew-relay"
  file "$bin" | grep -qi 'static' || { echo "stage-local-release: $bin is not statically linked" >&2; file "$bin"; exit 1; }
  local stage
  stage="$(mktemp -d)"
  cp "$bin" "$stage/hew-relay"
  cp shells/web/deploy/hew-relay.service "$stage/"
  cp crates/hew-relay/README.md "$stage/README.md"
  tar -czf "$OUT/hew-relay-$TAG-linux-$arch.tar.gz" -C "$stage" .
  rm -rf "$stage"
}

pack_relay x86_64-unknown-linux-musl x86_64
[ "$ARM64" = yes ] && pack_relay aarch64-unknown-linux-musl aarch64

printf '%s\n' "$TAG" >"$OUT/latest"

# The dev box's LAN address, best effort — the CT must be able to reach it.
HOST_IP="$( (ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}') || true)"
cat <<EOF

Staged $TAG in $OUT:
$(find "$OUT" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort | sed 's/^/  /')

Serve it (from $OUT):
  cd "$OUT" && python3 -m http.server 8000

Then, on the Proxmox host:
  var_release_base=http://${HOST_IP:-<this-machine>}:8000 ./hew-web.sh
(or ./hew-web.sh --update <CTID> with the same var_release_base — it is
persisted in the container, so a plain --update keeps using it.)
EOF
