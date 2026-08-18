#!/usr/bin/env bash
# Provisions a Debian LXC container on a Proxmox VE host running the Hew
# web app (and, optionally, its self-hosted "Open on Phone" relay and TLS),
# following the shape (banner, var_* overrides, whiptail Default/Advanced
# menu, msg_info/msg_ok progress lines, --update mode) that
# community-scripts.org Proxmox VE Scripts use.
#
# It deliberately does NOT source that project's misc/build.func: that
# helper's install step is hardcoded to fetch install/<app>.sh from
# community-scripts/ProxmoxVE's own repo, which has no entry for Hew (this
# app isn't in that catalog — yet). This script is the standalone
# equivalent for running today; a real submission there would later split
# it into their ct/+install/ file pair.
#
# Run from a Proxmox VE host shell, the way community scripts are run:
#   bash -c "$(curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh)"
#   bash -c "$(curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh)" hew-web.sh --update 105
# (the `hew-web.sh` before `--update` fills bash's $0 so the arguments land
# in $1/$2 — same shape as any `bash -c '…' name args`), or from a local
# copy:
#   ./hew-web.sh                # create a new container (whiptail wizard)
#   ./hew-web.sh --update 105   # re-deploy the latest release into CTID 105
#
# Updating, the community-scripts way: run the SAME script INSIDE the
# container and it updates instead of installing (it recognizes an
# existing install by /etc/hew/install.env). The container gets a
# `/usr/bin/update` command that does exactly that — fetching this script
# fresh (from CT_RELEASE_BASE's mirror if the container was installed from
# one, GitHub otherwise) when it can, else re-running the copy saved at
# install time — so `pct enter <CTID>` + `update` is the whole procedure.
# The host-side `--update <CTID>` above is the same operation driven from
# outside.
#
# Unattended (every prompt has a var_* twin; a missing/non-interactive
# whiptail silently falls back to defaults, same as pre-v2's storage
# picker):
#   var_relay=yes var_https=none var_ctid=120 ./hew-web.sh
#
# Installing from somewhere other than GitHub Releases — an internal
# mirror, an air-gapped copy, or a not-yet-released build staged with
# scripts/stage-local-release.sh and served with `python3 -m http.server`:
#   var_release_base=http://192.168.1.10:8000 ./hew-web.sh
# The base is any HTTP directory laid out like a release: the
# hew-web-vX.Y.Z.tar.gz and hew-relay-vX.Y.Z-linux-<arch>.tar.gz assets,
# plus a `latest` text file naming the tag (e.g. `v0.9.0`) that
# `var_version=latest` (the default) resolves through. Persisted inside the
# container, so `--update` and the weekly timer keep using it.
#
# scripts/stage-local-release.sh does the above automatically: it stages
# and serves a copy of THIS script with var_release_base already defaulted
# to its own mirror, so `bash -c "$(curl -fsSL http://<mac>:8000/hew-web.sh)"`
# — no var_release_base typed — installs (from a Proxmox host) or updates
# (typed inside an already-provisioned container's console) from it.
#
# See docs/SELF_HOSTING.md for what's actually being deployed and why.

set -euo pipefail

GN='\033[1;92m'; YW='\033[1;33m'; RD='\033[1;31m'; CL='\033[m'

# How to run this script again — the update hint at the end and the usage
# text must not say `bash --update 105` when this was `bash -c "$(curl …)"`
# (bash's $0 is then "bash", or whatever the caller put after the string).
SCRIPT_URL="https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh"
if [ -f "$0" ]; then
  SELF="$0"
else
  SELF="bash -c \"\$(curl -fsSL $SCRIPT_URL)\" hew-web.sh"
fi
# All three write to stderr, not just msg_err: several functions below
# return their answer over stdout via command substitution, and a msg_info
# sharing that stream would land inside the captured value instead of the
# terminal.
msg_info() { echo -e " ${YW}○${CL} $1" >&2; }
msg_ok()   { echo -e " ${GN}✓${CL} $1" >&2; }
msg_err()  { echo -e " ${RD}✗${CL} $1" >&2; }

# Where are we? On the Proxmox host (pct available) this script creates or
# updates a container; INSIDE a container it has already provisioned (no
# pct, but /etc/hew/install.env is there) it updates that install in
# place — the community-scripts convention that re-running the script from
# the CT means "update", and what the CT's /usr/bin/update runs.
IN_CT=no
if ! command -v pct >/dev/null 2>&1; then
  if [ -f /etc/hew/install.env ]; then
    IN_CT=yes
  else
    msg_err "pct not found — run this from a Proxmox VE host shell (to install), or inside a container this script set up (to update)."
    exit 1
  fi
fi

# `set -e` exits silently on an unexpected failure; say where. Replaced by
# the more specific "CT may be half-provisioned" trap once `pct create` has
# run.
# shellcheck disable=SC2154 # $rc is assigned inside the trap string itself
trap 'rc=$?; msg_err "Setup failed (exit $rc) at line $LINENO of hew-web.sh — nothing was created."' ERR

# Snapshot "did the caller explicitly set this Hew-option var" BEFORE any
# defaulting touches it below. --update needs this: it must let an explicit
# var_* override win over what was persisted in /etc/hew/install.env on the
# last install, while an *unset* var_* must NOT stomp that persisted value
# with this script's own built-in default (see settings_block()).
_explicit_version="${var_version+x}"
_explicit_release_base="${var_release_base+x}"
_explicit_relay="${var_relay+x}"
_explicit_relay_mem="${var_relay_mem_mb+x}"
_explicit_https="${var_https+x}"
_explicit_hostname_public="${var_hostname_public+x}"
_explicit_letsencrypt_email="${var_letsencrypt_email+x}"
_explicit_autoupdate="${var_autoupdate+x}"

# ---------------------------------------------------------------------------
# whiptail helpers
#
# Every one of these takes the name of the variable to resolve. If it's
# already non-empty (set directly, or via one of the backward-compat
# aliases below) it's used verbatim and nothing is asked. Otherwise, in an
# interactive advanced run it prompts; anywhere else (no tty, no whiptail,
# or a "Default settings" run for a non-"always" prompt) it silently takes
# the given default — exactly the fallback pre-v2's pick_storage used, now
# generalized to every prompt in the wizard.
# ---------------------------------------------------------------------------

have_whiptail() { command -v whiptail >/dev/null 2>&1 && [ -t 0 ] && [ -t 2 ]; }

