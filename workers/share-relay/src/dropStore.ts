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
 * Batched RPC protocol. Workers RPC caps ONE call's serialized arguments (or
 * return value) at 32 MiB — exactly the contract's per-drop maximum, which
 * with any envelope at all overflows. Shipping a whole drop's chunks in a
 * single `store(...)`/`consume()` call therefore fails for the largest legal
 * upload (found by the black-box conformance suite against real `workerd`;
 * the in-process unit suite's fake namespace never serializes, so it could
 * not see it). Both directions are batched at `RPC_BATCH_CHUNKS`:
 *
 *   write:  store(name, chunkCount, totalBytes, firstBatch) → append(batch)*
 *           — the drop is INCOMPLETE (invisible to consume/peek) until every
 *           chunk row is present; a PUT that dies mid-way leaves a partial
 *           the alarm wipes at uploadedAt + TTL like any other unread drop.
 *   read:   consume() claims the drop (one-shot happens HERE, synchronously)
 *           and returns its shape → take(from, count)* reads AND deletes each
 *           batch of chunk rows, wiping the drop after the last one.
 *
 * One-shot atomicity is a property of the DO wrapper, not this class: a
 * Durable Object instance is single-threaded per id, so `consume()`'s
 * synchronous claim runs to completion before any concurrent second
 * `consume()` is delivered — the second always finds `claimed = 1` and
 * returns `null`. This class's own concurrent-consume test only confirms the
 * logic doesn't double-serve when calls are serialized; the runtime is what
 * serializes them.
 */

import type { DurableObjectStorage } from './types.ts'

/** A drop is treated as expired 10 minutes after upload. The alarm (armed in
 *  `store`, at `uploadedAt + TTL_MS`, fired by `ShareDrop.alarm`) is the
 *  primary enforcement; `consume`/`peek`'s own check is a backstop for a late
 *  alarm. */
export const TTL_MS = 10 * 60 * 1000

/** Chunks per RPC call in both directions (`handlers.ts` groups `store`/
 *  `append` arguments and `take` requests by this). 8 × 1.9 MB ≈ 15.2 MB per
 *  call — comfortably under the 32 MiB RPC serialization cap with room for
 *  the envelope, and few enough calls (a full 32 MiB drop is 3 writes and 3
 *  reads) that the extra round trips are invisible next to the upload. */
export const RPC_BATCH_CHUNKS = 8

interface MetaRow {
  name: string
  uploadedAt: number
  totalBytes: number
  chunkCount: number
  claimed: number
}

interface ChunkRow {
  idx: number
  data: ArrayBuffer
}

/** What `consume()` hands back: the drop's shape, so the caller can `take`
 *  its chunk rows in RPC-sized batches. The bytes themselves never ride a
 *  single RPC return value. */
export interface DropHead {
  name: string
  totalBytes: number
  chunkCount: number
}

export class DropStore {
  private readonly storage: DurableObjectStorage

  constructor(storage: DurableObjectStorage) {
    this.storage = storage
    this.migrate()
  }

  /** `CREATE TABLE IF NOT EXISTS` — idempotent, and re-run at the TOP of
   *  every public method (not just in the constructor). It must be: a DO
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
         chunkCount INTEGER NOT NULL,
         claimed INTEGER NOT NULL DEFAULT 0
       )`,
    )
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS chunk (
         idx INTEGER PRIMARY KEY,
         data BLOB NOT NULL
       )`,
    )
    // Schema upgrade for a DO whose `meta` table predates the batched
    // protocol (no `claimed` column). `CREATE TABLE IF NOT EXISTS` is a no-op
    // on an existing table, so the column must be added explicitly — and
    // such tables DO survive a deploy: `deleteAll()` wipes the schema after
    // any consume/destroy, but a DO that only ever answered a `peek()` on a
    // never-stored token keeps its empty old-shape tables indefinitely (the
    // conformance suite reproduced this against a `wrangler dev` state dir).
    const hasClaimed = this.storage.sql
      .exec<{ n: number }>("SELECT COUNT(*) AS n FROM pragma_table_info('meta') WHERE name = 'claimed'")
      .toArray()[0].n
    if (hasClaimed === 0) {
      this.storage.sql.exec('ALTER TABLE meta ADD COLUMN claimed INTEGER NOT NULL DEFAULT 0')
    }
  }

  private readMeta(): MetaRow | null {
    const rows = this.storage.sql
      .exec<MetaRow>('SELECT name, uploadedAt, totalBytes, chunkCount, claimed FROM meta')
      .toArray()
    return rows.length === 0 ? null : rows[0]
  }

  private storedChunkCount(): number {
    return this.storage.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM chunk').toArray()[0].n
  }

  private insertChunks(startIdx: number, chunks: Uint8Array[]): void {
    chunks.forEach((chunk, i) => {
      // Bound as an ArrayBuffer (this project's `SqlStorageValue` contract),
      // sliced to a fresh buffer so a view with a non-zero `byteOffset` (a
      // `subarray`) never leaks sibling bytes.
      const buffer = chunk.slice().buffer
      this.storage.sql.exec('INSERT INTO chunk (idx, data) VALUES (?, ?)', startIdx + i, buffer)
    })
  }

  /** Begins a drop: writes the `meta` row (declaring the FULL `chunkCount` and
   *  `totalBytes` up front), the first batch of chunk rows, and arms the TTL
   *  alarm. Further batches arrive via `append`; until `chunkCount` rows are
   *  present the drop is incomplete and `consume`/`peek` treat it as absent.
   *  `name` is generic dead-drop metadata, not the document's display name —
   *  that never reaches this Worker (it rides the URL fragment `#recv=…`,
   *  which browsers never transmit). Throws if this drop is already populated:
   *  a token is single-use for write (a practically impossible 128-bit
   *  collision, not a normal-path guard). */
  async store(name: string, chunkCount: number, totalBytes: number, chunks: Uint8Array[]): Promise<void> {
    this.migrate()
    if (this.readMeta() !== null) {
      throw new Error('ShareDrop already populated for this token')
    }
    if (chunkCount < 1 || chunks.length > chunkCount) {
      throw new Error('ShareDrop.store: bad chunk count')
    }

    const uploadedAt = Date.now()
    this.storage.sql.exec(
      'INSERT INTO meta (id, name, uploadedAt, totalBytes, chunkCount, claimed) VALUES (0, ?, ?, ?, ?, 0)',
      name,
      uploadedAt,
      totalBytes,
      chunkCount,
    )
    this.insertChunks(0, chunks)

    await this.storage.setAlarm(uploadedAt + TTL_MS)
  }

