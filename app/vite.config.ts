import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { lingui } from '@lingui/vite-plugin'
import { VitePWA } from 'vite-plugin-pwa'
import { readFileSync } from 'node:fs'
import type { Plugin } from 'vite'
import { resolveBuildStamp } from './src/buildVersion'
import { parsePreviewFlag } from './src/preview/parseFlag'

// The app version's single source of truth is package.json; inject it at build
// time so exports (STL/3MF) and bug-report bundles carry the real version. The
// consumers declare `__HEW_VERSION__` and fall back to '0.0.0' when this define
// is absent (a bare `tsc`/vitest run without the config's define step).
const pkgVersion = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
).version as string

// Only the CI job that deploys the beta web app sets HEW_BETA_SHA. It stamps
// the version as `<version>+beta.<sha>` and flips `__HEW_BETA__`, which the
// UI reads for its beta cue (src/buildVersion.ts). Every other build is plain.
const { version: appVersion, beta: isBeta } = resolveBuildStamp(
  pkgVersion,
  process.env.HEW_BETA_SHA,
)

// Help ▸ Report Bug's web path posts same-origin to `/report/` (docs/design/
// report-bug.md §8), which only a production `app.hew3d.com` deploy actually
// serves. Under `vite dev`, set HEW_REPORT_PROXY to a `wrangler dev` URL
// (e.g. `HEW_REPORT_PROXY=http://127.0.0.1:8788 pnpm dev`) to forward
// `/report/` there and let the dialog send for real; leave it unset and a
// dev build behaves like a self-hosted web build (Send report hidden).
const reportProxyTarget = process.env.HEW_REPORT_PROXY

// The Preview wall's build flag (docs/design/preview-wall-build.md section 1):
// a literal `true` or `false` baked in at build time from `HEW_PREVIEW`
// (`1` on, `0` off; see src/preview/parseFlag.ts). Unset or empty, the dev server is ON and `vite build` is OFF;
// `HEW_PREVIEW=0` forces off. Nothing reads the variable at run time.
function previewFlag(command: 'build' | 'serve'): boolean {
  return parsePreviewFlag(process.env.HEW_PREVIEW, command === 'serve')
}

