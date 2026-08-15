# Hew Web Shell

This directory packages the Hew app as a static web deployment. Run `pnpm --dir shells/web build` to produce `shells/web/dist/`, which contains the complete web application ready for deployment.

Deploying your own copy (homelab, private network, anywhere other than
`app.hew3d.com`)? See `docs/SELF_HOSTING.md` — it walks through the same
headers and 404 handling documented below, packaged as a canonical nginx
config at `shells/web/deploy/nginx.conf`.

## Content Security Policy

The build injects a CSP `<meta>` into `dist/index.html` via `inject-csp.mjs`
(the last step of `build`). It is a web-only copy of the desktop policy in
`shells/tauri/src-tauri/tauri.conf.json`, minus Tauri's `ipc:` sources and plus
`manifest-src` for the PWA manifest.

The CSP lives here, not in the shared `app/index.html`, on purpose: the web
build copies `app/dist`, so anything in the source `index.html` would also land
in the desktop build and intersect with the Tauri CSP, breaking IPC.

**Deploy note — headers:** a `<meta>` CSP cannot express `frame-ancestors`
(clickjacking protection) or `report-uri`. Where you control the host, also send
an HTTP header, e.g. `Content-Security-Policy: frame-ancestors 'none'` (or
`X-Frame-Options: DENY`). Serving the CSP as a full HTTP header instead of a
`<meta>` is strictly better if the host allows it — the meta is the portable
floor for dumb static hosts.

Validated: the built bundle loads in Chromium with the app booting (React mount,
WASM kernel init, WebGL canvas) and **zero CSP violations**.

**Deploy note — no edge-injected scripts.** `script-src` admits `'self'` only,
and `connect-src` admits `'self'` plus the single, narrow exception of
`https://share.hew3d.com` (the "Open on Phone" E2E-encrypted relay —
`inject-csp.mjs`'s own comment on that line has the full reasoning), so any
tag a CDN or host injects into the served HTML — an analytics beacon, a RUM
agent, a tag manager — still violates the policy. Such a tag
is either blocked (collecting nothing, so the feature is a lie) or executing in
defiance of the policy, and its exceptions reach the page as muted
cross-origin errors with no attribution. Leave host-side script injection
switched off for this origin. Note also that a service worker precaches
`index.html` keyed on the *built* file's revision, so HTML altered at the edge
is cached by installed clients and never invalidated by a redeploy.

## 404 handling

`404.html` ships in `dist/` so the host returns a real 404 for unknown paths.
Without it, SPA-style hosts (Cloudflare Pages among them) answer any missing
path with `index.html` and a `200`, which turns a stale or mistyped
`/assets/*.js` reference into an HTML document that the browser then tries to
parse as JavaScript. The app routes on `window.location.hash`, never on the
path, so nothing depends on a catch-all rewrite.
