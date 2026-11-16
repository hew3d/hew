/* oxlint-disable lingui/no-unlocalized-strings -- storage names, file names and plugin install copy; English only until the feature graduates. */
/**
 * The web build's plugin install store, in the origin-private file system.
 * It keeps the layout the desktop store keeps on disk, under `plugins/`:
 *
 *   <id>/hew-plugin.json    the manifest the user consented to, as written
 *   <id>/installed.json     the record
 *   <id>/files/...          the package's files
 *   <id>/previous/          the version before, with the same layout
 *   .staging/<id>-<token>/  an install being written; ignored by `list`
 *
 * The checks (the package reader, the manifest parser, the consent payload,
 * the Hew version range, dependencies) run in the kernel through five
 * exports, so the desktop and the web agree; this file keeps the files and
 * the record rules of `store.rs`: a fresh install is enabled with the
 * manifest's permissions as its grant and may not start with Hew until the
 * user first turns it on; an update keeps the enabled state, the data
 * version, unknown fields and the optional grants still declared; turning a
 * plugin off turns off the plugins that need it; uninstall is refused while
 * an enabled plugin needs it; a record from a newer Hew is listed as
 * unreadable and never rewritten.
 *
 * OPFS has no rename, so an install is written whole into `.staging/`, the
 * old copy is copied into `previous/`, removed, and the new one copied into
 * place with its record last. A failure part way leaves a staging entry
 * (ignored by `list`, cleared by the next install of the same id) or, between
 * the removal and the last copy, a folder with no record, which lists as
 * unreadable and can be removed.
 *
 * Dependencies are injected so the tests run on an in-memory file system.
 */

import { appVersionString } from '../buildInfo'
import { readFileIn, subdir, writeFileIn } from '../io/opfs'
import { PluginError, toPluginError } from './pluginError'
import type { ConsentPayload, InstallSource, Installed, Listed, PendingView, PluginManifest } from './types'

/** The kernel's plugin exports, as the page calls them. */
export interface PluginWasm {
  plugin_package_from_zip(bytes: Uint8Array): string
  plugin_consent(
    manifestJson: string,
    sourceJson: string,
    previousInstalledJson?: string,
    previousManifestJson?: string,
    signer?: string,
  ): string
  plugin_manifest_parse(text: string): string
  plugin_hew_version_ok(manifestJson: string, running: string): boolean
  plugin_install_check(manifestJson: string, sourceJson: string, running: string, installedJson: string): void
}

export interface WebPluginDeps {
  /** The kernel's exports (production: the loaded module). */
  wasm: () => Promise<PluginWasm>
  /** OPFS root provider (production: `navigator.storage.getDirectory()`). */
  opfsRoot?: () => Promise<FileSystemDirectoryHandle>
  /** Change notices to other tabs, or null where unsupported. */
  channel?: BroadcastChannel | null
  /** Where a plugin's small settings live, for `purge`; null for none. */
  storage?: Pick<Storage, 'length' | 'key' | 'removeItem'> | null
  now?: () => number
  /** A fresh 32-digit hex token. */
  token?: () => string
  /** The running Hew's version. */
  hewVersion?: () => string
}

export interface WebPluginStore {
  list(): Promise<Listed[]>
  stageFile(file: File): Promise<PendingView>
  commit(token: string, accept: boolean): Promise<Installed | null>
  setEnabled(id: string, enabled: boolean): Promise<string[]>
  uninstall(id: string): Promise<void>
  rollback(id: string): Promise<Installed>
  /** The record and manifest `rollback` would restore. */
  previous(id: string): Promise<{ installed: Installed; manifest: PluginManifest }>
  /** One package file by served path. */
  readFile(id: string, rel: string): Promise<Uint8Array>
  onChanged(fn: () => void): () => void
}

const PLUGINS_DIR = 'plugins'
const STAGING = '.staging'
const MANIFEST_FILE = 'hew-plugin.json'
const INSTALLED_FILE = 'installed.json'
const FILES = 'files'
const PREVIOUS = 'previous'
const CHANNEL = 'hew-plugins'
const KEY_PREFIX = 'hew.plugin.'
/** The record format this build writes and reads. */
const SCHEMA = 1
const PENDING_TTL_MS = 10 * 60 * 1000
const MAX_PACKAGE_BYTES = 64 << 20
const RESERVED_FIRST_LABEL = 'hew'

