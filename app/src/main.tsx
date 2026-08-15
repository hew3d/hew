import React from 'react'
import ReactDOM from 'react-dom/client'
import './theme/tokens.css'
import './index.css'
import App from './App.tsx'
import { ShopApp } from './shop/ShopApp.tsx'
import { ErrorBoundary } from './ErrorBoundary.tsx'
import { SettingsWindow } from './settings/SettingsWindow.tsx'
import { resolveShellMode, readShellModeOverride } from './shop/shellMode'
import { isCoarsePointer } from './platform'

// Lazy: LibraryWindow statically imports @tauri-apps/api modules; loading
// it eagerly would drag them into the WEB bundle's entry chunk for a
// window variant the web build can never render (adversarial review S5).
const LibraryWindow = React.lazy(() =>
  import('./panels/LibraryWindow.tsx').then((m) => ({ default: m.LibraryWindow })),
)
import { isTauri } from './io/fileHost'
import { initThemeSync } from './theme/applyTheme'

// The Settings window (Tauri: a separate webview; web: unused — the modal
// fallback renders inline in App) loads this same entry with a `#settings`
// hash so it gets its own render root without a second HTML file. The
// Library window (Tauri only — the web build keeps its in-app modal) does
// the same with `#library`.
const isSettingsWindow = window.location.hash.startsWith('#settings')
const isLibraryWindow = window.location.hash.startsWith('#library')

// Shop Mode (`ShopApp`, `shop/shellMode.ts`): a THIRD render root selected
// the same hash-branch way as Settings/Library above, but — unlike those
// two — it is a full, independently-navigable main-window experience (a
// fullscreen touch viewer/inspector for a phone at the workbench), not a
// satellite Tauri webview. `resolveShellMode` is pure; this is the one spot
// that gathers its real inputs (`#shop` forces it — also the E2E hook;
// `isTauri` — the desktop shell never enters it; the persisted override;
// live device signals) and asks the one question "which shell renders".
const isShopWindow =
  resolveShellMode({
    hash: window.location.hash,
    isTauri,
    override: readShellModeOverride(),
    coarsePointer: isCoarsePointer(),
    innerWidth: window.innerWidth,
    innerHeight: window.innerHeight,
  }) === 'shop'

// Set the initial `data-theme` attribute before first paint, and keep it in
// sync thereafter. Runs unconditionally so both the main app window and the
// Settings window (each its own top-level document) stay themed identically.
initThemeSync()

// Native-app feel in the desktop shell: right-click must never surface the
// WebView's browser context menu (RMB is camera Pan in the viewport, and no
// native macOS app shows "Reload"/"Inspect Element" chrome). Editable fields
// keep it for the expected Copy/Paste menu. Applies to every window (main +
// Settings) since both load this entry.
if (isTauri) {
  window.addEventListener('contextmenu', (ev) => {
    const target = ev.target as HTMLElement
    const editable =
      target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable
    if (!editable) ev.preventDefault()
  })
}

// Clear any panic recorded by a previous session so the error boundary only
// ever shows a panic from the *current* run (the wasm hook re-records on panic).
try {
  localStorage.removeItem('hew:lastPanic')
} catch {
  /* ignore */
}

// Register the PWA service worker only in production web builds, and only
// for the main app window.
// Guarded off under Tauri (isTauri) so the desktop shell never registers a SW,
// and excluded from dev (import.meta.env.PROD) so hot-reload is unaffected.
// Shop Mode is DELIBERATELY NOT excluded here alongside Settings/Library: it
// IS a main app window in the web build (unlike those two satellite/modal
// windows), and offline availability at the workbench — no wifi, no
// problem — is the entire point of it existing (`shop/ShopApp.tsx`'s doc
// comment). Do not add `isShopWindow` to this condition.
if (!isSettingsWindow && !isLibraryWindow && !isTauri && import.meta.env.PROD && 'serviceWorker' in navigator) {
  import('virtual:pwa-register').then(({ registerSW }) =>
    registerSW({ immediate: true }),
  )
} else if (isTauri && 'serviceWorker' in navigator) {
  // Defensive: the desktop webview must never be controlled by a PWA service
  // worker. A stale SW left in the webview profile (e.g. from an earlier build)
  // would serve its cached app shell over the dev/bundled assets and blank the
  // window. Proactively unregister any SW and drop its caches.
  navigator.serviceWorker.getRegistrations().then((regs) => {
    for (const reg of regs) void reg.unregister()
  }).catch(() => { /* ignore */ })
  if ('caches' in window) {
    caches.keys().then((keys) => keys.forEach((k) => void caches.delete(k))).catch(() => { /* ignore */ })
  }
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <ErrorBoundary>
      {isSettingsWindow ? (
        <SettingsWindow />
      ) : isLibraryWindow ? (
        <React.Suspense fallback={null}>
          <LibraryWindow />
        </React.Suspense>
      ) : isShopWindow ? (
        <ShopApp />
      ) : (
        <App />
      )}
    </ErrorBoundary>
  </React.StrictMode>,
)
