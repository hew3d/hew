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
#   hew-web.sh                              scripts/hew-web.sh, with THIS
#                                           mirror's URL baked in as the
#                                           var_release_base default (see
#                                           below) — so a container that
#                                           fetches it doesn't need to be
#                                           told where it came from
#
# then serves $OUT with `python3 -m http.server` (skip with --no-serve) and
# prints the exact one-liner for a Proxmox host or LXC console:
#
#   bash -c "$(curl -fsSL http://<this-mac>:8000/hew-web.sh)"
#
# No `var_release_base=...` typed anywhere: the served hew-web.sh is a copy
# of the real installer with one line inserted near its own top —
# `: "${var_release_base:=http://<this-mac>:8000}"` — which only fills
# the variable if it's still unset, so an explicit var_release_base (or a
# genuine GitHub-sourced hew-web.sh) is never overridden. Run *inside* an
# already-provisioned container this updates it in place (hew-web.sh
# recognizes /etc/hew/install.env and switches to update mode); run on a
# Proxmox host it creates a new one. Once a container has updated from a
# mirror once, CT_RELEASE_BASE is persisted in /etc/hew/install.env, so a
# plain `update` typed later in its console keeps using this mirror even
# though `update` itself always tries to fetch a fresh installer from
# GitHub first (that fetch carries no var_release_base, so it falls back to
# the persisted value — see docs/SELF_HOSTING.md).
#
# Nothing here talks to GitHub, and nothing in the container-side path
# differs from a real release — the assets are laid out and named
# identically, so what gets tested is the real installer, not a special dev
# mode of it.
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
PORT=8000
SERVE=yes
for arg in "$@"; do
  case "$arg" in
    --x86_64-only) ARM64=no ;;
    --no-serve) SERVE=no ;;
    --port=*) PORT="${arg#--port=}" ;;
    --*) echo "stage-local-release: unknown option $arg" >&2; exit 2 ;;
    *) OUT="$arg" ;;
  esac
done

# Validate everything the mirror URL depends on BEFORE the expensive build
# below (a full pnpm build plus two Rust cross-compiles) instead of after —
# a bad --port or an undetectable LAN IP should fail in seconds, not after
# minutes of work whose output would just be discarded.
case "$PORT" in
  ''|*[!0-9]*) echo "stage-local-release: --port must be a number, got '$PORT'" >&2; exit 2 ;;
esac
# The dev box's LAN address, best effort — the CT must be able to reach it.
# Only auto-detected if not already given: `HOST_IP=<ip> $0 ...` overrides.
HOST_IP="${HOST_IP:-}"
[ -n "$HOST_IP" ] || HOST_IP="$( (ipconfig getifaddr en0 2>/dev/null || hostname -I 2>/dev/null | awk '{print $1}') || true)"
[ -n "$HOST_IP" ] || { echo "stage-local-release: could not determine this machine's LAN IP (checked en0, then hostname -I) — set it explicitly: HOST_IP=<ip> $0 ..." >&2; exit 1; }
MIRROR="http://${HOST_IP}:${PORT}"

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

echo "=== hew-web.sh → $OUT/hew-web.sh (var_release_base defaults to $MIRROR) ==="
# Insert right before the script's own color-code line (`GN=...`), which
# immediately follows its TOP-LEVEL `set -euo pipefail` — anchoring on that
# instead of `set -euo pipefail` itself, because that exact string also
# appears twice more, inside single-quoted heredocs that run INSIDE the
# container (where var_release_base plays no role); `GN=...` appears only
# once. Earliest safe point either way: before the _explicit_release_base
# snapshot decides whether this run's release_base gets persisted to
# /etc/hew/install.env.
anchor="GN='\\033[1;92m'; YW='\\033[1;33m'; RD='\\033[1;31m'; CL='\\033[m'"
insert_at="$(grep -n -m1 -F -x "$anchor" scripts/hew-web.sh | cut -d: -f1)"
[ -n "$insert_at" ] || { echo "stage-local-release: could not find the anchor line in scripts/hew-web.sh to insert var_release_base's default before — has its color-code line changed?" >&2; exit 1; }
awk -v n="$insert_at" -v line=": \"\${var_release_base:=$MIRROR}\"" \
  'NR==n{print line} {print}' scripts/hew-web.sh >"$OUT/hew-web.sh"
chmod +x "$OUT/hew-web.sh"

cat <<EOF

Staged $TAG in $OUT:
$(find "$OUT" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort | sed 's/^/  /')

On the Proxmox host, or inside an already-provisioned container's console
(pct enter <CTID>) to update it in place:
  bash -c "\$(curl -fsSL $MIRROR/hew-web.sh)"
No var_release_base to type — it's baked into the served hew-web.sh above.
EOF

if [ "$SERVE" = no ]; then
  echo "--no-serve: not starting a server. Serve $OUT yourself, e.g.:"
  echo "  cd \"$OUT\" && python3 -m http.server $PORT"
  exit 0
fi

echo "Serving $OUT on :$PORT — Ctrl-C to stop."
cd "$OUT"
exec python3 -m http.server "$PORT"
