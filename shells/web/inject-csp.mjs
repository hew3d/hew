// Injects a Content-Security-Policy <meta> into the web build's index.html.
//
// The web build (see package.json) just copies app/dist, which shares
// app/index.html with the Tauri desktop build. We must NOT put the CSP in the
// shared source file: the desktop build gets its CSP from tauri.conf.json, and a
// second (source) CSP would intersect with it and break Tauri's IPC. So the web
// CSP lives here, applied only to this shell's own dist copy.
//
// The policy text lives in ../csp/policies.json with the desktop shell's; this
// script injects `web.production`, or `web.preview` when the build is a preview
// build (HEW_PREVIEW=1, the same variable the app build reads), or `web.strict`
// when HEW_WEB_SOURCES=off (an error beside HEW_PREVIEW=1). The production and
// strict policies are pinned by scripts/check-production-artifacts.mjs.
//
// This is the WEB policy: same allowlist as the desktop one MINUS the Tauri
// `ipc:` sources (no invoke bridge in a browser), PLUS `manifest-src` for the
// PWA manifest. `frame-ancestors` is intentionally absent — it is ignored in a
// <meta> CSP and must be delivered as an HTTP header (see README).
//
// Directives are tuned to what the webview actually uses:
//   script-src 'wasm-unsafe-eval'  — wasm-bindgen kernel instantiation
//   style-src  'unsafe-inline'     — runtime-injected <style> (ImportingOverlay)
//   img-src    data: blob:         — SVG cursors (data:) + texture/thumbnail URLs
//   connect-src/worker-src/media-src blob: — object-URL blobs + service worker
//
// connect-src is 'self', blob: and any https: host in the default policy: the
// page reads Library sources from arbitrary https hosts. No NAMED third-party
// host belongs here (every self-hoster's deployment would also have to trust
// it). The "Open on Phone" relay is served from THIS origin under /relay/
// (app/src/io/shareRelay.ts; workers/share-relay/README.md), so 'self' admits
// it. The strict policy (HEW_WEB_SOURCES=off) keeps connect-src to 'self' and
// blob: only, the old own-origin promise.
//
// The script fails loudly if the expected anchor is missing, so a future Vite
// change can't silently ship an un-CSP'd build.

import { readFileSync, writeFileSync } from 'node:fs'
import { cspMetaOf, policyForBuild, withCspMeta } from '../csp/policy.mjs'

const htmlPath = new URL('./dist/index.html', import.meta.url)
const html = readFileSync(htmlPath, 'utf8')
const csp = policyForBuild('web')

const present = cspMetaOf(html)
if (present !== null) {
  if (present !== csp) {
    console.error('inject-csp: dist/index.html already carries a different policy; refusing to ship it.')
    process.exit(1)
  }
  console.log('inject-csp: CSP meta already present, skipping')
  process.exit(0)
}

try {
  writeFileSync(htmlPath, withCspMeta(html, csp))
} catch (e) {
  console.error(`inject-csp: ${e.message}; refusing to ship a build without CSP. Check the Vite HTML output.`)
  process.exit(1)
}
console.log('inject-csp: CSP meta injected into dist/index.html')
