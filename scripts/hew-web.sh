#!/usr/bin/env bash
# Provisions a Debian LXC container on a Proxmox VE host running the Hew
# web app, following the shape (banner, var_* overrides, msg_info/msg_ok
# progress lines, --update mode) that community-scripts.org Proxmox VE
# Scripts use.
#
# It deliberately does NOT source that project's misc/build.func: that
# helper's install step is hardcoded to fetch install/<app>.sh from
# community-scripts/ProxmoxVE's own repo, which has no entry for Hew (this
# app isn't in that catalog — yet). This script is the standalone
# equivalent for running today; a real submission there would later split
# it into their ct/+install/ file pair.
#
# Run from a Proxmox VE host shell:
#   ./hew-web.sh                # create a new container
#   ./hew-web.sh --update 105   # re-deploy the latest release into CTID 105
#
# See docs/SELF_HOSTING.md for what's actually being deployed and why.

set -euo pipefail

REPO="hew3d/hew"
CT_HOSTNAME="${CT_HOSTNAME:-hew-web}"
CORES="${CORES:-1}"
RAM_MB="${RAM_MB:-512}"
DISK_GB="${DISK_GB:-2}"
BRIDGE="${BRIDGE:-vmbr0}"
NET_IP="${NET_IP:-dhcp}"
STORAGE="${STORAGE:-}"
TEMPLATE_STORAGE="${TEMPLATE_STORAGE:-}"
OS_TEMPLATE="${OS_TEMPLATE:-}"
CTID="${CTID:-}"

GN='\033[1;92m'; YW='\033[1;33m'; RD='\033[1;31m'; CL='\033[m'
# All three write to stderr, not just msg_err: pick_storage() below returns
# its answer over stdout via command substitution, and a msg_info sharing
# that stream would land inside the captured value instead of the terminal.
msg_info() { echo -e " ${YW}○${CL} $1" >&2; }
msg_ok()   { echo -e " ${GN}✓${CL} $1" >&2; }
msg_err()  { echo -e " ${RD}✗${CL} $1" >&2; }

command -v pct >/dev/null 2>&1 || {
  msg_err "pct not found — run this from a Proxmox VE host shell, not inside a container."
  exit 1
}

# Interactive storage picker, community-scripts.org style: a whiptail menu
# when one's available and we're actually attached to a terminal, otherwise
# a silent fallback to the first active storage (scripted/cron runs, or a
# host without whiptail installed).
pick_storage() {
  local content="$1" title="$2" override_var="$3"
  local -a rows=()
  local name type status total used avail pct
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
    whiptail --title "Hew Web" --menu "$title" 18 70 8 "${rows[@]}" 3>&1 1>&2 2>&3
  else
    msg_info "Non-interactive shell — defaulting to storage '${rows[0]}' (override with $override_var=<name>)."
    echo "${rows[0]}"
  fi
}

