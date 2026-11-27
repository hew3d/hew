#!/usr/bin/env bash
#
# Put a wasm-bindgen that runs on this machine on PATH, at exactly the version
# Cargo.lock pins.
#
# wasm-pack uses a `wasm-bindgen` found on PATH when its version matches the
# lockfile's, and otherwise downloads upstream's prebuilt one. On Linux arm64
# that prebuilt is the dynamically linked aarch64-unknown-linux-gnu build, and
# a recent one needs a newer glibc than the ubuntu-22.04-arm runner (the
# release job stays on 22.04 for the AppImage/deb/rpm glibc baseline) has:
# "libc.so.6: version `GLIBC_2.38' not found". On x86_64 Linux wasm-pack asks
# for the musl build, which is static, so only arm64 needs this.
#
# Upstream also publishes a static aarch64-unknown-linux-musl build, which runs
# on any glibc. This script installs that one (checked against its published
# sha256, which guards against a corrupt download) and, if it somehow will not run, builds wasm-bindgen-cli from source.
# The version is read from Cargo.lock, never written here, so a dependency bump
# cannot leave this behind. The result is verified; a mismatch is a hard error.
#
# Installs to $CARGO_HOME/bin (default ~/.cargo/bin). Idempotent: does nothing
# if a matching, working wasm-bindgen is already on PATH.
#
# Usage: scripts/install-wasm-bindgen.sh
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"

# The wasm-bindgen crate's entry in Cargo.lock.
version="$(awk '
  $0 == "name = \"wasm-bindgen\"" { found = 1; next }
  found && /^version = / { gsub(/"/, "", $3); print $3; exit }
' "$repo_root/Cargo.lock")"
if [ -z "$version" ]; then
  echo "install-wasm-bindgen: no wasm-bindgen entry in Cargo.lock" >&2
  exit 1
fi

# Version of the wasm-bindgen that runs from PATH, or empty if none does.
installed() {
  command -v wasm-bindgen >/dev/null 2>&1 || return 0
  wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true
}

if [ "$(installed)" = "$version" ]; then
  echo "wasm-bindgen $version already installed"
  exit 0
fi

mkdir -p "$bin_dir"
export PATH="$bin_dir:$PATH"

case "$(uname -m)" in
  aarch64 | arm64) triple="aarch64-unknown-linux-musl" ;;
  x86_64) triple="x86_64-unknown-linux-musl" ;;
  *) triple="" ;;
esac

if [ "$(uname -s)" = "Linux" ] && [ -n "$triple" ]; then
  stem="wasm-bindgen-${version}-${triple}"
  base="https://github.com/wasm-bindgen/wasm-bindgen/releases/download/${version}"
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  echo "wasm-bindgen $version from $base/$stem.tar.gz"
  if curl -sSfL "$base/$stem.tar.gz" -o "$tmp/$stem.tar.gz" \
    && curl -sSfL "$base/$stem.tar.gz.sha256sum" -o "$tmp/sum"; then
    # The published checksum file names a path that is not the file's name
    # here, so compare the hash alone.
    want="$(awk '{print $1}' "$tmp/sum")"
    have="$(sha256sum "$tmp/$stem.tar.gz" | awk '{print $1}')"
    if [ -n "$want" ] && [ "$want" = "$have" ]; then
      tar -xzf "$tmp/$stem.tar.gz" -C "$tmp"
      install -m 0755 "$tmp/$stem/wasm-bindgen" "$bin_dir/wasm-bindgen"
    else
      echo "install-wasm-bindgen: checksum mismatch for $stem.tar.gz" >&2
    fi
  else
    echo "install-wasm-bindgen: could not download $stem" >&2
  fi
fi

if [ "$(installed)" != "$version" ]; then
  echo "wasm-bindgen $version via cargo install"
  cargo install wasm-bindgen-cli --version "$version" --locked --force
fi

have="$(installed)"
if [ "$have" != "$version" ]; then
  echo "install-wasm-bindgen: wanted wasm-bindgen $version (Cargo.lock), got '${have:-none}'" >&2
  exit 1
fi

# The PATH change above ends with this script. In a GitHub Actions job, put
# the directory first for the later steps too, so the wasm-bindgen wasm-pack
# finds there is the one checked here and not one earlier on the runner's PATH.
if [ -n "${GITHUB_PATH:-}" ] && [ -x "$bin_dir/wasm-bindgen" ]; then
  echo "$bin_dir" >> "$GITHUB_PATH"
fi
echo "wasm-bindgen $have ready ($(command -v wasm-bindgen))"
