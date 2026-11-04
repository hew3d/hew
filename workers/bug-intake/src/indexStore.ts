/**
 * IndexStore — the SQLite logic behind the single `ReportIndex` Durable
 * Object (`reportIndex.ts`), factored out the same way share-relay splits
 * `ShareDrop`/`DropStore`: framework-free, unit-testable under bare
 * `node --test`, with the DO class itself a thin RPC wrapper.
 *
 * Four tables, all tiny (row writes are the free-tier's least generous
 * quota — 100k/day account-wide — so this design opts for a handful of
 * upserted counter rows over one row per event):
 *
 *   - `reports`: one row per report, backing the admin list and the storage
 *     ceiling (`SUM(sizeBytes)`, not a separate counter — deleting a row is
 *     the entire "give the space back" step). A row starts life `committed
 *     = 0` the instant `reserveReport` accepts it — BEFORE a single byte of
 *     the upload has reached its `ReportDrop` — specifically so its size
 *     counts against the ceiling immediately (see "Storage ceiling"
 *     below); `commitReport` flips it to 1 once the upload actually
 *     finishes. `listReports`/the admin-visible half of `getReport` only
 *     ever surface committed rows. `lastActivity` tracks the most recent
 *     piece/start for an uncommitted row — `touchActivity` updates it, and
 *     `findAndDeleteIdleUncommittedIds` is the idle-upload backstop that
 *     reads it (docs/design/report-bug.md §4 "Chunked upload").
 *   - `rate_window`: one row per (clientHash, 10-minute bucket), enforcing
 *     the 5-per-10-minutes limit.
 *   - `rate_day`: one row per clientHash, enforcing the 20-per-day limit.
 *     `clientHash` already changes every UTC day (`clientHash.ts` salts with
 *     the date), so "one row per client per day" falls out for free — the
 *     `day` column exists only so stale rows can be pruned.
 *   - `email_day`: a single counter row per UTC day, enforcing the
 *     account-wide 50-notifications-a-day cap independent of `rate_day`
 *     (many distinct clients each submitting once should not multiply into
 *     many emails).
 *
 * Both rate tables prune expired rows opportunistically on write rather
 * than on a schedule — cheap because both tables stay small, and it avoids
 * giving this DO its own alarm (which would collide with a semantic reading
 * of "the alarm" as retention/abandonment, `ReportDrop`'s job).
 */

import type { DurableObjectStorage } from './types.ts'
import {
  RATE_LIMIT_DAY_MAX,
  RATE_LIMIT_WINDOW_MAX,
  RATE_LIMIT_WINDOW_MS,
  EMAIL_DAY_MAX,
  STORE_CEILING_BYTES,
  UPLOAD_LIFETIME_MS,
  CLIENT_PENDING_BYTES,
  IDLE_PRUNE_TIMEOUT_MS,
} from './constants.ts'

export interface ReportRow {
  id: string
  receivedAt: number
  appVersion: string
  platform: string
  descriptionPreview: string
  sizeBytes: number
  triaged: boolean
  committed: boolean
}

/** What `reserveReport` is called with — everything about the report
 *  `indexStore.ts` needs to know at reservation time. Unlike this Worker's
 *  earlier single-request-streaming design, the chunked-upload protocol
 *  validates the head (and so knows the real `appVersion`/`platform`/
 *  `descriptionPreview`) BEFORE ever calling `reserveReport` — the reserve
 *  happens after the first piece's head has already validated
 *  (docs/design/report-bug.md §4) — so these are the real values from the
 *  start, not placeholders `commitReport` has to fill in later.
 *  `triaged`/`committed` aren't inputs: every reservation starts life
 *  untriaged and uncommitted. */
export type ReportReservation = Omit<ReportRow, 'triaged' | 'committed'>

export type ReserveResult =
  | { ok: true }
  | { ok: false; reason: 'rate-limited'; retryAfterMs: number }
  | { ok: false; reason: 'full' }

/** See `commitReport`'s doc comment. */
export type CommitReportOutcome = 'flipped' | 'already-committed' | 'not-found'

function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10)
}

function rowToReportRow(r: {
  id: string
  receivedAt: number
  appVersion: string
  platform: string
  descriptionPreview: string
  sizeBytes: number
  triaged: number
  committed: number
}): ReportRow {
  return { ...r, triaged: r.triaged !== 0, committed: r.committed !== 0 }
}

const REPORT_COLUMNS = 'id, receivedAt, appVersion, platform, descriptionPreview, sizeBytes, triaged, committed'

export class IndexStore {
  private readonly storage: DurableObjectStorage

  constructor(storage: DurableObjectStorage) {
    this.storage = storage
    this.migrate()
  }