provision_script() {
  # Emitted into the container and run there. Mirrors docs/SELF_HOSTING.md
  # Option A almost verbatim, plus the "full replace" update behavior.
  cat <<'EOS'
set -euo pipefail
APP_DIR=/var/www/hew
CONF=/etc/nginx/sites-available/hew

if ! command -v nginx >/dev/null 2>&1; then
  LC_ALL=C apt-get update -qq
  LC_ALL=C DEBIAN_FRONTEND=noninteractive apt-get install -y -qq nginx curl ca-certificates >/dev/null
fi

# `pct create` never sets a root password (see hew-web.sh), so the console
# login prompt has no credentials that work. Auto-login root on the console
# instead — same mechanism community-scripts.org's install.func uses when a
# container has no password set.
GETTY_OVERRIDE=/etc/systemd/system/container-getty@1.service.d/override.conf
mkdir -p "$(dirname "$GETTY_OVERRIDE")"
cat <<'GETTY_EOF' >"$GETTY_OVERRIDE"
[Service]
ExecStart=
ExecStart=-/sbin/agetty --autologin root --noclear --keep-baud tty%I 115200,38400,9600 $TERM
GETTY_EOF
systemctl daemon-reload
systemctl restart container-getty@1 2>/dev/null || true

TARBALL_URL="$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)"
[ -n "$TARBALL_URL" ] || { echo "could not resolve latest hew-web release asset" >&2; exit 1; }

curl -fsSL "$TARBALL_URL" -o /tmp/hew-web.tar.gz
mkdir -p "$APP_DIR"
rm -rf "${APP_DIR:?}"/*
tar -xzf /tmp/hew-web.tar.gz -C "$APP_DIR"
rm -f /tmp/hew-web.tar.gz

curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/shells/web/deploy/nginx.conf -o "$CONF"
ln -sf "$CONF" /etc/nginx/sites-enabled/hew
rm -f /etc/nginx/sites-enabled/default
nginx -t
systemctl enable -q --now nginx
systemctl reload nginx
EOS
}

deploy_into() {
  local ctid="$1"
  local tmp
  tmp="$(mktemp)"
  provision_script >"$tmp"
  pct push "$ctid" "$tmp" /root/hew-web-provision.sh
  rm -f "$tmp"
  msg_info "Installing/updating Hew web app in CT $ctid"
  pct exec "$ctid" -- bash /root/hew-web-provision.sh
  pct exec "$ctid" -- rm -f /root/hew-web-provision.sh
  msg_ok "Deployed"
}

if [ "${1:-}" = "--update" ]; then
  ctid="${2:?usage: $0 --update <CTID>}"
  status="$(pct status "$ctid" 2>/dev/null | awk '{print $2}')"
  [ -n "$status" ] || { msg_err "CT $ctid not found."; exit 1; }
  if [ "$status" != "running" ]; then
    msg_info "Starting CT $ctid"
    pct start "$ctid"
    for _ in $(seq 1 30); do
      pct exec "$ctid" -- true >/dev/null 2>&1 && break
      sleep 1
    done
  fi
  msg_info "This replaces /var/www/hew and the default nginx site inside CT $ctid."
  deploy_into "$ctid"
  ip="$(pct exec "$ctid" -- hostname -I 2>/dev/null | awk '{print $1}')"
  msg_ok "Hew web app updated: http://${ip:-<container-ip>}/"
  exit 0
fi

echo -e "${GN}Hew Web${CL} — Proxmox LXC setup"

if [ -z "$CTID" ]; then
  CTID="$(pvesh get /cluster/nextid)"
fi

# A failure past this point leaves CT $CTID created but half-provisioned —
# no automatic rollback, so at least point at how to clean it up by hand.
trap 'msg_err "Setup failed. CT $CTID may be partially provisioned — inspect with: pct status $CTID (or pct destroy $CTID to remove it)"' ERR

if [ -z "$STORAGE" ]; then
  STORAGE="$(pick_storage rootdir "Storage for the container's root filesystem" STORAGE)"
fi

if [ -z "$TEMPLATE_STORAGE" ]; then
  TEMPLATE_STORAGE="$(pick_storage vztmpl "Storage for the OS template" TEMPLATE_STORAGE)"
fi

if [ -z "$OS_TEMPLATE" ]; then
  # dpkg's arch names (amd64/arm64/...) match the suffix pveam's template
  # filenames use. Proxmox VE itself only ships for amd64, but a host's
  # `pveam available` list still mixes in every other architecture's
  # templates — sort -V | tail -1 without this filter can silently pick a
  # foreign-arch template that fails to boot with an unrelated-looking error.
  HOST_ARCH="$(dpkg --print-architecture)"
  msg_info "Checking for a Debian template on $TEMPLATE_STORAGE ($HOST_ARCH)"
  pveam update >/dev/null 2>&1 || true
  OS_TEMPLATE="$(pveam available -section system 2>/dev/null \
    | awk '{print $2}' | grep -E "^debian-1[23]-standard.*_${HOST_ARCH}\.tar\.(gz|zst)\$" \
    | sort -V | tail -1)"
  [ -n "$OS_TEMPLATE" ] || { msg_err "No debian-12/13 $HOST_ARCH template found via pveam; set OS_TEMPLATE explicitly."; exit 1; }
  if ! pveam list "$TEMPLATE_STORAGE" 2>/dev/null | grep -q "$OS_TEMPLATE"; then
    msg_info "Downloading $OS_TEMPLATE"
    pveam download "$TEMPLATE_STORAGE" "$OS_TEMPLATE" >/dev/null
  fi
fi

msg_info "Creating CT $CTID ($CT_HOSTNAME, ${CORES}c/${RAM_MB}MB/${DISK_GB}GB, $STORAGE)"
pct create "$CTID" "$TEMPLATE_STORAGE:vztmpl/$OS_TEMPLATE" \
  --hostname "$CT_HOSTNAME" \
  --cores "$CORES" \
  --memory "$RAM_MB" \
  --swap "$RAM_MB" \
  --net0 "name=eth0,bridge=$BRIDGE,ip=$NET_IP" \
  --rootfs "$STORAGE:$DISK_GB" \
  --unprivileged 1 \
  --features nesting=0 \
  --onboot 1 >/dev/null
msg_ok "Created CT $CTID"

msg_info "Starting CT $CTID"
pct start "$CTID"
for _ in $(seq 1 30); do
  pct exec "$CTID" -- true >/dev/null 2>&1 && break
  sleep 1
done
msg_ok "Started"

deploy_into "$CTID"

ip="$(pct exec "$CTID" -- hostname -I 2>/dev/null | awk '{print $1}')"
msg_ok "Hew web app running: http://${ip:-<container-ip>}/"
echo -e "${YW}Re-run with '--update $CTID' after future releases to redeploy.${CL}"
