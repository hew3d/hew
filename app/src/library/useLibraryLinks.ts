/**
 * useLibraryLinks — keeps the model's Library links current (v1.5 OW-2).
 *
 * Asks the kernel (`Scene.library_status_json`) where every part that came
 * from the Library stands against the item now in the Library, and holds the
 * answer for the notice bar, the Review list, the panel markers and the Save
 * popover. It runs after a document opens, when the Library changes, when the
 * Library or the Components panel opens, and once the model has been quiet
 * after an edit (a debounce, never a timer that polls). Everything after the
 * first `await` is off the click that caused it, and the Library is read
 * through a cache so a file is read and hashed once per session.
 *
 * Two things hold the answer still. While a group or component is open for
 * editing the kernel scopes library placements to that session, so a fresh
 * check would read "no links" and the bar and markers would vanish; the last
 * answer stays instead (Update All then asks the kernel, which refuses each
 * part it cannot touch and names the open one). And with Settings > Folders
 * "Check the Library for newer parts" off, nothing runs on its own (open,
 * quiet after an edit, a Library change) and the bar is never raised, but a
 * check still runs when the Library or Components panel opens, or the Save
 * to Library popover does (`refreshNow`), so markers and "Update in Library"
 * keep working.
 *
 * Items made before prints existed carry none in their manifest: the first
 * time the kernel reads such a file's bytes it hands the prints back
 * (`itemPrints`), and they are kept by file hash from then on.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import type { LibraryItemMeta } from './types'
import { libraryStore } from '../io/libraryStore'
import { readItemSummary, sha256Hex } from './itemFiles'
import { buildLibraryItem } from './libraryModel'
import {
  dismissKey,
  linksByDefinition,
  newerLinks,
  parseLibraryStatus,
  pendingNewer,
  type LibraryLink,
} from './libraryUpdates'

/** What is known about one file in the Library folder. */
interface FileEntry {
  fileName: string
  id: string | undefined
  /** The name the Library shows for the item. */
  name: string
  /** The item's `hew.library` metadata, as the Library holds it. */
  meta: LibraryItemMeta
  hash: string
  /** The stable id of the item's first definition, as the summary reports it. */
  firstSid: string | undefined
  /** The prints the manifest carries, when it has them. */
  prints?: Record<string, string>
  /** Prints the kernel computed from the bytes (an item saved before prints). */
  computed?: Record<string, string>
  mtimeMs: number
}

export interface LibraryItemRef {
  fileName: string
  hash: string
  name: string
  meta: LibraryItemMeta
  /** The stable id of the item's root definition, when the file says. */
  rootSid: string | undefined
}

export interface UseLibraryLinksOptions {
  scene: WasmScene | null
  /** Bumped on every document change. */
  docRev: number
  /** Bumped when a new document replaces this one. */
  docGeneration: number
  /** Settings ▸ Folders: check on its own and raise the bar. Off still
   *  checks when the Library or Components panel opens, or on `refreshNow`. */
  enabled: boolean
  /** A group or component is open for editing: keep the last answer. */
  sessionOpen?: boolean
  /** The Library dialog or the Components panel just opened. */
  openedKey: number
  /** The model holds at least one Library-stamped thing (a cheap walk). */
  hasLinks: boolean
}

export interface LibraryLinksApi {
  links: LibraryLink[]
  byDefinition: Map<string, LibraryLink>
  /** Links the bar still counts (newer, not dismissed). */
  pending: LibraryLink[]
  /** Hide the bar for what is newer now. */
  dismiss: () => void
  /** Check now (after an update, or an edit the caller knows matters). */
  refreshNow: () => Promise<LibraryLink[]>
  /** The file and hash a Library item id is stored under now. */
  itemRef: (itemId: string) => LibraryItemRef | undefined
  /** Read the files an update needs, fresh, with the hash an insert passes. */
  readItems: (itemIds: readonly string[]) => Promise<Record<string, { bytes: Uint8Array; contentHash: string }>>
}

