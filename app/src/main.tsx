import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx'
import { isTauri } from './io/fileHost'

// Register the PWA service worker only in production web builds.
// Guarded off under Tauri (isTauri) so the desktop shell never registers a SW,
// and excluded from dev (import.meta.env.PROD) so hot-reload is unaffected.
if (!isTauri && import.meta.env.PROD && 'serviceWorker' in navigator) {
  import('virtual:pwa-register').then(({ registerSW }) =>
    registerSW({ immediate: true }),
  )
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
