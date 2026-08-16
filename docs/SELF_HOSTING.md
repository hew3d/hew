# Self-Hosting Hew

This is a step-by-step guide to running your own copy of the Hew web app —
on a homelab container (LXC/CT), a VM, a Docker/Podman host, or any Linux
box you control. Along with it you can, optionally, run your own copy of
the small relay service that makes "Open on Phone" work against your own
server instead of Hew's.

It does not cover `app.hew3d.com`, which is Cloudflare Pages, deployed by
CI on every release (`shells/web/README.md` covers that side, and none of
it is something you need to reproduce here).

## What you're deploying

The web app is a **static single-page app** — plain HTML/JS/CSS/wasm files,
no backend, no database, no API server. `shells/web`'s build step just
copies `app`'s Vite build and injects a CSP `<meta>` tag; the result is a
folder you can serve from anything that can serve files. Releases publish
that folder pre-built as a tarball, so self-hosting needs no Node, no Rust,
no build toolchain at all.

That simplicity is also the trap: a naive "point any file server at the
folder" setup gets a few things subtly wrong, because this app has real
requirements a generic static site doesn't:

- **Security headers.** The build's CSP is shipped as a `<meta>` tag, which
  is the portable floor for hosts you don't control — but it can't express
  `frame-ancestors` (clickjacking protection). Where you control the host
  (you do, here), send it as a real HTTP header instead.
- **No SPA history-fallback rewrite.** Hew routes on `location.hash`, not
  the path — every real URL is a real file. The common "rewrite unknown
  paths to `index.html`" SPA convention is *wrong* for this app: it turns a
  stale or mistyped `/assets/*.js` reference into an HTML document the
  browser then tries to parse as JavaScript. You want a real 404.
- **Root-relative asset paths.** The build references `/assets/...` from
  the domain root, so it must be served at `/`, not a subpath like
  `/hew/`.
- **Cache-Control for the service worker and `index.html`.** These are the
  update entry points; caching them long-term at the HTTP layer can leave a
  client stuck on a stale build even after you deploy a new one.
- **A secure context for PWA install and offline caching.** The service
  worker (install-to-home-screen, offline use) only registers over HTTPS or
  `localhost`. The app itself works fine over plain LAN HTTP — you just
  won't get those two features (and, if you also run the relay, the phone's
  in-app QR scanner needs a secure context too — see [HTTPS and
  certificates](#https-and-certificates)).

None of this is exotic, but it's also not what you get by dropping files
into a directory and hoping. Every release tarball carries a canonical,
version-locked `deploy/` directory — `nginx.conf`, the relay proxy stanza,
the relay's systemd unit — that encodes all of it, so every option below
installs from that instead of re-deriving it by hand.

### The relay ("Open on Phone"), in one paragraph

"Open on Phone" — the QR handoff from the desktop app to Shop Mode — is a
second, optional service: `hew-relay`, a small one-shot ciphertext dead-drop
that the desktop uploads an encrypted document to and the phone downloads
it from once. It holds nothing on disk, keeps a drop in memory for at most
ten minutes or until it's read once (whichever comes first), and never sees
a decryption key or a plaintext byte — the desktop encrypts before
uploading and the key rides the QR code's URL fragment, which never reaches
any server (`workers/share-relay/README.md`'s "Security model" section is
the full design). You don't need it to run the editor at all — skip every
"Relay" section below if "Open on Phone" isn't something you want. If you
do want it, the whole design rests on one rule:

> Whatever origin serves the Hew web app also serves the relay under
> `/relay/`. The phone computes this itself
> (`location.origin + '/relay/'`) — nothing to configure there. The desktop
> has exactly one setting for this, **Settings ▸ Advanced ▸ Server**: Hew
> cloud, or self-hosted with the origin you're running (plus an optional
> upload key). It derives the upload URL (`<origin>/relay/drop`) and the
> QR's receive URL (`<origin>/#recv=…`) from that one setting.

Same-origin is not a stylistic choice — it's what lets the phone need zero
configuration (no address to type in, no pairing step) and what avoids CORS
entirely: a proxied `/relay/` under the app's own origin is a same-origin
request from the phone's point of view.

## Pick a path