const KNOWN_RECORD_KEYS = new Set([
  'schema',
  'id',
  'version',
  'source',
  'installed_at_ms',
  'grant',
  'optional_granted',
  'enabled',
  'startup_allowed',
  'data_version',
  'signer',
])

/** True when this browser can host the store at all. */
export function webPluginsSupported(): boolean {
  return (
    typeof navigator !== 'undefined' &&
    typeof navigator.storage !== 'undefined' &&
    typeof navigator.storage.getDirectory === 'function'
  )
}

/** The plugin id grammar of `plugin-manifest`: dotted labels of lowercase
 *  letters, digits and hyphens, and not Hew's own first label. Checked
 *  before an id is used as a folder name. */
export function validPluginId(id: string): boolean {
  if (id.length === 0 || id.length > 128) return false
  const labels = id.split('.')
  return (
    labels.length >= 2 &&
    labels[0] !== RESERVED_FIRST_LABEL &&
    labels.every((l) => /^[a-z0-9][a-z0-9-]{0,62}$/.test(l))
  )
}

function checkedId(id: string): string {
  if (!validPluginId(id)) {
    throw new PluginError('invalid_id', `"${id}" is not a plugin id Hew accepts.`)
  }
  return id
}

/** A package path as the kernel accepts it, for reading a file back out. */
function checkedRel(rel: string): string[] {
  const segments = rel.split('/')
  const ok =
    rel !== '' &&
    rel.length <= 1024 &&
    /^[\x20-\x7e]+$/.test(rel) &&
    !/[\\:]/.test(rel) &&
    segments.every((s) => s !== '' && s !== '.' && s !== '..')
  if (!ok) throw new PluginError('package_path_invalid', `The package holds "${rel}", which is not a file name Hew accepts.`)
  return segments
}

class Unreadable extends Error {
  /** A folder with no record at all: an install that never finished. It is
   *  not someone else's data, so it may be replaced or removed. */
  constructor(
    message: string,
    readonly incomplete = false,
  ) {
    super(message)
  }
}

const INCOMPLETE_REASON = 'it was not finished installing; remove it, or install the plugin again'

/** Same JSON value, whatever the key order. */
function sameJson(a: unknown, b: unknown): boolean {
  const canon = (v: unknown): unknown =>
    Array.isArray(v)
      ? v.map(canon)
      : v !== null && typeof v === 'object'
        ? Object.fromEntries(Object.entries(v as object).sort(([x], [y]) => (x < y ? -1 : 1)).map(([k, x]) => [k, canon(x)]))
        : v
  return JSON.stringify(canon(a)) === JSON.stringify(canon(b))
}

function notInstalled(id: string): PluginError {
  return new PluginError('not_installed', `"${id}" is not installed.`)
}

function unreadableError(id: string, reason: string): PluginError {
  return new PluginError(
    'installed_unreadable',
    `Hew cannot read the record of "${id}" (${reason}). A newer Hew may have written it; it is left as it is.`,
  )
}

function isNotFound(err: unknown): boolean {
  return (
    (typeof DOMException !== 'undefined' && err instanceof DOMException && err.name === 'NotFoundError') ||
    (err instanceof Error && err.message.includes('NotFound'))
  )
}

async function removeIn(dir: FileSystemDirectoryHandle, name: string): Promise<void> {
  try {
    await dir.removeEntry(name, { recursive: true })
  } catch (e) {
    if (!isNotFound(e)) throw e
  }
}

async function mkdir(dir: FileSystemDirectoryHandle, name: string): Promise<FileSystemDirectoryHandle> {
  return dir.getDirectoryHandle(name, { create: true })
}

async function writeNested(dir: FileSystemDirectoryHandle, rel: string, bytes: Uint8Array): Promise<void> {
  const segments = rel.split('/')
  let at = dir
  for (const s of segments.slice(0, -1)) at = await mkdir(at, s)
  await writeFileIn(at, segments[segments.length - 1], bytes)
}

/** Copies a folder tree. `skip` names top-level entries to leave out;
 *  `last` names a top-level file written after everything else. */
