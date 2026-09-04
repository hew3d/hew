#!/usr/bin/env bash
# Take every shot (or the ones named as arguments) in sequence, waiting for
# the machine to go quiet between takes: a capture's own browser and video
# encoder push the 1-minute load average past the harness gate for the take
# that follows it, so back-to-back runs refuse without this pause.
# The gate defaults to 4 rather than the harness's 3 because the app's own
# dev server holds a quiet machine near 3; the driver still waits until the
# load is half a point under the gate before each take.
set -euo pipefail
cd "$(dirname "$0")"
MAX_LOAD="${CAPTURE_MAX_LOAD:-4}"
shots=("$@")
if [ ${#shots[@]} -eq 0 ]; then
  shots=()
  for f in shots/*.mjs; do shots+=("$(basename "$f" .mjs)"); done
fi
curl -sf -o /dev/null http://localhost:5199/ || { echo "app not served on :5199"; exit 1; }
quiet() { awk -v l="$1" -v m="$MAX_LOAD" 'BEGIN { exit !(l < m - 0.5) }'; }
for s in "${shots[@]}"; do
  for _ in $(seq 1 30); do
    load=$(sysctl -n vm.loadavg | awk '{print $2}')
    quiet "$load" && break
    sleep 10
  done
  if ! quiet "$load"; then
    echo "machine never went quiet (load $load after 5 min) — not taking $s"; exit 1
  fi
  echo "== $s (load $load)"
  CAPTURE_MAX_LOAD="$MAX_LOAD" node "shots/$s.mjs" 2>&1 | grep -E 'MARK|LOADED|Error|expected' | tr '\n' ' '
  echo
done
