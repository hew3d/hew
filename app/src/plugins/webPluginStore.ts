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
 * OPFS has no rename that works everywhere, so a change is copy then swap:
 * the new arrangement is written whole into `.staging/`, then the installed
 * folder is copied whole to `.staging/<id>-<token>.old` (its record last),
 * removed, and the new arrangement copied into place with its record last. A
 * failure part way puts the `.old` copy back; if even that is cut short, the
 * folder has no record and the `.old` copy holds a complete record, so the
 * next locked call restores it. `<id>/storage.json` belongs to the plugin id:
 * it is carried across an update and a rollback and goes with an uninstall.
 *
 * The `.old` rules. A `.old` copy is a restore source only when it holds a
 * complete record and the intent marker `.swap-intent`, which the swap writes
 * after the copy is whole and before it removes the installed folder, so a
 * copy cut short is never mistaken for one. Only a completed swap or a
 * completed recovery removes a `.old` copy of an installed plugin, and an
 * uninstall removes the plugin's copies before the folder, so an interrupted
 * uninstall leaves the plugin installed, never half-removed and restorable.
 * A swap that finds the folder healthy first removes the id's older copies,
 * which can only be leftovers. Staging a package never touches a `.old`.
 *
 * Every change runs under the Web Lock `hew-plugins`, so two tabs cannot
 * interleave; with no Web Locks a change is refused.
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
  /** The lock manager (production: `navigator.locks`); null for none. */
  locks?: LockProvider | null
}

/** The part of `navigator.locks` the store uses. */
export interface LockProvider {
  request<T>(name: string, callback: () => Promise<T>): Promise<T>
}

/** A plugin's saved data, as the host and the page call it. */
export interface WebPluginStorage {
  storageGet(id: string, key: string): Promise<string | null>
  storageSet(id: string, key: string, value: string): Promise<void>
  storageDelete(id: string, key: string): Promise<void>
  /** Every saved key, sorted. */
  storageList(id: string): Promise<string[]>
}

export interface WebPluginStore {
  list(): Promise<Listed[]>
  stageFile(file: File): Promise<PendingView>
  commit(token: string, accept: boolean): Promise<Installed | null>
  /** `shown` is `recordFingerprint` of the record the person was shown when
   *  turning a plugin on; the call refuses `plugin_changed` if it differs. */
  setEnabled(id: string, enabled: boolean, shown?: string): Promise<string[]>
  uninstall(id: string): Promise<void>
  /** `shown` is `rollbackFingerprint` of the two records the person was
   *  shown; the call refuses `plugin_changed` if either differs. */
  rollback(id: string, shown?: string): Promise<Installed>
  /** The record and manifest `rollback` would restore. */
  previous(id: string): Promise<{ installed: Installed; manifest: PluginManifest }>
  /** One package file by served path. */
  readFile(id: string, rel: string): Promise<Uint8Array>
  onChanged(fn: () => void): () => void
  /** The same four calls the module exports, on this store's file system,
   *  channel and lock. */
  storage: WebPluginStorage
}

const LOCK_NAME = 'hew-plugins'
const STORAGE_FILE = 'storage.json'
const MAX_STORAGE_BYTES = 5 * 1024 * 1024
const MAX_STORAGE_KEY_BYTES = 256
const PLUGINS_DIR = 'plugins'
const STAGING = '.staging'
const MANIFEST_FILE = 'hew-plugin.json'
const INSTALLED_FILE = 'installed.json'
const FILES = 'files'
/** Written into a `.old` copy once it is whole, before the folder it copies
 *  is removed; a copy without it is never restored from. */
const SWAP_INTENT = '.swap-intent'
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

/** True when `dir` holds a file called `name`. */
async function hasFile(dir: FileSystemDirectoryHandle, name: string): Promise<boolean> {
  try {
    await dir.getFileHandle(name)
    return true
  } catch {
    return false
  }
}

/** True when `dir` holds a record that parses as a JSON object. A file that
 *  exists is not enough: a write cut short leaves an empty one. */
async function hasRecord(dir: FileSystemDirectoryHandle): Promise<boolean> {
  try {
    const value: unknown = JSON.parse(decoder.decode(await readFileIn(dir, INSTALLED_FILE)))
    return typeof value === 'object' && value !== null && !Array.isArray(value)
  } catch {
    return false
  }
}

/** A file's bytes, or null when there is none. */
async function readOptional(dir: FileSystemDirectoryHandle, name: string): Promise<Uint8Array | null> {
  try {
    return await readFileIn(dir, name)
  } catch (e) {
    if (isNotFound(e)) return null
    throw e
  }
}

