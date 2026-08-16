#!/usr/bin/env bash
#
# Relay conformance gate — one contract, two servers (docs/design/self-
# hosting-relay.md §6; workers/share-relay/README.md "API surface").
#
# Runs, in order:
#   1. the share-relay Worker's unit suite (node --test, zero deps),
#   2. the black-box conformance suite (workers/share-relay/contract/) against
#      the Worker under `wrangler dev --local` — open and with an upload key,
#   3. the same suite against the hew-relay binary — default settings, with
#      an upload key, and with a 2 s TTL + tiny per-drop / total caps so the
#      expiry and "relay full" cases run.
#
# Shared by .github/workflows/ci.yml (the blocking `relay-contract` job) and
# scripts/verify-full.sh, so local prediction of CI stays honest. Needs
# node (per .node-version), npm, cargo, and network access the first time
# (npm ci for wrangler; cargo for the relay's crates).
#
# Env:
#   HEW_RELAY_SKIP_WRANGLER=1  skip the wrangler legs (e.g. an offline machine
#                              that has never installed wrangler).
#
set -euo pipefail

cd "$(dirname "$0")/.."
RELAY_DIR=workers/share-relay

# Ports well away from anything a dev session uses (vite 5173/4173, a hand-run
# relay on 8787, wrangler's own default 8787).
WRANGLER_PORT_OPEN=18787
WRANGLER_PORT_KEY=18788
RELAY_PORT_PLAIN=18790
RELAY_PORT_KEY=18791
RELAY_PORT_SMALL=18792
CI_KEY="contract-ci-key"

pids=()
# Kill a server and its children (wrangler forks a `workerd` that outlives a
# plain kill of the wrangler process and would keep its port).
stop_pid() {
  local pid="$1"
  [ -n "$pid" ] || return 0
  pkill -P "$pid" 2>/dev/null || true
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}
cleanup() {
  for pid in "${pids[@]:-}"; do
    stop_pid "$pid"
  done
}
trap cleanup EXIT

# Poll the identity route until the server answers with the expected auth
# mode (or give up). Checking the mode, not just liveness, is what makes
# sure each leg talks to ITS server and not a leftover from a previous one.
wait_ready() {
  local url="$1" name="$2" expect_auth="$3"
  for _ in $(seq 1 90); do
    if curl -sf "$url" 2>/dev/null | grep -q "\"auth\":\"$expect_auth\""; then
      return 0
    fi
    sleep 1
  done
  echo "relay-contract: $name never became ready at $url (auth=$expect_auth)" >&2
  return 1
}

run_suite() {
  # All args are VAR=value pairs for the suite's environment.
  (cd "$RELAY_DIR" && env "$@" node --test contract/relay.contract.test.ts)
}

echo "=== share-relay unit tests (node --test) ==="
(cd "$RELAY_DIR" && node --test src/*.test.ts)

if [ "${HEW_RELAY_SKIP_WRANGLER:-0}" = "1" ]; then
  echo "=== HEW_RELAY_SKIP_WRANGLER=1 — skipping the wrangler dev legs ==="
else
  if [ ! -x "$RELAY_DIR/node_modules/.bin/wrangler" ]; then
    echo "=== npm ci (wrangler) ==="
    (cd "$RELAY_DIR" && npm ci --silent)
  fi
  # `wrangler dev`'s local proxy buffers request bodies before the Worker sees
  # them, so the two header-only fail-fast cases are unobservable there (and
  # the client's disconnect crashes the dev server) — HEW_RELAY_BUFFERS_BODY=1
  # skips exactly those; hew-relay below runs them.
  export WRANGLER_SEND_METRICS=false

  # Two legs on two ports AND two state dirs, run strictly one after the
  # other: two live workerd processes over one Durable Object state dir
  # corrupt each other's SQLite files (the suite hits fixed token ids like
  # AAAA… from both), and a stopped wrangler releases its port lazily.
  echo "=== conformance: Worker under wrangler dev (open) ==="
  (cd "$RELAY_DIR" && exec node_modules/.bin/wrangler dev --local --port "$WRANGLER_PORT_OPEN" --persist-to .wrangler/state-contract-open >"${TMPDIR:-/tmp}/hew-wrangler-open.log" 2>&1) &
  wpid=$!
  pids+=("$wpid")
  wait_ready "http://127.0.0.1:$WRANGLER_PORT_OPEN/relay/" "wrangler dev (open)" none
  run_suite HEW_RELAY_URL="http://127.0.0.1:$WRANGLER_PORT_OPEN" HEW_RELAY_BUFFERS_BODY=1
  stop_pid "$wpid"

  echo "=== conformance: Worker under wrangler dev (upload key) ==="
  (cd "$RELAY_DIR" && exec node_modules/.bin/wrangler dev --local --port "$WRANGLER_PORT_KEY" --persist-to .wrangler/state-contract-key --var "HEW_RELAY_UPLOAD_KEY:$CI_KEY" >"${TMPDIR:-/tmp}/hew-wrangler-key.log" 2>&1) &
  wpid=$!
  pids+=("$wpid")
  wait_ready "http://127.0.0.1:$WRANGLER_PORT_KEY/relay/" "wrangler dev (key)" bearer
  run_suite HEW_RELAY_URL="http://127.0.0.1:$WRANGLER_PORT_KEY" HEW_RELAY_BUFFERS_BODY=1 HEW_RELAY_UPLOAD_KEY="$CI_KEY"
  stop_pid "$wpid"
fi

echo "=== cargo build -p hew-relay ==="
cargo build -p hew-relay
RELAY_BIN="target/debug/hew-relay"

echo "=== conformance: hew-relay (defaults) ==="
"$RELAY_BIN" --listen "127.0.0.1:$RELAY_PORT_PLAIN" --allow-origin https://app.hew3d.com >"${TMPDIR:-/tmp}/hew-relay-plain.log" 2>&1 &
pids+=($!)
"$RELAY_BIN" --listen "127.0.0.1:$RELAY_PORT_KEY" --allow-origin https://app.hew3d.com --upload-key "$CI_KEY" >"${TMPDIR:-/tmp}/hew-relay-key.log" 2>&1 &
pids+=($!)
"$RELAY_BIN" --listen "127.0.0.1:$RELAY_PORT_SMALL" --allow-origin https://app.hew3d.com --ttl-secs 2 --max-bytes 65536 --max-total-bytes 200000 >"${TMPDIR:-/tmp}/hew-relay-small.log" 2>&1 &
pids+=($!)
wait_ready "http://127.0.0.1:$RELAY_PORT_PLAIN/relay/" "hew-relay (defaults)" none
wait_ready "http://127.0.0.1:$RELAY_PORT_KEY/relay/" "hew-relay (key)" bearer
wait_ready "http://127.0.0.1:$RELAY_PORT_SMALL/relay/" "hew-relay (small)" none
run_suite HEW_RELAY_URL="http://127.0.0.1:$RELAY_PORT_PLAIN"

echo "=== conformance: hew-relay (upload key) ==="
run_suite HEW_RELAY_URL="http://127.0.0.1:$RELAY_PORT_KEY" HEW_RELAY_UPLOAD_KEY="$CI_KEY"

echo "=== conformance: hew-relay (2 s TTL, 64 KiB drops, 200 kB total) ==="
run_suite HEW_RELAY_URL="http://127.0.0.1:$RELAY_PORT_SMALL" HEW_RELAY_TTL_MS=2000 HEW_RELAY_MAX_BYTES=65536 HEW_RELAY_MAX_TOTAL_BYTES=200000

echo "relay-contract: all green"
