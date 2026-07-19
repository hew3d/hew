import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'
import { readFileSync } from 'node:fs'

// The app version's single source of truth is package.json; inject it at build
// time so exports (STL/3MF) and bug-report bundles carry the real version. The
// consumers declare `__HEW_VERSION__` and fall back to '0.0.0' when this define
// is absent (a bare `tsc`/vitest run without the config's define step).
const appVersion = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8'),
).version as string

export default defineConfig({
  define: {
    __HEW_VERSION__: JSON.stringify(appVersion),
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
  },
  plugins: [
    react(),
    VitePWA({
      // We register the SW manually in main.tsx (Tauri-guarded).
      registerType: 'autoUpdate',
      injectRegister: null,

      manifest: {
        name: 'Hew',
        short_name: 'Hew',
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
        // The WASM chunk is typically ~700KB today; raise the ceiling to 10 MB so
        // a future kernel growth never silently drops the WASM from the precache.
        globPatterns: ['**/*.{js,css,html,wasm,png,svg,ico,hew}'],
        maximumFileSizeToCacheInBytes: 10 * 1024 * 1024, // 10 MB
      },

      // Leave the service worker disabled in dev so `pnpm dev` is unaffected.
      devOptions: {
        enabled: false,
      },
    }),
  ],
})
