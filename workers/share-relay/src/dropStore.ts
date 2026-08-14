/**
 * DropStore — the SQLite read/write logic for one drop, factored out of the
 * `ShareDrop` Durable Object (`shareDrop.ts`) so it can be unit-tested under
 * bare `node --test` without importing `cloudflare:workers`. It operates on
 * any object implementing this project's `DurableObjectStorage` shim
 * (`types.ts`): the real runtime passes `ctx.storage`; the test suite passes
 * `testSupport/fakeDurableObject.ts`'s `node:sqlite`-backed fake. `ShareDrop`
 * itself is a thin `DurableObject` subclass that constructs one of these over
 * its own `ctx.storage` and forwards its RPC methods here — see that file.
 *
 * Why this split exists at all: Durable Object RPC (a `stub.store(...)` call
 * from `handlers.ts`) is ONLY exposed for a class that `extends DurableObject`
 * from `cloudflare:workers`. A plain class does not get an RPC surface, so
 * `ShareDrop` must do that `extends` — which pulls in the `cloudflare:workers`
 * module, which bare `node --test` cannot import. Keeping the actual logic
 * here, framework-free, lets the fast unit suite exercise it directly while
 * the (trivial) DO wrapper is validated against real `workerd` via
 * `wrangler dev`.
 *
 * Storage shape: a single `meta` row (pinned `id = 0`) plus `chunk(idx, data)`
 * rows. A SQLite row/BLOB caps at 2 MB and a bound statement's TEXT caps at
 * 100 KB, so `handlers.ts` pre-splits the upload into ≤1.9 MB pieces and each
 * is written as its own BOUND parameter — never inlined into SQL text.
 *
 * One-shot atomicity is a property of the DO wrapper, not this class: a
 * Durable Object instance is single-threaded per id, so `consume()`'s
 * synchronous read block runs to completion before any concurrent second
 * `consume()` is delivered, and the input gate holds that second call until
 * `destroy()`'s async wipe resolves — so it always sees empty storage. This
 * class's own concurrent-consume test only confirms the logic doesn't
 * double-serve when calls are serialized; the runtime is what serializes them.
 */

import type { DurableObjectStorage } from './types.ts'

/** A drop is treated as expired 10 minutes after upload. The alarm (armed in
 *  `store`, at `uploadedAt + TTL_MS`, fired by `ShareDrop.alarm`) is the
 *  primary enforcement; `consume`'s own check is a backstop for a late alarm. */
export const TTL_MS = 10 * 60 * 1000

interface MetaRow {
  name: string
  uploadedAt: number
  totalBytes: number
  chunkCount: number
}

interface ChunkRow {
  data: ArrayBuffer
}

export class DropStore {
  private readonly storage: DurableObjectStorage

  constructor(storage: DurableObjectStorage) {
    this.storage = storage
    this.migrate()
  }

