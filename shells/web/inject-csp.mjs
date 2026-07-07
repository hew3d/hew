// Injects a Content-Security-Policy <meta> into the web build's index.html.
//
// The web build (see package.json) just copies app/dist, which shares
// app/index.html with the Tauri desktop build. We must NOT put the CSP in the
// shared source file: the desktop build gets its CSP from tauri.conf.json, and a
// second (source) CSP would intersect with it and break Tauri's IPC. So the web
// CSP lives here, applied only to this shell's own dist copy.
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
// The script fails loudly if the expected anchor is missing, so a future Vite
// change can't silently ship an un-CSP'd build.

import { readFileSync, writeFileSync } from 'node:fs'

const CSP = [
  "default-src 'self'",
  "script-src 'self' 'wasm-unsafe-eval'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob:",
  "font-src 'self' data:",
  "connect-src 'self' blob:",
  "worker-src 'self' blob:",
  "manifest-src 'self'",
  "media-src 'self' blob:",
  "object-src 'none'",
  "base-uri 'self'",
].join('; ')

const META = `<meta http-equiv="Content-Security-Policy" content="${CSP}" />`

const htmlPath = new URL('./dist/index.html', import.meta.url)
let html = readFileSync(htmlPath, 'utf8')

if (html.includes('http-equiv="Content-Security-Policy"')) {
  console.log('inject-csp: CSP meta already present, skipping')
  process.exit(0)
}

// Insert immediately after the charset meta so the policy governs every
// subsequent resource in the document.
const charset = html.match(/<meta\s+charset=["'][^"']*["']\s*\/?>/i)
if (!charset) {
  console.error(
    'inject-csp: could not find <meta charset> in dist/index.html; ' +
      'refusing to ship a build without CSP. Check the Vite HTML output.',
  )
  process.exit(1)
}

html = html.replace(charset[0], `${charset[0]}\n    ${META}`)
writeFileSync(htmlPath, html)
console.log('inject-csp: CSP meta injected into dist/index.html')
