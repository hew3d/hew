/**
 * ReportDrop — the Durable Object that stores one report's chunked upload
 * and, once committed, its finished bytes — one instance per report id
 * (`env.REPORT_DROP.idFromName(id)` in `handlers.ts`), backed by that
 * instance's private SQLite database. Mirrors share-relay's `ShareDrop`
 * wrapper: the actual logic lives in `ReportStore` (`reportStore.ts`,
 * framework-free and unit-tested); this file is the thin runtime wrapper
 * that MUST `extend DurableObject` for its methods to be exposed as RPC at
 * all (a plain class's methods throw Cloudflare error 1101 when called
 * through a stub — see `ShareDrop`'s class doc for the full story of how
 * that bug slips past a mocked unit suite).
 *
 * The alarm serves two different purposes depending on when it fires,
 * because `ReportStore` re-arms it for a different reason at different
 * points (see its doc): ABANDONMENT (`ABANDON_TIMEOUT_MS` after the last
 * piece or the start, while still uncommitted) or RETENTION
 * (`RETENTION_MS` after the upload started, once `commit` has run). Either
 * way, the alarm handler's own job is identical — tell `ReportIndex` to
 * drop the corresponding row, THEN wipe the bytes (see `alarm()`'s own doc
 * for why that order, not the reverse, matters) — which is also how
 * `ReportIndex` computes the storage ceiling (`SUM(sizeBytes)`, not a
 * separate counter — removing the row is the entire "give the space back"
 * step) — so `alarm()` below needs no branch on which case it is.
 */

import { DurableObject } from 'cloudflare:workers'

import { ReportStore, type ReportHead, type StartFields, type PutPieceResult, type CommitResult, type AbandonResult } from './reportStore.ts'
import { expireReport } from './handlers.ts'
import type { BugIntakeEnv, DurableObjectStorage } from './types.ts'

export class ReportDrop extends DurableObject<BugIntakeEnv> {
  private readonly store_: ReportStore

  constructor(ctx: DurableObjectState, env: BugIntakeEnv) {
    super(ctx, env)
    this.store_ = new ReportStore(ctx.storage as unknown as DurableObjectStorage)
  }

  /** RPC — `handlers.ts`'s START handler (`POST /report/`). */
  startUpload(
    reportId: string,
    declaredTotal: number,
    tokenHash: string,
    firstPiece: Uint8Array,
    fields: StartFields,
  ): Promise<void> {
    return this.store_.startUpload(reportId, declaredTotal, tokenHash, firstPiece, fields)
  }

  /** RPC — `handlers.ts`'s PIECE handler (`PUT /report/<id>/<index>`). */
  putPiece(index: number, tokenHash: string, data: Uint8Array): Promise<PutPieceResult> {
    return this.store_.putPiece(index, tokenHash, data)
  }

  /** RPC — `handlers.ts`'s COMMIT handler (`POST /report/<id>/commit`). */
  commit(tokenHash: string): Promise<CommitResult> {
    return this.store_.commit(tokenHash)
  }

  /** RPC — `handlers.ts`'s PIECE handler abandons an oversized piece
   *  through this, ONLY once the presented token has been checked against
   *  it: a report id is public, so it must never by itself be enough to
   *  destroy someone else's report. */
  abandon(tokenHash: string): Promise<AbandonResult> {
    return this.store_.abandon(tokenHash)
  }

  /** RPC — the admin detail/download path checks this before reading. */
  head(): Promise<ReportHead | null> {
    return this.store_.head()
  }

  /** RPC — one batch of a report's pieces (admin detail render only reads
   *  piece 0 now; kept for whatever else wants a bounded batch read). */
  read(from: number, count: number): Promise<Uint8Array[]> {
    return this.store_.read(from, count)
  }

  /** RPC — the admin download route streams a report's bytes through this
   *  instead of reading every piece into memory first. */
  readStream(): Promise<ReadableStream<Uint8Array>> {
    return this.store_.readStream()
  }

  /** RPC — admin delete, and immediate abandonment (a 400 on a piece or
   *  commit). Also used by `alarm()` below. */
  async destroy(): Promise<void> {
    await this.store_.destroy()
  }

  /** DO alarm handler — fires either as an abandonment (no piece or commit
   *  for `ABANDON_TIMEOUT_MS`) or, for an already-committed report, as its
   *  retention expiry (`RETENTION_MS` after the upload started) —
   *  `ReportStore`'s doc on `commit`/`putPiece` explains which re-arms it
   *  when. Either way: tell `ReportIndex` which row to drop FIRST, then
   *  wipe the bytes — not the reverse. If `ReportIndex.deleteReport` throws
   *  (a transient RPC failure) with the destroy done first, the retried
   *  alarm finds `head() === null` and skips the `ReportIndex` call
   *  entirely, orphaning that row forever — it would keep counting against
   *  the storage ceiling with no `ReportDrop` left to ever free it. Doing
   *  the index delete first means a retry after ITS failure still finds the
   *  same `head()` and tries again; `deleteReport` is idempotent, so a
   *  retry after the index call actually succeeded (but this alarm
   *  invocation died before reaching `destroy()`) is harmless too. Reads
   *  the report id out of its own storage first (this DO's identity is an
   *  opaque hash of the id, not the id itself); a report already deleted
   *  through the admin page (or a prior abandonment) has no `head()` left,
   *  so this is a harmless no-op for it — a stale alarm fire racing an
   *  explicit delete's own `destroy()` (the runtime does not guarantee
   *  `deleteAlarm` beats an already-scheduled invocation) should never
   *  resurrect an index row. */
  async alarm(): Promise<void> {
    // `bundleExpired`, not `deleteReport`: a committed report someone still
    // follows keeps its index row with `bundleDeleted = 1` and `sizeBytes =
    // 0` so the maintainer can still answer it
    // (docs/design/follow-a-report.md §5.2); every other row is deleted,
    // exactly as before — and a deleted row takes any admin-only thread
    // with it (`expireReport`).
    await this.store_.expire((reportId) => expireReport(this.env, reportId))
  }
}
