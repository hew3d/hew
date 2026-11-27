/**
 * ReportStore — the SQLite read/write logic for one report's chunked
 * upload, factored out of the `ReportDrop` Durable Object (`reportDrop.ts`)
 * so it unit-tests under bare `node --test` without importing
 * `cloudflare:workers` — the same split share-relay's `dropStore.ts` makes.
 *
 * Storage shape: a single `meta` row (pinned `id = 0`, the whole upload's
 * state machine) plus `piece(idx, data, length)` rows — ONE row per piece,
 * written exactly once each as it arrives (docs/design/report-bug.md §4
 * "Chunked upload"): no rechunking, no batching, unlike share-relay's
 * `DropStore` or this Worker's own earlier single-request-streaming design.
 * A piece is at most `PIECE_BYTES` (1.9 MB), comfortably under the 2 MB
 * SQLite row/BLOB cap, so it's simply written as one bound parameter.
 *
 * The state machine `startUpload`/`putPiece`/`commit` implement matches
 * §8's wire contract almost one-to-one:
 *
 *   - `startUpload` (from `POST /report/`) writes `meta` with `nextIndex =
 *     1`, stores piece 0, and arms the abandonment alarm.
 *   - `putPiece` (from `PUT /report/<id>/<index>`) accepts the next
 *     expected index (stores it, advances, re-arms the alarm), silently
 *     accepts an identical retry of the last stored index (no double
 *     write), and refuses anything else as out-of-order.
 *   - `commit` (from `POST /report/<id>/commit`) checks every declared byte
 *     arrived, flips `committed`, and re-arms the alarm for RETENTION
 *     instead of abandonment.
 *
 * Every method checks the presented token's hash against the stored one in
 * constant time (`uploadToken.ts`) — this class never sees a raw token,
 * only its SHA-256 hex digest, computed by the caller (`handlers.ts`).
 *
 * Storage is created LAZILY, by `startUpload` alone. A report id is public
 * and a Durable Object is addressed by name, so any request naming a
 * well-formed id that was never started (`PUT`/`commit` with a guessed or
 * stale id, an alarm racing a delete) still instantiates this class over
 * an empty database. If the constructor ran `CREATE TABLE` the way an
 * earlier version did, every such request would leave a few KiB of empty
 * schema behind forever — durable, never alarmed, never cleaned up — and
 * an unauthenticated caller could mint those by the hundred thousand
 * against the account-wide Durable Object storage and daily row-write
 * quotas (shared with share-relay). So every read path first asks
 * `sqlite_master` (a read, which persists nothing) whether `meta` exists
 * and answers "absent" without touching the schema when it doesn't.
 */

import type { DurableObjectStorage } from './types.ts'
import { RETENTION_MS, ABANDON_TIMEOUT_MS, UPLOAD_LIFETIME_MS, PIECE_BYTES } from './constants.ts'
import { constantTimeEqual } from './uploadToken.ts'

interface MetaRow {
  reportId: string
  receivedAt: number
  declaredTotal: number
  tokenHash: string
  nextIndex: number
  receivedBytes: number
  lastActivity: number
  committed: number
  description: string
  appVersion: string
  platform: string
  followHash: string | null
  parentId: string | null
}

interface PieceRow {
  idx: number
  data: ArrayBuffer
}

/** What `startUpload` needs beyond the raw bytes/lengths — the streamed
 *  head's validated fields (`headScan.ts`), carried here so `commit` can
 *  hand them back for the notification email without re-reading anything. */
export interface StartFields {
  description: string
  appVersion: string
  platform: string
  /** SHA-256 hex of the follow secret START minted (`Hew-Follow: 1`), or
   *  absent. Kept until COMMIT creates the report's thread from it
   *  (docs/design/follow-a-report.md §4.1, D4). */
  followHash?: string | null
  /** The parent report id of a follow-up attachment (`Hew-Follow-Up`), or
   *  absent. COMMIT appends the attachment to that report's thread. */
  parentId?: string | null
}

