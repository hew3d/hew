# Self-Hosting the Hew Web App

This is a step-by-step guide to running your own copy of the Hew web app —
on a homelab container (LXC/CT, Docker, whatever), a VM, or any Linux box
you control. It does not cover `app.hew3d.com`, which is Cloudflare Pages
and deployed by CI (see `shells/web/README.md` for that side).

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
- **PWA install and offline caching need a secure context.** The service
  worker (install-to-home-screen, offline use) only registers over HTTPS or
  `localhost`. The app works fine over plain LAN HTTP — you just won't get
  those two features.

None of this is exotic, but it's also not what you get by dropping files
into a directory and hoping. This guide gives you a **canonical, maintained
nginx config** (`shells/web/deploy/nginx.conf` in this repo) that encodes
all of it, so you have exactly one thing to keep in sync rather than
re-deriving it per host.

## Option A: native nginx (recommended)

This is the standard, lowest-friction path for a dedicated container or VM
— and it's what a future Proxmox VE Script for Hew will do under the hood
(see [Proxmox VE / LXC](#proxmox-ve--lxc) below), since that ecosystem's
convention is to provision containers natively rather than nest Docker
inside an LXC.

Tested against Debian/Ubuntu; adjust package manager commands for your
distro.

### 1. Install nginx

```sh
sudo apt update
sudo apt install -y nginx curl
```

### 2. Download and extract the latest release

```sh
sudo mkdir -p /var/www/hew
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o /tmp/hew-web.tar.gz
sudo tar -xzf /tmp/hew-web.tar.gz -C /var/www/hew
rm /tmp/hew-web.tar.gz
```

To pin a specific version instead of latest, replace the release lookup
with a direct URL:
`https://github.com/hew3d/hew/releases/download/vX.Y.Z/hew-web-vX.Y.Z.tar.gz`

### 3. Install the canonical nginx config

```sh
sudo curl -fsSL \
  https://raw.githubusercontent.com/hew3d/hew/main/shells/web/deploy/nginx.conf \
  -o /etc/nginx/sites-available/hew
sudo ln -sf /etc/nginx/sites-available/hew /etc/nginx/sites-enabled/hew
sudo rm -f /etc/nginx/sites-enabled/default
sudo nginx -t && sudo systemctl reload nginx
```

(If your nginx layout uses `conf.d` instead of `sites-available`/
`sites-enabled`, drop the file in `/etc/nginx/conf.d/hew.conf` instead.)

### 4. Verify

Open `http://<host-ip>/` in a browser. You should see Hew boot (React
mounts, the WASM kernel initializes, the 3D viewport appears). Open dev
tools → Console and confirm there are no CSP violations.

```sh
curl -sI http://<host-ip>/ | grep -i "content-security-policy\|x-content-type"
```

should show both headers on the response.

### Updating

Repeat steps 2–3 whenever a new release ships — download the new tarball
over the old contents of `/var/www/hew` (it's a full replace, not a diff)
and reload nginx if the config file itself changed:

```sh
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o /tmp/hew-web.tar.gz
sudo rm -rf /var/www/hew/*
sudo tar -xzf /tmp/hew-web.tar.gz -C /var/www/hew
rm /tmp/hew-web.tar.gz
```

## Option B: Docker

If your host is Docker-first (Synology, Unraid, a bare Linux box you'd
rather not touch directly) rather than an LXC, run the same config in a
stock `nginx` image instead of installing nginx natively. There is no
Hew-specific image to build or maintain — it's the same static folder and
the same config file, just mounted in.

```sh
mkdir -p hew-web && cd hew-web
curl -fsSL "$(curl -fsSL https://api.github.com/repos/hew3d/hew/releases/latest \
  | grep -o '"browser_download_url": *"[^"]*hew-web-[^"]*\.tar\.gz"' \
  | cut -d'"' -f4)" -o hew-web.tar.gz
mkdir -p www && tar -xzf hew-web.tar.gz -C www && rm hew-web.tar.gz
curl -fsSL https://raw.githubusercontent.com/hew3d/hew/main/shells/web/deploy/nginx.conf \
  -o nginx.conf

docker run -d --name hew-web \
  -p 8080:80 \
  -v "$PWD/www:/var/www/hew:ro" \
  -v "$PWD/nginx.conf:/etc/nginx/conf.d/hew.conf:ro" \
  nginx:alpine
```

Compose equivalent:

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
```

Updating means re-running the download step into `./www` and restarting
the container — same "full replace, not a diff" approach as Option A.

## Option C: existing web server / reverse proxy

If you already run nginx, Apache, Caddy, or Traefik for other sites on the
same host, give Hew its own server block/vhost rather than folding it into
an existing one — it needs its own `root` at the domain root (see
[What you're deploying](#what-youre-deploying) above), and mixing its CSP
header with another site's would be wrong for both.

- **nginx**: reuse `shells/web/deploy/nginx.conf` as a new server block
  (Option A, step 3) — either on its own `listen` port, or fronted by a
  reverse proxy / another vhost that forwards a dedicated subdomain to it.
  The file claims `default_server` and `server_name _`, which assume it's
  the only site on port 80; if you're adding it alongside an existing site
  on the same port, drop both (`nginx -t` fails loudly with "duplicate
  default server" otherwise) and set `server_name` to this deployment's
  real hostname.
- **Apache**: translate the same requirements by hand — there's no
  maintained Apache config in this repo, since nginx is the canonical
  target, but the mapping is direct:

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
  </VirtualHost>
  ```

  Requires `mod_headers` enabled (`a2enmod headers`). Unlike nginx, Apache's
  default MIME table (`mod_mime`, from the system `mime-support` package)
  usually already maps `.webmanifest` to `application/manifest+json` — but
  verify with `curl -sI` after deploying (see [Verify](#4-verify) above); if
  it comes back `application/octet-stream`, add
  `AddType application/manifest+json .webmanifest` inside the
  `<VirtualHost>` block, same reasoning as the nginx config's explicit
  override.
- **Reverse-proxying from another host** (e.g. Cloudflare Tunnel, Traefik,
  a router-level proxy) works fine as long as the proxy forwards to the
  domain root and doesn't strip or rewrite the path.

## HTTPS

Plain HTTP is fine for LAN-only use — the app itself works fully. You only
lose install-to-home-screen and offline caching, both of which require a
secure context. If you want those on your LAN, put a reverse proxy with a
locally-trusted certificate (e.g. `step-ca`, your router's internal CA, or
a self-signed cert you trust manually) in front of whichever option above
you chose; nothing about Hew's own config needs to change.

## Proxmox VE / LXC

If you're deploying into a Proxmox LXC container specifically, `pct create`
a Debian container, then follow Option A inside it — that's exactly what a
Proxmox VE Script (in the style of
[community-scripts.org](https://community-scripts.github.io/ProxmoxVE/))
would automate. `scripts/hew-web.sh` in this repo is a standalone script
that does that automation today, ahead of a real submission to that
project; run it from the Proxmox host shell. See the header comment in
that script for what it does and doesn't handle.