const DEBOUNCE_MS = 1200
const EMPTY: LibraryLink[] = []

export function useLibraryLinks({ scene, docRev, docGeneration, enabled, sessionOpen = false, openedKey, hasLinks }: UseLibraryLinksOptions): LibraryLinksApi {
  const [links, setLinksState] = useState<LibraryLink[]>(EMPTY)
  const linksRef = useRef<LibraryLink[]>(EMPTY)
  const setLinks = useCallback((next: LibraryLink[]) => {
    linksRef.current = next
    setLinksState(next)
  }, [])
  const sessionOpenRef = useRef(sessionOpen)
  sessionOpenRef.current = sessionOpen
  const [dismissedRev, setDismissedRev] = useState(0)
  const dismissedRef = useRef<Set<string>>(new Set())
  const cacheRef = useRef<Map<string, FileEntry>>(new Map())
  const itemsRef = useRef<Map<string, FileEntry>>(new Map())
  const seqRef = useRef(0)
  const sceneRef = useRef(scene)
  sceneRef.current = scene
  const hasLinksRef = useRef(hasLinks)
  hasLinksRef.current = hasLinks
  const enabledRef = useRef(enabled)
  enabledRef.current = enabled
  // A Library or Components panel is open: its markers want a current answer
  // even with the setting off, so every trigger below checks as asked.
  const panelOpenRef = useRef(openedKey !== 0)
  panelOpenRef.current = openedKey !== 0
  const [storeTick, setStoreTick] = useState(0)

  /** Read, parse and hash one Library file, through the cache. Null when it
   *  cannot be read (it holds no item the model could be linked to). */
  const entryFor = useCallback(async (f: { fileName: string; size: number; mtimeMs: number }): Promise<FileEntry | null> => {
    const store = libraryStore()
    const cache = cacheRef.current
    const key = `${f.fileName}|${f.size}|${f.mtimeMs}`
    const hit = cache.get(key)
    if (hit !== undefined && f.mtimeMs !== 0) return hit
    try {
      const bytes = await store.read(f.fileName)
      const summary = await readItemSummary(bytes)
      const raw = summary.doc_attrs?.['hew.library']
      const item = buildLibraryItem(f, summary, '')
      const prints = readPrints(raw)
      const entry: FileEntry = {
        fileName: f.fileName,
        id: item.meta.id,
        name: item.displayName,
        meta: item.meta,
        hash: await sha256Hex(bytes),
        firstSid: summary.first_component_sid ?? undefined,
        mtimeMs: f.mtimeMs,
      }
      if (prints !== undefined) entry.prints = prints
      cache.set(key, entry)
      return entry
    } catch {
      return null
    }
  }, [])

  /**
   * The Library items the model's links name, as {id -> entry}. Only files
   * that can hold one of `wanted` are read: first those whose name carries
   * the id's short suffix (how every save names its file), then, only for an
   * id still unfound, the rest. Two files claiming one id (a copied item):
   * the first in the Library's own listing order wins, which is the file the
   * API's `hew.library.*` commands pick, so the app and the console agree.
   */
  const scanLibrary = useCallback(
    async (wanted: ReadonlySet<string>): Promise<Map<string, FileEntry>> => {
      const store = libraryStore()
      const out = new Map<string, FileEntry>()
      if (!store.available() || wanted.size === 0) return out
      const files = await store.list()
      // Entries of files that are gone leave the cache, so it cannot grow without bound.
      const live = new Set(files.map((f) => `${f.fileName}|${f.size}|${f.mtimeMs}`))
      for (const key of [...cacheRef.current.keys()]) if (!live.has(key)) cacheRef.current.delete(key)
      const suffixes = new Set([...wanted].map(idSuffix))
      const likely = files.filter((f) => suffixes.has(fileSuffix(f.fileName)))
      const rest = files.filter((f) => !likely.includes(f))
      for (const group of [likely, rest]) {
        for (const f of group) {
          // Everything the model names is found: the rest of the folder stays unread.
          if ([...wanted].every((id) => out.has(id))) break
          const entry = await entryFor(f)
          if (entry?.id !== undefined && wanted.has(entry.id) && !out.has(entry.id)) out.set(entry.id, entry)
        }
      }
      return out
    },
    [entryFor],
  )

  /** `force`: a check somebody asked for (a panel opening, an update, the
   *  Save popover), which runs even with the setting off. */
  const runCheck = useCallback(async (force = false): Promise<LibraryLink[]> => {
    // A session scopes the kernel's placements: its answer would be "nothing".
    if (sessionOpenRef.current) return linksRef.current
    if (!force && !enabledRef.current) return linksRef.current
    const mySeq = ++seqRef.current
    const s = sceneRef.current
    if (s === null || !hasLinksRef.current) {
      if (mySeq === seqRef.current) setLinks(EMPTY)
      return EMPTY
    }
    // A first, cheap pass with an empty Library: every link reads "missing",
    // which is all it takes to learn which items the model names. Only those
    // files are read.
    let wanted: Set<string>
    try {
      wanted = new Set(parseLibraryStatus(s.library_status_json({})).links.map((l) => l.item))
    } catch {
      return EMPTY
    }
    if (wanted.size === 0) {
      if (mySeq === seqRef.current) setLinks(EMPTY)
      return EMPTY
    }
    let items: Map<string, FileEntry>
    try {
      items = await scanLibrary(wanted)
    } catch {
      // The Library cannot be listed right now: say nothing rather than guess.
      return EMPTY
    }
    // A newer check, or a different document, took over while the files were read.
    if (mySeq !== seqRef.current || sceneRef.current !== s) return EMPTY
    const request: Record<string, { prints?: Record<string, string>; bytes?: Uint8Array }> = {}
    for (const [id, entry] of items) {
      const prints = entry.prints ?? entry.computed
      if (prints !== undefined) {
        request[id] = { prints }
        continue
      }
      // A pre-1.5 item: the kernel computes its prints from the bytes, once.
      try {
        request[id] = { bytes: await libraryStore().read(entry.fileName) }
      } catch {
        // Unreadable now: it reads as missing, which says nothing.
      }
    }
    if (mySeq !== seqRef.current || sceneRef.current !== s) return EMPTY
    let status
    try {
      status = parseLibraryStatus(s.library_status_json(request))
    } catch {
      return EMPTY
    }
    for (const [id, prints] of Object.entries(status.itemPrints)) {
      const entry = items.get(id)
      if (entry !== undefined) entry.computed = prints
    }
    // A session opened while the files were read: its answer is the scoped one.
    if (sessionOpenRef.current) return linksRef.current
    itemsRef.current = items
    setLinks(status.links)
    return status.links
  }, [scanLibrary, setLinks])

  // After a document opens or is replaced, when the setting is turned on or a
  // group or component closes, and when the Library folder changes. With the
  // setting off these do nothing on their own, unless a panel is open (then
  // the check is the forced one, so its markers follow a new document or an
  // insert).
  useEffect(() => {
    void runCheck(panelOpenRef.current)
  }, [runCheck, scene, docGeneration, enabled, sessionOpen, storeTick, hasLinks])

  // A different document starts with nothing known when no check runs on its own.
  useEffect(() => {
    if (!enabledRef.current && !sessionOpenRef.current && !panelOpenRef.current) setLinks(EMPTY)
  }, [scene, docGeneration, setLinks])

  // The Library or the Components panel opens (or closes): always checked.
  useEffect(() => {
    if (openedKey !== 0) void runCheck(true)
  }, [runCheck, openedKey])

  // After an edit, once the model has been quiet.
  const firstRevRef = useRef(docRev)
  useEffect(() => {
    if (docRev === firstRevRef.current) return
    firstRevRef.current = docRev
    // With the setting off this is a no-op unless a panel is open.
    const handle = setTimeout(() => void runCheck(panelOpenRef.current), DEBOUNCE_MS)
    return () => clearTimeout(handle)
  }, [docRev, runCheck])

  // Another window, or the app itself, wrote to the Library folder.
  useEffect(() => {
    let pending: ReturnType<typeof setTimeout> | null = null
    const unsubscribe = libraryStore().subscribe(() => {
      if (pending !== null) clearTimeout(pending)
      pending = setTimeout(() => {
        pending = null
        setStoreTick((n) => n + 1)
      }, 300)
    })
    return () => {
      unsubscribe()
      if (pending !== null) clearTimeout(pending)
    }
  }, [])

  // "Later" is for this document: a different one starts with a clean slate.
  useEffect(() => {
    if (dismissedRef.current.size === 0) return
    dismissedRef.current = new Set()
    setDismissedRev((n) => n + 1)
  }, [scene, docGeneration])

  const dismiss = useCallback(() => {
    for (const l of newerLinks(links)) dismissedRef.current.add(dismissKey(l))
    setDismissedRev((n) => n + 1)
  }, [links])

  const itemRef = useCallback((itemId: string): LibraryItemRef | undefined => {
    const e = itemsRef.current.get(itemId)
    return e === undefined ? undefined : { fileName: e.fileName, hash: e.hash, name: e.name, meta: e.meta, rootSid: e.firstSid }
  }, [])

  const readItems = useCallback(async (itemIds: readonly string[]) => {
    const out: Record<string, { bytes: Uint8Array; contentHash: string }> = {}
    const store = libraryStore()
    for (const id of new Set(itemIds)) {
      const e = itemsRef.current.get(id)
      if (e === undefined) continue
      try {
        const bytes = await store.read(e.fileName)
        out[id] = { bytes, contentHash: await sha256Hex(bytes) }
      } catch {
        // Left out: the kernel reports it as an item it cannot find.
      }
    }
    return out
  }, [])

  const byDefinition = useMemo(() => linksByDefinition(links), [links])
  const pending = useMemo(
    // With the setting off the bar is never raised, whatever a panel check found.
    () => (enabled ? pendingNewer(links, dismissedRef.current) : EMPTY),
    // dismissedRev: the set is mutated in place.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [links, dismissedRev, enabled],
  )
  const refreshNow = useCallback(() => runCheck(true), [runCheck])

  return { links, byDefinition, pending, dismiss, refreshNow, itemRef, readItems }
}

