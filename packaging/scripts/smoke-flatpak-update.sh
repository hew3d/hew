#!/usr/bin/env bash
# Smoke-tests the Flatpak update path after a release's packages are
# published, the way a user would meet it:
#
#   smoke-flatpak-update.sh v1.5.0
#
#  1. Installs the PREVIOUS release's single-file bundle (user scope,
#     without runtimes: only the app's version is checked), runs
#     `flatpak update`, and expects the installed version to become the new
#     tag. That is the path an installed Hew takes when its in-app notice
#     says to run `flatpak update com.hew3d.Hew`.
#  2. Removes the app and installs it fresh from the published `hew` remote
#     (the documented install command), expecting the new version.
#
# Both steps retry, because GitHub Pages can take a few minutes to serve a
# push that already finished. A previous release older than 1.4.0 is skipped:
# bundles before 1.4.0 were made without an update address by design (the
# recovery for those installs is documented on the update help page).
#
# Reporting only. The workflow runs this after the publish has happened and
# marks the job continue-on-error: a failure here is a flag to look at the
# Flatpak channel, never a reason to undo a publish. Tool paths and timing
# come from the environment so packaging/tests can run it against doubles.
set -euo pipefail
tag="${1:?usage: smoke-flatpak-update.sh vX.Y.Z}"
repo="${HEW_REPO:-hew3d/hew}"
remote_url="${HEW_FLATPAK_REPO_URL:-https://hew3d.github.io/hew-packages/flatpak/com.hew3d.Hew.flatpakrepo}"
attempts="${SMOKE_ATTEMPTS:-12}"
delay="${SMOKE_DELAY:-30}"
want="${tag#v}"
app="com.hew3d.Hew"
# The oldest release whose bundle carries an update address.
first_updatable="v1.4.0"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The previous stable release: the highest stable tag below this one.
previous="$(
  gh release list --repo "$repo" --exclude-drafts --exclude-pre-releases --limit 100 \
    --json tagName -q '.[].tagName' \
    | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | grep -vxF "$tag" \
    | { cat; echo "$tag"; } | sort -V | grep -xF -B1 "$tag" | head -n1 || true
)"
if [ -z "$previous" ] || [ "$previous" = "$tag" ]; then
  echo "::notice::No earlier stable release than $tag; nothing to update from."
  exit 0
fi
if [ "$previous" != "$first_updatable" ] \
  && [ "$(printf '%s\n%s\n' "$previous" "$first_updatable" | sort -V | head -n1)" = "$previous" ]; then
  echo "::notice::Previous release $previous predates $first_updatable, whose Flatpak bundle has no update address; skipping."
  exit 0
fi

# Empty when the app is not installed (flatpak info exits nonzero, which
# pipefail would otherwise turn into the caller's failure).
installed_version() {
  { flatpak info --user "$app" 2>/dev/null || true; } | sed -n 's/^ *Version: *//p' | head -n1
}

# Run "$@" until the installed version is $want, or attempts run out.
until_version() {
  local n=1
  while :; do
    "$@" || true
    local got
    got="$(installed_version)"
    if [ "$got" = "$want" ]; then
      echo "installed version is $got"
      return 0
    fi
    if [ "$n" -ge "$attempts" ]; then
      echo "::error::installed version is '${got:-none}' after $n attempts; wanted $want" >&2
      flatpak info --user "$app" >&2 || true
      return 1
    fi
    echo "installed version is '${got:-none}', wanted $want; attempt $n of $attempts, retrying in ${delay}s"
    n=$((n + 1))
    sleep "$delay"
  done
}

echo "=== update from $previous's bundle ==="
if ! gh release download "$previous" --repo "$repo" --pattern hew.flatpak --dir "$work"; then
  echo "::notice::$previous has no hew.flatpak bundle; skipping the update check."
else
  flatpak install --user --noninteractive --no-deps "$work/hew.flatpak"
  echo "bundle installed as version $(installed_version)"
  until_version flatpak update --user --noninteractive --no-deps -y "$app"
  flatpak uninstall --user --noninteractive -y "$app"
fi

echo "=== fresh install from the published remote ==="
flatpak remote-add --user --if-not-exists hew "$remote_url"
until_version flatpak install --user --noninteractive --no-deps -y hew "$app"
echo "flatpak update smoke passed for $tag"
