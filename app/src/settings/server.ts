/**
 * server — the "Open on Phone" server setting (Settings ▸ Advanced ▸ Server;
 * docs/design/self-hosting-relay.md §3): which origin the desktop uploads
 * to and points the QR at. Two modes — Hew cloud (app.hew3d.com) or a
 * self-hosted origin, optionally with an upload key.
 *
 * Unlike `units.ts` / `theme.ts` / `debugMode.ts`, the source of truth is
 * NOT a localStorage key: it is a Rust-held value persisted in the app
 * config directory (`server.json`), read and written only through the
 * `get_server_setting` / `set_server_setting` commands. The relay requests
 * themselves (`io/relayClient.ts`) never see this value from JS — Rust
 * reads it directly — so what this module holds is a CACHE for the UI (the
 * pane, the dialog's QR text) plus the usual `subscribe` idiom. Cross-window
 * sync rides the same `settings-changed` Tauri event the other settings
 * use, with a `server` payload key carrying the new value.
 *
 * In the browser build there is nothing to configure: whatever origin
 * serves the app is its server (the phone side derives `<origin>/relay/`
 * from `location.origin`, `io/shareRelay.ts`). `available()` is false and
 * the pane shows that read-only.
 */

import { isTauri } from '../io/fileHost'

export type ServerMode = 'cloud' | 'self-hosted'

/** Mirrors `ServerSetting` in relay_client.rs (serde camelCase / kebab-case
 *  mode). `origin` and `uploadKey` are kept in cloud mode too (inert there)
 *  so switching back to self-hosted does not lose them. */
export interface ServerSetting {
  mode: ServerMode
  origin: string
  uploadKey: string
}

/** The public origin used in cloud mode — mirrors `CLOUD_ORIGIN` in
 *  relay_client.rs, which is the value that actually governs requests. */
export const CLOUD_ORIGIN = 'https://app.hew3d.com'

export const DEFAULT_SERVER_SETTING: ServerSetting = {
  mode: 'cloud',
  origin: CLOUD_ORIGIN,
  uploadKey: '',
}

/** The origin requests and QR codes actually target under `setting`. */
export function effectiveOrigin(setting: ServerSetting): string {
  return setting.mode === 'cloud' ? CLOUD_ORIGIN : setting.origin
}

/** Whether the setting exists on this platform (desktop only). */
export function serverSettingAvailable(): boolean {
  return isTauri
}

// ---------------------------------------------------------------------------
// Cache + subscribers
// ---------------------------------------------------------------------------

let cached: ServerSetting | null = null
let pending: Promise<ServerSetting> | null = null
const listeners = new Set<() => void>()

function notify(): void {
  listeners.forEach((l) => l())
}

function applyExternal(next: unknown): void {
  if (!isServerSetting(next)) return
  cached = next
  notify()
}

function isServerSetting(value: unknown): value is ServerSetting {
  if (typeof value !== 'object' || value === null) return false
  const v = value as Record<string, unknown>
  return (
    (v.mode === 'cloud' || v.mode === 'self-hosted') &&
    typeof v.origin === 'string' &&
    typeof v.uploadKey === 'string'
  )
}

/** The last value fetched from Rust (or set), or `null` before the first
 *  `getServerSetting()` resolves. Synchronous, for render-time reads. */
export function getCachedServerSetting(): ServerSetting | null {
  return cached
}

/** The current setting. Web build: the default (cloud) — nothing consults
 *  it there. Desktop: asks Rust once, then serves the cache (kept fresh by
 *  `setServerSetting` and the cross-window event). */
export function getServerSetting(): Promise<ServerSetting> {
  if (!isTauri) return Promise.resolve(DEFAULT_SERVER_SETTING)
  if (cached !== null) return Promise.resolve(cached)
  if (pending === null) {
    pending = import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke<ServerSetting>('get_server_setting'))
      .then((setting) => {
        cached = setting
        return setting
      })
      .finally(() => {
        pending = null
      })
  }
  return pending
}

/** Validate + persist through Rust; resolves with the canonical form
 *  (origin normalized). Rejects with the Rust `RelayError` (`{ kind:
 *  'invalidOrigin' | 'io', message }`) — the pane shows `message` inline. */
export async function setServerSetting(next: ServerSetting): Promise<ServerSetting> {
  if (!isTauri) throw new Error('the server setting is desktop-only')
  const { invoke } = await import('@tauri-apps/api/core')
  const saved = await invoke<ServerSetting>('set_server_setting', { setting: next })
  cached = saved
  notify()
  return saved
}

/** Subscribe to changes (this window's own `setServerSetting`, or another
 *  window's via the Tauri event). Returns an unsubscribe. */
export function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

/** Drops the cache — tests only. */
export function resetServerSettingForTest(): void {
  cached = null
  pending = null
}

// Cross-window: the Settings window is its own webview; Rust broadcasts
// `settings-changed` with `{ key: 'server', server: <setting> }` on every
// successful `set_server_setting`, so the main window's cache follows.
if (typeof window !== 'undefined' && isTauri) {
  import('@tauri-apps/api/event')
    .then(({ listen }) => {
      return listen<{ key?: unknown; server?: unknown }>('settings-changed', (event) => {
        if (event.payload?.key === 'server') applyExternal(event.payload.server)
      })
    })
    .catch(() => {
      /* ignore — event subscription is best-effort */
    })
}
