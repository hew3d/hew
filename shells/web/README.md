# Hew Web Shell

This directory packages the Hew app as a static web deployment. Run `pnpm --dir shells/web build` to produce `shells/web/dist/`, which contains the complete web application ready for deployment.

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