# $1=outvar $2=title $3=text $4=default
resolve_input() {
  local outvar="$1" title="$2" text="$3" default="$4"
  [ -n "${!outvar:-}" ] && return 0
  if [ "$ADVANCED" = yes ] && have_whiptail; then
    printf -v "$outvar" '%s' "$(whiptail --backtitle "Hew Web" --title "$title" --inputbox "$text" 12 74 "$default" 3>&1 1>&2 2>&3)"
  else
    printf -v "$outvar" '%s' "$default"
  fi
}

# $1=outvar $2=title $3=text $4=default(yes|no); prompts only in Advanced.
resolve_yesno() { _resolve_yesno_impl "$ADVANCED" "$@"; }
# Same, but prompts in BOTH Default and Advanced (still gated on a real
# terminal) — used for the one question Default mode itself asks.
resolve_yesno_always() { _resolve_yesno_impl yes "$@"; }

_resolve_yesno_impl() {
  local gate="$1" outvar="$2" title="$3" text="$4" default="$5"
  [ -n "${!outvar:-}" ] && return 0
  if [ "$gate" = yes ] && have_whiptail; then
    local flag=""
    [ "$default" = no ] && flag="--defaultno"
    # shellcheck disable=SC2086 # $flag is either empty or one literal flag
    if whiptail --backtitle "Hew Web" --title "$title" --yesno "$text" 12 74 $flag; then
      printf -v "$outvar" yes
    else
      printf -v "$outvar" no
    fi
  else
    printf -v "$outvar" '%s' "$default"
  fi
}

# $1=outvar $2=title $3=text $4=default_tag, then tag/description pairs.
resolve_menu() {
  local outvar="$1" title="$2" text="$3" default="$4"; shift 4
  [ -n "${!outvar:-}" ] && return 0
  if [ "$ADVANCED" = yes ] && have_whiptail; then
    printf -v "$outvar" '%s' "$(whiptail --backtitle "Hew Web" --title "$title" --default-item "$default" --menu "$text" 20 78 10 "$@" 3>&1 1>&2 2>&3)"
  else
    printf -v "$outvar" '%s' "$default"
  fi
}