  /** `CREATE TABLE IF NOT EXISTS` — idempotent, and re-run at the TOP of
   *  every `store`/`consume` (not just in the constructor). It must be: a DO
   *  instance is reused across requests, and `destroy()`'s `deleteAll()` wipes
   *  the SQLite SCHEMA, not just rows (see `destroy`), so the tables are gone
   *  after any consume/destroy on the same live instance. Re-ensuring here is
   *  what makes a second `consume()` return `null` instead of throwing `no
   *  such table` (a real 500 the tests now reproduce via the fake's DROP-based
   *  `deleteAll`). `meta.id` is pinned to 0 so the table holds zero or one row;
   *  a `SELECT` against it is how "populated" is told from "empty". */
  private migrate(): void {
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS meta (
         id INTEGER PRIMARY KEY CHECK (id = 0),
         name TEXT NOT NULL,
         uploadedAt INTEGER NOT NULL,
         totalBytes INTEGER NOT NULL,
         chunkCount INTEGER NOT NULL
       )`,
    )
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS chunk (
         idx INTEGER PRIMARY KEY,
         data BLOB NOT NULL
       )`,
    )
  }

  /** Writes `chunks` (already ≤`CHUNK_BYTES` pieces from `handlers.ts`) and
   *  arms the TTL alarm. `name` is generic dead-drop metadata, not the
   *  document's display name — that never reaches this Worker (it rides the
   *  URL fragment `#recv=…`, which browsers never transmit). Throws if this
   *  drop is already populated: a token is single-use for write (a practically
   *  impossible 128-bit collision, not a normal-path guard). */
  async store(name: string, chunks: Uint8Array[]): Promise<void> {
    this.migrate()
    const existing = this.storage.sql.exec<{ id: number }>('SELECT id FROM meta').toArray()
    if (existing.length > 0) {
      throw new Error('ShareDrop already populated for this token')
    }

    const uploadedAt = Date.now()
    const totalBytes = chunks.reduce((sum, chunk) => sum + chunk.byteLength, 0)
    this.storage.sql.exec(
      'INSERT INTO meta (id, name, uploadedAt, totalBytes, chunkCount) VALUES (0, ?, ?, ?, ?)',
      name,
      uploadedAt,
      totalBytes,
      chunks.length,
    )
    chunks.forEach((chunk, idx) => {
      // Bound as an ArrayBuffer (this project's `SqlStorageValue` contract),
      // sliced to a fresh buffer so a view with a non-zero `byteOffset` (a
      // `subarray`) never leaks sibling bytes.
      const buffer = chunk.slice().buffer
      this.storage.sql.exec('INSERT INTO chunk (idx, data) VALUES (?, ?)', idx, buffer)
    })

    await this.storage.setAlarm(uploadedAt + TTL_MS)
  }

  /** One-shot read: reassembles chunks in index order, then wipes storage and
   *  the alarm before returning. Returns `null` for an empty drop (never
   *  stored, or already consumed) and for one already past `TTL_MS`. */
  async consume(): Promise<{ name: string; bytes: Uint8Array } | null> {
    this.migrate()
    const metaRows = this.storage.sql
      .exec<MetaRow>('SELECT name, uploadedAt, totalBytes, chunkCount FROM meta')
      .toArray()
    if (metaRows.length === 0) return null
    const meta = metaRows[0]

    if (Date.now() - meta.uploadedAt > TTL_MS) {
      await this.destroy()
      return null
    }

    const chunkRows = this.storage.sql.exec<ChunkRow>('SELECT data FROM chunk ORDER BY idx ASC').toArray()
    const bytes = new Uint8Array(meta.totalBytes)
    let offset = 0
    for (const row of chunkRows) {
      bytes.set(new Uint8Array(row.data), offset)
      offset += row.data.byteLength
    }

    await this.destroy()
    return { name: meta.name, bytes }
  }

  /** Non-consuming existence check: same "populated and not past `TTL_MS`"
   *  test `consume` uses, but SELECTs only the `meta` row and never touches
   *  `chunk` or calls `destroy()`/`deleteAll()` — a drop `peek()` finds
   *  present is still there, byte-for-byte, for a following `consume()`.
   *  Backs the desktop dialog's pickup-detection poll (`handlers.ts`'s `HEAD
   *  /drop/<token>`): it needs to know the drop is gone without being the
   *  request that consumes it. Like `consume`, re-runs `migrate()` first so
   *  a peek against a never-populated (never `store`d) token's DO — which has
   *  no `meta` table yet — returns `{ exists: false }` instead of throwing
   *  `no such table`. */
  async peek(): Promise<{ exists: boolean }> {
    this.migrate()
    const metaRows = this.storage.sql.exec<{ uploadedAt: number }>('SELECT uploadedAt FROM meta').toArray()
    if (metaRows.length === 0) return { exists: false }
    const { uploadedAt } = metaRows[0]
    if (Date.now() - uploadedAt > TTL_MS) return { exists: false }
    return { exists: true }
  }

  /** Wipes all storage and cancels the alarm — idempotent. Used by `consume`'s
   *  one-shot cleanup, the dialog-close `DELETE /drop/<token>`, and TTL
   *  self-expiry (`ShareDrop.alarm`). */
  async destroy(): Promise<void> {
    await this.storage.deleteAll()
    await this.storage.deleteAlarm()
  }
}