  // Every public method below is declared `async` even though the SQLite
  // calls inside are synchronous — purely so this class's methods return
  // `Promise<T>` and structurally satisfy `ReportIndexStub` (`types.ts`),
  // which lets `testSupport/fakeDurableObject.ts`'s `FakeDurableObjectNamespace`
  // wrap this class directly, the same way share-relay's tests wrap
  // `DropStore` as a `ShareDropStub` stand-in. Critically, none of them
  // ever `await` anything mid-body (every `sql.exec` call is genuinely
  // synchronous) — `reserveReport` in particular depends on running to
  // completion as one uninterrupted synchronous burst before the runtime
  // could possibly deliver a second, concurrent call to this same DO
  // instance; see its doc comment.

  private migrate(): void {
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS reports (
         id TEXT PRIMARY KEY,
         receivedAt INTEGER NOT NULL,
         appVersion TEXT NOT NULL,
         platform TEXT NOT NULL,
         descriptionPreview TEXT NOT NULL,
         sizeBytes INTEGER NOT NULL,
         triaged INTEGER NOT NULL DEFAULT 0,
         committed INTEGER NOT NULL DEFAULT 0,
         lastActivity INTEGER NOT NULL DEFAULT 0,
         clientHash TEXT NOT NULL DEFAULT ''
       )`,
    )
    // Older deployments did not retain an upload's owner. Their existing
    // rows remain subject to the global ceiling and normal abandonment
    // cleanup; ownership cannot be inferred retroactively. Keep the hash
    // only while uploading, clearing it on commit rather than linking an
    // IP-derived identifier to a retained report.
    const columns = this.storage.sql.exec<{ name: string }>('PRAGMA table_info(reports)').toArray()
    if (!columns.some((column) => column.name === 'clientHash')) {
      this.storage.sql.exec("ALTER TABLE reports ADD COLUMN clientHash TEXT NOT NULL DEFAULT ''")
    }
    this.storage.sql.exec('CREATE INDEX IF NOT EXISTS reports_upload_client ON reports (clientHash, committed)')
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS rate_window (
         clientHash TEXT NOT NULL,
         bucket INTEGER NOT NULL,
         count INTEGER NOT NULL,
         PRIMARY KEY (clientHash, bucket)
       )`,
    )
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS rate_day (
         clientHash TEXT PRIMARY KEY,
         day TEXT NOT NULL,
         count INTEGER NOT NULL
       )`,
    )
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS email_day (
         day TEXT PRIMARY KEY,
         count INTEGER NOT NULL
       )`,
    )
  }

  // -------------------------------------------------------------------
  // Submission: rate limit + storage ceiling + reservation, fused
  // -------------------------------------------------------------------

  /** Checks the per-client rate limit AND the storage ceiling, and — only
   *  if both pass — immediately inserts an uncommitted `reports` row for
   *  `entry`, all inside this one call. Fusing all three into a single
   *  synchronous DO call is what closes the TOCTOU race a separate
   *  check-then-later-record pair would have: `entry.sizeBytes` (the
   *  declared TOTAL upload length) counts against `totalStoredBytes()` (and
   *  thus against every OTHER concurrent caller's ceiling check) from the
   *  instant this returns `{ok: true}`, not from whenever the caller
   *  eventually finishes the (potentially 50-request) upload — the same
   *  "single DO, single-threaded, one call" reasoning `reserveEmail` and
   *  the rate-limit counters below already rely on. The caller
   *  (`handlers.ts`) MUST follow an `{ok: true}` with either
   *  `commitReport(entry.id)` (the upload finished) or `deleteReport(entry.id)`
   *  (it was abandoned) — never leave a reservation hanging on purpose;
   *  `findAndDeleteIdleUncommittedIds` (below) is the backstop for when a
   *  caller can't. */
  async reserveReport(clientHash: string, now: number, entry: ReportReservation): Promise<ReserveResult> {
    this.migrate()

    const bucket = Math.floor(now / RATE_LIMIT_WINDOW_MS)
    const day = utcDay(now)

    // Prune opportunistically: window buckets more than two windows old,
    // day rows from a previous day. Cheap — both tables hold at most a
    // handful of live rows per active client.
    this.storage.sql.exec('DELETE FROM rate_window WHERE bucket < ?', bucket - 2)
    this.storage.sql.exec('DELETE FROM rate_day WHERE day <> ?', day)

    const windowCount =
      this.storage.sql
        .exec<{ count: number }>('SELECT count FROM rate_window WHERE clientHash = ? AND bucket = ?', clientHash, bucket)
        .toArray()[0]?.count ?? 0
    const dayCount =
      this.storage.sql.exec<{ count: number }>('SELECT count FROM rate_day WHERE clientHash = ?', clientHash).toArray()[0]
        ?.count ?? 0

    if (windowCount >= RATE_LIMIT_WINDOW_MAX) {
      const windowEnd = (bucket + 1) * RATE_LIMIT_WINDOW_MS
      return { ok: false, reason: 'rate-limited', retryAfterMs: windowEnd - now }
    }
    if (dayCount >= RATE_LIMIT_DAY_MAX) {
      const dayEnd = Date.parse(`${day}T00:00:00.000Z`) + 24 * 60 * 60 * 1000
      return { ok: false, reason: 'rate-limited', retryAfterMs: dayEnd - now }
    }

    // The ceiling check and the reservation insert below happen in the same
    // synchronous burst as the rate-limit checks above and their counter
    // increments below — nothing here awaits I/O, so no concurrent call to
    // this same DO instance can observe a state in between.
    const total =
      this.storage.sql.exec<{ total: number | null }>('SELECT SUM(sizeBytes) AS total FROM reports').toArray()[0]
        ?.total ?? 0
    if (total + entry.sizeBytes > STORE_CEILING_BYTES) {
      return { ok: false, reason: 'full' }
    }

    const pending = this.storage.sql
      .exec<{ total: number | null }>('SELECT SUM(sizeBytes) AS total FROM reports WHERE clientHash = ? AND committed = 0', clientHash)
      .toArray()[0]?.total ?? 0
    if (pending + entry.sizeBytes > CLIENT_PENDING_BYTES) {
      // The existing rate-limited response keeps this admission limit in
      // the same wire contract. Abandonment/pruning releases stalled
      // reservations; a successful commit or delete releases it sooner.
      return { ok: false, reason: 'rate-limited', retryAfterMs: IDLE_PRUNE_TIMEOUT_MS }
    }

    this.storage.sql.exec(
      `INSERT INTO rate_window (clientHash, bucket, count) VALUES (?, ?, 1)
       ON CONFLICT (clientHash, bucket) DO UPDATE SET count = count + 1`,
      clientHash,
      bucket,
    )
    this.storage.sql.exec(
      `INSERT INTO rate_day (clientHash, day, count) VALUES (?, ?, 1)
       ON CONFLICT (clientHash) DO UPDATE SET count = count + 1`,
      clientHash,
      day,
    )
    this.storage.sql.exec(
      `INSERT INTO reports (id, receivedAt, appVersion, platform, descriptionPreview, sizeBytes, triaged, committed, lastActivity, clientHash)
       VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?)`,
      entry.id,
      entry.receivedAt,
      entry.appVersion,
      entry.platform,
      entry.descriptionPreview,
      entry.sizeBytes,
      now,
      clientHash,
    )
    return { ok: true }
  }

  /** Marks a reservation's row committed once its upload has finished
   *  (`ReportDrop.commit` succeeded) — only then does it appear in
   *  `listReports()` or count as a real report to the admin/email side of
   *  things (it already counted against the storage ceiling from the
   *  moment it was reserved). Returns `'flipped'` only the first time this
   *  succeeds for a given row: `handlers.ts`'s COMMIT handler is idempotent
   *  for a retried request (`ReportDrop.commit` itself already answers `ok`
   *  again for an already-committed drop), and it must send the maintainer
   *  notification email at most once per report — `'already-committed'`
   *  tells it not to on a retry. `'not-found'` is the genuinely abnormal
   *  case (the row is gone even though the drop just committed
   *  successfully), which the caller needs to tell apart from
   *  `'already-committed'` rather than silently answering success for a
   *  report the index no longer knows about. */
  async commitReport(id: string): Promise<CommitReportOutcome> {
    this.migrate()
    const row = this.storage.sql.exec<{ committed: number }>('SELECT committed FROM reports WHERE id = ?', id).toArray()[0]
    if (row === undefined) return 'not-found'
    if (row.committed !== 0) return 'already-committed'
    this.storage.sql.exec("UPDATE reports SET committed = 1, clientHash = '' WHERE id = ?", id)
    return 'flipped'
  }

  /** Bumps an uncommitted row's `lastActivity` — called on every piece a
   *  `ReportDrop` newly stores (not on an idempotent retry), so
   *  `findAndDeleteIdleUncommittedIds` measures genuine silence, not age
   *  since the upload started. A no-op once a row is committed or gone —
   *  callers don't need to check either first. */
  async touchActivity(id: string, now: number): Promise<void> {
    this.migrate()
    this.storage.sql.exec('UPDATE reports SET lastActivity = ? WHERE id = ? AND committed = 0', now, id)
  }

  /** The idle-upload backstop (docs/design/report-bug.md §4): every
   *  uncommitted row whose `lastActivity` is older than `now - maxIdleMs`
   *  is deleted here and its id returned, so the caller (`reportIndex.ts`,
   *  which has the `env` access this framework-free class doesn't) can also
   *  destroy each one's `ReportDrop`. This is independent of any single
   *  `ReportDrop`'s own abandonment alarm — it exists for whatever that
   *  alarm somehow missed. ACTIVITY time, not age since `receivedAt`: a
   *  genuinely slow but still-progressing upload keeps touching activity
   *  the whole time and is never at risk. */
  async findAndDeleteIdleUncommittedIds(now: number, maxIdleMs: number, maxLifetimeMs: number = UPLOAD_LIFETIME_MS): Promise<string[]> {
    this.migrate()
    const idleThreshold = now - maxIdleMs
    // Idle by activity, OR simply too old: an upload that keeps sending
    // pieces past `UPLOAD_LIFETIME_MS` never goes idle, and its reservation
    // would otherwise hold the ceiling for as long as it cares to keep
    // trickling (`reportStore.ts`'s `putPiece` refuses it as `expired` and
    // its alarm is never armed past the same bound — this is the index's
    // own copy of that rule).
    const ageThreshold = now - maxLifetimeMs
    const where = 'committed = 0 AND (lastActivity < ? OR receivedAt < ?)'
    const ids = this.storage.sql
      .exec<{ id: string }>(`SELECT id FROM reports WHERE ${where}`, idleThreshold, ageThreshold)
      .toArray()
      .map((r) => r.id)
    if (ids.length > 0) {
      this.storage.sql.exec(`DELETE FROM reports WHERE ${where}`, idleThreshold, ageThreshold)
    }
    return ids
  }

  /** Read-only, for tests/diagnostics — sums `sizeBytes` across every row,
   *  reserved-but-uncommitted included (a pending reservation's bytes are
   *  "spent" against the ceiling the instant `reserveReport` accepts it). */
  async totalStoredBytes(): Promise<number> {
    this.migrate()
    return (
      this.storage.sql.exec<{ total: number | null }>('SELECT SUM(sizeBytes) AS total FROM reports').toArray()[0]
        ?.total ?? 0
    )
  }

  // -------------------------------------------------------------------
  // Reports
  // -------------------------------------------------------------------

  /** Newest first, committed reports only — a reservation still uploading
   *  (or one abandoned and not yet pruned) has no business appearing on the
   *  admin list. */
  async listReports(): Promise<ReportRow[]> {
    this.migrate()
    return this.storage.sql
      .exec<{
        id: string
        receivedAt: number
        appVersion: string
        platform: string
        descriptionPreview: string
        sizeBytes: number
        triaged: number
        committed: number
      }>(`SELECT ${REPORT_COLUMNS} FROM reports WHERE committed = 1 ORDER BY receivedAt DESC`)
      .toArray()
      .map(rowToReportRow)
  }

  /** Returns a row regardless of commit state — callers that care whether
   *  a report is actually finished (the admin detail/download routes) check
   *  `.committed` themselves. */
  async getReport(id: string): Promise<ReportRow | null> {
    this.migrate()
    const rows = this.storage.sql
      .exec<{
        id: string
        receivedAt: number
        appVersion: string
        platform: string
        descriptionPreview: string
        sizeBytes: number
        triaged: number
        committed: number
      }>(`SELECT ${REPORT_COLUMNS} FROM reports WHERE id = ?`, id)
      .toArray()
    if (rows.length === 0) return null
    return rowToReportRow(rows[0])
  }

  /** Deletes a row outright — the admin page's delete action, AND the
   *  upload protocol's way of releasing a reservation whose upload was
   *  abandoned (giving its bytes back to the ceiling immediately rather
   *  than waiting for the idle prune). Idempotent. */
  async deleteReport(id: string): Promise<void> {
    this.migrate()
    this.storage.sql.exec('DELETE FROM reports WHERE id = ?', id)
  }

  async setTriaged(id: string, triaged: boolean): Promise<void> {
    this.migrate()
    this.storage.sql.exec('UPDATE reports SET triaged = ? WHERE id = ?', triaged ? 1 : 0, id)
  }

  // -------------------------------------------------------------------
  // Email cap
  // -------------------------------------------------------------------

  /** Reserves one of today's email slots, returning whether one was
   *  available. Like `reserveReport`'s rate-limit half, check-and-increment
   *  in one call so two concurrent sends can't both observe `count < max`. */
  async reserveEmail(now: number): Promise<boolean> {
    this.migrate()
    const day = utcDay(now)
    this.storage.sql.exec('DELETE FROM email_day WHERE day <> ?', day)
    const count =
      this.storage.sql.exec<{ count: number }>('SELECT count FROM email_day WHERE day = ?', day).toArray()[0]?.count ?? 0
    if (count >= EMAIL_DAY_MAX) return false
    this.storage.sql.exec(
      `INSERT INTO email_day (day, count) VALUES (?, 1)
       ON CONFLICT (day) DO UPDATE SET count = count + 1`,
      day,
    )
    return true
  }
}
