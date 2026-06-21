/**
 * WebRecoveryStore — browser implementation of RecoveryStore using IndexedDB.
 *
 * db: "hew-recovery", object store: "snapshot", single fixed key: "current".
 * The Uint8Array is stored directly inside the record — structured clone
 * handles typed arrays natively, no serialization needed.
 *
 * Guarded throughout: if indexedDB is unavailable (privacy mode, or any
 * environment that doesn't expose it), write/clear are no-ops and read()
 * resolves to null. This module must never throw.
 */

import type { RecoveryMeta, RecoverySnapshot, RecoveryStore } from './recoveryStore'

const DB_NAME = 'hew-recovery'
const STORE_NAME = 'snapshot'
const KEY = 'current'

interface StoredRecord {
  bytes: Uint8Array
  meta: RecoveryMeta
}

function hasIndexedDB(): boolean {
  return typeof indexedDB !== 'undefined'
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
  async write(bytes: Uint8Array, meta: RecoveryMeta): Promise<void> {
    if (!hasIndexedDB()) return
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readwrite')
        const record: StoredRecord = { bytes, meta }
        tx.objectStore(STORE_NAME).put(record, KEY)
        await new Promise<void>((resolve, reject) => {
          tx.oncomplete = () => resolve()
          tx.onerror = () => reject(tx.error ?? new Error('transaction failed'))
        })
      } finally {
        db.close()
      }
    } catch {
      // Best-effort — never throw.
    }
  }

  async read(): Promise<RecoverySnapshot | null> {
    if (!hasIndexedDB()) return null
    try {
      const db = await openDb()
      try {
        const tx = db.transaction(STORE_NAME, 'readonly')
        const record = await requestToPromise<StoredRecord | undefined>(
          tx.objectStore(STORE_NAME).get(KEY),
        )
        if (record == null) return null
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
        tx.objectStore(STORE_NAME).delete(KEY)
        await new Promise<void>((resolve, reject) => {
          tx.oncomplete = () => resolve()
          tx.onerror = () => reject(tx.error ?? new Error('transaction failed'))
        })
      } finally {
        db.close()
      }
    } catch {
      // Best-effort — never throw.
    }
  }
}