/** The `prints` a `hew.library` manifest entry carries, when it is a plain map of text. */
function readPrints(raw: unknown): Record<string, string> | undefined {
  if (raw === null || typeof raw !== 'object') return undefined
  const prints = (raw as { prints?: unknown }).prints
  if (prints === null || typeof prints !== 'object' || Array.isArray(prints)) return undefined
  const out: Record<string, string> = {}
  for (const [sid, p] of Object.entries(prints as Record<string, unknown>)) {
    // Only the scheme this build reads; anything else is worked out from the file.
    if (typeof p !== 'string' || !p.startsWith('p1:')) return undefined
    out[sid] = p
  }
  return Object.keys(out).length > 0 ? out : undefined
}

/** The short hex suffix `itemFileName` puts before `.hew` for an item id. */
function idSuffix(id: string): string {
  const hex = id.replace(/[^a-f0-9]/gi, '').toLowerCase()
  return (hex.length >= 6 ? hex.slice(0, 6) : hex.padEnd(6, '0')) || '000000'
}

/** The `-xxxxxx` suffix of a Library file's name, or '' when it has none. */
function fileSuffix(fileName: string): string {
  const m = /-([a-f0-9]{6})\.hew$/i.exec(fileName)
  return m === null ? '' : m[1]!.toLowerCase()
}
