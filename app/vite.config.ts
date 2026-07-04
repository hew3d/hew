import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import { VitePWA } from 'vite-plugin-pwa'

export default defineConfig({
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
        // Precache the app shell (js, css, html), icons, and the Rust kernel WASM.
        // The WASM chunk is typically ~700KB today; raise the ceiling to 10 MB so
        // a future kernel growth never silently drops the WASM from the precache.
        globPatterns: ['**/*.{js,css,html,wasm,png,svg,ico}'],
        maximumFileSizeToCacheInBytes: 10 * 1024 * 1024, // 10 MB
      },

      // Leave the service worker disabled in dev so `pnpm dev` is unaffected.
      devOptions: {
        enabled: false,
      },
    }),
  ],
})