export type PutPieceResult =
  | { ok: true; stored: boolean } // stored=false means an accepted idempotent retry
  | { ok: false; reason: 'not-found' }
  | { ok: false; reason: 'forbidden' }
  | { ok: false; reason: 'too-large' }
  | { ok: false; reason: 'short-piece' } // a non-final piece that isn't exactly PIECE_BYTES
  | { ok: false; reason: 'bytes-past-total' }
  | { ok: false; reason: 'expired' } // UPLOAD_LIFETIME_MS since START, still uncommitted
  | { ok: false; reason: 'out-of-order'; expected: number }

export type CommitResult =
  | {
      ok: true
      description: string
      appVersion: string
      platform: string
      sizeBytes: number
      followHash: string | null
      parentId: string | null
    }
  | { ok: false; reason: 'not-found' }
  | { ok: false; reason: 'forbidden' }
  | { ok: false; reason: 'incomplete' }

/** Result of `abandon` — see its doc comment for why a report id alone must
 *  never be enough to trigger this. */
export type AbandonResult = { ok: true } | { ok: false; reason: 'not-found' } | { ok: false; reason: 'forbidden' }

export interface ReportHead {
  reportId: string
  totalBytes: number
  pieceCount: number
  receivedAt: number
}

/** When the abandonment alarm should fire for an uncommitted upload that
 *  started at `receivedAt` and just saw activity at `now`: the usual
 *  silence window, but never past the upload's absolute lifetime — so a
 *  caller trickling pieces cannot push the alarm out forever. */
function abandonAlarmAt(receivedAt: number, now: number): number {
  return Math.min(now + ABANDON_TIMEOUT_MS, receivedAt + UPLOAD_LIFETIME_MS)
}

export class ReportStore {
  private readonly storage: DurableObjectStorage
  /** The size every piece but the last must have (and no piece may exceed)
   *  — `PIECE_BYTES` in production; a small number in unit tests so the
   *  protocol can be exercised with ten-byte pieces. */
  private readonly pieceBytes: number
  /** Whether `ensureColumns` has already run against the current schema. */
  private columnsChecked = false

  constructor(storage: DurableObjectStorage, pieceBytes: number = PIECE_BYTES) {
    this.storage = storage
    this.pieceBytes = pieceBytes
  }

  /** Whether this Durable Object's database holds the schema at all —
   *  i.e. `startUpload` has run on it and no `destroy()` has wiped it since
   *  (`deleteAll()` drops the schema, not just rows). A `sqlite_master`
   *  read, so a never-started DO answering "no" leaves no storage behind
   *  (see the class doc). Every read path checks this INSTEAD of creating
   *  tables it would then have to treat as empty anyway. */
  private hasSchema(): boolean {
    const present =
      this.storage.sql
        .exec<{ n: number }>("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table' AND name = 'meta'")
        .toArray()[0].n > 0
    if (present) this.ensureColumns()
    return present
  }

  /** Adds the follow columns (`followHash`, `parentId`) to a `meta` table
   *  created before they existed — a report stored by the previous deploy
   *  is read for up to `RETENTION_DAYS` after this one. Runs only on a DO
   *  that already HAS a schema (never creates one), once per instance. */
  private ensureColumns(): void {
    if (this.columnsChecked) return
    const names = new Set(
      this.storage.sql.exec<{ name: string }>('PRAGMA table_info(meta)').toArray().map((r) => r.name),
    )
    if (!names.has('followHash')) this.storage.sql.exec('ALTER TABLE meta ADD COLUMN followHash TEXT')
    if (!names.has('parentId')) this.storage.sql.exec('ALTER TABLE meta ADD COLUMN parentId TEXT')
    this.columnsChecked = true
  }

