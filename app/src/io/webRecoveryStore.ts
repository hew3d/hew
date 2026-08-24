/**
 * WebRecoveryStore — browser implementation of RecoveryStore using IndexedDB.
 *
 * db: "hew-recovery", object store: "snapshot". One record PER TAB, keyed by a
 * per-tab id, mirroring the desktop store's per-window `recovery-<label>.hew`
 * files. IndexedDB is scoped per-ORIGIN, not per-tab, so a single shared key
 * would let a second tab's autosave silently clobber the first tab's
 * crash-recovery snapshot (audit q-web-robustness, critical) — per-tab keys
 * are what keep two open documents from destroying each other's safety net.
 *
 * The per-tab id lives in `sessionStorage`: unique per tab, stable across a
 * reload, and gone when the tab closes or crashes. A crashed tab therefore
 * leaves an ORPHANED slot that a freshly opened tab (with a new id) finds and
 * offers to recover — exactly the crash-recovery path. To avoid offering a
 * snapshot still owned by a LIVE sibling tab, `list()` pings every tab over a
 * BroadcastChannel and excludes the slots that answer.
 *
 * Guarded throughout: if IndexedDB is unavailable (privacy mode, or any
 * environment that doesn't expose it), write/clear are no-ops and reads
 * resolve empty. This module must never throw.
 */

import type { RecoveryListing, RecoveryMeta, RecoverySnapshot, RecoveryStore } from './recoveryStore'

const DB_NAME = 'hew-recovery'
const STORE_NAME = 'snapshot'
const TAB_ID_KEY = 'hew.recovery.tab'
const PRESENCE_CHANNEL = 'hew-recovery-presence'
/** How long `list()` waits for live tabs to answer a presence ping. */
const PRESENCE_WINDOW_MS = 200

interface StoredRecord {
  bytes: Uint8Array
  meta: RecoveryMeta
}

function hasIndexedDB(): boolean {
  return typeof indexedDB !== 'undefined'
}

/** This tab's stable id. sessionStorage survives a reload but not a close, so
 * it is unique per tab and a closed/crashed tab's slot becomes orphaned. Falls
 * back to a module-scoped id if sessionStorage is unavailable (still unique
 * per page load, just not across a reload). */
let fallbackTabId: string | null = null
function tabId(): string {
  try {
    const existing = sessionStorage.getItem(TAB_ID_KEY)
    if (existing !== null && existing !== '') return existing
    const fresh = newId()
    sessionStorage.setItem(TAB_ID_KEY, fresh)
    return fresh
  } catch {
    if (fallbackTabId === null) fallbackTabId = newId()
    return fallbackTabId
  }
}

function newId(): string {
  try {
    return crypto.randomUUID()
  } catch {
    // Non-secure contexts without randomUUID: getRandomValues is still fine
    // for a non-security-sensitive slot id.
    const a = new Uint32Array(4)
    crypto.getRandomValues(a)
    return Array.from(a, (n) => n.toString(16)).join('')
  }
}

/** Open (creating if needed) the recovery database. */
function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, 1)
    req.onupgradeneeded = () => {
      const db = req.result
      if (!db.objectStoreNames.contains(STORE_NAME)) {
        db.createObjectStore(STORE_NAME)
      }
    }
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('indexedDB.open failed'))
  })
}

/** Wrap an IDBRequest in a Promise. */
function requestToPromise<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result)
    req.onerror = () => reject(req.error ?? new Error('IDBRequest failed'))
  })
}

export class WebRecoveryStore implements RecoveryStore {
  /** Responds to other tabs' presence pings so this tab's live slot is never
   * offered to them for recovery. Null where BroadcastChannel is absent. */
  private readonly presence: BroadcastChannel | null

