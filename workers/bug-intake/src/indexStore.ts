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
  EMAIL_THREAD_RESERVE,
  STORE_CEILING_BYTES,
  UPLOAD_LIFETIME_MS,
  THREAD_RATE_WINDOW_MAX,
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
  /** Display caches for the admin list (docs/design/follow-a-report.md
   *  §5.2) — the report's `ReportThread` is the source of truth, and every
   *  handler that changes a thread updates these in the same request.
   *  `following`: a thread with a reporter's secret exists. */
  following: boolean
  status: string
  statusVersion: string | null
  /** When the reporter last wrote or attached something. */
  reporterActivityAt: number | null
  /** When the maintainer last opened the detail page. */
  adminViewedAt: number | null
  /** A follow-up attachment's parent report. */
  parentId: string | null
  /** The bundle's retention ran out while the report was followed: the row
   *  stays (metadata and thread, `sizeBytes = 0`) but there is nothing to
   *  download. */
  bundleDeleted: boolean
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
export type ReportReservation = Pick<
  ReportRow,
  'id' | 'receivedAt' | 'appVersion' | 'platform' | 'descriptionPreview' | 'sizeBytes'
> & { parentId?: string | null }

/** The display-cache fields a thread change writes into a row. */
export interface ThreadCachePatch {
  status?: string
  statusVersion?: string | null
  reporterActivityAt?: number
}

export type ReserveResult =
  | { ok: true }
  | { ok: false; reason: 'rate-limited'; retryAfterMs: number }
  | { ok: false; reason: 'full' }

/** See `commitReport`'s doc comment. */
export type CommitReportOutcome = 'flipped' | 'already-committed' | 'not-found'

function utcDay(now: number): string {
  return new Date(now).toISOString().slice(0, 10)
}

interface RawReportRow {
  id: string
  receivedAt: number
  appVersion: string
  platform: string
  descriptionPreview: string
  sizeBytes: number
  triaged: number
  committed: number
  following: number
  status: string | null
  statusVersion: string | null
  reporterActivityAt: number | null
  adminViewedAt: number | null
  parentId: string | null
  bundleDeleted: number
}

function rowToReportRow(r: RawReportRow): ReportRow {
  return {
    ...r,
    triaged: r.triaged !== 0,
    committed: r.committed !== 0,
    following: r.following !== 0,
    status: r.status ?? 'open',
    bundleDeleted: r.bundleDeleted !== 0,
  }
}

const REPORT_COLUMNS =
  'id, receivedAt, appVersion, platform, descriptionPreview, sizeBytes, triaged, committed, ' +
  'following, status, statusVersion, reporterActivityAt, adminViewedAt, parentId, bundleDeleted'

/** Columns added after the first deploy, with the definition each gets on
 *  a `reports` table that predates it (docs/design/follow-a-report.md
 *  §5.2). Also part of `CREATE TABLE` for a fresh database. */
const ADDED_REPORT_COLUMNS: ReadonlyArray<[string, string]> = [
  ['following', 'INTEGER NOT NULL DEFAULT 0'],
  ['status', "TEXT NOT NULL DEFAULT 'open'"],
  ['statusVersion', 'TEXT'],
  ['reporterActivityAt', 'INTEGER'],
  ['adminViewedAt', 'INTEGER'],
  ['parentId', 'TEXT'],
  ['bundleDeleted', 'INTEGER NOT NULL DEFAULT 0'],
]