  /** `CREATE TABLE IF NOT EXISTS` — idempotent, run by `startUpload` only
   *  (the one method that legitimately brings a report into existence).
   *  `destroy()`'s `deleteAll()` wipes the SQLite schema, not just rows (see
   *  share-relay's `dropStore.ts` for the full argument), so a DO instance
   *  reused after a delete has no tables until a fresh `startUpload` runs
   *  this again — and until then `hasSchema()` is what every other method
   *  consults. */
  private migrate(): void {
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS meta (
         id INTEGER PRIMARY KEY CHECK (id = 0),
         reportId TEXT NOT NULL,
         receivedAt INTEGER NOT NULL,
         declaredTotal INTEGER NOT NULL,
         tokenHash TEXT NOT NULL,
         nextIndex INTEGER NOT NULL,
         receivedBytes INTEGER NOT NULL,
         lastActivity INTEGER NOT NULL,
         committed INTEGER NOT NULL DEFAULT 0,
         description TEXT NOT NULL,
         appVersion TEXT NOT NULL,
         platform TEXT NOT NULL,
         followHash TEXT,
         parentId TEXT
       )`,
    )
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS piece (
         idx INTEGER PRIMARY KEY,
         data BLOB NOT NULL,
         length INTEGER NOT NULL
       )`,
    )
  }

  private readMeta(): MetaRow | null {
    const rows = this.storage.sql
      .exec<MetaRow>(
        `SELECT reportId, receivedAt, declaredTotal, tokenHash, nextIndex, receivedBytes, lastActivity, committed,
                description, appVersion, platform, followHash, parentId
         FROM meta`,
      )
      .toArray()
    return rows.length === 0 ? null : rows[0]
  }

  private insertPiece(idx: number, data: Uint8Array): void {
    const buffer = data.slice().buffer
    this.storage.sql.exec('INSERT INTO piece (idx, data, length) VALUES (?, ?, ?)', idx, buffer, data.byteLength)
  }

  /** Begins an upload: writes `meta` (declaring the total up front, the
   *  hashed token, and the head-validated fields `commit` will need later),
   *  stores piece 0, and arms the abandonment alarm. Throws if this DO is
   *  already populated — each report gets a freshly generated id, so this
   *  only fires on a genuine caller bug, never a normal-path race. */
  async startUpload(
    reportId: string,
    declaredTotal: number,
    tokenHash: string,
    firstPiece: Uint8Array,
    fields: StartFields,
  ): Promise<void> {
    this.migrate()
    this.ensureColumns()
    if (this.readMeta() !== null) {
      throw new Error('ReportStore already populated')
    }
    const receivedAt = Date.now()
    this.storage.sql.exec(
      `INSERT INTO meta (id, reportId, receivedAt, declaredTotal, tokenHash, nextIndex, receivedBytes, lastActivity,
                          committed, description, appVersion, platform, followHash, parentId)
       VALUES (0, ?, ?, ?, ?, 1, ?, ?, 0, ?, ?, ?, ?, ?)`,
      reportId,
      receivedAt,
      declaredTotal,
      tokenHash,
      firstPiece.byteLength,
      receivedAt,
      fields.description,
      fields.appVersion,
      fields.platform,
      fields.followHash ?? null,
      fields.parentId ?? null,
    )
    this.insertPiece(0, firstPiece)
    await this.storage.setAlarm(abandonAlarmAt(receivedAt, receivedAt))
  }

  /** Accepts the next piece of an in-progress upload. See `PutPieceResult`
   *  for every outcome; `not-found` covers "never started", "already
   *  committed", and "abandoned" (an abandoned upload's `meta` row is gone
   *  entirely) alike — the caller can't (and doesn't need to) tell those
   *  apart.
   *
   *  Two rules bound how long an upload can stay open, because an
   *  uncommitted upload holds its whole declared size against the storage
   *  ceiling (`indexStore.ts`) and the abandonment alarm measures only
   *  silence: every piece except the last must be EXACTLY `PIECE_BYTES`
   *  (`short-piece` otherwise — both clients split at that size, so a
   *  smaller or empty middle piece is never legitimate, and it is what
   *  would let a caller re-arm the alarm forever with free keepalives),
   *  which caps the piece count at `ceil(declaredTotal / PIECE_BYTES)`; and
   *  `UPLOAD_LIFETIME_MS` after START the upload is `expired` no matter how
   *  recently a piece arrived. Both refusals reach the caller only after
   *  the token check, so the caller may abandon the upload on them. */
  async putPiece(index: number, tokenHash: string, data: Uint8Array): Promise<PutPieceResult> {
    if (!this.hasSchema()) return { ok: false, reason: 'not-found' }
    const meta = this.readMeta()
    if (meta === null || meta.committed !== 0) {
      return { ok: false, reason: 'not-found' }
    }
    if (!constantTimeEqual(tokenHash, meta.tokenHash)) {
      return { ok: false, reason: 'forbidden' }
    }
    if (data.byteLength > PIECE_BYTES) {
      return { ok: false, reason: 'too-large' }
    }
    const now = Date.now()
    if (now - meta.receivedAt > UPLOAD_LIFETIME_MS) {
      return { ok: false, reason: 'expired' }
    }

    if (index === meta.nextIndex) {
      const after = meta.receivedBytes + data.byteLength
      if (after > meta.declaredTotal) {
        return { ok: false, reason: 'bytes-past-total' }
      }
      if (after < meta.declaredTotal && data.byteLength !== this.pieceBytes) {
        return { ok: false, reason: 'short-piece' }
      }
      this.insertPiece(index, data)
      this.storage.sql.exec(
        'UPDATE meta SET nextIndex = ?, receivedBytes = ?, lastActivity = ? WHERE id = 0',
        meta.nextIndex + 1,
        after,
        now,
      )
      await this.storage.setAlarm(abandonAlarmAt(meta.receivedAt, now))
      return { ok: true, stored: true }
    }

    if (index === meta.nextIndex - 1) {
      // A retry of the last stored index — accepted without storing twice
      // ONLY if the length matches exactly (`length`, not the BLOB itself:
      // cheap to check, and a mismatched length already proves it isn't a
      // faithful retry).
      const row = this.storage.sql.exec<{ length: number }>('SELECT length FROM piece WHERE idx = ?', index).toArray()[0]
      if (row !== undefined && row.length === data.byteLength) {
        return { ok: true, stored: false }
      }
    }

    return { ok: false, reason: 'out-of-order', expected: meta.nextIndex }
  }

  /** Finishes an upload once every declared byte has arrived. Re-arms the
   *  alarm for RETENTION (from the upload's original `receivedAt`) rather
   *  than cancelling it outright — a committed report still needs its
   *  eventual retention wipe, just on a much longer clock.
   *
   *  Idempotent for the token holder: a `commit` against a drop that is
   *  ALREADY committed returns the same `ok: true` fields again (without
   *  re-arming the alarm a second time) as long as `tokenHash` still
   *  matches, rather than `not-found` — a client that lost its `201`
   *  response (a dropped connection, a timeout) and retries the same
   *  COMMIT must land on a definite answer, not the same ambiguous
   *  "not-found" an unrelated stranger's guess at the id would get. The
   *  token check runs BEFORE the idempotency check specifically so a wrong
   *  token on an already-committed report still gets `forbidden`, never a
   *  free read of its fields. */
  async commit(tokenHash: string): Promise<CommitResult> {
    if (!this.hasSchema()) return { ok: false, reason: 'not-found' }
    const meta = this.readMeta()
    if (meta === null) {
      return { ok: false, reason: 'not-found' }
    }
    if (!constantTimeEqual(tokenHash, meta.tokenHash)) {
      return { ok: false, reason: 'forbidden' }
    }
    if (meta.committed !== 0) {
      return {
        ok: true,
        description: meta.description,
        appVersion: meta.appVersion,
        platform: meta.platform,
        sizeBytes: meta.declaredTotal,
        followHash: meta.followHash,
        parentId: meta.parentId,
      }
    }
    if (meta.receivedBytes !== meta.declaredTotal) {
      return { ok: false, reason: 'incomplete' }
    }
    this.storage.sql.exec('UPDATE meta SET committed = 1 WHERE id = 0')
    await this.storage.setAlarm(meta.receivedAt + RETENTION_MS)
    return {
      ok: true,
      description: meta.description,
      appVersion: meta.appVersion,
      platform: meta.platform,
      sizeBytes: meta.declaredTotal,
      followHash: meta.followHash,
      parentId: meta.parentId,
    }
  }

  /** Abandons an in-progress (uncommitted) upload, authorized ONLY by
   *  `tokenHash` matching the upload's own stored token — a report id is
   *  PUBLIC (a user pastes it into a GitHub issue), so the id alone must
   *  never be enough to destroy someone's report. `not-found` covers both
   *  "never started" and "already committed" (same collapse `putPiece`/
   *  `commit` make); `forbidden` is a token mismatch; otherwise this
   *  destroys the upload's storage and reports `ok: true` so the caller
   *  knows to release its `ReportIndex` reservation too. */
  async abandon(tokenHash: string): Promise<AbandonResult> {
    if (!this.hasSchema()) return { ok: false, reason: 'not-found' }
    const meta = this.readMeta()
    if (meta === null || meta.committed !== 0) {
      return { ok: false, reason: 'not-found' }
    }
    if (!constantTimeEqual(tokenHash, meta.tokenHash)) {
      return { ok: false, reason: 'forbidden' }
    }
    await this.destroy()
    return { ok: true }
  }

  /** The report's shape for admin readback — however many pieces have
   *  actually arrived so far (`nextIndex`), regardless of `committed`;
   *  callers that care whether an upload is finished check that
   *  separately. `null` if nothing was ever started (or it's been
   *  destroyed). */
  async head(): Promise<ReportHead | null> {
    if (!this.hasSchema()) return null
    const meta = this.readMeta()
    if (meta === null) return null
    return {
      reportId: meta.reportId,
      totalBytes: meta.receivedBytes,
      pieceCount: meta.nextIndex,
      receivedAt: meta.receivedAt,
    }
  }

  /** Reads piece rows `[from, from + count)` in index order, WITHOUT
   *  deleting them — a report is read as many times as the admin page
   *  wants. Returns an empty array past the end or against a never-stored
   *  report. */
  async read(from: number, count: number): Promise<Uint8Array[]> {
    if (!this.hasSchema()) return []
    const rows = this.storage.sql
      .exec<PieceRow>('SELECT idx, data FROM piece WHERE idx >= ? AND idx < ? ORDER BY idx ASC', from, from + count)
      .toArray()
    return rows.map((row) => new Uint8Array(row.data))
  }

  /** A byte `ReadableStream` over every stored piece, in index order,
   *  reading ONE piece row at a time — never all of them at once. Used by
   *  the admin download route (`handlers.ts`) so serving a report never
   *  costs the Worker isolate the ~2x-the-report memory (up to ~180 MiB)
   *  that reading every piece into an array and concatenating them would.
   *  Non-destructive, same as `read()`; the returned stream is a `type:
   *  "bytes"` `ReadableStream`, the shape Workers RPC forwards across the
   *  Durable Object boundary without buffering it whole (see
   *  https://developers.cloudflare.com/workers/runtime-apis/rpc/). */
  async readStream(): Promise<ReadableStream<Uint8Array>> {
    const present = this.hasSchema()
    const storage = this.storage
    let nextIndex = 0
    return new ReadableStream<Uint8Array>({
      type: 'bytes',
      pull(controller) {
        if (!present) {
          controller.close()
          return
        }
        const rows = storage.sql.exec<PieceRow>('SELECT idx, data FROM piece WHERE idx = ?', nextIndex).toArray()
        if (rows.length === 0) {
          controller.close()
          return
        }
        controller.enqueue(new Uint8Array(rows[0].data))
        nextIndex++
      },
    })
  }

  /** Wipes all storage and cancels the alarm — idempotent. Used by admin
   *  delete, by abandonment (immediate, on a 400, or via the alarm), and by
   *  retention expiry. */
  async destroy(): Promise<void> {
    await this.storage.deleteAll()
    await this.storage.deleteAlarm()
    this.columnsChecked = false
  }

  /** The alarm handler's logic (abandonment or retention — `ReportDrop`'s
   *  class doc): hand the report id to `onExpire` FIRST (the DO wrapper
   *  points it at `ReportIndex.bundleExpired`), then wipe. A drop with no
   *  `head()` left (already deleted) skips the callback, so a stale alarm
   *  never touches an index row. See `ReportDrop.alarm` for why this order. */
  async expire(onExpire: (reportId: string) => Promise<void>): Promise<void> {
    const head = await this.head()
    if (head !== null) await onExpire(head.reportId)
    await this.destroy()
  }
}
