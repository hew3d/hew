/* oxlint-disable lingui/no-unlocalized-strings -- command, event and channel names; nothing here is shown to a person. */
/**
 * What the plugin host reads from the install store: the installed plugins
 * (the stored record and the stored manifest, never a folder's live one),
 * a plugin's packaged files, and the notices that the store or a plugin's
 * storage changed. Desktop: the shell's commands and events. Web: the
 * browser store and its channel.
 */

import { isTauri } from '../io/fileHost'
import { cleanText } from './pluginText'
import type { DevChange, DevRequest, Listed } from './types'

export interface PluginStoreAccess {
  list(): Promise<Listed[]>
  /** One packaged file, by its path inside the package. */
  readFile(id: string, rel: string): Promise<ArrayBuffer>
  /** Calls `fn` when the store changed, here or in another window. */
  onChanged(fn: () => void): () => void
  /** Calls `fn` with a plugin's changed storage keys, from any window. */
  onStorageChanged(fn: (id: string, keys: string[]) => void): () => void
  /** Whether the shell's developer mode is on, asked of the shell each
   *  time. Absent where there is none (the web): nothing runs from a
   *  folder there. */
  devMode?(): Promise<boolean>
  /** Calls `fn` with each checked `plugins-dev-changed`. */
  onDevChanged?(fn: (change: DevChange) => void): () => void
}

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

export const invokeShell: Invoke = async (command, args) => {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke(command, args)
}

/** A storage notice's payload, checked: it can come from any window. */
function storageNotice(payload: unknown): { id: string; keys: string[] } | null {
  if (typeof payload !== 'object' || payload === null) return null
  const p = payload as { id?: unknown; keys?: unknown; kind?: unknown }
  if (typeof p.id !== 'string' || p.id.length > 128 || !Array.isArray(p.keys) || p.keys.length > 1024) return null
  const keys = p.keys.filter((k): k is string => typeof k === 'string' && k.length <= 256)
  return keys.length === p.keys.length ? { id: p.id, keys } : null
}

/** A refusal or reason name as the shell writes them, or null. */
const NAME = /^[a-z][a-z0-9_]{0,63}$/

/** `plugins-dev-changed`'s payload, checked: a malformed one is dropped. */
export function devChangeNotice(payload: unknown): DevChange | null {
  if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) return null
  const p = payload as Record<string, unknown>
  if (typeof p.id !== 'string' || p.id.length === 0 || p.id.length > 128 || typeof p.manifest_changed !== 'boolean') return null
  const name = (v: unknown): string | null | undefined => (v === null || v === undefined ? null : typeof v === 'string' && NAME.test(v) ? v : undefined)
  const reason = name(p.reason)
  const problem = name(p.problem)
  if (reason === undefined || problem === undefined) return null
  return { id: p.id, manifest_changed: p.manifest_changed, reason, problem }
}

/** The most characters of a requested path the page shows; the shell cuts
 *  it to this already. */
export const MAX_REQUEST_PATH_CHARS = 512

/** A request id as the shell makes them: 32 lowercase hex digits. */
const REQUEST_ID = /^[0-9a-f]{32}$/

/** `plugins-dev-request`'s payload (or `plugin_dev_request`'s answer),
 *  checked and cleaned again: the path is display text only, never passed
 *  back; only the id is. Null for a malformed payload, and for `null`. */
export function devRequestNotice(payload: unknown): DevRequest | null {
  if (typeof payload !== 'object' || payload === null || Array.isArray(payload)) return null
  const { id, path: raw } = payload as { id?: unknown; path?: unknown }
  if (typeof id !== 'string' || !REQUEST_ID.test(id) || typeof raw !== 'string') return null
  const path = cleanText(raw.slice(0, 4 * MAX_REQUEST_PATH_CHARS), MAX_REQUEST_PATH_CHARS)
  return path === '' ? null : { id, path }
}

function listenShell(event: string, fn: (payload: unknown) => void): () => void {
  let stop: (() => void) | null = null
  let closed = false
  import('@tauri-apps/api/event')
    .then(({ listen }) => listen(event, (e) => fn(e.payload)))
    .then((off) => {
      if (closed) off()
      else stop = off
    })
    .catch(() => {
      /* no event bridge: nothing to hear */
    })
  return () => {
    closed = true
    stop?.()
  }
}

export function desktopStoreAccess(invoke: Invoke = invokeShell): PluginStoreAccess {
  return {
    list: () => invoke<Listed[]>('plugin_list'),
    readFile: (id, rel) => invoke<ArrayBuffer>('plugin_read_file', { id, rel }),
    onChanged: (fn) => listenShell('plugins-changed', () => fn()),
    onStorageChanged: (fn) =>
      listenShell('plugins-storage-changed', (payload) => {
        const notice = storageNotice(payload)
        if (notice !== null) fn(notice.id, notice.keys)
      }),
    devMode: async () => (await invoke<unknown>('plugin_dev_mode')) === true,
    onDevChanged: (fn) =>
      listenShell('plugins-dev-changed', (payload) => {
        const change = devChangeNotice(payload)
        if (change !== null) fn(change)
      }),
  }
}

export function webStoreAccess(): PluginStoreAccess {
  const store = import('./webPluginStore').then(({ makeWebPluginStore }) =>
    makeWebPluginStore({ wasm: async () => (await import('./kernelPlugins')).pluginWasm() }),
  )
  return {
    list: async () => (await store).list(),
    async readFile(id, rel) {
      const bytes = await (await store).readFile(id, rel)
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer
    },
    onChanged(fn) {
      let stop: (() => void) | null = null
      let closed = false
      void store.then((s) => {
        if (!closed) stop = s.onChanged(fn)
      })
      return () => {
        closed = true
        stop?.()
      }
    },
    onStorageChanged(fn) {
      if (typeof BroadcastChannel === 'undefined') return () => {}
      const channel = new BroadcastChannel('hew-plugins')
      channel.onmessage = (ev: MessageEvent) => {
        const data = ev.data as { kind?: unknown } | null
        if (data === null || typeof data !== 'object' || data.kind !== 'storage') return
        const notice = storageNotice(data)
        if (notice !== null) fn(notice.id, notice.keys)
      }
      return () => channel.close()
    },
  }
}

export function storeAccessForWindow(): PluginStoreAccess {
  return isTauri ? desktopStoreAccess() : webStoreAccess()
}
