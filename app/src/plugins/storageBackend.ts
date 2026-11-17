/* oxlint-disable lingui/no-unlocalized-strings -- command names and refusal names; nothing here is shown to a person. */
/**
 * Where a plugin's own storage (`hew.storage`) lives: one JSON object of
 * string keys to string values per plugin, `plugins/<id>/storage.json`, on
 * disk on the desktop and in the browser's private file system on the web.
 * The host talks to it only through this interface.
 *
 * Every refusal is a `PluginError` whose `refusal` names it
 * (`storage_quota`, `storage_key_invalid`, `not_installed`, ...), so the
 * host can hand the plugin the name and nothing else.
 */

import { toPluginError } from './pluginError'

export interface PluginStorageBackend {
  get(id: string, key: string): Promise<string | null>
  set(id: string, key: string, value: string): Promise<void>
  delete(id: string, key: string): Promise<void>
  /** The plugin's keys, sorted. */
  list(id: string): Promise<string[]>
}

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

/** The desktop store, over the shell's four `plugin_storage_*` commands. */
export function desktopStorage(invoke: Invoke): PluginStorageBackend {
  const call = async <T>(command: string, args: Record<string, unknown>): Promise<T> => {
    try {
      return await invoke<T>(command, args)
    } catch (e) {
      throw toPluginError(e)
    }
  }
  return {
    get: (id, key) => call<string | null>('plugin_storage_get', { id, key }),
    set: (id, key, value) => call<null>('plugin_storage_set', { id, key, value }).then(() => undefined),
    delete: (id, key) => call<null>('plugin_storage_delete', { id, key }).then(() => undefined),
    list: (id) => call<string[]>('plugin_storage_list', { id }),
  }
}

/** The web store's storage functions, as `webPluginStore.ts` exports them. */
export interface WebStorageFunctions {
  storageGet(id: string, key: string): Promise<string | null>
  storageSet(id: string, key: string, value: string): Promise<void>
  storageDelete(id: string, key: string): Promise<void>
  storageList(id: string): Promise<string[]>
}

/**
 * The web store, through the four functions `webPluginStore.ts` exports.
 * They are looked up when first used, so a build whose store lacks them
 * refuses `storage_unavailable` instead of failing to load.
 */
export function webStorage(load: () => Promise<Partial<WebStorageFunctions>> = loadWebStore): PluginStorageBackend {
  let fns: Promise<WebStorageFunctions> | null = null
  const store = (): Promise<WebStorageFunctions> => {
    fns ??= load().then((m) => {
      if (
        typeof m.storageGet !== 'function' ||
        typeof m.storageSet !== 'function' ||
        typeof m.storageDelete !== 'function' ||
        typeof m.storageList !== 'function'
      ) {
        throw toPluginError('storage_unavailable: plugin storage is not available in this build')
      }
      return m as WebStorageFunctions
    })
    return fns
  }
  const wrap = async <T>(fn: (s: WebStorageFunctions) => Promise<T>): Promise<T> => {
    try {
      return await fn(await store())
    } catch (e) {
      throw toPluginError(e)
    }
  }
  return {
    get: (id, key) => wrap((s) => s.storageGet(id, key)),
    set: (id, key, value) => wrap((s) => s.storageSet(id, key, value)),
    delete: (id, key) => wrap((s) => s.storageDelete(id, key)),
    list: (id) => wrap((s) => s.storageList(id)),
  }
}

async function loadWebStore(): Promise<Partial<WebStorageFunctions>> {
  return (await import('./webPluginStore')) as unknown as Partial<WebStorageFunctions>
}