  constructor() {
    let channel: BroadcastChannel | null = null
    try {
      if (typeof BroadcastChannel !== 'undefined') {
        channel = new BroadcastChannel(PRESENCE_CHANNEL)
        const myId = tabId()
        channel.addEventListener('message', (e: MessageEvent) => {
          const msg = e.data as { type?: string; nonce?: string } | null
          if (msg?.type === 'ping' && typeof msg.nonce === 'string') {
            channel?.postMessage({ type: 'alive', tabId: myId, nonce: msg.nonce })
          }
        })
      }
    } catch {
      channel = null
    }
    this.presence = channel
  }

  async write(bytes: Uint8Array, meta: RecoveryMeta): Promise<void> {
    if (!hasIndexedDB()) return
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readwrite')
        const record: StoredRecord = { bytes, meta }
        tx.objectStore(STORE_NAME).put(record, tabId())
        await txDone(tx)
      } finally {
        db.close()
      }
    } catch {
      // Best-effort — never throw.
    }
  }

  async list(): Promise<RecoveryListing[]> {
    if (!hasIndexedDB()) return []
    let entries: { slot: string; meta: RecoveryMeta }[] = []
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readonly')
        const store = tx.objectStore(STORE_NAME)
        const keys = await requestToPromise<IDBValidKey[]>(store.getAllKeys())
        const records = await requestToPromise<StoredRecord[]>(store.getAll())
        entries = keys.map((k, i) => ({ slot: String(k), meta: records[i].meta }))
      } finally {
        db.close()
      }
    } catch {
      return []
    }
    // Exclude slots owned by a currently-live tab (its own in-progress
    // document), so only orphaned (crashed/closed) snapshots are offered.
    const live = await this.liveTabs()
    return entries
      .filter((e) => !live.has(e.slot))
      .sort((a, b) => b.meta.savedAt - a.meta.savedAt)
  }

  /** Ping every tab and collect the ids that answer within the window. */
  private liveTabs(): Promise<Set<string>> {
    const channel = this.presence
    if (channel === null) return Promise.resolve(new Set())
    return new Promise((resolve) => {
      const nonce = newId()
      const alive = new Set<string>()
      const onMessage = (e: MessageEvent) => {
        const msg = e.data as { type?: string; tabId?: string; nonce?: string } | null
        if (msg?.type === 'alive' && msg.nonce === nonce && typeof msg.tabId === 'string') {
          alive.add(msg.tabId)
        }
      }
      channel.addEventListener('message', onMessage)
      try {
        channel.postMessage({ type: 'ping', nonce })
      } catch {
        channel.removeEventListener('message', onMessage)
        resolve(alive)
        return
      }
      setTimeout(() => {
        channel.removeEventListener('message', onMessage)
        resolve(alive)
      }, PRESENCE_WINDOW_MS)
    })
  }

  async claim(slot: string): Promise<RecoverySnapshot | null> {
    if (!hasIndexedDB()) return null
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readwrite')
        const store = tx.objectStore(STORE_NAME)
        const record = await requestToPromise<StoredRecord | undefined>(store.get(slot))
        if (record == null) return null
        // Re-home: delete the adopted slot so it is never re-offered — this
        // tab's own future autosaves write under its own id.
        store.delete(slot)
        await txDone(tx)
        return { bytes: record.bytes, meta: record.meta }
      } finally {
        db.close()
      }
    } catch {
      return null
    }
  }

  async clear(): Promise<void> {
    if (!hasIndexedDB()) return
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readwrite')
        tx.objectStore(STORE_NAME).delete(tabId())
        await txDone(tx)
      } finally {
        db.close()
      }
    } catch {
      // Best-effort — never throw.
    }
  }

  async discardAll(): Promise<void> {
    if (!hasIndexedDB()) return
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readwrite')
        tx.objectStore(STORE_NAME).clear()
        await txDone(tx)
      } finally {
        db.close()
      }
    } catch {
      // Best-effort — never throw.
    }
  }
}

function txDone(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve()
    tx.onerror = () => reject(tx.error ?? new Error('transaction failed'))
  })
}