# Interactive storage picker: a whiptail menu when one's available and
# we're actually attached to a terminal, otherwise a silent fallback to
# the first active storage (scripted/cron runs, a host without whiptail,
# or a "Default settings" run — passed via stdin redirection below, since
# that's the cheapest way to reuse this exact fallback logic without
# duplicating it per call site).
pick_storage() {
  local content="$1" title="$2" override_var="$3"
  local -a rows=()
  local name type status total used avail pct
  # shellcheck disable=SC2034 # total/used/pct are unused table columns from `pvesm status`; named for readability
  while read -r name type status total used avail pct; do
    [ "$status" = "active" ] || continue
    rows+=("$name" "$type, $(( avail / 1024 / 1024 ))G free")
  done < <(pvesm status -content "$content" | tail -n +2)

  [ ${#rows[@]} -gt 0 ] || { msg_err "No active storage with '$content' content found."; exit 1; }

  if [ ${#rows[@]} -eq 2 ]; then
    echo "${rows[0]}"
    return
  fi

  # fd 1 is always a pipe here — command substitution captures this
  # function's stdout to return its value, so `[ -t 1 ]` would read false
  # even at a real interactive prompt. Check fd 2 instead: whiptail's UI
  # paints there via the 3>&1 1>&2 2>&3 swap below, and stderr is untouched
  # by the substitution.
  if command -v whiptail >/dev/null 2>&1 && [ -t 0 ] && [ -t 2 ]; then
    whiptail --backtitle "Hew Web" --title "Hew Web" --menu "$title" 18 74 8 "${rows[@]}" 3>&1 1>&2 2>&3
  else
    msg_info "Defaulting to storage '${rows[0]}' (override with $override_var=<name>)."
    echo "${rows[0]}"
  fi
}

# ---------------------------------------------------------------------------
# The provisioning script: pushed into the container and run there via
# `pct exec`. It is a single-quoted heredoc (no host-side substitution) —
# every host-resolved choice crosses over explicitly as a `KEY=value` line
# ahead of it, shell-quoted with printf %q so nothing here can be corrupted
# by word-splitting or injected by a hostname/key containing shell
# metacharacters.
# ---------------------------------------------------------------------------

# $1=mode (create|update)
settings_block() {
  local mode="$1"
  printf 'CT_MODE=%q\n' "$mode"
  if [ "$mode" = create ]; then
    # A fresh create always states every choice explicitly — there is
    # nothing yet in /etc/hew/install.env for the container script to fall
    # back to.
    printf 'CT_VERSION=%q\n' "${var_version:-latest}"
    printf 'CT_RELEASE_BASE=%q\n' "${var_release_base:-}"
    printf 'CT_RELAY=%q\n' "$var_relay"
    printf 'CT_RELAY_CAP_MB=%q\n' "$var_relay_mem_mb"
    printf 'CT_HTTPS=%q\n' "$var_https"
    printf 'CT_HOSTNAME_PUBLIC=%q\n' "$var_hostname_public"
    printf 'CT_LETSENCRYPT_EMAIL=%q\n' "${var_letsencrypt_email:-}"
    printf 'CT_AUTOUPDATE=%q\n' "$var_autoupdate"
  else
    # --update never re-runs the wizard. Only emit a line for a setting
    # this invocation actually overrode on the command line (the
    # _explicit_* snapshot from the top of the file) — everything else is
    # left unset here so the container script's own
    # `. /etc/hew/install.env` (settings_block below) fills it in from
    # what was persisted at the last install, instead of silently
    # resetting it to this script's built-in default.
    [ -n "$_explicit_version" ]           && printf 'CT_VERSION=%q\n' "${var_version:-latest}"
    [ -n "$_explicit_release_base" ]      && printf 'CT_RELEASE_BASE=%q\n' "${var_release_base:-}"
    [ -n "$_explicit_relay" ]             && printf 'CT_RELAY=%q\n' "$var_relay"
    [ -n "$_explicit_relay_mem" ]         && printf 'CT_RELAY_CAP_MB=%q\n' "$var_relay_mem_mb"
    [ -n "$_explicit_https" ]             && printf 'CT_HTTPS=%q\n' "$var_https"
    [ -n "$_explicit_hostname_public" ]   && printf 'CT_HOSTNAME_PUBLIC=%q\n' "$var_hostname_public"
    [ -n "$_explicit_letsencrypt_email" ] && printf 'CT_LETSENCRYPT_EMAIL=%q\n' "${var_letsencrypt_email:-}"
    [ -n "$_explicit_autoupdate" ]        && printf 'CT_AUTOUPDATE=%q\n' "$var_autoupdate"
  fi
  # An upload key is never (re)supplied through install.env or the wizard
  # on --update — see the EXISTING_KEY handling below, which reads any
  # already-installed key straight out of /etc/hew/relay.env instead.
  printf 'CT_UPLOAD_KEY=%q\n' "${RESOLVED_UPLOAD_KEY:-}"
}

provision_script() {
  cat <<'EOS'
set -euo pipefail
APP_DIR=/var/www/hew
NGINX_CONF=/etc/nginx/sites-available/hew
REDIRECT_CONF=/etc/nginx/sites-available/hew-redirect
ETC_DIR=/etc/hew
INSTALL_ENV="$ETC_DIR/install.env"
mkdir -p "$ETC_DIR"

# Fill in anything this run didn't explicitly override from what the last
# install persisted (see settings_block() on the host side for why this is
# safe to source unconditionally: it's written with `: "${VAR:=...}"`
# below, so it only fills gaps, never clobbers a value already set above).
[ -f "$INSTALL_ENV" ] && . "$INSTALL_ENV"
# Belt-and-braces fallback for the very first run, or if install.env is
# ever missing a key a newer version of this script expects.
: "${CT_VERSION:=latest}"
: "${CT_RELEASE_BASE:=}"
: "${CT_RELAY:=yes}"
: "${CT_RELAY_CAP_MB:=256}"
: "${CT_HTTPS:=none}"
: "${CT_HOSTNAME_PUBLIC:=}"
: "${CT_LETSENCRYPT_EMAIL:=}"
: "${CT_AUTOUPDATE:=no}"

if ! command -v nginx >/dev/null 2>&1; then
  LC_ALL=C apt-get update -qq
  LC_ALL=C DEBIAN_FRONTEND=noninteractive apt-get install -y -qq nginx curl ca-certificates >/dev/null
fi

# `pct create` never sets a root password (see hew-web.sh), so the console
# login prompt has no credentials that work. Auto-login root on the console
# instead — same mechanism community-scripts.org's install.func uses when a
# container has no password set. CREATE ONLY: restarting container-getty@1
# tears down whatever is currently attached to tty1, which on `update` run
# from inside the container's own console (not `pct exec`, which uses a
# pts/N pseudo-terminal instead) is the very login session running THIS
# script — it would kill its own shell mid-provision. The override is
# idempotent config written once at create time; there's nothing to redo
# on later updates.
if [ "$CT_MODE" = create ]; then
  GETTY_OVERRIDE=/etc/systemd/system/container-getty@1.service.d/override.conf
  mkdir -p "$(dirname "$GETTY_OVERRIDE")"
  cat <<'GETTY_EOF' >"$GETTY_OVERRIDE"
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --noclear --keep-baud tty%I 115200,38400,9600 $TERM
GETTY_EOF
  systemctl daemon-reload
  systemctl restart container-getty@1 2>/dev/null || true
fi

# --- Resolve this release's assets (app tarball, and the matching-arch
# relay tarball if the relay is enabled) from ONE release payload, so both
# always come from the exact same tag. --------------------------------
echo "Resolving release '$CT_VERSION'${CT_RELEASE_BASE:+ from $CT_RELEASE_BASE}..." >&2
# dpkg's arch name doesn't match the relay tarball's Rust target triple
# arch name, so map the two conventions explicitly.
case "$(dpkg --print-architecture)" in
  amd64) RELAY_ARCH=x86_64 ;;
  arm64) RELAY_ARCH=aarch64 ;;
  *) echo "unsupported architecture for hew-relay: $(dpkg --print-architecture)" >&2; exit 1 ;;
esac
# `|| true` on every one of these: under `set -eo pipefail`, a bare
# `VAR=$(pipeline)` assignment takes on the pipeline's exit status (e.g.
# grep finding no match), which would otherwise abort the script right
# here with no message instead of reaching the friendly `[ -n "$VAR" ] ||
# { ...; exit 1; }` checks just below each one.
if [ -n "$CT_RELEASE_BASE" ]; then
  # A release-shaped HTTP directory (mirror, air-gapped copy, or a locally
  # staged build): assets by name, and a `latest` file naming the tag.
  BASE="${CT_RELEASE_BASE%/}"
  if [ "$CT_VERSION" = "latest" ]; then
    RESOLVED_VERSION="$(curl -fsSL "$BASE/latest" | tr -d '[:space:]')" || true
    [ -n "$RESOLVED_VERSION" ] || { echo "could not read $BASE/latest (the mirror needs a 'latest' file naming the release tag)" >&2; exit 1; }
  else
    RESOLVED_VERSION="$CT_VERSION"
  fi
  WEB_URL="$BASE/hew-web-${RESOLVED_VERSION}.tar.gz"
  RELAY_URL="$BASE/hew-relay-${RESOLVED_VERSION}-linux-${RELAY_ARCH}.tar.gz"
  curl -fsSLI "$WEB_URL" >/dev/null || { echo "no hew-web asset at $WEB_URL" >&2; exit 1; }
  if [ "$CT_RELAY" = yes ]; then
    curl -fsSLI "$RELAY_URL" >/dev/null || { echo "no hew-relay asset at $RELAY_URL" >&2; exit 1; }
  else
    RELAY_URL=""
  fi
else
  if [ "$CT_VERSION" = "latest" ]; then
    RELEASE_JSON="$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest)" || true
  else
    RELEASE_JSON="$(curl -fsSL "https://api.github.com/repos/hew3d/hew/releases/tags/${CT_VERSION}")" || true
  fi
  [ -n "$RELEASE_JSON" ] || { echo "could not fetch release '$CT_VERSION' from github.com/hew3d/hew" >&2; exit 1; }

  WEB_URL="$(printf '%s' "$RELEASE_JSON" | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' | cut -d'"' -f4)" || true
  [ -n "$WEB_URL" ] || { echo "could not resolve a hew-web release asset for version '$CT_VERSION'" >&2; exit 1; }

  RELAY_URL=""
  if [ "$CT_RELAY" = yes ]; then
    RELAY_URL="$(printf '%s' "$RELEASE_JSON" | grep -o "\"browser_download_url\": *\"[^\"]*hew-relay-[^\"]*linux-${RELAY_ARCH}\\.tar\\.gz\"" | cut -d'"' -f4)" || true
    [ -n "$RELAY_URL" ] || { echo "could not resolve a hew-relay release asset for $RELAY_ARCH" >&2; exit 1; }
  fi
fi

# --- Web app: full replace, not a diff. --------------------------------
echo "Installing Hew web app..." >&2
curl -fsSL "$WEB_URL" -o /tmp/hew-web.tar.gz
mkdir -p "$APP_DIR"
rm -rf "${APP_DIR:?}"/*
tar -xzf /tmp/hew-web.tar.gz -C "$APP_DIR"
rm -f /tmp/hew-web.tar.gz

# Install the canonical nginx config FROM THE TARBALL's own deploy/, not a
# live fetch off `main` — that is the whole point of shipping deploy/
# inside the release: an installation is version-locked to the app it
# deploys, not whatever `main` happens to contain today.
install -d /etc/nginx/hew.d
install -m 0644 "$APP_DIR/deploy/nginx.conf" "$NGINX_CONF"
ln -sf "$NGINX_CONF" /etc/nginx/sites-enabled/hew
rm -f /etc/nginx/sites-enabled/default

# Keep a reference copy of what shipped (for debugging / support), then
# strip it back out of the served tree: nginx.conf already 404s /deploy/
# itself, but there is no reason to leave unserved files sitting there.
rm -rf "${ETC_DIR:?}/deploy"
mkdir -p "$ETC_DIR/deploy"
cp -r "$APP_DIR/deploy/." "$ETC_DIR/deploy/"
rm -rf "${APP_DIR:?}/deploy"

# Get a plain-HTTP nginx serving before anything below (relay's own probe,
# and certbot's --nginx plugin) needs it already up.
nginx -t
systemctl enable -q --now nginx
systemctl reload nginx

# --- Relay ("Open on Phone") -------------------------------------------
if [ "$CT_RELAY" = yes ]; then
  echo "Installing hew-relay ($RELAY_ARCH)..." >&2
  RELAY_TMP="$(mktemp -d)"
  curl -fsSL "$RELAY_URL" -o "$RELAY_TMP/hew-relay.tar.gz"
  tar -xzf "$RELAY_TMP/hew-relay.tar.gz" -C "$RELAY_TMP"
  install -m 0755 "$RELAY_TMP/hew-relay" /usr/local/bin/hew-relay
  install -m 0644 "$RELAY_TMP/hew-relay.service" /etc/systemd/system/hew-relay.service
  rm -rf "$RELAY_TMP"

  # --update must never regenerate an upload key: if one is already on
  # disk from a previous install, it wins over whatever CT_UPLOAD_KEY this
  # run carries (which is empty on every --update anyway — see
  # settings_block() on the host side).
  EXISTING_KEY=""
  [ -f "$ETC_DIR/relay.env" ] && EXISTING_KEY="$(sed -n 's/^HEW_RELAY_UPLOAD_KEY=//p' "$ETC_DIR/relay.env" | tail -1)"
  FINAL_KEY="${EXISTING_KEY:-${CT_UPLOAD_KEY:-}}"

  CAP_BYTES=$(( CT_RELAY_CAP_MB * 1048576 ))
  {
    echo "HEW_RELAY_LISTEN=127.0.0.1:8787"
    echo "HEW_RELAY_MAX_TOTAL_BYTES=${CAP_BYTES}"
    [ -n "$FINAL_KEY" ] && echo "HEW_RELAY_UPLOAD_KEY=${FINAL_KEY}"
  } >"$ETC_DIR/relay.env"
  chmod 600 "$ETC_DIR/relay.env"
  chown root:root "$ETC_DIR/relay.env"

  # The unit's own MemoryMax (512M) is a generic backstop; this drop-in
  # sizes it to what was actually asked for, with headroom above the
  # relay's own in-memory cap for runtime overhead.
  install -d /etc/systemd/system/hew-relay.service.d
  {
    echo "[Service]"
    echo "MemoryMax=$(( CT_RELAY_CAP_MB + 256 ))M"
  } >/etc/systemd/system/hew-relay.service.d/override.conf

  install -m 0644 "$ETC_DIR/deploy/hew.d/relay.conf" /etc/nginx/hew.d/relay.conf

  systemctl daemon-reload
  systemctl enable -q hew-relay
  systemctl restart hew-relay

  # Verify it actually answers before declaring success — a silently-dead
  # relay would otherwise only surface later, as a confusing 502 from
  # nginx the first time someone taps "Open on Phone".
  RELAY_OK=""
  for _ in $(seq 1 10); do
    if curl -fsS http://127.0.0.1:8787/relay/ 2>/dev/null | grep -q '"service":"hew-relay"'; then
      RELAY_OK=1
      break
    fi
    sleep 1
  done
  [ -n "$RELAY_OK" ] || { echo "hew-relay did not come up (curl http://127.0.0.1:8787/relay/)" >&2; exit 1; }
else
  systemctl disable -q --now hew-relay 2>/dev/null || true
  rm -f /etc/nginx/hew.d/relay.conf
fi

# --- HTTPS ---------------------------------------------------------------
# nginx.conf above was just re-copied fresh from the release tarball, so
# every branch here starts from the pristine canonical file and applies a
# deterministic transform — safe to re-run on every deploy, including
# --update, with no risk of double-editing a file that was already edited
# last time.
rm -f "$REDIRECT_CONF" /etc/nginx/sites-enabled/hew-redirect
case "$CT_HTTPS" in
  none)
    : # deploy/nginx.conf ships HTTP-only (port 80) already — nothing to do.
    ;;
  upstream)
    # TLS terminates in front of us (reverse proxy / tunnel); stay on
    # plain HTTP and just answer to the right name if one was given. The
    # app is static and doesn't read X-Forwarded-Proto itself, so there is
    # nothing else "trivial" to wire up here.
    if [ -n "$CT_HOSTNAME_PUBLIC" ]; then
      sed -i "s/server_name _;/server_name ${CT_HOSTNAME_PUBLIC};/" "$NGINX_CONF"
    fi
    ;;
  self-signed)
    command -v openssl >/dev/null 2>&1 || LC_ALL=C DEBIAN_FRONTEND=noninteractive apt-get install -y -qq openssl >/dev/null
    mkdir -p "$ETC_DIR/tls"
    LOCAL_IP="$(hostname -I | awk '{print $1}')"
    CERT_CN="${CT_HOSTNAME_PUBLIC:-$LOCAL_IP}"
    if [ ! -s "$ETC_DIR/tls/hew.crt" ]; then
      echo "Generating a 10-year self-signed certificate for ${CERT_CN}..." >&2
      openssl req -x509 -nodes -newkey rsa:2048 -days 3650 \
        -keyout "$ETC_DIR/tls/hew.key" -out "$ETC_DIR/tls/hew.crt" \
        -subj "/CN=${CERT_CN}" \
        -addext "subjectAltName=DNS:${CERT_CN},IP:${LOCAL_IP}"
      chmod 600 "$ETC_DIR/tls/hew.key"
    fi
    sed -i \
      -e "s/listen 80 default_server;/listen 443 ssl default_server;/" \
      -e "s#listen \[::\]:80 default_server;#listen [::]:443 ssl default_server;#" \
      -e "s#server_name _;#server_name ${CT_HOSTNAME_PUBLIC:-_};\n\n    ssl_certificate ${ETC_DIR}/tls/hew.crt;\n    ssl_certificate_key ${ETC_DIR}/tls/hew.key;#" \
      "$NGINX_CONF"
    cat >"$REDIRECT_CONF" <<REDIRECT
server {
    listen 80;
    listen [::]:80;
    server_name ${CT_HOSTNAME_PUBLIC:-_};
    return 301 https://\$host\$request_uri;
}
REDIRECT
    ln -sf "$REDIRECT_CONF" /etc/nginx/sites-enabled/hew-redirect
    ;;
  letsencrypt)
    [ -n "$CT_HOSTNAME_PUBLIC" ] || { echo "CT_HTTPS=letsencrypt requires a public hostname (var_hostname_public)" >&2; exit 1; }
    sed -i "s/server_name _;/server_name ${CT_HOSTNAME_PUBLIC};/" "$NGINX_CONF"
    nginx -t
    systemctl reload nginx
    LC_ALL=C DEBIAN_FRONTEND=noninteractive apt-get install -y -qq certbot python3-certbot-nginx >/dev/null
    # certbot reuses a still-valid existing certificate for these names
    # rather than reissuing one, so this is safe to run on every deploy —
    # which is what makes it "never regenerate the certificate" on
    # --update in practice, without needing our own extra bookkeeping.
    if [ -n "$CT_LETSENCRYPT_EMAIL" ]; then
      certbot --nginx -d "$CT_HOSTNAME_PUBLIC" --non-interactive --agree-tos --email "$CT_LETSENCRYPT_EMAIL"
    else
      certbot --nginx -d "$CT_HOSTNAME_PUBLIC" --non-interactive --agree-tos --register-unsafely-without-email
    fi
    ;;
  *)
    echo "unknown CT_HTTPS mode: $CT_HTTPS" >&2
    exit 1
    ;;
esac

nginx -t
systemctl reload nginx

# --- Persist settings so --update can re-read them without a wizard. ---
{
  printf ': "${CT_VERSION:=%s}"\n' "$(printf %q "$CT_VERSION")"
  printf ': "${CT_RELEASE_BASE:=%s}"\n' "$(printf %q "$CT_RELEASE_BASE")"
  printf ': "${CT_RELAY:=%s}"\n' "$(printf %q "$CT_RELAY")"
  printf ': "${CT_RELAY_CAP_MB:=%s}"\n' "$(printf %q "$CT_RELAY_CAP_MB")"
  printf ': "${CT_HTTPS:=%s}"\n' "$(printf %q "$CT_HTTPS")"
  printf ': "${CT_HOSTNAME_PUBLIC:=%s}"\n' "$(printf %q "$CT_HOSTNAME_PUBLIC")"
  printf ': "${CT_LETSENCRYPT_EMAIL:=%s}"\n' "$(printf %q "$CT_LETSENCRYPT_EMAIL")"
  printf ': "${CT_AUTOUPDATE:=%s}"\n' "$(printf %q "$CT_AUTOUPDATE")"
} >"$INSTALL_ENV"
chmod 600 "$INSTALL_ENV"

# --- Weekly auto-update timer -------------------------------------------
# --- The in-container update path (always installed). ------------------
mkdir -p /usr/local/lib/hew
# Save a copy of THIS run's provisioning script for later updates, but
# strip every `CT_X=` line it carried (this run's settings-block choices)
# and pin it to update mode instead: the saved copy must always re-derive
# its settings from install.env at run time (see the top of this script),
# or a refresh would silently re-apply whatever was true at creation time
# forever, undoing any later `update var_...=` override. CT_ variable names
# used as *local* variables anywhere above (e.g. CERT_CN, not CT_-prefixed)
# are deliberately kept out of the CT_ namespace so this filter can't catch
# them by accident.
{
  echo 'CT_MODE=update'
  sed -e '/^CT_[A-Z_]*=/d' "$0"
} >/usr/local/lib/hew/update.sh
chmod 700 /usr/local/lib/hew/update.sh

# `update` — the community-scripts convention: re-run the installer inside
# the container and it updates. Fresh from wherever this container's assets
# actually come from when reachable (so fixes to the installer itself
# arrive too), else the copy saved just above. A container installed from a
# mirror (CT_RELEASE_BASE) fetches the installer from THAT mirror, not
# GitHub — the mirror is a dev/test loop the maintainer is actively
# iterating on, and GitHub's `main` won't have those changes yet (it may
# not even have that release's assets). A GitHub-sourced install is
# unaffected: CT_RELEASE_BASE is unset, so this is exactly the old
# GitHub-only behavior. Either way it re-reads /etc/hew/install.env;
# `var_*` overrides in front of it work like they do for the host-side
# --update.
cat >/usr/bin/update <<'UPD'
#!/usr/bin/env bash
set -euo pipefail
[ -f /etc/hew/install.env ] && . /etc/hew/install.env
url="https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh"
[ -n "${CT_RELEASE_BASE:-}" ] && url="${CT_RELEASE_BASE%/}/hew-web.sh"
if script="$(curl -fsSL --max-time 20 "$url" 2>/dev/null)" && [ -n "$script" ]; then
  exec bash -c "$script" hew-web.sh "$@"
fi
echo "update: could not fetch the current installer from $url; using the copy saved at install time" >&2
exec bash /usr/local/lib/hew/update.sh
UPD
chmod 755 /usr/bin/update

if [ "$CT_AUTOUPDATE" = yes ]; then
  cat >/etc/systemd/system/hew-web-update.service <<'UNIT'
[Unit]
Description=Update the self-hosted Hew web app
[Service]
Type=oneshot
ExecStart=/usr/local/lib/hew/update.sh
UNIT

  cat >/etc/systemd/system/hew-web-update.timer <<'UNIT'
[Unit]
Description=Weekly Hew web app update check
[Timer]
OnCalendar=weekly
Persistent=true
[Install]
WantedBy=timers.target
UNIT

  systemctl daemon-reload
  systemctl enable -q --now hew-web-update.timer
else
  systemctl disable -q --now hew-web-update.timer 2>/dev/null || true
  rm -f /etc/systemd/system/hew-web-update.service /etc/systemd/system/hew-web-update.timer
fi
EOS
}

deploy_into() {
  local ctid="$1" mode="${2:-create}"
  local tmp
  tmp="$(mktemp)"
  { settings_block "$mode"; provision_script; } >"$tmp"
  pct push "$ctid" "$tmp" /root/hew-web-provision.sh
  rm -f "$tmp"
  msg_info "Installing/updating Hew web app in CT $ctid"
  pct exec "$ctid" -- bash /root/hew-web-provision.sh
  pct exec "$ctid" -- rm -f /root/hew-web-provision.sh
  msg_ok "Deployed"
}

wait_for_ct() {
  local ctid="$1"
  for _ in $(seq 1 30); do
    pct exec "$ctid" -- true >/dev/null 2>&1 && return 0
    sleep 1
  done
  return 1
}

# ---------------------------------------------------------------------------
# --update <CTID>: refresh in place, no wizard, no key/cert regeneration.
# ---------------------------------------------------------------------------

if [ "$IN_CT" = yes ]; then
  # Inside a container this script provisioned: update in place — the same
  # provisioning body `pct exec` would run, executed here. Any argument
  # (`--update`, a stray CTID) means the same thing; there is only one
  # container to act on.
  [ "$(id -u)" = 0 ] || { msg_err "update must run as root inside the container."; exit 1; }
  msg_info "Updating this container's Hew install from its persisted settings (/etc/hew/install.env)."
  tmp="$(mktemp)"
  { settings_block update; provision_script; } >"$tmp"
  bash "$tmp"
  rm -f "$tmp"
  ip="$(hostname -I 2>/dev/null | awk '{print $1}')" || true
  msg_ok "Hew web app updated: http://${ip:-<container-ip>}/"
  exit 0
fi

if [ "${1:-}" = "--update" ]; then
  ctid="${2:?usage: $SELF --update <CTID>}"
  # `|| true`: pct status on a nonexistent CTID exits non-zero, which under
  # pipefail would otherwise trip `set -e` here instead of the friendly
  # "not found" check on the next line.
  status="$(pct status "$ctid" 2>/dev/null | awk '{print $2}')" || true
  [ -n "$status" ] || { msg_err "CT $ctid not found."; exit 1; }
  if [ "$status" != "running" ]; then
    msg_info "Starting CT $ctid"
    pct start "$ctid"
    wait_for_ct "$ctid" || { msg_err "CT $ctid did not come up."; exit 1; }
  fi
  msg_info "Refreshing the Hew web app (and relay/TLS config, if enabled) in CT $ctid from its persisted settings."
  deploy_into "$ctid" update
  ip="$(pct exec "$ctid" -- hostname -I 2>/dev/null | awk '{print $1}')" || true
  msg_ok "Hew web app updated: http://${ip:-<container-ip>}/"
  exit 0
fi

# ---------------------------------------------------------------------------
# Create path
# ---------------------------------------------------------------------------

echo -e "${GN}Hew Web${CL} — Proxmox LXC setup"

# Backward-compatible env aliases (v1 names) — resolved before any
# defaulting below, so an old-style invocation still works unattended.
var_ctid="${var_ctid:-${CTID:-}}"
var_hostname="${var_hostname:-${CT_HOSTNAME:-}}"
var_cpu="${var_cpu:-${CORES:-}}"
var_ram="${var_ram:-${RAM_MB:-}}"
var_disk="${var_disk:-${DISK_GB:-}}"
var_brg="${var_brg:-${BRIDGE:-}}"
var_net="${var_net:-${NET_IP:-}}"
var_storage="${var_storage:-${STORAGE:-}}"
var_template_storage="${var_template_storage:-${TEMPLATE_STORAGE:-}}"
var_template="${var_template:-${OS_TEMPLATE:-}}"

# Hardcoded fallback defaults — today's sizing, used verbatim whenever a
# prompt below isn't asked (non-interactive, or "Default settings"). Kept
# in their own def_* names and passed to each prompt as its default: the
# resolve_* helpers skip the prompt whenever the var_* is already set, so
# pre-filling var_* here would silently turn every Advanced question into
# a no-op.
def_hostname=hew-web
def_disk=2
def_cpu=1
def_brg=vmbr0
def_net=dhcp
def_onboot=yes
def_unprivileged=yes
var_gateway="${var_gateway:-}"
var_dns="${var_dns:-}"
# Empty = "not set by the caller" (ask, or take the def_* default) — spelled
# out so an unset var_* is never a `set -u` trip.
var_onboot="${var_onboot:-}"
var_unprivileged="${var_unprivileged:-}"

# --- Default settings / Advanced settings (community-scripts' first
# question) -----------------------------------------------------------
if have_whiptail; then
  MODE_CHOICE="$(whiptail --backtitle "Hew Web" --title "Hew Web" \
    --menu "How do you want to configure this container?" 14 70 2 \
    default  "Default settings (recommended)" \
    advanced "Advanced settings" \
    3>&1 1>&2 2>&3)" || { msg_err "Cancelled."; exit 1; }
else
  MODE_CHOICE=default
fi
ADVANCED=no
[ "$MODE_CHOICE" = advanced ] && ADVANCED=yes

# --- Container sizing / identity ---------------------------------------
if [ -z "$var_ctid" ]; then
  CTID_DEFAULT="$(pvesh get /cluster/nextid)"
  resolve_input var_ctid "Hew Web" "Container ID:" "$CTID_DEFAULT"
fi
resolve_input var_hostname "Hew Web" "Container hostname:" "$def_hostname"
resolve_input var_disk "Hew Web" "Disk size (GB):" "$def_disk"
resolve_input var_cpu "Hew Web" "CPU cores:" "$def_cpu"
# The relay question hasn't been asked yet at this point in the flow (it's
# a "Hew option", asked after container sizing) — assume its default (on)
# for sizing purposes unless the caller already told us otherwise via
# var_relay. Extra RAM never hurts; there's no need to re-ask this once
# the relay choice is actually made below.
RAM_DEFAULT=512
[ "${var_relay:-yes}" != no ] && RAM_DEFAULT=1024
resolve_input var_ram "Hew Web" "RAM (MB) — the relay holds in-flight drops in memory, hence the bump when it's on:" "$RAM_DEFAULT"
resolve_input var_brg "Hew Web" "Network bridge:" "$def_brg"
resolve_input var_net "Hew Web" "IPv4 address ('dhcp', or a static CIDR like 192.168.1.50/24):" "$def_net"
if [ "$var_net" != dhcp ]; then
  resolve_input var_gateway "Hew Web" "Gateway IP (required for a static address):" "$var_gateway"
fi
resolve_input var_dns "Hew Web" "DNS server (blank = inherit from the host/DHCP):" "$var_dns"

if [ -z "$var_storage" ]; then
  # Storage selection is needed either way, not just in Advanced mode —
  # redirecting stdin from /dev/null re-triggers pick_storage's own
  # non-interactive fallback branch on a "Default settings" run without
  # duplicating its logic.
  if [ "$ADVANCED" = yes ]; then
    var_storage="$(pick_storage rootdir "Storage for the container's root filesystem" var_storage)"
  else
    var_storage="$(pick_storage rootdir "Storage for the container's root filesystem" var_storage </dev/null)"
  fi
fi
if [ -z "$var_template_storage" ]; then
  if [ "$ADVANCED" = yes ]; then
    var_template_storage="$(pick_storage vztmpl "Storage for the OS template" var_template_storage)"
  else
    var_template_storage="$(pick_storage vztmpl "Storage for the OS template" var_template_storage </dev/null)"
  fi
fi

if [ -z "$var_template" ]; then
  # dpkg's arch names (amd64/arm64/...) match the suffix pveam's template
  # filenames use. Proxmox VE itself only ships for amd64, but a host's
  # `pveam available` list still mixes in every other architecture's
  # templates — sort -V | tail -1 without this filter can silently pick a
  # foreign-arch template that fails to boot with an unrelated-looking
  # error.
  HOST_ARCH="$(dpkg --print-architecture)"
  msg_info "Checking for a Debian template on $var_template_storage ($HOST_ARCH)"
  pveam update >/dev/null 2>&1 || true
  # `|| true`: grep finding no match makes the pipeline (and, under
  # pipefail, this bare assignment) exit non-zero, which would otherwise
  # trip `set -e` right here instead of the friendly check on the next line.
  AUTO_TEMPLATE="$(pveam available -section system 2>/dev/null \
    | awk '{print $2}' | grep -E "^debian-1[23]-standard.*_${HOST_ARCH}\.tar\.(gz|zst)\$" \
    | sort -V | tail -1)" || true
  [ -n "$AUTO_TEMPLATE" ] || { msg_err "No debian-12/13 $HOST_ARCH template found via pveam; set var_template explicitly."; exit 1; }
  var_template="$AUTO_TEMPLATE"
  if [ "$ADVANCED" = yes ] && have_whiptail; then
    # Offer a menu built from what's already downloaded to this storage
    # (fast, no network) plus the auto-picked template, defaulting to it.
    TEMPLATE_ROWS=()
    while read -r line; do
      tmpl="$(basename "$line")"
      [ -n "$tmpl" ] || continue
      TEMPLATE_ROWS+=("$tmpl" "")
    done < <(pveam list "$var_template_storage" 2>/dev/null | tail -n +2 | awk '{print $1}')
    if ! printf '%s\n' "${TEMPLATE_ROWS[@]-}" | grep -qx "$AUTO_TEMPLATE"; then
      TEMPLATE_ROWS+=("$AUTO_TEMPLATE" "(auto-picked, will download)")
    fi
    CHOICE="$(whiptail --backtitle "Hew Web" --title "Hew Web" --default-item "$AUTO_TEMPLATE" \
      --menu "OS template:" 20 78 10 "${TEMPLATE_ROWS[@]}" 3>&1 1>&2 2>&3)" || true
    [ -n "$CHOICE" ] && var_template="$CHOICE"
  fi
  if ! pveam list "$var_template_storage" 2>/dev/null | grep -q "$var_template"; then
    msg_info "Downloading $var_template"
    pveam download "$var_template_storage" "$var_template" >/dev/null
  fi
fi

resolve_yesno var_onboot "Hew Web" "Start the container on host boot?" "$def_onboot"
resolve_yesno var_unprivileged "Hew Web" "Create as an unprivileged container?" "$def_unprivileged"

# --- Hew options: asked in BOTH Default and Advanced runs for the first
# question; the rest only in Advanced (Default takes their defaults). ---
resolve_yesno_always var_relay "Hew Web" "Enable the \"Open on Phone\" relay?" yes

if [ "$var_relay" = yes ]; then
  resolve_input var_relay_mem_mb "Hew Web" "Relay memory cap (MB) — becomes HEW_RELAY_MAX_TOTAL_BYTES:" 256
  resolve_yesno var_upload_key "Hew Web" "Require an upload key for the relay? (recommended if it's reachable from the internet)" no
else
  var_relay_mem_mb="${var_relay_mem_mb:-256}"
  var_upload_key="${var_upload_key:-no}"
fi

resolve_menu var_https "Hew Web" "HTTPS:" none \
  none         "None — plain HTTP (LAN use)" \
  self-signed  "Self-signed certificate (LAN, trust it manually)" \
  letsencrypt  "Let's Encrypt (public hostname, ports 80+443 reachable)" \
  upstream     "Terminated upstream (reverse proxy / tunnel already does TLS)"

if [ "$var_https" != none ]; then
  resolve_input var_hostname_public "Hew Web" "Hostname for $var_https (FQDN for Let's Encrypt; LAN name or IP is fine for self-signed/upstream):" "${var_hostname_public:-}"
fi
var_hostname_public="${var_hostname_public:-}"

if [ "$var_https" = letsencrypt ]; then
  [ -n "$var_hostname_public" ] || { msg_err "var_https=letsencrypt requires var_hostname_public (a public FQDN)."; exit 1; }
  resolve_input var_letsencrypt_email "Hew Web" "Email for Let's Encrypt renewal notices (blank = none):" "${var_letsencrypt_email:-}"
fi
var_letsencrypt_email="${var_letsencrypt_email:-}"

resolve_yesno var_autoupdate "Hew Web" "Enable weekly automatic updates?" no

# Resolve the upload key itself now (host-side, before provisioning) so it
# can be shown once on the final screen below.
RESOLVED_UPLOAD_KEY=""
case "$var_upload_key" in
  # 24 random bytes → 32 base64url characters. Deliberately NOT the classic
  # `tr -dc … </dev/urandom | head -c 32`: there `head` closes the pipe while
  # `tr` is still streaming an endless /dev/urandom, `tr` dies of SIGPIPE,
  # and under `set -o pipefail` that is exit 141 — which `set -e` turned
  # into a silent exit right here. Every writer below finishes on its own.
  yes) RESOLVED_UPLOAD_KEY="$(head -c 24 /dev/urandom | base64 | tr -d '\n=' | tr '+/' '-_')" ;;
  no)  RESOLVED_UPLOAD_KEY="" ;;
  *)   RESOLVED_UPLOAD_KEY="$var_upload_key" ;;
esac

# --- Summary + confirm ---------------------------------------------------
UPLOAD_KEY_SUMMARY="none"
[ "$var_upload_key" = yes ] && UPLOAD_KEY_SUMMARY="generated"
[ "$var_upload_key" != yes ] && [ "$var_upload_key" != no ] && UPLOAD_KEY_SUMMARY="provided"

# Built as a plain string, not `$([ cond ] && echo ...)` embedded in the
# assignment: under `set -e`, a bare `VAR=$(pipeline)` takes on the
# pipeline's exit status, and `[ cond ]` failing there would silently kill
# the whole script right here instead of just skipping the cap suffix.
RELAY_CAP_SUFFIX=""
[ "$var_relay" = yes ] && RELAY_CAP_SUFFIX=" (cap ${var_relay_mem_mb}MB)"

SUMMARY="CTID:          ${var_ctid:-<next free>}
Hostname:      $var_hostname
Resources:     ${var_cpu}c / ${var_ram}MB / ${var_disk}GB
Network:       bridge=$var_brg ip=$var_net${var_gateway:+ gw=$var_gateway}${var_dns:+ dns=$var_dns}
Storage:       $var_storage (rootfs) / $var_template_storage (template)
Template:      $var_template
Start on boot: $var_onboot
Unprivileged:  $var_unprivileged
Open on Phone: ${var_relay}${RELAY_CAP_SUFFIX}
Upload key:    $UPLOAD_KEY_SUMMARY
HTTPS:         $var_https${var_hostname_public:+ ($var_hostname_public)}
Auto-update:   $var_autoupdate
Version:       ${var_version:-latest}${var_release_base:+ (from $var_release_base)}"

if have_whiptail; then
  whiptail --backtitle "Hew Web" --title "Hew Web — confirm" --yesno "$SUMMARY

Proceed with these settings?" 24 76 || { msg_err "Cancelled."; exit 1; }
else
  echo -e "$SUMMARY" >&2
fi

if [ -z "$var_ctid" ]; then
  var_ctid="$(pvesh get /cluster/nextid)"
fi

# A failure past this point leaves CT $var_ctid created but
# half-provisioned — no automatic rollback, so at least point at how to
# clean it up by hand.
trap 'msg_err "Setup failed. CT $var_ctid may be partially provisioned — inspect with: pct status $var_ctid (or pct destroy $var_ctid to remove it)"' ERR

NET0="name=eth0,bridge=${var_brg},ip=${var_net}"
[ "$var_net" != dhcp ] && [ -n "$var_gateway" ] && NET0="${NET0},gw=${var_gateway}"
UNPRIV=0; [ "$var_unprivileged" = yes ] && UNPRIV=1
ONBOOT=0; [ "$var_onboot" = yes ] && ONBOOT=1
PCT_CREATE_ARGS=(
  --hostname "$var_hostname"
  --cores "$var_cpu"
  --memory "$var_ram"
  --swap "$var_ram"
  --net0 "$NET0"
  --rootfs "$var_storage:$var_disk"
  --unprivileged "$UNPRIV"
  --features nesting=0
  --onboot "$ONBOOT"
)
[ -n "$var_dns" ] && PCT_CREATE_ARGS+=(--nameserver "$var_dns")

msg_info "Creating CT $var_ctid ($var_hostname, ${var_cpu}c/${var_ram}MB/${var_disk}GB, $var_storage)"
pct create "$var_ctid" "$var_template_storage:vztmpl/$var_template" "${PCT_CREATE_ARGS[@]}" >/dev/null
msg_ok "Created CT $var_ctid"

msg_info "Starting CT $var_ctid"
pct start "$var_ctid"
wait_for_ct "$var_ctid" || { msg_err "CT $var_ctid did not come up."; exit 1; }
msg_ok "Started"

deploy_into "$var_ctid" create

# --- Final screen ---------------------------------------------------------
ip="$(pct exec "$var_ctid" -- hostname -I 2>/dev/null | awk '{print $1}')" || true
case "$var_https" in
  none)        scheme=http;  origin_host="${ip:-<container-ip>}" ;;
  upstream)    scheme=http;  origin_host="${var_hostname_public:-${ip:-<container-ip>}}" ;;
  self-signed) scheme=https; origin_host="${var_hostname_public:-${ip:-<container-ip>}}" ;;
  letsencrypt) scheme=https; origin_host="$var_hostname_public" ;;
esac
origin="${scheme}://${origin_host}"

msg_ok "Hew web app running: ${origin}/"

# Everything below this line is the deliverable the self-hoster actually
# needs to copy — it goes to real stdout, unlike the msg_* progress lines
# above. The upload key is the only secret this script ever prints, and
# only here, only once.
echo "Desktop: Settings > Advanced > Server"
echo "  ${origin}"
if [ -n "$RESOLVED_UPLOAD_KEY" ]; then
  echo "Upload key (shown once — store it, it is not printed again):"
  echo "  ${RESOLVED_UPLOAD_KEY}"
fi
if [ "$var_https" = self-signed ]; then
  echo "Self-signed certificate to trust on your phone and desktop:"
  echo "  scp root@${ip:-<container-ip>}:/etc/hew/tls/hew.crt ."
fi
echo "Update after future releases (inside the container: pct enter $var_ctid):"
echo "  update"
echo "or from this host:"
echo "  $SELF --update $var_ctid"
