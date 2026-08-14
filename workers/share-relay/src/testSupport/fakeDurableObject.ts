/**
 * A fake Durable Object namespace/storage stack for `handlers.test.ts` and
 * `shareDrop.test.ts`, standing in for the real Cloudflare binding the same
 * way the old `handlers.test.ts` hand-rolled an in-memory R2 mock (see git
 * history) — no `miniflare`, no network, no Cloudflare account required.
 *
 * `FakeDurableObjectStorage` is backed by Node's built-in `node:sqlite`
 * (`DatabaseSync`, stable since Node 22.5 — this repo's `.node-version`
 * pins well past that), so `shareDrop.ts`'s actual SQL runs against a real
 * SQLite engine rather than a hand-rolled query-matcher: `CREATE TABLE`,
 * bound-parameter `INSERT`/`SELECT`, `ORDER BY` — all real. The only
 * translation this layer does is at the BLOB boundary: `shareDrop.ts` binds
 * and reads BLOB columns as `ArrayBuffer` (this project's `SqlStorageValue`
 * contract, matching the real runtime's `ArrayBuffer | string | number |
 * null` union — see `types.ts`), but `node:sqlite` requires a `Uint8Array`/
 * `Buffer` for a BLOB parameter and hands one back on read. `execImpl`
 * converts in both directions so `shareDrop.ts` never has to know it's
 * talking to `node:sqlite` instead of the real thing.
 *
 * One-shot atomicity note (see `shareDrop.ts`'s class doc for the full
 * argument): every method below has a fully synchronous body — no `await`
 * on anything that isn't already resolved — so calling `store`/`consume`/
 * `destroy`/`alarm` runs to completion in one synchronous burst even though
 * they're declared `async`. That's what makes `Promise.all([a.consume(),
 * b.consume()])` in the tests behave exactly like two requests hitting a
 * real single-threaded DO: JS evaluates `a.consume()` fully (including its
 * `await this.destroy()` chain, which bottoms out in synchronous SQLite
 * calls) before it even starts evaluating `b.consume()`.
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
    // Real SQLite-backed DO `deleteAll()` wipes the whole database — SCHEMA
    // included, not just rows — so any table a caller created is gone
    // afterward and a later `SELECT` against it throws `no such table` until
    // it is re-created. DROP (not DELETE) here so this fake reproduces that:
    // an earlier revision used `DELETE FROM`, which kept the schema and let
    // the one-shot-consume test pass against a store that 500'd on real
    // `workerd` (the second GET's `SELECT ... FROM meta` hit a dropped table).
    for (const { name } of this.db.prepare("SELECT name FROM sqlite_master WHERE type = 'table'").all() as Array<{
      name: string
    }>) {
      this.db.exec(`DROP TABLE IF EXISTS "${name}"`)
    }
  }
}

/** A fake `DurableObjectNamespace<T>`: `idFromName` just wraps the string,
 *  and `get` lazily constructs (and memoizes) one `T` per distinct name via
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