/** Makes `dir/storage.json` a copy of `bytes`, or removes it when null. */
async function putStorage(dir: FileSystemDirectoryHandle, bytes: Uint8Array | null): Promise<void> {
  if (bytes === null) await removeIn(dir, STORAGE_FILE)
  else await writeFileIn(dir, STORAGE_FILE, bytes)
}

/** The `.old` copies of `id` in staging, by name. */
async function oldsOf(staging: FileSystemDirectoryHandle, id: string): Promise<string[]> {
  const out: string[] = []
  for await (const [name, handle] of staging.entries()) {
    if (handle.kind === 'directory' && name.startsWith(id + '-') && /^[0-9a-f]{32}\.old$/.test(name.slice(id.length + 1))) out.push(name)
  }
  return out
}

async function removeOlds(staging: FileSystemDirectoryHandle, id: string): Promise<void> {
  for (const name of await oldsOf(staging, id)) await removeIn(staging, name)
}

/** Puts `source` in place of `plugins/<id>`, copy then swap. The installed
 *  folder is first copied whole to `.staging/<id>-<token>.old`; only then is
 *  it removed and `source` copied in with its record last. A failure puts
 *  the `.old` copy back (or, with no installed folder before, leaves none);
 *  if that is cut short the `.old` copy stays, and `recover` finishes it. */
async function swapIn(
  plugins: FileSystemDirectoryHandle,
  staging: FileSystemDirectoryHandle,
  id: string,
  source: FileSystemDirectoryHandle,
  token: string,
): Promise<void> {
  const current = await subdir(plugins, id, false)
  const oldName = `${id}-${token}.old`
  let backup: FileSystemDirectoryHandle | null = null
  if (current !== null) {
    try {
      // A folder with a record is the plugin as it is, so any `.old` copy of
      // this id still lying about is a leftover and goes. A folder with none
      // is an unfinished install, and a copy beside it may be the only
      // complete one: those stay.
      if (await hasRecord(current)) await removeOlds(staging, id)
      await removeIn(staging, oldName)
      backup = await mkdir(staging, oldName)
      await copyTree(current, backup, { last: INSTALLED_FILE })
      // The copy is whole: from here on the folder may be removed, and this
      // marker says the copy is what to restore it from.
      await writeFileIn(backup, SWAP_INTENT, encoder.encode('1\n'))
    } catch (e) {
      await removeIn(staging, oldName).catch(() => undefined)
      throw e
    }
  }
  try {
    await removeIn(plugins, id)
    await copyTree(source, await mkdir(plugins, id), { last: INSTALLED_FILE })
  } catch (e) {
    try {
      // With nothing installed before, the half-written folder has no record
      // and lists as an unfinished install, which can be removed or replaced.
      if (backup !== null) {
        await removeIn(plugins, id)
        await copyTree(backup, await mkdir(plugins, id), { skip: SWAP_INTENT, last: INSTALLED_FILE })
        await removeIn(staging, oldName)
      }
    } catch {
      /* the original failure is the one to report; `recover` finishes the undo */
    }
    throw e
  }
  if (backup !== null) await removeIn(staging, oldName).catch(() => undefined)
}

const encoder = new TextEncoder()
const decoder = new TextDecoder('utf-8', { fatal: true })

/** What a confirmation shows of a record, for the call that acts on it to
 *  compare: the version, what it was granted, and when it was installed (so
 *  an install of the same version over itself counts as a change too). */
export function recordFingerprint(r: Installed): string {
  return JSON.stringify([r.version, [...r.grant].sort(), [...r.optional_granted].sort(), r.installed_at_ms])
}

/** The same for a rollback: the installed record and the one it would restore. */
export function rollbackFingerprint(current: Installed, previous: Installed): string {
  return JSON.stringify([recordFingerprint(current), recordFingerprint(previous)])
}

function pluginChanged(): PluginError {
  return new PluginError('plugin_changed', 'The plugin changed while Hew was asking. Try again.')
}

/** Orders by code point, as the desktop store's sorted map orders its keys
 *  (UTF-8 byte order); `<` on strings compares UTF-16 code units, which puts
 *  a character outside the Basic Multilingual Plane before U+E000..U+FFFF. */
