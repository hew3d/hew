#!/usr/bin/env bash
#
# Install the pinned wasm-pack.
#
# wasm-pack compiles the kernel that the app, the replay runner, and every
# shipped binary actually execute, and it was previously installed by piping
# the upstream init.sh to sh — which always fetches whatever the newest release
# happens to be. That is the same class of bug that `channel = "stable"` was in
# rust-toolchain.toml: an unreviewed toolchain change arriving through the back
# door, differing between machines and between CI lanes. So the version is
# pinned here, in one place, and Renovate proposes bumps as reviewable PRs
# (.github/renovate.json5 — the `# renovate:` comment below is what makes the
# pin visible to it).
#
# Installs to $CARGO_HOME/bin (default ~/.cargo/bin) and is idempotent: if the
# pinned version is already on PATH it does nothing, so a warm CI cache costs a
# version check and nothing more.
#
# Usage: scripts/install-wasm-pack.sh
set -euo pipefail

# renovate: datasource=crates-io depName=wasm-pack
WASM_PACK_VERSION="0.15.0"

bin_dir="${CARGO_HOME:-$HOME/.cargo}/bin"

if command -v wasm-pack >/dev/null 2>&1; then
  have="$(wasm-pack --version 2>/dev/null | awk '{print $2}')"
  if [ "$have" = "$WASM_PACK_VERSION" ]; then
    echo "wasm-pack $WASM_PACK_VERSION already installed"
    exit 0
  fi
  echo "wasm-pack $have installed, want $WASM_PACK_VERSION — replacing"
fi

mkdir -p "$bin_dir"

case "$(uname -s)" in
  Darwin)
    case "$(uname -m)" in
      arm64 | aarch64) triple="aarch64-apple-darwin" ;;
      x86_64) triple="x86_64-apple-darwin" ;;
      *) echo "install-wasm-pack: unsupported macOS arch $(uname -m)" >&2; exit 1 ;;
    esac
    ;;
  Linux)
    # The musl builds are the ones upstream publishes for Linux; they are
    # statically linked, so they do not care about the runner's glibc.
    case "$(uname -m)" in
      x86_64) triple="x86_64-unknown-linux-musl" ;;
      aarch64 | arm64) triple="aarch64-unknown-linux-musl" ;;
      *) echo "install-wasm-pack: unsupported Linux arch $(uname -m)" >&2; exit 1 ;;
    esac
    ;;
  MINGW* | MSYS* | CYGWIN* | Windows_NT)
    # Deliberately NOT the prebuilt tarball. Upstream publishes no
    # aarch64-pc-windows-msvc build at all, and the release matrix includes a
    # windows-arm64 leg; compiling from source is the one path that works on
    # both Windows architectures. It is slow, which is why the release
    # workflow caches the resulting exe (keyed on this version).
    echo "wasm-pack $WASM_PACK_VERSION via cargo install (Windows)"
    cargo install wasm-pack --version "$WASM_PACK_VERSION" --locked
    wasm-pack --version
    exit 0
    ;;
  *)
    echo "install-wasm-pack: unsupported OS $(uname -s)" >&2
    exit 1
    ;;
esac

stem="wasm-pack-v${WASM_PACK_VERSION}-${triple}"
url="https://github.com/rustwasm/wasm-pack/releases/download/v${WASM_PACK_VERSION}/${stem}.tar.gz"

echo "wasm-pack $WASM_PACK_VERSION from $url"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -sSfL "$url" -o "$tmp/wasm-pack.tar.gz"
tar -xzf "$tmp/wasm-pack.tar.gz" -C "$tmp"
install -m 0755 "$tmp/$stem/wasm-pack" "$bin_dir/wasm-pack"

"$bin_dir/wasm-pack" --version
