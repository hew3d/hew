import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx'
import { SettingsWindow } from './settings/SettingsWindow.tsx'
import { isTauri } from './io/fileHost'

// The Settings window (Tauri: a separate webview; web: unused — the modal
// fallback renders inline in App) loads this same entry with a `#settings`
// hash so it gets its own render root without a second HTML file.
const isSettingsWindow = window.location.hash.startsWith('#settings')

// Register the PWA service worker only in production web builds, and only
// for the main app window.
// Guarded off under Tauri (isTauri) so the desktop shell never registers a SW,
// and excluded from dev (import.meta.env.PROD) so hot-reload is unaffected.
if (!isSettingsWindow && !isTauri && import.meta.env.PROD && 'serviceWorker' in navigator) {
  import('virtual:pwa-register').then(({ registerSW }) =>
    registerSW({ immediate: true }),
  )
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    {isSettingsWindow ? <SettingsWindow /> : <App />}
  </React.StrictMode>,
)