export function compareCodePoints(a: string, b: string): number {
  let i = 0
  let j = 0
  while (i < a.length && j < b.length) {
    const x = a.codePointAt(i) as number
    const y = b.codePointAt(j) as number
    if (x !== y) return x < y ? -1 : 1
    i += x > 0xffff ? 2 : 1
    j += y > 0xffff ? 2 : 1
  }
  return Number(i < a.length) - Number(j < b.length)
}

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

  // A storage notice is for the host; only a change notice moves the pane.
  channel?.addEventListener?.('message', (ev?: MessageEvent) => {
    const data = ev?.data as { kind?: unknown } | undefined
    if (data?.kind === 'storage') return
    listeners.forEach((l) => l())
  })

  const notify = (): void => {
    listeners.forEach((l) => l())
    try {
      channel?.postMessage({ kind: 'changed' })
    } catch {
      /* a closed channel must never break a write */
    }
  }

  const locksOf = (): LockProvider | null => {
    if (deps.locks !== undefined) return deps.locks
    return typeof navigator !== 'undefined' && navigator.locks ? (navigator.locks as unknown as LockProvider) : null
  }

  // One change at a time, across tabs: every step below reads what an
  // earlier one wrote. Reads without Web Locks still queue within this tab.
  let tail: Promise<unknown> = Promise.resolve()
  function run<T>(mutating: boolean, fn: () => Promise<T>): Promise<T> {
    const locks = locksOf()
    let out: Promise<T>
    if (locks === null) {
      if (mutating) {
        return Promise.reject(
          new PluginError(
            'locks_unavailable',
            'This browser cannot keep two tabs from changing plugins at the same time, so plugins cannot be changed here.',
          ),
        )
      }
      out = tail.then(fn, fn)
      tail = out.catch(() => undefined)
    } else {
      out = locks.request(LOCK_NAME, async () => {
        await recover()
        return fn()
      })
    }
    return out.catch((e) => {
      throw toPluginError(e)
    })
  }
  const serial = <T>(fn: () => Promise<T>): Promise<T> => run(true, fn)
  const reading = <T>(fn: () => Promise<T>): Promise<T> => run(false, fn)

  const pluginsDir = async (create: boolean): Promise<FileSystemDirectoryHandle | null> =>
    subdir(await opfsRoot(), PLUGINS_DIR, create)

  /** Finishes an undo a failed swap could not: a plugin folder with no record
   *  whose `.old` copy holds a complete one, and the swap's intent marker,
   *  gets that copy back. Best effort; a folder with no record and no such
   *  copy is left as it is, and no copy is removed but the one restored from
   *  (and, then, the id's other leftovers). */
  async function recover(): Promise<void> {
    try {
      const plugins = await pluginsDir(false)
      const staging = plugins === null ? null : await subdir(plugins, STAGING, false)
      if (plugins === null || staging === null) return
      const olds = new Map<string, string[]>()
      for await (const [name, handle] of staging.entries()) {
        const m = /^(.+)-[0-9a-f]{32}\.old$/.exec(name)
        if (m !== null && handle.kind === 'directory' && validPluginId(m[1])) olds.set(m[1], [...(olds.get(m[1]) ?? []), name])
      }
      for (const [id, names] of olds) {
        const folder = await subdir(plugins, id, false)
        if (folder !== null && (await hasRecord(folder))) continue
        for (const name of names) {
          const backup = await subdir(staging, name, false)
          if (backup === null || !(await hasFile(backup, SWAP_INTENT)) || !(await hasRecord(backup))) continue
          await removeIn(plugins, id)
          await copyTree(backup, await mkdir(plugins, id), { skip: SWAP_INTENT, last: INSTALLED_FILE })
          await removeOlds(staging, id)
          break
        }
      }
    } catch {
      /* a failed recovery changes nothing; the next locked call tries again */
    }
  }

  const storageApi = makeStorage({
    root: opfsRoot,
    post: (m) => {
      try {
        channel?.postMessage(m)
      } catch {
        /* a closed channel must never break a write */
      }
    },
    write: serial,
    read: reading,
  })

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

  /** Removes the staged installs of `id`. A `.old` copy is not one: it is a
   *  swap's restore source, and only `purge` (the plugin's end) removes it
   *  without a swap or a recovery. */
  async function clearStaging(plugins: FileSystemDirectoryHandle, id: string, withOlds = false): Promise<void> {
    const staging = await subdir(plugins, STAGING, false)
    if (staging === null) return
    const mine: string[] = []
    for await (const [name] of staging.entries()) {
      // `<id>-<32 hex digits>`, with an optional `.<suffix>`; another
      // plugin whose id starts the same way is not matched.
      if (!name.startsWith(id + '-') || !/^[0-9a-f]{32}(\..*)?$/.test(name.slice(id.length + 1))) continue
      if (name.endsWith('.old') && !withOlds) continue
      mine.push(name)
    }
    for (const name of mine) await removeIn(staging, name)
  }

  /** Removes everything the store keeps for `id`: its folder, its staging
   *  entries and its small settings. The one removal path uninstall takes;
   *  stores added later join it here. */
  async function purge(id: string): Promise<void> {
    const plugins = await pluginsDir(false)
    if (plugins !== null) {
      // The `.old` copies first: with them gone, an uninstall cut short
      // leaves the plugin installed, and `recover` has nothing to restore.
      const staging = await subdir(plugins, STAGING, false)
      if (staging !== null) await removeOlds(staging, id)
      await removeIn(plugins, id)
      await clearStaging(plugins, id, true)
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
    list: () => reading(listNow),

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
        // `previous/` is dropped. Saved data belongs to the id, so it comes
        // along. Everything is ready in staging before the installed folder
        // is touched.
        const current = await subdir(plugins, p.id, false)
        if (current !== null) {
          await putStorage(fresh, await readOptional(current, STORAGE_FILE))
          if (nowJson !== null) await copyTree(current, await mkdir(fresh, PREVIOUS), { skip: PREVIOUS, last: INSTALLED_FILE })
        }
        try {
          await swapIn(plugins, staging, p.id, fresh, token)
        } catch (e) {
          await removeIn(staging, name).catch(() => undefined)
          throw e
        }
        await removeIn(staging, name).catch(() => undefined)
        notify()
        return record
      }),

    setEnabled: (id, enabled, shown) =>
      serial(async () => {
        const plugins = (await pluginsDir(false)) as FileSystemDirectoryHandle | null
        const { installed } = await installedNow(id)
        if (enabled && shown !== undefined && recordFingerprint(installed) !== shown) throw pluginChanged()
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
      reading(async () => {
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

    rollback: (id, shown) =>
      serial(async () => {
        const cur = await installedNow(id)
        const plugins = (await pluginsDir(false)) as FileSystemDirectoryHandle
        const current = (await subdir(plugins, id, false)) as FileSystemDirectoryHandle
        const previous = await subdir(current, PREVIOUS, false)
        if (previous === null) throw new PluginError('no_previous_version', `"${id}" has no earlier version to go back to.`)
        let before: Installed
        try {
          before = (await readIn(previous, id)).installed
        } catch (e) {
          if (e instanceof Unreadable) throw unreadableError(id, e.message)
          throw e
        }
        // What was shown is what comes back, or nothing does.
        if (shown !== undefined && rollbackFingerprint(cur.installed, before) !== shown) throw pluginChanged()
        // The earlier version becomes the plugin; the one being replaced
        // becomes its `previous/`, so a second rollback goes forward again.
        // The new arrangement is built whole in staging first; saved data
        // stays the plugin's, so the version coming back gets the current
        // copy, not the one it had when it was replaced.
        const staging = await mkdir(plugins, STAGING)
        const token = newToken()
        const name = `${id}-${token}`
        try {
          await removeIn(staging, name)
          const swap = await mkdir(staging, name)
          await copyTree(previous, swap, { last: INSTALLED_FILE })
          await putStorage(swap, await readOptional(current, STORAGE_FILE))
          await copyTree(current, await mkdir(swap, PREVIOUS), { skip: PREVIOUS, last: INSTALLED_FILE })
          await swapIn(plugins, staging, id, swap, token)
        } catch (e) {
          await removeIn(staging, name).catch(() => undefined)
          throw e
        }
        await removeIn(staging, name).catch(() => undefined)
        notify()
        return (await installedNow(id)).installed
      }),

    readFile: (id, rel) =>
      reading(async () => {
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

    storage: storageApi,

    onChanged(fn) {
      listeners.add(fn)
      return () => {
        listeners.delete(fn)
      }
    },
  }
}

interface StorageCtx {
  root: () => Promise<FileSystemDirectoryHandle>
  post: (message: { kind: 'storage'; id: string; keys: string[] }) => void
  /** Runs a change under the plugin lock. */
  write: <T>(fn: () => Promise<T>) => Promise<T>
  /** Runs a read, under the lock where there is one. */
  read: <T>(fn: () => Promise<T>) => Promise<T>
}

type SavedData = Record<string, string>

const hasOwn = (o: object, k: string): boolean => Object.prototype.hasOwnProperty.call(o, k)

function makeStorage(ctx: StorageCtx): WebPluginStorage {
  const encode = new TextEncoder()

  const checkedKey = (key: string): void => {
    const n = encode.encode(key).length
    if (n === 0 || n > MAX_STORAGE_KEY_BYTES) {
      throw new PluginError('storage_key_invalid', `A storage key must be 1 to ${MAX_STORAGE_KEY_BYTES} bytes long.`)
    }
  }
  const quota = (): PluginError =>
    new PluginError('storage_quota', `This plugin's saved data would pass ${MAX_STORAGE_BYTES >> 20} MB, the most a plugin may keep.`)
  const unreadable = (): PluginError =>
    new PluginError('storage_unreadable', "Hew cannot read this plugin's saved data; it is left as it is.")

  /** `plugins/<id>`, only when it holds a record: no call creates it. */
  async function folderOf(id: string): Promise<FileSystemDirectoryHandle> {
    checkedId(id)
    const plugins = await subdir(await ctx.root(), PLUGINS_DIR, false)
    const folder = plugins === null ? null : await subdir(plugins, id, false)
    if (folder === null || !(await hasFile(folder, INSTALLED_FILE))) throw notInstalled(id)
    return folder
  }

  async function load(folder: FileSystemDirectoryHandle): Promise<SavedData> {
    const bytes = await readOptional(folder, STORAGE_FILE)
    if (bytes === null) return {}
    if (bytes.length > MAX_STORAGE_BYTES) throw unreadable()
    let value: unknown
    try {
      value = JSON.parse(decoder.decode(bytes))
    } catch {
      throw unreadable()
    }
    if (typeof value !== 'object' || value === null || Array.isArray(value)) throw unreadable()
    if (!Object.values(value).every((v) => typeof v === 'string')) throw unreadable()
    // No prototype, so a key named `__proto__` is just a key.
    return Object.assign(Object.create(null) as SavedData, value)
  }

  /** Compact JSON with sorted keys, as the desktop file is written. */
  const dump = (data: SavedData): Uint8Array =>
    encode.encode(JSON.stringify(Object.fromEntries(Object.entries(data).sort(([a], [b]) => compareCodePoints(a, b)))))

  const wrap = <T>(fn: () => Promise<T>): Promise<T> =>
    fn().catch((e) => {
      throw toPluginError(e)
    })

  return {
    storageGet: (id, key) =>
      wrap(() =>
        ctx.read(async () => {
          checkedKey(key)
          const data = await load(await folderOf(id))
          return hasOwn(data, key) ? data[key] : null
        }),
      ),

    storageList: (id) =>
      wrap(() =>
        ctx.read(async () => Object.keys(await load(await folderOf(id))).sort(compareCodePoints)),
      ),

    storageSet: (id, key, value) =>
      wrap(() =>
        ctx.write(async () => {
          checkedKey(key)
          // Too large on its own: refused before the file is read.
          if (encode.encode(key).length + encode.encode(value).length > MAX_STORAGE_BYTES) throw quota()
          const folder = await folderOf(id)
          const data = await load(folder)
          data[key] = value
          const bytes = dump(data)
          if (bytes.length > MAX_STORAGE_BYTES) throw quota()
          await writeFileIn(folder, STORAGE_FILE, bytes)
          ctx.post({ kind: 'storage', id, keys: [key] })
        }),
      ),

    storageDelete: (id, key) =>
      wrap(() =>
        ctx.write(async () => {
          checkedKey(key)
          const folder = await folderOf(id)
          const data = await load(folder)
          if (!hasOwn(data, key)) return
          delete data[key]
          if (Object.keys(data).length === 0) await removeIn(folder, STORAGE_FILE)
          else await writeFileIn(folder, STORAGE_FILE, dump(data))
          ctx.post({ kind: 'storage', id, keys: [key] })
        }),
      ),
  }
}

let defaultStore: WebPluginStore | null = null

/** The store the module-level storage calls use: this tab's file system,
 *  the `hew-plugins` channel and `navigator.locks`. */
function defaultPluginStore(): WebPluginStore {
  if (defaultStore === null) {
    defaultStore = makeWebPluginStore({
      wasm: () => Promise.reject(new Error('the kernel is not used for saved data')),
    })
  }
  return defaultStore
}

/** The value saved under `key`, or null. */
export const storageGet = (id: string, key: string): Promise<string | null> => defaultPluginStore().storage.storageGet(id, key)
/** Saves `value` under `key`; other windows hear `{kind:'storage', id, keys}`. */
export const storageSet = (id: string, key: string, value: string): Promise<void> =>
  defaultPluginStore().storage.storageSet(id, key, value)
/** Removes `key`; a missing key is not an error and announces nothing. */
export const storageDelete = (id: string, key: string): Promise<void> => defaultPluginStore().storage.storageDelete(id, key)
/** Every saved key, sorted. */
export const storageList = (id: string): Promise<string[]> => defaultPluginStore().storage.storageList(id)