async function copyTree(
  from: FileSystemDirectoryHandle,
  to: FileSystemDirectoryHandle,
  opts: { skip?: string; last?: string } = {},
): Promise<void> {
  const entries: [string, FileSystemHandle][] = []
  for await (const entry of from.entries()) entries.push(entry as [string, FileSystemHandle])
  const ordered = entries.filter(([n]) => n !== opts.skip)
  ordered.sort(([a], [b]) => Number(a === opts.last) - Number(b === opts.last))
  for (const [name, handle] of ordered) {
    if (handle.kind === 'directory') {
      await copyTree(handle as FileSystemDirectoryHandle, await mkdir(to, name))
    } else {
      await writeFileIn(to, name, await readFileIn(from, name))
    }
  }
}

/** Puts `source` in place of `plugins/<id>`. The old folder goes only when
 *  the copy is about to start; if the copy fails, `backup` (the old version)
 *  is copied back, and with none the half-written folder, which has no record
 *  yet, lists as an unfinished install. */
async function replaceFolder(
  plugins: FileSystemDirectoryHandle,
  id: string,
  source: FileSystemDirectoryHandle,
  backup: FileSystemDirectoryHandle | null,
): Promise<void> {
  await removeIn(plugins, id)
  try {
    await copyTree(source, await mkdir(plugins, id), { last: INSTALLED_FILE })
  } catch (e) {
    if (backup !== null) {
      try {
        await removeIn(plugins, id)
        await copyTree(backup, await mkdir(plugins, id), { last: INSTALLED_FILE })
      } catch {
        /* the original failure is the one to report */
      }
    }
    throw e
  }
}

const encoder = new TextEncoder()
const decoder = new TextDecoder('utf-8', { fatal: true })

function recordBytes(record: Installed): Uint8Array {
  return encoder.encode(JSON.stringify(record, null, 2) + '\n')
}

/** Reads and checks a record. Throws `Unreadable` with a reason. */
function parseRecord(id: string, bytes: Uint8Array): Installed {
  let value: unknown
  try {
    value = JSON.parse(decoder.decode(bytes))
  } catch (e) {
    throw new Unreadable(e instanceof Error ? e.message : 'it is not JSON')
  }
  if (typeof value !== 'object' || value === null || Array.isArray(value)) throw new Unreadable('it is not a record')
  const o = value as Record<string, unknown>
  if (typeof o.schema !== 'number') throw new Unreadable('no record format')
  if (o.schema > SCHEMA) throw new Unreadable(`record format ${o.schema}`)
  const strings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === 'string')
  const source = o.source as { kind?: unknown } | null
  if (
    typeof o.id !== 'string' ||
    typeof o.version !== 'string' ||
    typeof source !== 'object' ||
    source === null ||
    typeof source.kind !== 'string' ||
    typeof o.installed_at_ms !== 'number' ||
    !strings(o.grant) ||
    typeof o.enabled !== 'boolean' ||
    typeof o.data_version !== 'number'
  ) {
    throw new Unreadable('a field is missing or the wrong kind')
  }
  if (o.id !== id) throw new Unreadable(`it names "${o.id}"`)
  const optional = o.optional_granted === undefined ? [] : o.optional_granted
  if (!strings(optional)) throw new Unreadable('optional_granted is not a list')
  return {
    ...(o as Installed),
    optional_granted: optional,
    startup_allowed: o.startup_allowed === true,
    signer: typeof o.signer === 'string' ? o.signer : null,
  }
}

interface Pending {
  id: string
  files: Map<string, Uint8Array>
  source: InstallSource
  consent: ConsentPayload
  manifest: PluginManifest
  previous: Installed | null
  /** The installed record as it was at stage time, to notice a change. */
  previousJson: string | null
  createdAt: number
}

