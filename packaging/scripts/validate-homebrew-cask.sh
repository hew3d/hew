#!/usr/bin/env bash
# Validates the rendered Homebrew cask for real: copies it into a throwaway
# local tap, runs brew's style and audit checks, installs it (the .dmg
# download and checksum, the app, and the hew-cli link), confirms the linked
# CLI reports the release's version, and uninstalls it again with --zap, so a
# wrong digest, a moved sidecar, or a zap list that reaches past the app's own
# state fails loudly here instead of on a user's machine. Runs on a macOS
# runner.
#
#   packaging/scripts/validate-homebrew-cask.sh 1.1.0
#
# The zap runs under a scratch HOME (brew resolves `~` and its trash from
# HOME), so it never touches the real one. On a machine that already has Hew
# in /Applications, point the install somewhere else with
# HOMEBREW_CASK_OPTS=--appdir=<dir>, and quit Hew first: the cask's uninstall
# stanza quits the app by bundle identifier.
set -euo pipefail
version="${1:?usage: validate-homebrew-cask.sh X.Y.Z}"
here="$(cd "$(dirname "$0")/.." && pwd)"
export HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 HOMEBREW_NO_ENV_HINTS=1

# render-templates.sh leaves the cask alone when the release has no .dmg.
grep -qx "  version \"${version}\"" "$here/homebrew/Casks/hew.rb" \
  || { echo "the cask was not rendered for $version (does the release have its .dmg?)" >&2; exit 1; }

# A directory under Taps/ is a tap as far as brew is concerned; `brew tap-new`
# would also scaffold formula and CI templates this check has no use for.
tap="hew3d/validate"
taproot="$(brew --repository)/Library/Taps/hew3d/homebrew-validate"
cask="$tap/hew"
fakehome="$(mktemp -d)"
# brew style and brew audit are developer commands, which switch Homebrew's
# persistent developer mode on; put it back the way it was.
developer_was_off=false
case "$(brew developer)" in *disabled*) developer_was_off=true ;; esac
trusted=false
cleanup() {
  brew uninstall --cask "$cask" >/dev/null 2>&1 || true
  if $trusted; then brew untrust --cask "$cask" >/dev/null 2>&1 || true; fi
  rm -rf "$taproot" "$fakehome"
  rmdir "$(dirname "$taproot")" 2>/dev/null || true
  if $developer_was_off; then brew developer off >/dev/null 2>&1 || true; fi
}
trap cleanup EXIT
rm -rf "$taproot"
mkdir -p "$taproot/Casks"
cp "$here/homebrew/Casks/hew.rb" "$taproot/Casks/hew.rb"

# Homebrew 6 and later load nothing from a tap that has not been trusted
# (docs.brew.sh/Tap-Trust), and `brew audit` loads the cask just as
# `brew install` does; trust this one cask, not the tap, before either
# runs, and take the trust back in cleanup. Older brews have no `trust`
# command and need none.
if brew trust --cask "$cask" >/dev/null 2>&1; then trusted=true; fi

brew style --cask "$cask"
brew audit --cask --strict --online "$cask"

brew install --cask "$cask"
cli="$(brew --prefix)/bin/hew-cli"
got="$("$cli" --version)"
[ "$got" = "hew-cli $version" ] || { echo "linked hew-cli reports '$got', expected 'hew-cli $version'" >&2; exit 1; }

# Seed the scratch HOME with every path the zap stanza names, plus stand-ins
# for things that must survive it: the default Library folder, documents, and
# another app's state.
zap_paths=()
while IFS= read -r p; do zap_paths+=("$fakehome/${p#\~/}"); done < <(
  brew info --cask --json=v2 "$cask" | python3 -c '
import json, sys
for artifact in json.load(sys.stdin)["casks"][0]["artifacts"]:
    for stanza in artifact.get("zap", []):
        for key in ("trash", "delete", "rmdir"):
            paths = stanza.get(key, [])
            for p in [paths] if isinstance(paths, str) else paths:
                print(p)
')
[ "${#zap_paths[@]}" -gt 0 ] || { echo "the cask has no zap paths to check" >&2; exit 1; }
for p in "${zap_paths[@]}"; do
  case "$p" in
    "$fakehome"/*) ;;
    *) echo "zap path outside the home directory: $p" >&2; exit 1 ;;
  esac
  if [[ "$p" == *.plist ]]; then mkdir -p "$(dirname "$p")"; touch "$p"
  else mkdir -p "$p"; touch "$p/state"; fi
done
keep=("$fakehome/Hew Library/model.hew" "$fakehome/Documents/model.hew"
      "$fakehome/Library/Application Support/SomeOtherApp/state")
for k in "${keep[@]}"; do mkdir -p "$(dirname "$k")"; touch "$k"; done

HOME="$fakehome" brew uninstall --zap --cask "$cask"
[ ! -e "$cli" ] && [ ! -L "$cli" ] || { echo "uninstall left $cli behind" >&2; exit 1; }
for p in "${zap_paths[@]}"; do
  [ ! -e "$p" ] || { echo "zap left $p behind" >&2; exit 1; }
done
for k in "${keep[@]}"; do
  [ -e "$k" ] || { echo "zap removed $k, which is not the app's own state" >&2; exit 1; }
done
echo "homebrew cask validated: hew $version"