// With the flag off, the registry is stripped to the ids the wall needs: the
// feature names, state lines and API names stay out of the production bundle.
// Runs before Vite's own JSON handling, so it sees the file's text.
function stripPreviewRegistry(): Plugin {
  return {
      name: 'hew-strip-preview-registry',
      enforce: 'pre',
      transform(code, id) {
        if (!id.split('?')[0].endsWith('/preview-features.json')) return null
        const parsed = JSON.parse(code) as { features: Array<Record<string, unknown>> }
        const features = parsed.features.map(({ id: featureId, tools, actions, dockChips, behaviors }) => ({
          id: featureId, tools, actions, dockChips, behaviors,
        }))
        return { code: JSON.stringify({ features }), map: null }
      },
    }
  }

  // The function form of defineConfig — not the plain object literal — is
  // load-bearing: `command` distinguishes `vite build` from `vite dev`/
  // `vite serve`, and `__HEW_REPORT_DEV_PROXY__` MUST be false in every build
  // (`command !== 'serve'`) regardless of whatever HEW_REPORT_PROXY happens to
  // be set in the build environment's shell. Gating only on `Boolean(
  // reportProxyTarget)` (as an earlier version of this file did) bakes
  // whatever that env var was at BUILD time into the production bundle too —
  // a CI/dev machine with the var set for local testing would have shipped a
  // production build that thinks it can send reports off a bare static host.
  export default defineConfig(({ command }) => {
    const previewOn = previewFlag(command)
    return {
    define: {
      __HEW_PREVIEW__: JSON.stringify(previewOn),
      __HEW_VERSION__: JSON.stringify(appVersion),
      __HEW_BETA__: JSON.stringify(isBeta),
      __HEW_REPORT_DEV_PROXY__: JSON.stringify(command === 'serve' && Boolean(reportProxyTarget)),
    },
    server: {
      // Under `tauri dev` the shell's webview loads the FIXED devUrl from
      // tauri.conf.json (http://localhost:5173). If another dev server —
      // typically a sibling worktree's — already owns the port, vite's default
      // is to drift silently to 5174+, leaving the shell a blank white window
      // pointed at a stranger. Fail loudly instead ("Port 5173 is already in
      // use"). Plain web dev keeps vite's auto-port behavior: the Tauri CLI
      // sets TAURI_ENV_* only when it spawns the beforeDevCommand.
      strictPort: process.env.TAURI_ENV_PLATFORM !== undefined,
      proxy: reportProxyTarget
        ? {
            '/report/': {
              target: reportProxyTarget,
              changeOrigin: true,
            },
          }
        : undefined,
    },
    plugins: [
      ...(previewOn ? [] : [stripPreviewRegistry()]),
      react(),
      // Lingui: the native macro transform (t``/<Trans>) and on-the-fly .po
      // catalog compilation (docs/LOCALIZATION.md). No Babel/SWC plugin needed.
      lingui({ macroTransform: true }),
      VitePWA({
        // We register the SW manually in main.tsx (Tauri-guarded).
        registerType: 'autoUpdate',
        injectRegister: null,

        manifest: {
          name: isBeta ? 'Hew Beta' : 'Hew',
          short_name: isBeta ? 'Hew Beta' : 'Hew',
          description: 'A solids-first 3D modeler',
          // Brand "Charcoal" (Hew Brand Sheet v1) — the PWA splash / OS chrome color.
          theme_color: '#1b1a17',
          background_color: '#1b1a17',
          display: 'standalone',
          start_url: '/',
          icons: [
            {
              src: 'pwa-192x192.png',
              sizes: '192x192',
              type: 'image/png',
            },
            {
              src: 'pwa-512x512.png',
              sizes: '512x512',
              type: 'image/png',
            },
            {
              src: 'pwa-512x512-maskable.png',
              sizes: '512x512',
              type: 'image/png',
              purpose: 'maskable',
            },
          ],
        },

        workbox: {
          // Precache the app shell (js, css, html), icons, the Rust kernel WASM,
          // and the bundled sample models (.hew — the welcome screen fetches
          // them, so they must work offline too).
          // The optimized WASM kernel is a little under 11 MB; the ceiling sits at
          // 16 MB so kernel growth fails the build here rather than silently
          // dropping the WASM from the precache.
          globPatterns: ['**/*.{js,css,html,wasm,png,svg,ico,hew}'],
          // Test lanes build against an unoptimized (--no-opt) kernel of ~18 MB
          // (the harness-enabled desktop build, VITE_HEW_TEST=1, or a lane that
          // sets HEW_UNOPTIMIZED_WASM=1) and are never shipped, so they get a
          // larger ceiling; every shipped build keeps the 16 MB guard.
          maximumFileSizeToCacheInBytes:
            (process.env.VITE_HEW_TEST === '1' || process.env.HEW_UNOPTIMIZED_WASM === '1' ? 64 : 16) * 1024 * 1024,
          // The "Open on Phone" relay lives at <origin>/relay/ on the SAME
          // origin as this app (app/src/io/shareRelay.ts). Nothing there is
          // ever cacheable — a GET consumes the drop, a HEAD is a live poll —
          // so pin it to NetworkOnly explicitly rather than relying on the
          // absence of a matching runtimeCaching rule, and keep the SPA
          // navigation fallback away from it too (a navigation to /relay/
          // must reach the server, not resolve to the precached index.html).
          // The bug-report intake service (workers/bug-intake) is routed at
          // <origin>/report/ the same way, and its admin pages sit behind
          // Cloudflare Access: a navigation there answered from the precache
          // shows this app instead of the Access login or the admin page.
          runtimeCaching: [
            {
              urlPattern: ({ url, sameOrigin }) => sameOrigin && url.pathname.startsWith('/relay/'),
              handler: 'NetworkOnly',
            },
            {
              urlPattern: ({ url, sameOrigin }) => sameOrigin && url.pathname.startsWith('/report/'),
              handler: 'NetworkOnly',
            },
          ],
          navigateFallbackDenylist: [/^\/relay(\/|$)/, /^\/report(\/|$)/],
        },

        // Leave the service worker disabled in dev so `pnpm dev` is unaffected.
        devOptions: {
          enabled: false,
        },
      }),
    ],
  }
})