export function makeWebPluginStore(deps: WebPluginDeps): WebPluginStore {
  const opfsRoot = deps.opfsRoot ?? (() => navigator.storage.getDirectory())
  const channel =
    deps.channel !== undefined ? deps.channel : typeof BroadcastChannel !== 'undefined' ? new BroadcastChannel(CHANNEL) : null
  const storage = deps.storage !== undefined ? deps.storage : typeof localStorage !== 'undefined' ? localStorage : null
  const now = deps.now ?? (() => Date.now())
  const newToken =
    deps.token ??
    (() => Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join(''))
  const hewVersion = deps.hewVersion ?? appVersionString
  const listeners = new Set<() => void>()
  const pendings = new Map<string, Pending>()

  channel?.addEventListener?.('message', () => listeners.forEach((l) => l()))

  const notify = (): void => {
    listeners.forEach((l) => l())
    try {
      channel?.postMessage({ kind: 'changed' })
    } catch {
      /* a closed channel must never break a write */
    }
  }

  // One change at a time: every step below reads what an earlier one wrote.
  let tail: Promise<unknown> = Promise.resolve()
  function serial<T>(fn: () => Promise<T>): Promise<T> {
    const run = tail.then(fn, fn)
    tail = run.catch(() => undefined)
    return run.catch((e) => {
      throw toPluginError(e)
    })
  }

  const pluginsDir = async (create: boolean): Promise<FileSystemDirectoryHandle | null> =>
    subdir(await opfsRoot(), PLUGINS_DIR, create)

  /** One plugin's record and manifest from its folder. Throws `Unreadable`. */
  async function readIn(folder: FileSystemDirectoryHandle, id: string): Promise<{ installed: Installed; manifest: PluginManifest }> {
    let recordBytes: Uint8Array
    try {
      recordBytes = await readFileIn(folder, INSTALLED_FILE)
    } catch (e) {
      if (isNotFound(e)) throw new Unreadable(INCOMPLETE_REASON, true)
      throw new Unreadable('its record cannot be read')
    }
    const installed = parseRecord(id, recordBytes)
    let text: string
    try {
      text = decoder.decode(await readFileIn(folder, MANIFEST_FILE))
    } catch {
      throw new Unreadable('its manifest cannot be read')
    }
    let manifest: PluginManifest
    try {
      manifest = JSON.parse((await deps.wasm()).plugin_manifest_parse(text)) as PluginManifest
    } catch (e) {
      throw new Unreadable(toPluginError(e).message)
    }
    if (manifest.id !== id) throw new Unreadable(`its manifest names "${manifest.id}"`)
    return { installed, manifest }
  }

  async function ids(plugins: FileSystemDirectoryHandle): Promise<string[]> {
    const out: string[] = []
    for await (const [name, handle] of plugins.entries()) {
      if (handle.kind === 'directory' && validPluginId(name)) out.push(name)
    }
    return out.sort()
  }

  /** Ids whose folder holds no record at all. */
  const incomplete = new Set<string>()

  async function listNow(): Promise<Listed[]> {
    const plugins = await pluginsDir(false)
    incomplete.clear()
    if (plugins === null) return []
    const out: Listed[] = []
    for (const id of await ids(plugins)) {
      const folder = await subdir(plugins, id, false)
      if (folder === null) continue
      try {
        const { installed, manifest } = await readIn(folder, id)
        let previousVersion: string | null = null
        const prev = await subdir(folder, PREVIOUS, false)
        if (prev !== null) {
          try {
            previousVersion = (await readIn(prev, id)).installed.version
          } catch {
            previousVersion = null
          }
        }
        out.push({ kind: 'installed', installed, manifest, previous_version: previousVersion })
      } catch (e) {
        if (!(e instanceof Unreadable)) throw e
        if (e.incomplete) incomplete.add(id)
        out.push({ kind: 'unreadable', id, reason: e.message })
      }
    }
    return out
  }

  const readable = async (): Promise<{ installed: Installed; manifest: PluginManifest }[]> =>
    (await listNow()).flatMap((l) => (l.kind === 'installed' ? [{ installed: l.installed, manifest: l.manifest }] : []))

  /** One installed plugin, with the refusals `store::installed` makes. */
  async function installedNow(id: string): Promise<{ installed: Installed; manifest: PluginManifest }> {
    checkedId(id)
    const plugins = await pluginsDir(false)
    const folder = plugins === null ? null : await subdir(plugins, id, false)
    if (folder === null) throw notInstalled(id)
    try {
      return await readIn(folder, id)
    } catch (e) {
      if (e instanceof Unreadable) throw unreadableError(id, e.message)
      throw e
    }
  }

  async function dependentsOf(id: string): Promise<string[]> {
    return (await readable())
      .filter((p) => p.installed.enabled && (p.manifest.requires ?? []).some((r) => r.id === id))
      .map((p) => p.installed.id)
      .sort()
  }

  async function clearStaging(plugins: FileSystemDirectoryHandle, id: string): Promise<void> {
    const staging = await subdir(plugins, STAGING, false)
    if (staging === null) return
    const mine: string[] = []
    for await (const [name] of staging.entries()) {
      // `<id>-<32 hex digits>`, with an optional `.<suffix>`; another
      // plugin whose id starts the same way is not matched.
      if (name.startsWith(id + '-') && /^[0-9a-f]{32}(\..*)?$/.test(name.slice(id.length + 1))) mine.push(name)
    }
    for (const name of mine) await removeIn(staging, name)
  }

  /** Removes everything the store keeps for `id`: its folder, its staging
   *  entries and its small settings. The one removal path uninstall takes;
   *  stores added later join it here. */
  async function purge(id: string): Promise<void> {
    const plugins = await pluginsDir(false)
    if (plugins !== null) {
      await removeIn(plugins, id)
      await clearStaging(plugins, id)
    }
    if (storage !== null) {
      const prefix = `${KEY_PREFIX}${id}.`
      const keys: string[] = []
      for (let i = 0; i < storage.length; i++) {
        const k = storage.key(i)
        if (k !== null && k.startsWith(prefix)) keys.push(k)
      }
      for (const k of keys) storage.removeItem(k)
    }
  }

  function recordFor(p: Pending, at: number): Installed {
    const prev = p.previous
    const startupNewly = p.consent.update?.startup_newly ?? false
    const declared = p.manifest.optionalPermissions ?? []
    const extra: Record<string, unknown> = {}
    if (prev !== null) for (const [k, v] of Object.entries(prev)) if (!KNOWN_RECORD_KEYS.has(k)) extra[k] = v
    const data = p.manifest.data as { version?: number } | undefined
    return {
      schema: SCHEMA,
      id: p.id,
      version: p.manifest.version,
      source: p.source,
      installed_at_ms: at,
      grant: [...(p.manifest.permissions ?? [])],
      optional_granted: prev === null ? [] : prev.optional_granted.filter((o) => declared.includes(o)),
      enabled: prev === null ? true : prev.enabled,
      startup_allowed: prev !== null && prev.startup_allowed && !startupNewly,
      data_version: prev === null ? (data?.version ?? 1) : prev.data_version,
      signer: null,
      ...extra,
    }
  }

  return {
    list: () => serial(listNow),

    stageFile: (file) =>
      serial(async () => {
        if (!file.name.toLowerCase().endsWith('.hewplugin')) {
          throw new PluginError('package_unreadable', 'This file is not a plugin package. Plugin packages end in .hewplugin.')
        }
        if (file.size > MAX_PACKAGE_BYTES) {
          throw new PluginError('package_too_large', 'This package is too large.')
        }
        const wasm = await deps.wasm()
        const bytes = new Uint8Array(await file.arrayBuffer())
        const parsed = JSON.parse(wasm.plugin_package_from_zip(bytes)) as {
          manifest: PluginManifest
          files: { path: string; offset: number; size: number }[]
        }

        // The kernel read the archive and says where each file's bytes are;
        // this takes exactly those ranges and reads nothing else of the zip.
        const files = new Map<string, Uint8Array>()
        for (const f of parsed.files) {
          const end = f.offset + f.size
          if (!Number.isSafeInteger(f.offset) || !Number.isSafeInteger(f.size) || f.offset < 0 || f.size < 0 || end > bytes.length) {
            throw new PluginError('package_unreadable', `This file is not a plugin package Hew can read (${f.path} is out of range).`)
          }
          files.set(f.path, bytes.slice(f.offset, end))
        }
        // What is stored is the kernel-listed manifest file, and the manifest
        // the person consents to is read from those same bytes.
        const manifestBytes = files.get(MANIFEST_FILE)
        if (manifestBytes === undefined) throw new PluginError('manifest_missing', 'The package has no hew-plugin.json at its top level.')
        let manifest: PluginManifest
        try {
          manifest = JSON.parse(wasm.plugin_manifest_parse(decoder.decode(manifestBytes))) as PluginManifest
        } catch (e) {
          throw toPluginError(e)
        }
        if (!sameJson(manifest, parsed.manifest)) {
          throw new PluginError('package_unreadable', 'This file is not a plugin package Hew can read (its manifest is not the one it lists).')
        }
        const id = checkedId(manifest.id)
        const manifestJson = JSON.stringify(manifest)
        const source: InstallSource = { kind: 'file', name: file.name }
        const sourceJson = JSON.stringify(source)

        const all = await listNow()
        const rows = all.flatMap((l) => (l.kind === 'installed' ? [{ installed: l.installed, manifest: l.manifest }] : []))
        wasm.plugin_install_check(manifestJson, sourceJson, hewVersion(), JSON.stringify(rows))
        const existing = all.find((l) => (l.kind === 'installed' ? l.installed.id : l.id) === id)
        // A record from a newer Hew is refused; a folder with no record at
        // all is an install that never finished, and is replaced.
        if (existing?.kind === 'unreadable' && !incomplete.has(id)) throw unreadableError(id, existing.reason)
        const previous = existing?.kind === 'installed' ? existing : null

        const consent = JSON.parse(
          wasm.plugin_consent(
            manifestJson,
            sourceJson,
            previous === null ? undefined : JSON.stringify(previous.installed),
            previous === null ? undefined : JSON.stringify(previous.manifest),
            undefined,
          ),
        ) as ConsentPayload

        const plugins = await pluginsDir(false)
        if (plugins !== null) await clearStaging(plugins, id)
        // One waiting install per plugin: a newer one replaces the older.
        for (const [t, other] of pendings) if (other.id === id) pendings.delete(t)
        const token = newToken()
        pendings.set(token, {
          id,
          files,
          source,
          consent,
          manifest,
          previous: previous === null ? null : previous.installed,
          previousJson: previous === null ? null : JSON.stringify(previous.installed),
          createdAt: now(),
        })
        return { token, consent }
      }),

    commit: (token, accept) =>
      serial(async () => {
        const p = pendings.get(token)
        if (p === undefined) {
          throw new PluginError('pending_unknown', 'This install is no longer waiting for an answer. Start it again.')
        }
        pendings.delete(token)
        if (!accept) return null
        const at = now()
        if (at - p.createdAt > PENDING_TTL_MS) {
          throw new PluginError('pending_expired', 'This install waited too long for an answer. Start it again.')
        }
        // What was installed may have changed since the sheet was shown: the
        // same checks run again, before anything is written.
        const wasm = await deps.wasm()
        const all = await listNow()
        const rows = all.flatMap((l) => (l.kind === 'installed' ? [{ installed: l.installed, manifest: l.manifest }] : []))
        wasm.plugin_install_check(JSON.stringify(p.manifest), JSON.stringify(p.source), hewVersion(), JSON.stringify(rows))
        const nowRow = all.find((l) => (l.kind === 'installed' ? l.installed.id : l.id) === p.id)
        if (nowRow?.kind === 'unreadable' && !incomplete.has(p.id)) throw unreadableError(p.id, nowRow.reason)
        const nowJson = nowRow?.kind === 'installed' ? JSON.stringify(nowRow.installed) : null
        if (nowJson !== p.previousJson) {
          throw new PluginError('pending_stale', 'This plugin changed while the install was waiting. Start the install again.')
        }
        const manifestBytes = p.files.get(MANIFEST_FILE)
        if (manifestBytes === undefined) throw new PluginError('manifest_missing', 'The package has no hew-plugin.json at its top level.')
        const plugins = (await pluginsDir(true)) as FileSystemDirectoryHandle
        const staging = await mkdir(plugins, STAGING)
        const name = `${p.id}-${token}`
        await removeIn(staging, name)
        const fresh = await mkdir(staging, name)
        const record = recordFor(p, at)
        await writeFileIn(fresh, MANIFEST_FILE, manifestBytes)
        const filesDir = await mkdir(fresh, FILES)
        for (const [rel, bytes] of p.files) await writeNested(filesDir, rel, bytes)
        await writeFileIn(fresh, INSTALLED_FILE, recordBytes(record))

        // The installed version becomes the new one's `previous/`; its own
        // `previous/` is dropped.
        // The old folder is removed only after everything is ready, and put
        // back if the copy into place fails.
        const current = await subdir(plugins, p.id, false)
        let backup: FileSystemDirectoryHandle | null = null
        if (current !== null && nowJson !== null) {
          backup = await mkdir(fresh, PREVIOUS)
          await copyTree(current, backup, { skip: PREVIOUS, last: INSTALLED_FILE })
        }
        await replaceFolder(plugins, p.id, fresh, backup)
        await removeIn(staging, name)
        notify()
        return record
      }),

    setEnabled: (id, enabled) =>
      serial(async () => {
        const plugins = (await pluginsDir(false)) as FileSystemDirectoryHandle | null
        const { installed } = await installedNow(id)
        const write = async (record: Installed): Promise<void> => {
          const folder = await subdir(plugins as FileSystemDirectoryHandle, record.id, false)
          if (folder === null) throw notInstalled(record.id)
          await writeFileIn(folder, INSTALLED_FILE, recordBytes(record))
        }
        installed.enabled = enabled
        if (enabled) installed.startup_allowed = true
        await write(installed)
        const also: string[] = []
        if (!enabled) {
          const queue = [id]
          while (queue.length > 0) {
            const off = queue.pop() as string
            for (const dependent of await dependentsOf(off)) {
              const { installed: record } = await installedNow(dependent)
              record.enabled = false
              await write(record)
              also.push(dependent)
              queue.push(dependent)
            }
          }
        }
        notify()
        return also
      }),

    uninstall: (id) =>
      serial(async () => {
        checkedId(id)
        const plugins = await pluginsDir(false)
        if (plugins === null || (await subdir(plugins, id, false)) === null) throw notInstalled(id)
        const dependents = await dependentsOf(id)
        if (dependents.length > 0) {
          throw new PluginError(
            'dependency_missing',
            `This needs other plugins first, or other plugins still need it: ${dependents.join(', ')}.`,
          )
        }
        await purge(id)
        notify()
      }),

    previous: (id) =>
      serial(async () => {
        await installedNow(id)
        const plugins = (await pluginsDir(false)) as FileSystemDirectoryHandle
        const current = (await subdir(plugins, id, false)) as FileSystemDirectoryHandle
        const previous = await subdir(current, PREVIOUS, false)
        if (previous === null) throw new PluginError('no_previous_version', `"${id}" has no earlier version to go back to.`)
        try {
          return await readIn(previous, id)
        } catch (e) {
          if (e instanceof Unreadable) throw unreadableError(id, e.message)
          throw e
        }
      }),

    rollback: (id) =>
      serial(async () => {
        await installedNow(id)
        const plugins = (await pluginsDir(false)) as FileSystemDirectoryHandle
        const current = (await subdir(plugins, id, false)) as FileSystemDirectoryHandle
        const previous = await subdir(current, PREVIOUS, false)
        if (previous === null) throw new PluginError('no_previous_version', `"${id}" has no earlier version to go back to.`)
        try {
          await readIn(previous, id)
        } catch (e) {
          if (e instanceof Unreadable) throw unreadableError(id, e.message)
          throw e
        }
        // The earlier version becomes the plugin; the one being replaced
        // becomes its `previous/`, so a second rollback goes forward again.
        const staging = await mkdir(plugins, STAGING)
        const name = `${id}-${newToken()}`
        const swap = await mkdir(staging, name)
        await copyTree(previous, swap, { last: INSTALLED_FILE })
        await copyTree(current, await mkdir(swap, PREVIOUS), { skip: PREVIOUS, last: INSTALLED_FILE })
        await replaceFolder(plugins, id, swap, await subdir(swap, PREVIOUS, false))
        await removeIn(staging, name)
        notify()
        return (await installedNow(id)).installed
      }),

    readFile: (id, rel) =>
      serial(async () => {
        const segments = checkedRel(rel)
        await installedNow(id)
        const plugins = (await pluginsDir(false)) as FileSystemDirectoryHandle
        let at = (await subdir(plugins, id, false)) as FileSystemDirectoryHandle
        for (const s of [FILES, ...segments.slice(0, -1)]) {
          const next = await subdir(at, s, false)
          if (next === null) throw new PluginError('package_path_invalid', `The package holds "${rel}", which is not a file name Hew accepts.`)
          at = next
        }
        try {
          return await readFileIn(at, segments[segments.length - 1])
        } catch {
          throw new PluginError('package_path_invalid', `The package holds "${rel}", which is not a file name Hew accepts.`)
        }
      }),

    onChanged(fn) {
      listeners.add(fn)
      return () => {
        listeners.delete(fn)
      }
    },
  }
}