export class IndexStore {
  private readonly storage: DurableObjectStorage
  private columnsChecked = false

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
         ${ADDED_REPORT_COLUMNS.map(([name, def]) => `${name} ${def}`).join(',\n         ')}
       )`,
    )
    // A `reports` table from the first deploy lacks the follow columns: add
    // each one missing, guarded by `PRAGMA table_info`, once per instance.
    if (!this.columnsChecked) {
      const present = new Set(
        this.storage.sql.exec<{ name: string }>('PRAGMA table_info(reports)').toArray().map((r) => r.name),
      )
      for (const [name, def] of ADDED_REPORT_COLUMNS) {
        if (!present.has(name)) this.storage.sql.exec(`ALTER TABLE reports ADD COLUMN ${name} ${def}`)
      }
      this.columnsChecked = true
    }
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
      `INSERT INTO reports (id, receivedAt, appVersion, platform, descriptionPreview, sizeBytes, triaged, committed, lastActivity, parentId)
       VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, ?)`,
      entry.id,
      entry.receivedAt,
      entry.appVersion,
      entry.platform,
      entry.descriptionPreview,
      entry.sizeBytes,
      now,
      entry.parentId ?? null,
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
  async commitReport(id: string, following: boolean = false): Promise<CommitReportOutcome> {
    this.migrate()
    const row = this.storage.sql.exec<{ committed: number }>('SELECT committed FROM reports WHERE id = ?', id).toArray()[0]
    if (row === undefined) return 'not-found'
    // `following` is set on a retry too: COMMIT creates the thread before
    // calling this, so a retry after this call failed must still mark it.
    if (following) this.storage.sql.exec('UPDATE reports SET following = 1 WHERE id = ?', id)
    if (row.committed !== 0) return 'already-committed'
    this.storage.sql.exec('UPDATE reports SET committed = 1 WHERE id = ?', id)
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
      .exec<RawReportRow>(`SELECT ${REPORT_COLUMNS} FROM reports WHERE committed = 1 ORDER BY receivedAt DESC`)
      .toArray()
      .map(rowToReportRow)
  }

  /** Returns a row regardless of commit state — callers that care whether
   *  a report is actually finished (the admin detail/download routes) check
   *  `.committed` themselves. */
  async getReport(id: string): Promise<ReportRow | null> {
    this.migrate()
    const rows = this.storage.sql
      .exec<RawReportRow>(`SELECT ${REPORT_COLUMNS} FROM reports WHERE id = ?`, id)
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
  // Following (docs/design/follow-a-report.md §5.2)
  // -------------------------------------------------------------------

  /** A `ReportDrop`'s alarm fired (abandonment or retention). A committed
   *  row someone follows keeps its metadata and thread: `bundleDeleted =
   *  1`, `sizeBytes = 0` (the ceiling frees). Every other row — not
   *  followed, or never committed — is deleted, as `deleteReport` did
   *  before. Idempotent. Returns whether the row is gone afterwards (also
   *  `true` when it already was), which tells the caller (`expireReport`
   *  in `handlers.ts`) to destroy any admin-only thread for `id` too. */
  async bundleExpired(id: string): Promise<boolean> {
    this.migrate()
    this.storage.sql.exec(
      'UPDATE reports SET bundleDeleted = 1, sizeBytes = 0 WHERE id = ? AND committed = 1 AND following = 1',
      id,
    )
    this.storage.sql.exec('DELETE FROM reports WHERE id = ? AND NOT (committed = 1 AND following = 1)', id)
    return this.storage.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM reports WHERE id = ?', id).toArray()[0].n === 0
  }

  /** A `ReportThread` ended (its alarm, or Forget): a row whose bundle is
   *  already gone is deleted; a live one stops showing as followed and its
   *  cached status goes back to the default (`open`, no version) — the
   *  thread that status belonged to is gone, and a later admin action
   *  starts a fresh one at `open`. Idempotent; a no-op for a row that no
   *  longer exists. */
  async threadEnded(id: string): Promise<void> {
    this.migrate()
    this.storage.sql.exec('DELETE FROM reports WHERE id = ? AND bundleDeleted = 1', id)
    this.storage.sql.exec("UPDATE reports SET following = 0, status = 'open', statusVersion = NULL WHERE id = ?", id)
  }

  /** Copies a thread change into the row's display caches. */
  async setThreadCache(id: string, patch: ThreadCachePatch): Promise<void> {
    this.migrate()
    if (patch.status !== undefined) {
      this.storage.sql.exec(
        'UPDATE reports SET status = ?, statusVersion = ? WHERE id = ?',
        patch.status,
        patch.statusVersion ?? null,
        id,
      )
    }
    if (patch.reporterActivityAt !== undefined) {
      this.storage.sql.exec('UPDATE reports SET reporterActivityAt = ? WHERE id = ?', patch.reporterActivityAt, id)
    }
  }

  /** The maintainer opened the detail page (clears the list's "new"). */
  async markAdminViewed(id: string, now: number): Promise<void> {
    this.migrate()
    this.storage.sql.exec('UPDATE reports SET adminViewedAt = ? WHERE id = ?', now, id)
  }

  /** A report's committed follow-up attachments, oldest first. */
  async listChildren(parentId: string): Promise<ReportRow[]> {
    this.migrate()
    return this.storage.sql
      .exec<RawReportRow>(`SELECT ${REPORT_COLUMNS} FROM reports WHERE parentId = ? AND committed = 1 ORDER BY receivedAt ASC`, parentId)
      .toArray()
      .map(rowToReportRow)
  }

  /** The thread-route quota guard (§5.4): `THREAD_RATE_WINDOW_MAX` requests
   *  per client hash per `RATE_LIMIT_WINDOW_MS`, counted in `rate_window`
   *  under a `thread:` prefix so it never mixes with the submission limit.
   *  Called only after the thread DO accepted the secret (design D10). A
   *  request over the limit is refused WITHOUT incrementing, so a client
   *  hammering past it costs a read, not a write. */
  async countThreadRequest(clientHash: string, now: number): Promise<{ ok: true } | { ok: false; retryAfterMs: number }> {
    this.migrate()
    const key = `thread:${clientHash}`
    const bucket = Math.floor(now / RATE_LIMIT_WINDOW_MS)
    this.storage.sql.exec('DELETE FROM rate_window WHERE bucket < ?', bucket - 2)
    const count =
      this.storage.sql
        .exec<{ count: number }>('SELECT count FROM rate_window WHERE clientHash = ? AND bucket = ?', key, bucket)
        .toArray()[0]?.count ?? 0
    if (count >= THREAD_RATE_WINDOW_MAX) {
      return { ok: false, retryAfterMs: (bucket + 1) * RATE_LIMIT_WINDOW_MS - now }
    }
    this.storage.sql.exec(
      `INSERT INTO rate_window (clientHash, bucket, count) VALUES (?, ?, 1)
       ON CONFLICT (clientHash, bucket) DO UPDATE SET count = count + 1`,
      key,
      bucket,
    )
    return { ok: true }
  }

  // -------------------------------------------------------------------
  // Email cap
  // -------------------------------------------------------------------

  /** Reserves one of today's email slots, returning whether one was
   *  available. Like `reserveReport`'s rate-limit half, check-and-increment
   *  in one call so two concurrent sends can't both observe `count < max`.
   *  A `'thread'` email (a reporter reply or follow-up attachment) stops
   *  at `EMAIL_DAY_MAX - EMAIL_THREAD_RESERVE`, so thread traffic can never
   *  use up the slots a new report's email needs; a `'report'` email may
   *  use the whole cap. */
  async reserveEmail(now: number, purpose: 'report' | 'thread' = 'report'): Promise<boolean> {
    this.migrate()
    const day = utcDay(now)
    this.storage.sql.exec('DELETE FROM email_day WHERE day <> ?', day)
    const count =
      this.storage.sql.exec<{ count: number }>('SELECT count FROM email_day WHERE day = ?', day).toArray()[0]?.count ?? 0
    const max = purpose === 'thread' ? EMAIL_DAY_MAX - EMAIL_THREAD_RESERVE : EMAIL_DAY_MAX
    if (count >= max) return false
    this.storage.sql.exec(
      `INSERT INTO email_day (day, count) VALUES (?, 1)
       ON CONFLICT (day) DO UPDATE SET count = count + 1`,
      day,
    )
    return true
  }
}
