/**
 * A fake Durable Object namespace/storage stack for the unit suites —
 * copied from share-relay's `testSupport/fakeDurableObject.ts` verbatim
 * (down to the comments explaining the `node:sqlite` BLOB-boundary
 * conversion and the `deleteAll` DROP-not-DELETE behavior), since both
 * Workers share the same storage shim (`types.ts`'s `DurableObjectStorage`)
 * and the same one-shot-per-request-burst reasoning. See that file for the
 * full explanation; this copy exists so `bug-intake` has zero dependency on
 * `share-relay`'s source tree.
 */

import { DatabaseSync } from 'node:sqlite'

import type { DurableObjectId, DurableObjectState, DurableObjectStorage, SqlStorageCursor } from '../types.ts'

function toBindable(value: unknown): unknown {
  return value instanceof ArrayBuffer ? new Uint8Array(value) : value
}

function fromColumn(value: unknown): unknown {
  return value instanceof Uint8Array ? value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength) : value
}

export class FakeDurableObjectStorage implements DurableObjectStorage {
  private readonly db = new DatabaseSync(':memory:')
  private alarmTime: number | null = null

  readonly sql = {
    exec: <T = Record<string, unknown>>(query: string, ...bindings: unknown[]): SqlStorageCursor<T> => {
      // Real Durable Object SQLite refuses more than 100 bound parameters.
      if (bindings.length > 100) throw new Error(`too many SQL variables (${bindings.length} > 100)`)
      const rows = this.db.prepare(query).all(...bindings.map(toBindable)) as Array<Record<string, unknown>>
      const mapped = rows.map((row) => {
        const out: Record<string, unknown> = {}
        for (const [key, value] of Object.entries(row)) out[key] = fromColumn(value)
        return out as T
      })
      return { toArray: () => mapped }
    },
  }

  async setAlarm(scheduledTime: number): Promise<void> {
    this.alarmTime = scheduledTime
  }

  async getAlarm(): Promise<number | null> {
    return this.alarmTime
  }

  async deleteAlarm(): Promise<void> {
    this.alarmTime = null
  }

  async deleteAll(): Promise<void> {
    // Real SQLite-backed DO `deleteAll()` wipes the whole database — schema
    // included, not just rows — so a later `SELECT` against a table this
    // instance created throws `no such table` until it's re-created. DROP
    // (not DELETE) here so this fake reproduces that.
    for (const { name } of this.db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all() as Array<{
      name: string
    }>) {
      this.db.exec(`DROP TABLE IF EXISTS "${name}"`)
    }
  }
}

/** A fake `DurableObjectNamespace<T>`: `idFromName` wraps the string,
 *  `get` lazily constructs (and memoizes) one `T` per distinct name via
 *  `factory` — mirroring the real runtime's guarantee that the same id
 *  always resolves to the same DO instance and its storage. */
export class FakeDurableObjectNamespace<T> {
  private readonly instances = new Map<string, T>()
  private readonly factory: (state: DurableObjectState) => T

  constructor(factory: (state: DurableObjectState) => T) {
    this.factory = factory
  }

  idFromName(name: string): DurableObjectId {
    return { toString: () => name }
  }

  get(id: DurableObjectId): T {
    const key = id.toString()
    let instance = this.instances.get(key)
    if (instance === undefined) {
      const storage = new FakeDurableObjectStorage()
      instance = this.factory({ storage })
      this.instances.set(key, instance)
    }
    return instance
  }
}
