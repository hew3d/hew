/* oxlint-disable lingui/no-unlocalized-strings -- plugin install copy and identifiers; English only until the feature graduates. */
/**
 * Shared pieces for the plugin store tests: a stand-in for the kernel's five
 * exports (the real ones need a built preview kernel), and a way to make a
 * `.hewplugin` and read the fake file system back.
 */

import { FakeDir } from '../test/opfsFake'
import { storedZipOf as storedZip } from './storedZip'
import type { PluginWasm } from './webPluginStore'

export interface ManifestInput {
  id?: string
  name?: unknown
  version?: string
  permissions?: string[]
  optionalPermissions?: string[]
  requires?: { id: string; range: string }[]
  icon?: string
  [k: string]: unknown
}

export function manifestJson(over: ManifestInput = {}): string {
  return JSON.stringify({
    manifest: 1,
    id: 'com.example.one',
    name: 'One',
    version: '1.0.0',
    api: 1,
    main: 'dist/background.js',
    permissions: ['model.read'],
    ...over,
  })
}

/** A `.hewplugin` as a File. `extra` adds or replaces entries. */
export function pluginFile(over: ManifestInput = {}, extra: Record<string, string> = {}, name = 'one.hewplugin'): File {
  const bytes = storedZip({
    'hew-plugin.json': manifestJson(over),
    'dist/': '',
    'dist/background.js': '// background',
    ...extra,
  })
  return new File([new Uint8Array(bytes)], name)
}

export interface ListedFile {
  path: string
  offset: number
  size: number
}

/** Where each file's bytes are in a stored zip, read from its central
 *  directory (the last end record, as most readers do). A test stand-in for
 *  the kernel's own reader. */
export function indexStoredZip(bytes: Uint8Array): ListedFile[] {
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  let eocd = bytes.length - 22
  while (eocd >= 0 && view.getUint32(eocd, true) !== 0x06054b50) eocd--
  if (eocd < 0) throw 'INSTALL_PACKAGE_UNREADABLE: This file is not a plugin package Hew can read (no end of archive).'
  const count = view.getUint16(eocd + 10, true)
  let at = view.getUint32(eocd + 16, true)
  const out: ListedFile[] = []
  for (let i = 0; i < count; i++) {
    const size = view.getUint32(at + 24, true)
    const nameLen = view.getUint16(at + 28, true)
    const extra = view.getUint16(at + 30, true)
    const comment = view.getUint16(at + 32, true)
    const local = view.getUint32(at + 42, true)
    const path = new TextDecoder().decode(bytes.subarray(at + 46, at + 46 + nameLen))
    at += 46 + nameLen + extra + comment
    if (path.endsWith('/')) continue
    out.push({ path, offset: local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true), size })
  }
  return out
}

function normalize(text: string): Record<string, unknown> {
  const m = JSON.parse(text) as Record<string, unknown>
  return { permissions: [], optionalPermissions: [], requires: [], data: { version: 1 }, ...m }
}

function refuse(code: string, text: string): never {
  throw `${code}: ${text}`
}

/** What the kernel does, in the small: enough for the store's own rules. */
export function fakeWasm(over: { files?: (bytes: Uint8Array) => ListedFile[] } = {}): PluginWasm {
  return {
    plugin_package_from_zip(bytes) {
      const files = (over.files ?? indexStoredZip)(bytes)
      const at = files.find((f) => f.path === 'hew-plugin.json')
      if (at === undefined) refuse('INSTALL_MANIFEST_MISSING', 'The package has no hew-plugin.json at its top level.')
      return JSON.stringify({
        manifest: normalize(new TextDecoder().decode(bytes.subarray(at.offset, at.offset + at.size))),
        files: [...files].sort((a, b) => (a.path < b.path ? -1 : 1)),
      })
    },
    plugin_manifest_parse(text) {
      try {
        return JSON.stringify(normalize(text))
      } catch {
        return refuse('MANIFEST_MANIFEST_MALFORMED', 'The manifest is not valid JSON.')
      }
    },
    plugin_hew_version_ok: () => true,
    plugin_install_check(manifestJson, _source, _running, installedJson) {
      const manifest = JSON.parse(manifestJson) as { id: string; requires: { id: string }[] }
      if (manifest.id.startsWith('org.hew3d.')) refuse('INSTALL_RESERVED_ID', `"${manifest.id}" is kept for plugins Hew publishes.`)
      const have = new Set((JSON.parse(installedJson) as { installed: { id: string } }[]).map((r) => r.installed.id))
      const missing = manifest.requires.filter((r) => !have.has(r.id)).map((r) => r.id)
      if (missing.length > 0) refuse('INSTALL_DEPENDENCY_MISSING', `This needs other plugins first: ${missing.join(', ')}.`)
    },
    plugin_consent(manifestJson, sourceJson, previousInstalledJson, previousManifestJson) {
      const m = JSON.parse(manifestJson) as { id: string; name: string; version: string; permissions: string[] }
      const prev = previousInstalledJson === undefined ? null : (JSON.parse(previousInstalledJson) as { version: string; grant: string[] })
      const prevManifest = previousManifestJson === undefined ? null : (JSON.parse(previousManifestJson) as { name: string })
      return JSON.stringify({
        id: m.id,
        name: m.name,
        version: m.version,
        permissions: m.permissions.map((name) => ({ name, text: name })),
        optional: [],
        warnings: [],
        on_startup: false,
        activation_reason: null,
        signer: null,
        unsigned: true,
        source: JSON.parse(sourceJson),
        replaces: prev === null ? null : { name: prevManifest?.name ?? m.id, version: prev.version },
        update:
          prev === null
            ? null
            : {
                added: m.permissions.filter((p) => !prev.grant.includes(p)).map((name) => ({ name, text: name })),
                new_optional: [],
                startup_newly: false,
                from_version: prev.version,
              },
      })
    },
  }
}

/** A file's text from a fake directory tree, by slash path. */
export async function textAt(root: FakeDir, path: string): Promise<string | null> {
  const parts = path.split('/')
  let dir = root
  for (const p of parts.slice(0, -1)) {
    const next = dir.dirs.get(p)
    if (next === undefined) return null
    dir = next
  }
  const f = dir.files.get(parts[parts.length - 1])
  return f === undefined ? null : new TextDecoder().decode(f.bytes)
}

/** Every file path under a fake directory, sorted. */
export function treeOf(dir: FakeDir, prefix = ''): string[] {
  const out: string[] = []
  for (const name of dir.files.keys()) out.push(prefix + name)
  for (const [name, d] of dir.dirs) out.push(...treeOf(d, `${prefix}${name}/`))
  return out.sort()
}