  /** Appends the next batch of chunk rows to a drop begun by `store`,
   *  continuing at the current row count. Throws on a drop that was never
   *  begun (or was already wiped) or would overflow its declared count. */
  async append(chunks: Uint8Array[]): Promise<void> {
    this.migrate()
    const meta = this.readMeta()
    if (meta === null) {
      throw new Error('ShareDrop.append: no drop in progress')
    }
    const have = this.storedChunkCount()
    if (have + chunks.length > meta.chunkCount) {
      throw new Error('ShareDrop.append: more chunks than declared')
    }
    this.insertChunks(have, chunks)
  }

  /** Whether `meta` describes a drop that is complete, unclaimed, and not past
   *  `TTL_MS` — the single "present" predicate `consume` and `peek` share. */
  private isPresent(meta: MetaRow): boolean {
    if (meta.claimed !== 0) return false
    if (Date.now() - meta.uploadedAt > TTL_MS) return false
    return this.storedChunkCount() === meta.chunkCount
  }

  /** One-shot claim: returns the drop's shape (for the caller's following
   *  `take` calls) and marks it claimed in the SAME synchronous burst, so a
   *  concurrent second `consume()` deterministically gets `null`. Returns
   *  `null` for an empty drop (never stored, already consumed/claimed, or
   *  still uploading — see `store`) and for one already past `TTL_MS`, which
   *  it wipes on the way out. */
  async consume(): Promise<DropHead | null> {
    this.migrate()
    const meta = this.readMeta()
    if (meta === null) return null

    if (Date.now() - meta.uploadedAt > TTL_MS) {
      await this.destroy()
      return null
    }
    if (!this.isPresent(meta)) return null

    this.storage.sql.exec('UPDATE meta SET claimed = 1 WHERE id = 0')
    return { name: meta.name, totalBytes: meta.totalBytes, chunkCount: meta.chunkCount }
  }

  /** Reads chunk rows `[from, from + count)` in index order — and DELETES them
   *  as it goes: this is the destructive half of the one-shot read, split
   *  from `consume` only so each RPC return value stays under the cap. When
   *  the last row is gone the drop is destroyed (alarm included), so a
   *  caller that reads to the end never needs a separate `destroy`.
   *
   *  A drop that VANISHED between the claim and this call — the TTL alarm
   *  fired, or the desktop's dialog-close `DELETE` landed mid-download —
   *  answers an empty array, not an error: `handleGetDrop` sees the byte
   *  count fall short and turns that into the same 404 the phone gets for
   *  any other gone drop. Calling this on a drop that exists but was never
   *  claimed IS a protocol error and throws. */
  async take(from: number, count: number): Promise<Uint8Array[]> {
    this.migrate()
    const meta = this.readMeta()
    if (meta === null) return []
    if (meta.claimed === 0) {
      throw new Error('ShareDrop.take: drop is not claimed')
    }
    const rows = this.storage.sql
      .exec<ChunkRow>('SELECT idx, data FROM chunk WHERE idx >= ? AND idx < ? ORDER BY idx ASC', from, from + count)
      .toArray()
    const out = rows.map((row) => new Uint8Array(row.data))
    this.storage.sql.exec('DELETE FROM chunk WHERE idx >= ? AND idx < ?', from, from + count)
    if (this.storedChunkCount() === 0) {
      await this.destroy()
    }
    return out
  }

  /** Non-consuming existence check: the same "complete, unclaimed, and not
   *  past `TTL_MS`" predicate `consume` uses, without the claim — a drop
   *  `peek()` finds present is still there, byte-for-byte, for a following
   *  `consume()`. Backs the desktop dialog's pickup-detection poll
   *  (`handlers.ts`'s `HEAD /drop/<token>`): it needs to know the drop is
   *  gone without being the request that consumes it. Like `consume`, re-runs
   *  `migrate()` first so a peek against a never-populated (never `store`d)
   *  token's DO — which has no `meta` table yet — returns `{ exists: false }`
   *  instead of throwing `no such table`. */
  async peek(): Promise<{ exists: boolean }> {
    this.migrate()
    const meta = this.readMeta()
    if (meta === null) return { exists: false }
    return { exists: this.isPresent(meta) }
  }

  /** Wipes all storage and cancels the alarm — idempotent. Used by `take`'s
   *  end-of-drop cleanup, the dialog-close `DELETE /drop/<token>`, and TTL
   *  self-expiry (`ShareDrop.alarm`). */
  async destroy(): Promise<void> {
    await this.storage.deleteAll()
    await this.storage.deleteAlarm()
  }
}
