/* oxlint-disable lingui/no-unlocalized-strings -- command names, storage names and plugin install copy; English only until the feature graduates. */
/**
 * The Plugins pane's only door to the install store. On the desktop it calls
 * the shell's twelve `plugin_*` commands; in the browser it uses the OPFS
 * store (`webPluginStore.ts`). Both answer with the same shapes (`types.ts`)
 * and the same refusals (`pluginError.ts`), so the pane does not know which
 * it has.
 *
 * Nothing here is on the live command bus: managing plugins is a person's
 * job, done in Settings.
 */

import { isTauri } from '../io/fileHost'
import { toPluginError } from './pluginError'
import type { Installed, Listed, PendingView, PluginInstaller } from './types'
import { makeWebPluginStore, type WebPluginDeps } from './webPluginStore'

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>
type Listen = (event: string, handler: () => void) => Promise<() => void>

export interface DesktopDeps {
  invoke?: Invoke
  listen?: Listen
}

const MIME: Record<string, string> = {
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  svg: 'image/svg+xml',
}

/** A blob URL for icon bytes, or null for a file type that is not an image. */
function iconBlobUrl(rel: string, bytes: Uint8Array): string | null {
  const type = MIME[rel.slice(rel.lastIndexOf('.') + 1).toLowerCase()]
  if (type === undefined) return null
  return URL.createObjectURL(new Blob([new Uint8Array(bytes)], { type }))
}

/** The desktop installer, over the shell's commands. */
export function makeDesktopInstaller(deps: DesktopDeps = {}): PluginInstaller {
  const invoke: Invoke =
    deps.invoke ??
    (async (command, args) => {
      const { invoke: call } = await import('@tauri-apps/api/core')
      return call(command, args)
    })
  const listen: Listen =
    deps.listen ??
    (async (event, handler) => {
      const { listen: on } = await import('@tauri-apps/api/event')
      return on(event, handler)
    })
  const call = async <T>(command: string, args?: Record<string, unknown>): Promise<T> => {
    try {
      return await invoke<T>(command, args)
    } catch (e) {
      throw toPluginError(e)
    }
  }
  return {
    tier: 'desktop',
    list: () => call<Listed[]>('plugin_list'),
    async pickAndStage(kind) {
      const path = await call<string | null>(kind === 'file' ? 'plugin_pick_package' : 'plugin_pick_dir')
      if (path === null) return null
      return call<PendingView>(kind === 'file' ? 'plugin_stage_file' : 'plugin_stage_dir', { path })
    },
    stageFile: () => Promise.reject(new Error('On the desktop, packages are chosen with the file picker.')),
    commit: (token, accept) => call<Installed | null>('plugin_commit', { token, accept }),
    setEnabled: (id, enabled) => call<string[]>('plugin_set_enabled', { id, enabled }),
    uninstall: (id) => call<void>('plugin_uninstall', { id }),
    rollback: (id) => call<Installed>('plugin_rollback', { id }),
    async iconUrl(id, rel) {
      try {
        const buf = await invoke<ArrayBuffer>('plugin_read_file', { id, rel })
        return iconBlobUrl(rel, new Uint8Array(buf))
      } catch {
        return null
      }
    },
    onChanged(fn) {
      let stop: (() => void) | null = null
      let closed = false
      listen('plugins-changed', fn)
        .then((off) => {
          if (closed) off()
          else stop = off
        })
        .catch(() => {
          /* no event bridge: the pane still refreshes after its own changes */
        })
      return () => {
        closed = true
        stop?.()
      }
    },
  }
}

/** The browser installer, over the OPFS store. */
export function makeWebInstaller(deps: WebPluginDeps): PluginInstaller {
  const store = makeWebPluginStore(deps)
  return {
    tier: 'web',
    list: () => store.list(),
    pickAndStage: () => Promise.reject(new Error('In the browser, choose the package with the file input.')),
    stageFile: (file) => store.stageFile(file),
    commit: (token, accept) => store.commit(token, accept),
    setEnabled: (id, enabled) => store.setEnabled(id, enabled),
    uninstall: (id) => store.uninstall(id),
    rollback: (id) => store.rollback(id),
    previous: (id) => store.previous(id),
    async iconUrl(id, rel) {
      try {
        return iconBlobUrl(rel, await store.readFile(id, rel))
      } catch {
        return null
      }
    },
    onChanged: (fn) => store.onChanged(fn),
  }
}

let shared: PluginInstaller | null = null

/** The installer for this window: desktop when the Tauri shell is present
 *  (the same test the rest of the app uses), else the browser store. */
export function pluginInstaller(): PluginInstaller {
  if (shared === null) {
    shared = isTauri
      ? makeDesktopInstaller()
      : makeWebInstaller({
          wasm: async () => (await import('./kernelPlugins')).pluginWasm(),
        })
  }
  return shared
}