| Path | Best for |
| --- | --- |
| [A: native nginx](#option-a-native-nginx) | A dedicated LXC or VM; the recommended default |
| [B: containers](#option-b-containers-docker-or-podman) | A Docker- or Podman-first host (Synology, Unraid, a bare box) |
| [C: existing web server](#option-c-an-existing-web-server-or-reverse-proxy) | You already run nginx/Apache/Caddy/Traefik for other sites |
| [D: Proxmox VE installer](#option-d-proxmox-ve-lxc-installer) | A Proxmox VE host — a wizard that does Option A for you |

Every path can install from a private mirror instead of GitHub — see
[Get the release assets](#get-the-release-assets).

## Get the release assets

Latest release, resolved through the GitHub API:

```sh
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o hew-web.tar.gz
```

To pin a specific version instead, use the direct URL pattern:
`https://github.com/hew3d/hew/releases/download/vX.Y.Z/hew-web-vX.Y.Z.tar.gz`
(and the equivalent `hew-relay-vX.Y.Z-linux-<arch>.tar.gz` for the relay).

**`hew-web-vX.Y.Z.tar.gz`** — the self-hosting tarball. Its root (`.`) *is*
the web root — extract it straight into whatever directory you're serving
from. It also carries a top-level `deploy/` directory (`nginx.conf`,
`hew.d/relay.conf`, `hew-relay.service`) so an installation is version-
locked to the exact release it's running, not to whatever `main` says
today. Never `curl` these configs from `main` separately — always take them
from the tarball you actually deployed.

**`hew-relay-vX.Y.Z-linux-<arch>.tar.gz`** — the relay, only if you want
"Open on Phone". Both architectures ship for every release:

```sh
case "$(uname -m)" in
  x86_64)          ARCH=x86_64 ;;
  aarch64|arm64)   ARCH=aarch64 ;;
esac
```

carries `hew-relay` (a static musl binary), `hew-relay.service` (a systemd
unit), and `README.md`.

**`ghcr.io/hew3d/hew-relay:<version>`** (also tagged `:latest`) — the same
relay binary as a scratch container image, multi-arch (`linux/amd64` +
`linux/arm64`), listening on `0.0.0.0:8787` and running as uid `65534`.
Used by Option B.

## Option A: native nginx

The standard, lowest-friction path for a dedicated container or VM — and
what the [Proxmox VE LXC installer](#option-d-proxmox-ve-lxc-installer)
does under the hood. Tested against Debian/Ubuntu; adjust package manager
commands for your distro.

### 1. Install nginx

```sh
sudo apt update
sudo apt install -y nginx curl
```

### 2. Download and extract the app

```sh
sudo mkdir -p /var/www/hew
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o /tmp/hew-web.tar.gz
sudo tar -xzf /tmp/hew-web.tar.gz -C /var/www/hew
rm /tmp/hew-web.tar.gz
```

### 3. Install the canonical nginx config

Install from the tarball's own `deploy/`, not a separate download — that's
the whole point of shipping it alongside the app:

```sh
sudo cp /var/www/hew/deploy/nginx.conf /etc/nginx/sites-available/hew
sudo ln -sf /etc/nginx/sites-available/hew /etc/nginx/sites-enabled/hew
sudo rm -f /etc/nginx/sites-enabled/default
sudo mkdir -p /etc/nginx/hew.d
sudo nginx -t && sudo systemctl reload nginx
```

(If your nginx layout uses `conf.d` instead of `sites-available`/
`sites-enabled`, drop the file in `/etc/nginx/conf.d/hew.conf` instead.)

### 4. Verify

Open `http://<host-ip>/` in a browser — Hew should boot (React mounts, the
WASM kernel initializes, the 3D viewport appears) with no CSP violations in
the console. From the command line:

```sh
curl -sI http://<host-ip>/ | grep -i "content-security-policy\|x-content-type"
```

should show both headers on the response.

### Relay

Optional — skip this section entirely if you don't want "Open on Phone"
against your own server.

1. **Download the relay archive** for your host's architecture:

   ```sh
   case "$(uname -m)" in x86_64) ARCH=x86_64 ;; aarch64|arm64) ARCH=aarch64 ;; esac
   curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
     | grep -o "\"browser_download_url\": *\"[^\"]*hew-relay-[^\"]*-linux-${ARCH}\\.tar\\.gz\"" \
     | cut -d'"' -f4)" -o /tmp/hew-relay.tar.gz
   sudo mkdir -p /tmp/hew-relay && sudo tar -xzf /tmp/hew-relay.tar.gz -C /tmp/hew-relay
   ```

2. **Install the binary and the unit:**

   ```sh
   sudo install -m 755 /tmp/hew-relay/hew-relay /usr/local/bin/hew-relay
   sudo install -m 644 /tmp/hew-relay/hew-relay.service /etc/systemd/system/hew-relay.service
   rm -rf /tmp/hew-relay /tmp/hew-relay.tar.gz
   ```

3. **Configure it.** Every setting is an environment variable read from
   `/etc/hew/relay.env` (mode 600 — there is no config file):

   ```sh
   sudo mkdir -p /etc/hew
   sudo tee /etc/hew/relay.env >/dev/null <<'EOF'
   HEW_RELAY_LISTEN=127.0.0.1:8787
   HEW_RELAY_MAX_TOTAL_BYTES=268435456
   # HEW_RELAY_UPLOAD_KEY=choose-a-key       # set this if the server is reachable from the internet
   # HEW_RELAY_MAX_BYTES=33554432            # per-drop cap; default is 32 MiB
   # HEW_RELAY_TTL_SECS=600                  # how long an unread drop lives
   # HEW_RELAY_ALLOW_ORIGINS=                # only for a phone served from a DIFFERENT origin than the relay
   EOF
   sudo chmod 600 /etc/hew/relay.env
   ```

   `HEW_RELAY_ALLOW_ORIGINS` is the one setting you almost never need — it's
   for a split-origin layout, not the same-origin one this guide sets up.
   Leave it unset.

4. **Wire nginx to it, then remove the tarball's `deploy/` directory** —
   nginx already serves `/deploy/` as a 404, but it isn't part of the app
   either:

   ```sh
   sudo cp /var/www/hew/deploy/hew.d/relay.conf /etc/nginx/hew.d/relay.conf
   sudo rm -rf /var/www/hew/deploy
   sudo nginx -t && sudo systemctl reload nginx
   ```

   (Not installing the relay? Just run the `rm -rf` line — `deploy/` has
   nothing else to give you.)

5. **Start it and verify, first behind loopback, then through nginx:**

   ```sh
   sudo systemctl enable --now hew-relay
   curl -s http://127.0.0.1:8787/relay/
   curl -s http://<host-ip>/relay/
   ```

   Both should return `{"service":"hew-relay","contract":1,...}`. A drop
   that would push the server past `HEW_RELAY_MAX_TOTAL_BYTES` gets a `503
   {"error":"relay full"}` with a `Retry-After` header rather than being
   accepted — the desktop shows this as "the relay is full" rather than a
   bare failure. If you raise `HEW_RELAY_MAX_TOTAL_BYTES`, also raise the
   unit's `MemoryMax` (default `512M`) — it's the hard backstop above the
   relay's own limit, and the two should move together.

### Updating

Full replace, not a diff — repeat the download/extract steps over the old
contents of `/var/www/hew`, then reinstall the config and reload:

```sh
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o /tmp/hew-web.tar.gz
sudo rm -rf /var/www/hew/*
sudo tar -xzf /tmp/hew-web.tar.gz -C /var/www/hew
rm /tmp/hew-web.tar.gz
sudo cp /var/www/hew/deploy/nginx.conf /etc/nginx/sites-available/hew
sudo rm -rf /var/www/hew/deploy
sudo nginx -t && sudo systemctl reload nginx
```

If you run the relay too, it's a drop-in binary swap — the unit and env
file rarely change:

```sh
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o "\"browser_download_url\": *\"[^\"]*hew-relay-[^\"]*-linux-${ARCH}\\.tar\\.gz\"" \
  | cut -d'"' -f4)" -o /tmp/hew-relay.tar.gz
sudo tar -xzf /tmp/hew-relay.tar.gz -C /tmp hew-relay
sudo install -m 755 /tmp/hew-relay /usr/local/bin/hew-relay
rm /tmp/hew-relay /tmp/hew-relay.tar.gz
sudo systemctl restart hew-relay
```

## Option B: containers (Docker or Podman)

If your host is container-first (Synology, Unraid, a bare Linux box you'd
rather not touch directly) rather than an LXC, run the same config in a
stock `nginx` image instead of installing nginx natively. There is no
Hew-specific web image to build or maintain — it's the same static folder
and the same config file, mounted in.

Common layout for both Docker and Podman:

```sh
mkdir -p hew-web && cd hew-web
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o hew-web.tar.gz
mkdir -p www && tar -xzf hew-web.tar.gz -C www && rm hew-web.tar.gz
cp www/deploy/nginx.conf .
cp www/deploy/hew.d/relay.conf .      # only if you're running the relay
rm -rf www/deploy
```

If you're running the relay, its settings live in a `relay.env` file the
relay container reads (mode 600 — it may hold the upload key). The image
already sets `HEW_RELAY_LISTEN=0.0.0.0:8787`; everything else keeps the
binary's defaults unless you say otherwise:

```sh
cat > relay.env <<'EOF'
HEW_RELAY_MAX_TOTAL_BYTES=268435456
# HEW_RELAY_UPLOAD_KEY=choose-a-key    # set this if the server is reachable from the internet
EOF
chmod 600 relay.env
```

One layout detail differs between compose and a Podman pod, and it's the
single most common thing to get wrong: the shipped `relay.conf` proxies to
`127.0.0.1:8787`. Under **compose** (Docker or Podman) each service is its
own network namespace, so nginx must reach the relay by *service name* —
rewrite the target once:

```sh
sed -i 's/127\.0\.0\.1:8787/hew-relay:8787/' relay.conf    # compose only
```

Under a **Podman pod** (below) both containers share one network namespace
and `localhost`, so `relay.conf` is used exactly as shipped.

### Docker

```yaml
services:
  hew-web:
    image: nginx:alpine
    restart: unless-stopped
    ports:
      - "8080:80"
    volumes:
      - ./www:/var/www/hew:ro
      - ./nginx.conf:/etc/nginx/conf.d/hew.conf:ro
      - ./relay.conf:/etc/nginx/hew.d/relay.conf:ro
    depends_on:
      - hew-relay
  hew-relay:
    image: ghcr.io/hew3d/hew-relay:X.Y.Z  # release version WITHOUT the leading v (`latest` also works)
    restart: unless-stopped
    env_file: relay.env
```

Not running the relay? Drop the `hew-relay` service, the `relay.conf`
volume line, and `depends_on` — nothing else changes.

**Don't publish the relay's port to the host** (no `ports:` under
`hew-relay`) — only nginx should be able to reach it, over the compose
network; the browser only ever talks to nginx.

```sh
docker compose up -d
curl -s http://127.0.0.1:8080/relay/    # from the host, if the relay is enabled
```

### Podman

Podman ≥ 4.7 ships `podman compose`, which wraps `podman-compose` or
`docker-compose` if either is installed — the exact same compose file above
works unchanged (including the `sed` rewrite of `relay.conf`, since compose
networking is the same):

```sh
podman compose up -d
```

**Native rootless Podman**, without compose — a pod, so both containers
share one network namespace and `localhost`, and `relay.conf` is used
exactly as shipped (no `sed`). Only nginx's port is published on the pod;
the relay's 8787 stays inside it:

```sh
podman pod create --name hew -p 8080:80
podman run -d --pod hew --name hew-relay --env-file relay.env \
  ghcr.io/hew3d/hew-relay:X.Y.Z
podman run -d --pod hew --name hew-web \
  -v ./www:/var/www/hew:ro,Z \
  -v ./nginx.conf:/etc/nginx/conf.d/hew.conf:ro,Z \
  -v ./relay.conf:/etc/nginx/hew.d/relay.conf:ro,Z \
  docker.io/library/nginx:alpine
```

Rootless-Podman specifics that differ from the Docker recipe:

- **Ports below 1024** aren't bindable by a rootless container by default —
  hence `8080` above. To use `80` directly, either front it with a host
  reverse proxy or raise the limit once:
  `sudo sysctl net.ipv4.ip_unprivileged_port_start=80`.
- **`:Z` (or `:z`) on volume mounts** relabels the bind mount for SELinux
  (Fedora, RHEL, and derivatives) so the container can actually read it;
  harmless, and usually necessary, wherever SELinux is enforcing.
- **Fully-qualified image names** (`docker.io/library/nginx:alpine`, not
  bare `nginx:alpine`) avoid Podman prompting you to pick a registry.
- **Surviving a reboot** without a login session: Quadlet (Podman ≥ 5 for
  the `.pod` unit; on Podman 4.x, run the two `.container` units without
  `Pod=` and give each `Network=`/`PublishPort=` instead, with `relay.conf`
  rewritten to the relay container's name as under compose). A minimal
  `~/.config/containers/systemd/hew.pod`:

  ```ini
  [Pod]
  PublishPort=8080:80

  [Install]
  WantedBy=default.target
  ```

  `~/.config/containers/systemd/hew-web.container`:

  ```ini
  [Container]
  Pod=hew.pod
  Image=docker.io/library/nginx:alpine
  Volume=%h/hew-web/www:/var/www/hew:ro,Z
  Volume=%h/hew-web/nginx.conf:/etc/nginx/conf.d/hew.conf:ro,Z
  Volume=%h/hew-web/relay.conf:/etc/nginx/hew.d/relay.conf:ro,Z

  [Install]
  WantedBy=default.target
  ```

  `~/.config/containers/systemd/hew-relay.container`:

  ```ini
  [Container]
  Pod=hew.pod
  Image=ghcr.io/hew3d/hew-relay:X.Y.Z
  EnvironmentFile=%h/hew-web/relay.env

  [Install]
  WantedBy=default.target
  ```

  Then:

  ```sh
  systemctl --user daemon-reload
  systemctl --user start hew-pod.service
  loginctl enable-linger "$USER"   # lets the user service run without a login session
  ```

  (`Pod=`, `Image=`, `Volume=`, `EnvironmentFile=`, and `PublishPort=` are
  standard Quadlet keys; adjust paths above to wherever you staged
  `www/`, `nginx.conf`, `relay.conf`, and `relay.env`.)

**Troubleshooting note:** if `docker pull`/`podman pull
ghcr.io/hew3d/hew-relay` fails with an access/permission error, the GHCR
package may still be private on the publisher's side — nothing fixable
locally; it needs to be flipped to public once by a maintainer.

### Updating (containers)

Re-extract `www/`, re-copy `nginx.conf` and `relay.conf` from the fresh
`deploy/`, bump the image tag in your compose file or Quadlet unit, then:

```sh
docker compose up -d      # or: podman compose up -d — pulls and restarts what changed
# native Podman pod:
podman pull ghcr.io/hew3d/hew-relay:X.Y.Z && podman restart hew-relay hew-web
```

(Re-apply the `sed` to the fresh `relay.conf` under compose.)

## Option C: an existing web server or reverse proxy

If you already run nginx, Apache, Caddy, or Traefik for other sites on the
same host, give Hew its own server block/vhost rather than folding it into
an existing one — it needs its own `root` at the domain root (see [What
you're deploying](#what-youre-deploying)), and mixing its CSP header with
another site's would be wrong for both.

### nginx

Reuse `deploy/nginx.conf` (from the release tarball) as a new server block
— either on its own `listen` port, or fronted by a reverse proxy /
another vhost that forwards a dedicated subdomain to it. The file claims
`default_server` and `server_name _`, which assume it's the only site on
port 80; alongside an existing site on the same port, drop both (`nginx -t`
fails loudly with "duplicate default server" otherwise) and set
`server_name` to this deployment's real hostname. Its `include
/etc/nginx/hew.d/*.conf;` line is what picks up the relay stanza below, so
create `/etc/nginx/hew.d/` and drop `deploy/hew.d/relay.conf` (bound to
`127.0.0.1:8787`, or wherever you're running the relay) into it, same as
Option A.

### Apache

No maintained Apache config ships in this repo — nginx is the canonical
target — but the mapping is direct:

```apacheconf
<VirtualHost *:80>
    DocumentRoot /var/www/hew
    ErrorDocument 404 /404.html

    Header always set Content-Security-Policy "frame-ancestors 'none'"
    Header always set X-Content-Type-Options "nosniff"

    <Files "sw.js">
        Header set Cache-Control "no-cache"
    </Files>
    <Files "index.html">
        Header set Cache-Control "no-cache"
    </Files>
    <Directory "/var/www/hew/assets">
        Header set Cache-Control "public, max-age=31536000, immutable"
    </Directory>

    # No SPA fallback rewrite — every real path is a real file.

    # Relay ("Open on Phone"), if you're running it:
    RedirectMatch 308 ^/relay$ /relay/
    ProxyPass /relay/ http://127.0.0.1:8787/
    ProxyPassReverse /relay/ http://127.0.0.1:8787/
    <Location /relay/>
        LimitRequestBody 41943040
    </Location>
</VirtualHost>
```

Requires `mod_headers` (`a2enmod headers`) and, for the relay stanza,
`mod_proxy` and `mod_proxy_http` (`a2enmod proxy proxy_http`). Unlike
nginx, Apache's default MIME table (`mod_mime`, from the system
`mime-support` package) usually already maps `.webmanifest` to
`application/manifest+json` — verify with `curl -sI` after deploying (see
[Verify](#4-verify)); if it comes back `application/octet-stream`, add
`AddType application/manifest+json .webmanifest` inside the
`<VirtualHost>` block.

`LimitRequestBody 41943040` is a hair over the relay's own 32 MiB per-drop
cap, so an oversized upload gets the relay's typed `413`, not Apache's
bare one. `mod_proxy_http` streams a request body that carries a
`Content-Length` straight through — which is what the desktop sends — so
nothing further is needed for uploads.

### Caddy

```caddyfile
hew.example.org {
    root * /var/www/hew
    file_server

    header {
        Content-Security-Policy "frame-ancestors 'none'"
        X-Content-Type-Options "nosniff"
    }
    header /assets/* Cache-Control "public, max-age=31536000, immutable"
    header /sw.js Cache-Control "no-cache"
    header /index.html Cache-Control "no-cache"

    # Relay ("Open on Phone"), if you're running it:
    redir /relay /relay/ 308
    handle_path /relay/* {
        reverse_proxy 127.0.0.1:8787
    }
}
```

Caddy streams request bodies by default (no separate body-size directive
needed for the relay to work correctly) and handles HTTPS automatically
for a public hostname — see [HTTPS and certificates](#https-and-certificates).

### Traefik / Cloudflare Tunnel

Forward the whole domain to the app's origin — don't strip or rewrite the
path. `/relay/` has to land on the same nginx (or Apache/Caddy) instance
that's already proxying it onward per the sections above; routing `/relay/`
to a *different* upstream works too as long as whatever receives it strips
the `/relay/` prefix itself before talking to `hew-relay`.

## Option D: Proxmox VE (LXC) installer

`scripts/hew-web.sh` provisions a Debian LXC container on a Proxmox VE
host and deploys Hew into it, in the style of a [Proxmox VE
Script](https://community-scripts.github.io/ProxmoxVE/) (ahead of a real
submission to that project). Run it from the Proxmox host shell the way
those scripts are run — straight from GitHub:

```sh
bash -c "$(curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh)"
```

Updating follows the same convention those scripts use: run the **same
script inside the container** and it updates instead of installing (it
recognizes an existing install by `/etc/hew/install.env`). The container
gets a `/usr/bin/update` command that does exactly that, so the whole
procedure is:

```sh
pct enter 105
update
```

`update` fetches the current installer from GitHub when it can (so fixes
to the installer itself arrive too) and falls back to the copy saved at
install time when it can't; either way it re-reads the persisted
settings and never regenerates the upload key or certificate. The same
operation can be driven from the Proxmox host instead:

```sh
bash -c "$(curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh)" hew-web.sh --update 105
```

(the `hew-web.sh` before `--update` is just a name for bash's `$0`, so the
arguments land where the script expects them). Or from a local copy:
`./hew-web.sh` and `./hew-web.sh --update 105`. Use the `bash -c
"$(curl …)"` form, not `curl … | bash` — the wizard's whiptail dialogs need
the terminal on stdin. Unattended `var_*` overrides (below) go in front
of any of these, `update` included:
`var_relay=yes bash -c "$(curl -fsSL …/hew-web.sh)"`.

### Default vs Advanced

Interactively, the first question is **Default settings** vs **Advanced
settings**. **Default** asks exactly one more thing — whether to enable the
relay (yes by default) — and otherwise takes the defaults: today's
container sizing, DHCP, no upload key, plain HTTP, no auto-update — good
for a first try on a trusted LAN. **Advanced** walks through every question
by hand, including container
sizing/network (CTID, hostname, disk, CPU, RAM, bridge, IP/gateway/DNS,
storage, OS template, start-on-boot, unprivileged) and the Hew-specific
options:

- Enable the relay ("Open on Phone")?
- Relay memory cap (MB) — becomes `HEW_RELAY_MAX_TOTAL_BYTES`.
- Require an upload key for the relay? (recommended if it's reachable from
  the internet) — a 32-character key is generated for you and shown once at
  the end (`var_upload_key=<literal>` supplies your own, unattended).
- HTTPS mode: **none** (plain HTTP, LAN use), **self-signed** (the script
  generates a 10-year certificate you trust manually), **Let's Encrypt**
  (needs a public hostname and ports 80+443 reachable), or **upstream**
  (TLS already terminates in front of the container — a reverse proxy or
  tunnel — and the container stays plain HTTP).
- A public hostname, for the self-signed/Let's Encrypt/upstream modes (FQDN
  required for Let's Encrypt; a LAN name or IP is fine for the others).
- An email for Let's Encrypt renewal notices (Let's Encrypt mode only,
  optional).
- Enable weekly automatic updates? — installs a `hew-web-update.timer`
  systemd timer that re-runs the deploy step (from the copy of the installer
  saved at install time) against new releases.

A summary screen lists every choice before it creates anything.

### What it prints at the end

```
Desktop: Settings > Advanced > Server
  https://hew.example.org
Upload key (shown once — store it, it is not printed again):
  <the generated or provided key>
Self-signed certificate to trust on your phone and desktop:
  scp root@192.168.1.50:/etc/hew/tls/hew.crt .
Update after future releases (inside the container: pct enter 105):
  update
or from this host:
  bash -c "$(curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/scripts/hew-web.sh)" hew-web.sh --update 105
```

The app's URL, the exact value to paste into the desktop's **Settings ▸
Advanced ▸ Server** field, and — once — the upload key if one was generated
or entered (it's never stored anywhere the script can print it again, so
save it now); the `scp` hint only appears in self-signed HTTPS mode; the
`--update` line always appears, with this run's CTID filled in, and in
the same form the script was run (the `curl` one-liner, or the local
path).

### Unattended runs

Every question has a scriptable `var_*` override — set them and it never
prompts (or falls back silently to defaults whenever it detects it isn't
attached to a real terminal, or `whiptail` isn't installed):

```sh
var_relay=yes var_upload_key=yes var_https=none ./hew-web.sh
```

The full set: `var_ctid`, `var_hostname`, `var_disk`, `var_cpu`, `var_ram`,
`var_brg`, `var_net`, `var_gateway`, `var_dns`, `var_storage`,
`var_template_storage`, `var_template`, `var_onboot`, `var_unprivileged`,
`var_relay`, `var_relay_mem_mb`, `var_upload_key` (`yes` / `no` / a literal
key), `var_https`, `var_hostname_public`, `var_letsencrypt_email`,
`var_autoupdate`, `var_version`, `var_release_base`. Old-name aliases
(`CTID`, `CT_HOSTNAME`, `CORES`, `RAM_MB`, `DISK_GB`, `BRIDGE`, `NET_IP`,
`STORAGE`, `TEMPLATE_STORAGE`, `OS_TEMPLATE`) still work.

### Updating: `update` inside the container, or `--update <CTID>` from the host

Both re-read `/etc/hew/install.env` (what the last install persisted) and
refresh the app, relay, and TLS config from it — no wizard. Any `var_*`
you set explicitly on the command line overrides what's stored; anything
you leave unset keeps the persisted value. Neither ever regenerates the
upload key or the TLS certificate. The optional weekly timer runs the copy
of the installer saved at install time (never code fetched unattended from
the network); the interactive `update` prefers a fresh copy from GitHub.

### Installing from a mirror

`var_release_base=<url>` points the installer at any HTTP directory laid
out like a release — the `hew-web-vX.Y.Z.tar.gz` and
`hew-relay-vX.Y.Z-linux-<arch>.tar.gz` assets, plus a `latest` text file
naming the tag — for an internal mirror, an air-gapped copy, or a
not-yet-released build. It's persisted inside the container, so `--update`
and the weekly timer keep using it:

```sh
var_release_base=http://192.168.1.50:8000 ./hew-web.sh
```

Maintainers testing this path before a release exists:
`scripts/stage-local-release.sh` builds exactly those assets (both
architectures) from the working tree into `dist/local-release/` and prints
the `python3 -m http.server` one-liner to serve them.

## HTTPS and certificates

Plain HTTP is fine for LAN-only use of the app itself. You lose two
things: install-to-home-screen and offline caching (both need a secure
context), and — if you also run the relay — Shop Mode's in-app QR scanner
can't open the camera over `http://`, so scanning has to fall back to the
phone's regular camera app (which hands off to the browser, and isn't
under that restriction).

HTTPS is recommended once more than one device is involved, and required
if the server is reachable beyond your own LAN. Options:

- **A public CA (Let's Encrypt or similar), with a real hostname.** Nothing
  further to do — every device already trusts it.
- **Your own CA, or a self-signed certificate, on a LAN.** `step-ca`, a
  router's internal CA, `mkcert`, and similar. Needs to be trusted **on
  every device that will use Hew against this server**, and there is
  deliberately no "accept invalid certificate" toggle anywhere in Hew to
  skip that step:
  - **Desktop**: the relay client uses the operating system's own trust
    store, not a bundled list, specifically so a CA you've already trusted
    system-wide just works. Install it in the OS keychain (Keychain Access
    on macOS, the Certificate Manager on Windows, your distro's CA trust
    tooling on Linux) the way you would for any other internal service.
  - **iOS**: install the CA's profile (AirDrop it, email it, or serve it
    from a URL and open it in Safari), then go to **Settings ▸ General ▸
    About ▸ Certificate Trust Settings** and turn on full trust for it —
    installing the profile alone is not enough.
  - **Android**: **Settings ▸ Security ▸ Install a certificate ▸ CA
    certificate** (exact wording varies by OEM/version).
- **TLS terminated upstream** — a reverse proxy or tunnel in front of
  whichever option you chose does the TLS, and Hew's own config stays
  plain HTTP behind it (`X-Forwarded-Proto` is the signal a fronting proxy
  should set; the app itself is static and doesn't read it, so there's
  nothing else to wire up on the Hew side).

Where nginx's own TLS goes: a second `server { listen 443 ssl; ... }` block
alongside the shipped one (Option C), or the LXC installer's self-signed
mode, which edits the deployed `nginx.conf` for you.

If a device hasn't trusted the CA, the desktop's **Test connection** button
reports it plainly — "Reachable, but the server's certificate isn't
trusted by this computer — install its certificate authority in the system
keychain." — rather than silently failing or offering to bypass it.

## Connect the desktop and the phone

**Desktop** (macOS/Linux: **Settings ▸ Advanced**; Windows has the same
section on its Settings page):

1. **Server**: choose **Self-hosted** (the other option is **Hew cloud
   (app.hew3d.com)**).
2. **Address**: a bare origin, e.g. `https://hew.example.org` — no path.
3. **Upload key**: only if the server requires one; stored locally in
   plain text, sent only to this address.
4. **Test connection**. Success reports something like `Reachable:
   hew.example.org — hew-relay v1 · 32 MB max · 10 min TTL · open uploads`
   (or `upload key required`). Failure is specific: "Could not
   reach `<host>` — check the address, your connection, and that the
   server is up," a certificate-trust message, "Reachable, but `<host>`
   isn't serving a Hew relay at /relay/," or "The server rejected the
   upload key."

**Phone**: open the *same origin* in the browser (bookmark it, or **Add to
Home Screen**) — not `app.hew3d.com`. Scan from inside Shop Mode as usual.
A code minted for a different server shows "This code is for `<host>`, not
this server — open it there?" with a button that navigates you to the
right one. Airplane-mode/offline recents are unaffected either way — they
don't touch the relay at all.

## Relay operations

| Variable | Flag | Default | What it does |
| --- | --- | --- | --- |
| `HEW_RELAY_LISTEN` | `--listen` | `127.0.0.1:8787` | Bind address. Keep it on loopback and let nginx (or Apache/Caddy) proxy `/relay/` to it. |
| `HEW_RELAY_MAX_BYTES` | `--max-bytes` | `33554432` (32 MiB) | Per-drop size cap; `413` above it. |
| `HEW_RELAY_MAX_TOTAL_BYTES` | `--max-total-bytes` | `268435456` (256 MiB) | Total memory across all live drops; `503 {"error":"relay full"}` + `Retry-After` above it — fail closed, never swap. |
| `HEW_RELAY_TTL_SECS` | `--ttl-secs` | `600` | Seconds an unread drop lives before it's forgotten. |
| `HEW_RELAY_UPLOAD_KEY` | `--upload-key` | unset | When set, `PUT /drop` requires `Authorization: Bearer <key>`. Reads stay keyless — the token is the capability. Set it if the relay is reachable from the internet. |
| `HEW_RELAY_ALLOW_ORIGINS` | `--allow-origin` (repeatable) | unset | Comma list of browser origins allowed to read cross-origin. Only for a phone served from a *different* origin than the relay — the recommended same-origin layout never needs it. |

**Memory sizing**: `HEW_RELAY_MAX_TOTAL_BYTES` is the relay's own soft cap;
the systemd unit's `MemoryMax` (default `512M`) is the hard OS-level
backstop above it. Raise them together — the LXC installer does this
automatically via a drop-in override sized `cap + 256M`.

**The upload key** is a PUT-only bearer token: reads (`GET`/`HEAD`/`DELETE`)
never require it, because possessing the token is itself the read
capability. The desktop stores the key locally in plain text and sends it
only to the configured self-hosted origin.

**Logs**: `journalctl -u hew-relay` — one line per request (method, route
kind, status, payload size), never a token, a body, or the key.
`RUST_LOG` filters the usual way (e.g. `RUST_LOG=debug`).

**Health check**: `GET /relay/` (the identity route) — `200
{"service":"hew-relay","contract":1,"maxBytes":...,"ttlMs":...,"auth":"none"|"bearer"}`
with `cache-control: no-store`. This is what the desktop's *Test
connection* hits, and a reasonable target for an external uptime check.

**A full relay** answers `PUT /drop` with `503 {"error":"relay full"}` and
a `Retry-After` header once `HEW_RELAY_MAX_TOTAL_BYTES` would be exceeded;
the desktop shows this as "the relay is full" rather than a bare failure.

## Validating a deployment

The same black-box conformance suite both relay implementations (this
Rust binary and the Cloudflare Worker) are tested against in CI can be
pointed at a live deployment:

```sh
cd workers/share-relay
HEW_RELAY_URL=https://hew.example.org HEW_RELAY_PREFIX_ONLY=1 npm run test:contract
# add HEW_RELAY_UPLOAD_KEY=... if the relay requires one
```

Needs Node ≥ 23.6, no other dependencies. `HEW_RELAY_PREFIX_ONLY=1` tells
the suite `HEW_RELAY_URL` is a web origin that proxies only `/relay/`
through to the relay — this guide's nginx/Apache/Caddy layouts — rather
than the relay's own port directly. It exercises uploads, one-shot
downloads, the size and TTL limits, and CORS against the real server, not
just the identity route. Worth running once after standing up a relay,
especially behind a hand-written Apache or Caddy config; the desktop's own
*Test connection* covers the everyday case afterward.

## Troubleshooting

| Symptom | Likely cause |
| --- | --- |
| `/relay/` answers 502 | The relay process is down, or `relay.conf`'s proxy target is wrong (most often: a Docker/Podman-compose deployment where the `sed` rewrite to the service name was skipped). |
| Desktop says "Reachable, but `<host>` isn't serving a Hew relay at /relay/" | `hew.d/relay.conf` was never copied in, `/etc/nginx/hew.d/` doesn't exist (the `include` glob silently matches nothing), or `nginx -t` was never run after adding it. |
| Desktop says "the server's certificate isn't trusted by this computer" | The CA isn't in the OS keychain (desktop) or the device's trust store (phone) — see [HTTPS and certificates](#https-and-certificates). |
| Desktop says "The server rejected the upload key" | The upload key in **Settings ▸ Advanced** doesn't match `HEW_RELAY_UPLOAD_KEY` on the server. |
| Phone's in-app scanner won't open the camera | The origin is plain `http://` — use the camera app instead, or add HTTPS. |
| Phone says "This code is for `<host>`, not this server" | You scanned a code minted by a desktop pointed at a *different* server than the one the phone is open on — tap the button it offers, or open the phone at the matching origin. |
| `/deploy/` is visible, or 404s | Expected either way — nginx's config explicitly 404s it, and it's harmless if you didn't get around to deleting it from the served tree. |
| App is HTTPS but you're worried about the relay proxy being "plain HTTP" internally | Not an issue: nginx (or Apache/Caddy) proxies to the relay over loopback or a private container network, and the browser only ever talks to the public HTTPS origin — same-origin the whole way, no mixed content. |
| Podman rootless can't bind port 80 | Use 8080 (as in this guide), or raise `net.ipv4.ip_unprivileged_port_start`. |
| Podman container can't read a mounted file (SELinux) | Add `:Z` (or `:z`) to the volume mount. |
| `docker pull`/`podman pull ghcr.io/hew3d/hew-relay` denied | The GHCR package may still be private on the publisher's side — ask for it to be made public; nothing fixable locally. |
