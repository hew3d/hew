/* oxlint-disable lingui/no-unlocalized-strings -- command names, storage names and plugin install copy; English only until the feature graduates. */
/**
 * The Plugins pane's only door to the install store. On the desktop it calls
 * the shell's `plugin_*` commands; in the browser it uses the OPFS store
 * (`webPluginStore.ts`). Both answer with the same shapes (`types.ts`) and
 * the same refusals (`pluginError.ts`), so the pane does not know which it
 * has. Developer mode, the review of a folder plugin's changed manifest and
 * the live API's request to load a folder are the desktop's alone; copying a
 * folder in as a package is the web's.
 *
 * Nothing here is on the live command bus: managing plugins is a person's
 * job, done in Settings.
 */

import { isTauri } from '../io/fileHost'
import { toPluginError } from './pluginError'
import { devChangeNotice, devRequestNotice } from './storeAccess'
import type { DevChange, DevRequest, Installed, Listed, PendingView, PermissionLine, PluginInstaller } from './types'
import { makeWebPluginStore, type WebPluginDeps } from './webPluginStore'

/** An installer that can change a plugin's grant (`PluginInstaller`'s
 *  `setGrant`, present). */
export type GrantCapable = Required<Pick<PluginInstaller, 'setGrant'>> & Pick<PluginInstaller, 'grantLines'>

/** `inst` as a `GrantCapable`, or null for an installer without the calls. */
export function grantCapable(inst: PluginInstaller): (PluginInstaller & GrantCapable) | null {
  return typeof inst.setGrant === 'function' ? (inst as PluginInstaller & GrantCapable) : null
}

type Invoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>
type Listen = (event: string, handler: (event: { payload: unknown }) => void) => Promise<() => void>

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
export function makeDesktopInstaller(deps: DesktopDeps = {}): PluginInstaller & GrantCapable {
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
    async pickAndStage(kind, opts) {
      // The page passes the folder picker only a request's id, never a
      // path: the shell opens it near that request's folder, and what is
      // staged is what the person picked there.
      const path =
        kind === 'file'
          ? await call<string | null>('plugin_pick_package')
          : typeof opts?.useRequest === 'string'
            ? await call<string | null>('plugin_pick_dir', { useRequest: opts.useRequest })
            : await call<string | null>('plugin_pick_dir')
      if (path === null) return null
      return call<PendingView>(kind === 'file' ? 'plugin_stage_file' : 'plugin_stage_dir', { path })
    },
    stageFile: () => Promise.reject(new Error('On the desktop, packages are chosen with the file picker.')),
    commit: (token, accept) => call<Installed | null>('plugin_commit', { token, accept }),
    setEnabled: (id, enabled) => call<string[]>('plugin_set_enabled', { id, enabled }),
    uninstall: (id) => call<void>('plugin_uninstall', { id }),
    rollback: (id) => call<Installed>('plugin_rollback', { id }),
    setGrant: (id, grant, optional) => call<Installed>('plugin_set_grant', { id, grant, optional }),
    devMode: async () => (await call<unknown>('plugin_dev_mode')) === true,
    setDevMode: async (on) => (await call<unknown>('plugin_set_dev_mode', { on })) === true,
    review: (id) => call<PendingView>('plugin_dev_review', { id }),
    devRequest: async () => devRequestNotice(await call<unknown>('plugin_dev_request')),
    dismissDevRequest: (id) => call<void>('plugin_dev_request_dismiss', { id }),
    onDevRequest: (fn: (request: DevRequest | null) => void) =>
      subscribe(listen, 'plugins-dev-request', (e) => {
        // `null` says the request is gone; anything malformed is dropped.
        if (e.payload === null) fn(null)
        else {
          const request = devRequestNotice(e.payload)
          if (request !== null) fn(request)
        }
      }),
    async iconUrl(id, rel) {
      try {
        const buf = await invoke<ArrayBuffer>('plugin_read_file', { id, rel })
        return iconBlobUrl(rel, new Uint8Array(buf))
      } catch {
        return null
      }
    },
    onChanged: (fn) => subscribe(listen, 'plugins-changed', () => fn()),
    onDevChanged: (fn: (change: DevChange) => void) =>
      subscribe(listen, 'plugins-dev-changed', (e) => {
        const change = devChangeNotice(e.payload)
        if (change !== null) fn(change)
      }),
  }
}

/** Listens for `event` until the returned function is called. */
function subscribe(listen: Listen, event: string, fn: (event: { payload: unknown }) => void): () => void {
  let stop: (() => void) | null = null
  let closed = false
  listen(event, fn)
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
}

/** The browser installer, over the OPFS store. */
export function makeWebInstaller(deps: WebPluginDeps): PluginInstaller & GrantCapable {
  const store = makeWebPluginStore(deps)
  return {
    tier: 'web',
    list: () => store.list(),
    pickAndStage: () => Promise.reject(new Error('In the browser, choose the package with the file input.')),
    stageFile: (file) => store.stageFile(file),
    ...(typeof window !== 'undefined' && typeof (window as { showDirectoryPicker?: unknown }).showDirectoryPicker === 'function'
      ? { stageFolder: (dir: FileSystemDirectoryHandle) => store.stageFolder(dir) }
      : {}),
    commit: (token, accept) => store.commit(token, accept),
    setEnabled: (id, enabled, shown) => store.setEnabled(id, enabled, shown),
    uninstall: (id) => store.uninstall(id),
    rollback: (id, shown) => store.rollback(id, shown),
    previous: (id) => store.previous(id),
    setGrant: (id, grant, optional) => store.setGrant(id, grant, optional),
    grantLines: (id, grant, optional) => store.grantLines(id, grant, optional),
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
