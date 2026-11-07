#!/usr/bin/env bash
#
# Live relay probe — is the hosted relay actually answering where the
# clients look for it?
#
# The conformance gate (relay-contract.sh) proves the code; this proves the
# deployment. A Worker route that was never added, or was dropped, passes
# every test and still breaks "Open on Phone" for every desktop: the client
# PUTs to <origin>/relay/drop and the request falls through to the web app.
#
# Checks, against each origin:
#   1. GET <prefix>/ answers the relay identity JSON (not the web app's HTML),
#   2. a 16-byte PUT /drop, GET /drop/<token>, second GET round trip
#      (200, same bytes, then the one-shot 404).
# The bare `/relay` (no slash) is checked on the app origin too.
#
# Usage: scripts/relay-probe.sh [origin ...]
#   default origins: https://app.hew3d.com/relay  https://share.hew3d.com
# Exit status is non-zero on the first failing origin; the output names the
# failing step.
set -uo pipefail

origins=("$@")
if [ ${#origins[@]} -eq 0 ]; then
  origins=("https://app.hew3d.com/relay" "https://share.hew3d.com")
fi

fail=0
note() { printf '  %s\n' "$*"; }
bad() { printf '  FAIL: %s\n' "$*"; fail=1; }

for base in "${origins[@]}"; do
  base="${base%/}"
  echo "== $base"

  id=$(curl -sS -m 20 -w '\n%{http_code}' "$base/" 2>&1)
  code="${id##*$'\n'}"
  body="${id%$'\n'*}"
  if [ "$code" != "200" ] || ! printf '%s' "$body" | grep -q '"service":"hew-relay"'; then
    bad "identity: GET $base/ gave HTTP $code, not the relay identity JSON"
    continue
  fi
  note "identity ok"

  case "$base" in
    */relay)
      bare=$(curl -sS -m 20 -o /dev/null -w '%{http_code}' "$base")
      case "$bare" in
        200 | 308) note "bare /relay ok ($bare)" ;;
        *) bad "bare $base answered HTTP $bare" ;;
      esac
      ;;
  esac

  payload=$(head -c 16 /dev/urandom | base64)
  put=$(printf '%s' "$payload" | curl -sS -m 30 -X PUT --data-binary @- \
    -H 'content-type: application/octet-stream' -w '\n%{http_code}' "$base/drop" 2>&1)
  pcode="${put##*$'\n'}"
  pbody="${put%$'\n'*}"
  if [ "$pcode" != "200" ]; then
    bad "upload: PUT $base/drop gave HTTP $pcode"
    continue
  fi
  token=$(printf '%s' "$pbody" | sed -n 's/.*"token"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p')
  if [ -z "$token" ]; then
    bad "upload: no token in the response"
    continue
  fi
  got=$(curl -sS -m 20 "$base/drop/$token")
  if [ "$got" != "$payload" ]; then
    bad "download: bytes differ from what was uploaded"
    continue
  fi
  again=$(curl -sS -m 20 -o /dev/null -w '%{http_code}' "$base/drop/$token")
  if [ "$again" != "404" ]; then
    bad "one-shot: second GET gave HTTP $again, expected 404"
    continue
  fi
  note "round trip ok"
done

exit "$fail"
