/**
 * ThreadStore — the SQLite logic behind one report's `ReportThread` Durable
 * Object (`reportThread.ts`): the two-way channel between a reporter who
 * holds the report's follow secret and the maintainer
 * (docs/design/follow-a-report.md §5.1). Framework-free, so it unit-tests
 * under bare `node --test` on the fake DO namespace, the same split as
 * `ReportStore`/`ReportDrop` and `IndexStore`/`ReportIndex`.
 *
 * Storage shape: one `meta` row (pinned `id = 0`) and a `messages` table
 * whose `seq` is the single ordered timeline — maintainer text, reporter
 * text, follow-up attachments, and status changes alike — so a client polls
 * everything with one `after=<seq>` cursor.
 *
 * Lazy schema, exactly the `reportStore.ts` rule: a thread route names a
 * report by its PUBLIC id and the runtime instantiates this class for any
 * well-formed id, so every read path first asks `sqlite_master` (which
 * persists nothing) whether `meta` exists. Only `create` (COMMIT, or an
 * admin action on a report nobody follows) writes the schema.
 *
 * Every reporter-facing method takes the presented secret's SHA-256 hex
 * (never the raw secret; `handlers.ts` hashes it) and compares it in
 * constant time before doing anything. A thread created for the admin's own
 * bookkeeping (a reply on a report nobody follows) stores an empty
 * `secretHash`, which no presented hash ever matches.
 *
 * The alarm is re-armed on every write to the earliest of
 * `finalSeenAt + THREAD_FINAL_SEEN_GRACE_MS` and `lastActivityAt +
 * THREAD_IDLE_MS` (§5.5). On destruction — the alarm or a reporter's
 * Forget — the store calls `onDestroy(reportId)` FIRST (the DO wrapper
 * points it at `ReportIndex.threadEnded`) and only then wipes storage, for
 * the reason `ReportDrop.alarm` orders its own steps that way: a retry after
 * a failed index call still finds the thread and tries again, while the
 * reverse order would orphan the index row's `following = 1` forever.
 *
 * Because `onDestroy` is an RPC the destroying call awaits, other calls to
 * this DO can run in between. So destruction first marks the meta row
 * `dying = 1` — a synchronous write, visible before the await — and every
 * reader treats a dying thread as gone (reporter calls `forbidden`,
 * maintainer calls `not-found`): nothing can be posted into a thread that
 * is about to be wiped. If `onDestroy` throws, the flag is cleared again so
 * the retry (the next alarm, or a repeated Forget) starts from a live
 * thread. A dying thread's alarm is due at once, and Forget still accepts
 * its secret, so a destroy that died mid-flight is always finishable.
 *
 * Room for the maintainer: the last `THREAD_MAINTAINER_RESERVE` slots of
 * `THREAD_MAX_MESSAGES` are closed to reporter posts and follow-up
 * attachment messages, so a reporter cannot fill a thread to the point the
 * maintainer can no longer reply or change its status.
 */

import type { DurableObjectStorage } from './types.ts'
import {
  THREAD_FINAL_SEEN_GRACE_MS,
  THREAD_FORMAT,
  THREAD_IDLE_MS,
  THREAD_MAINTAINER_RESERVE,
  THREAD_MAX_MESSAGES,
  THREAD_REPORTER_POSTS_DAY_MAX,
} from './constants.ts'
import { constantTimeEqual } from './uploadToken.ts'

export type ThreadStatus = 'open' | 'need-info' | 'fixed' | 'closed'
export type ThreadAuthor = 'maintainer' | 'reporter' | 'system'
export type ThreadMessageKind = 'text' | 'status' | 'attachment'

export const THREAD_STATUSES: readonly ThreadStatus[] = ['open', 'need-info', 'fixed', 'closed']

export function isThreadStatus(value: unknown): value is ThreadStatus {
  return typeof value === 'string' && (THREAD_STATUSES as readonly string[]).includes(value)
}

export function isFinalStatus(status: ThreadStatus): boolean {
  return status === 'fixed' || status === 'closed'
}

/** One message on the wire (`GET /report/<id>/thread`) and on the admin
 *  page. `attachmentReportId` is present only on `kind: "attachment"`. */
export interface ThreadMessage {
  seq: number
  at: number
  author: ThreadAuthor
  kind: ThreadMessageKind
  text: string
  attachmentReportId?: string
}

/** `GET /report/<id>/thread`'s 200 body, exactly (§5.4). */
export interface ThreadView {
  format: number
  status: ThreadStatus
  statusVersion: string | null
  statusAt: number
  latest: number
  messages: ThreadMessage[]
  now: number
}

/** What the admin detail page reads. `following` is false for a thread the
 *  admin created for bookkeeping (no secret). `previousAdminSeenUpTo` is
 *  the cursor BEFORE this view moved it, so the page can mark what is new. */
export interface ThreadAdminView {
  reportId: string
  following: boolean
  status: ThreadStatus
  statusVersion: string | null
  statusAt: number
  createdAt: number
  lastActivityAt: number
  lastPolledAt: number | null
  reporterSeenUpTo: number
  previousAdminSeenUpTo: number
  finalSeenAt: number | null
  latest: number
  messages: ThreadMessage[]
}

/** The thread-level fields a handler copies into the `ReportIndex` row's
 *  display caches after a change (§5.2). */
export interface ThreadStatusSnapshot {
  status: ThreadStatus
  statusVersion: string | null
}

export type PostReporterResult =
  | ({ ok: true; seq: number; reopened: boolean } & ThreadStatusSnapshot)
  | { ok: false; reason: 'forbidden' }
  | { ok: false; reason: 'too-many'; retryAfterMs: number }
  | { ok: false; reason: 'full' }

export type AppendResult = ({ ok: true; seq: number } & ThreadStatusSnapshot) | { ok: false; reason: 'not-found' | 'full' }

export type AppendAttachmentResult =
  | ({ ok: true; seq: number; appended: boolean } & ThreadStatusSnapshot)
  | { ok: false; reason: 'not-found' | 'full' }

interface MetaRow {
  reportId: string
  secretHash: string
  status: ThreadStatus
  statusVersion: string | null
  statusAt: number
  createdAt: number
  lastActivityAt: number
  lastPolledAt: number | null
  reporterSeenUpTo: number
  adminSeenUpTo: number
  finalSeenAt: number | null
  dying: number
}

/** The message count reporter posts and attachment messages may reach. */
const REPORTER_MESSAGE_CAP = THREAD_MAX_MESSAGES - THREAD_MAINTAINER_RESERVE

interface MessageRow {
  seq: number
  at: number
  author: ThreadAuthor
  kind: ThreadMessageKind
  text: string
  attachmentReportId: string | null
}

function rowToMessage(row: MessageRow): ThreadMessage {
  const msg: ThreadMessage = { seq: row.seq, at: row.at, author: row.author, kind: row.kind, text: row.text }
  if (row.attachmentReportId !== null) msg.attachmentReportId = row.attachmentReportId
  return msg
}

function utcDayStart(now: number): number {
  return Date.parse(`${new Date(now).toISOString().slice(0, 10)}T00:00:00.000Z`)
}

/** The system line a status change writes when the admin gave no note. */
export function defaultStatusText(status: ThreadStatus, version: string | null): string {
  switch (status) {
    case 'open':
      return 'Marked open'
    case 'need-info':
      return 'Needs more information'
    case 'fixed':
      return version ? `Fixed in ${version}` : 'Fixed'
    case 'closed':
      return 'Closed'
  }
}

/** The system line a reporter message on a final thread writes. */
export const REOPEN_TEXT = 'Reopened by the reporter'

export type OnThreadDestroy = (reportId: string) => Promise<void>

export class ThreadStore {
  private readonly storage: DurableObjectStorage
  private readonly onDestroy: OnThreadDestroy

  constructor(storage: DurableObjectStorage, onDestroy: OnThreadDestroy = async () => {}) {
    this.storage = storage
    this.onDestroy = onDestroy
  }

  // Every public method is `async` only so this class structurally satisfies
  // `ReportThreadStub` (`types.ts`) and the fake namespace can wrap it
  // directly; the SQL inside is synchronous, and nothing between a read and
  // the write that depends on it awaits (the alarm `setAlarm` is the last
  // step of each write).

  private hasSchema(): boolean {
    return (
      this.storage.sql
        .exec<{ n: number }>("SELECT COUNT(*) AS n FROM sqlite_master WHERE type = 'table' AND name = 'meta'")
        .toArray()[0].n > 0
    )
  }

  private migrate(): void {
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS meta (
         id INTEGER PRIMARY KEY CHECK (id = 0),
         reportId TEXT NOT NULL,
         secretHash TEXT NOT NULL,
         status TEXT NOT NULL,
         statusVersion TEXT,
         statusAt INTEGER NOT NULL,
         createdAt INTEGER NOT NULL,
         lastActivityAt INTEGER NOT NULL,
         lastPolledAt INTEGER,
         reporterSeenUpTo INTEGER NOT NULL DEFAULT 0,
         adminSeenUpTo INTEGER NOT NULL DEFAULT 0,
         finalSeenAt INTEGER,
         dying INTEGER NOT NULL DEFAULT 0
       )`,
    )
    this.storage.sql.exec(
      `CREATE TABLE IF NOT EXISTS messages (
         seq INTEGER PRIMARY KEY,
         at INTEGER NOT NULL,
         author TEXT NOT NULL,
         kind TEXT NOT NULL,
         text TEXT NOT NULL,
         attachmentReportId TEXT
       )`,
    )
  }

  private readMeta(): MetaRow | null {
    if (!this.hasSchema()) return null
    const rows = this.storage.sql
      .exec<MetaRow>(
        `SELECT reportId, secretHash, status, statusVersion, statusAt, createdAt, lastActivityAt, lastPolledAt,
                reporterSeenUpTo, adminSeenUpTo, finalSeenAt, dying
         FROM meta`,
      )
      .toArray()
    return rows.length === 0 ? null : rows[0]
  }

  /** The meta row of a live thread: `null` for no thread and for one that
   *  is being destroyed (see the class doc on `dying`). */
  private liveMeta(): MetaRow | null {
    const meta = this.readMeta()
    return meta === null || meta.dying !== 0 ? null : meta
  }

  /** The meta row, but only when `secretHash` matches a real (non-empty)
   *  stored secret — `null` covers no thread, an admin-only thread, and a
   *  wrong secret alike, so every caller answers them the same way. */
  private authorize(secretHash: string, allowDying = false): MetaRow | null {
    const meta = allowDying ? this.readMeta() : this.liveMeta()
    if (meta === null || meta.secretHash === '' || secretHash === '') return null
    return constantTimeEqual(secretHash, meta.secretHash) ? meta : null
  }

  private latestSeq(): number {
    return this.storage.sql.exec<{ m: number | null }>('SELECT MAX(seq) AS m FROM messages').toArray()[0]?.m ?? 0
  }

  private messageCount(): number {
    return this.storage.sql.exec<{ n: number }>('SELECT COUNT(*) AS n FROM messages').toArray()[0].n
  }

  private messagesAfter(after: number): ThreadMessage[] {
    return this.storage.sql
      .exec<MessageRow>(
        `SELECT seq, at, author, kind, text, attachmentReportId FROM messages WHERE seq > ? ORDER BY seq ASC LIMIT ?`,
        after,
        THREAD_MAX_MESSAGES,
      )
      .toArray()
      .map(rowToMessage)
  }

  private insertMessage(at: number, author: ThreadAuthor, kind: ThreadMessageKind, text: string, attachmentReportId: string | null = null): number {
    const seq = this.latestSeq() + 1
    this.storage.sql.exec(
      'INSERT INTO messages (seq, at, author, kind, text, attachmentReportId) VALUES (?, ?, ?, ?, ?, ?)',
      seq,
      at,
      author,
      kind,
      text,
      attachmentReportId,
    )
    return seq
  }

  /** When the alarm should fire for `meta` (§5.5). */
  static alarmTimeFor(meta: Pick<MetaRow, 'lastActivityAt' | 'finalSeenAt'> & { dying?: number }): number {
    if (meta.dying) return 0 // a destroy that never finished: due now
    const idle = meta.lastActivityAt + THREAD_IDLE_MS
    return meta.finalSeenAt === null ? idle : Math.min(idle, meta.finalSeenAt + THREAD_FINAL_SEEN_GRACE_MS)
  }

  private async rearm(): Promise<void> {
    const meta = this.readMeta()
    if (meta !== null) await this.storage.setAlarm(ThreadStore.alarmTimeFor(meta))
  }

  private snapshot(): ThreadStatusSnapshot {
    const meta = this.readMeta()!
    return { status: meta.status, statusVersion: meta.statusVersion }
  }

  // -------------------------------------------------------------------
  // Creation
  // -------------------------------------------------------------------

  /** Creates the thread (status `open`, no messages) — idempotent: a
   *  retried COMMIT, or an admin action on a thread that already exists,
   *  leaves the existing thread and its secret untouched. `secretHash` is
   *  `''` for a thread the admin creates on a report nobody follows. */
  async create(reportId: string, secretHash: string, now: number): Promise<void> {
    this.migrate()
    if (this.readMeta() !== null) return
    this.storage.sql.exec(
      `INSERT INTO meta (id, reportId, secretHash, status, statusVersion, statusAt, createdAt, lastActivityAt,
                         lastPolledAt, reporterSeenUpTo, adminSeenUpTo, finalSeenAt, dying)
       VALUES (0, ?, ?, 'open', NULL, ?, ?, ?, NULL, 0, 0, NULL, 0)`,
      reportId,
      secretHash,
      now,
      now,
      now,
    )
    await this.rearm()
  }

  // -------------------------------------------------------------------
  // Reporter side (secret-gated)
  // -------------------------------------------------------------------

  /** Follow-up START's check. A read only: no activity, no alarm. */
  async verifySecret(secretHash: string): Promise<boolean> {
    return this.authorize(secretHash) !== null
  }

  /** The thread view (§5.4). Records the poll (`lastPolledAt`, activity —
   *  design D22: the one read that writes). */
  async read(secretHash: string, afterSeq: number, now: number): Promise<ThreadView | 'forbidden'> {
    const meta = this.authorize(secretHash)
    if (meta === null) return 'forbidden'
    this.storage.sql.exec('UPDATE meta SET lastPolledAt = ?, lastActivityAt = ? WHERE id = 0', now, now)
    await this.rearm()
    return {
      format: THREAD_FORMAT,
      status: meta.status,
      statusVersion: meta.statusVersion,
      statusAt: meta.statusAt,
      latest: this.latestSeq(),
      messages: this.messagesAfter(afterSeq),
      now,
    }
  }

  /** A reporter message. The caller has already checked the text's length.
   *  On a `fixed`/`closed` thread it reopens (status `open`, a system status
   *  message, `finalSeenAt` cleared — §5.4, D13); on a `need-info` thread it
   *  moves the status back to `open` without a system line (the reply is
   *  the answer being waited for, the same rule an attachment follows). */
  async postReporter(secretHash: string, text: string, now: number): Promise<PostReporterResult> {
    const meta = this.authorize(secretHash)
    if (meta === null) return { ok: false, reason: 'forbidden' }

    const dayStart = utcDayStart(now)
    const today = this.storage.sql
      .exec<{ n: number }>(
        "SELECT COUNT(*) AS n FROM messages WHERE author = 'reporter' AND kind = 'text' AND at >= ?",
        dayStart,
      )
      .toArray()[0].n
    if (today >= THREAD_REPORTER_POSTS_DAY_MAX) {
      return { ok: false, reason: 'too-many', retryAfterMs: dayStart + 24 * 60 * 60 * 1000 - now }
    }

    const reopen = isFinalStatus(meta.status)
    if (this.messageCount() + (reopen ? 2 : 1) > REPORTER_MESSAGE_CAP) {
      return { ok: false, reason: 'full' }
    }

    const seq = this.insertMessage(now, 'reporter', 'text', text)
    if (reopen) {
      this.insertMessage(now, 'system', 'status', REOPEN_TEXT)
      this.storage.sql.exec(
        "UPDATE meta SET status = 'open', statusVersion = NULL, statusAt = ?, finalSeenAt = NULL, lastActivityAt = ? WHERE id = 0",
        now,
        now,
      )
    } else if (meta.status === 'need-info') {
      this.storage.sql.exec("UPDATE meta SET status = 'open', statusAt = ?, lastActivityAt = ? WHERE id = 0", now, now)
    } else {
      this.storage.sql.exec('UPDATE meta SET lastActivityAt = ? WHERE id = 0', now)
    }
    await this.rearm()
    return { ok: true, seq, reopened: reopen, ...this.snapshot() }
  }

  /** The reporter has viewed through `upTo` (clamped to the latest seq).
   *  Seeing the last status message while the status is final starts the
   *  30-day grace clock (`finalSeenAt`, set once). */
  async markSeen(secretHash: string, upTo: number, now: number): Promise<'ok' | 'forbidden'> {
    const meta = this.authorize(secretHash)
    if (meta === null) return 'forbidden'
    const latest = this.latestSeq()
    const seen = Math.max(meta.reporterSeenUpTo, Math.min(Math.max(0, Math.floor(upTo)), latest))
    let finalSeenAt = meta.finalSeenAt
    if (finalSeenAt === null && isFinalStatus(meta.status)) {
      const lastStatusSeq =
        this.storage.sql.exec<{ m: number | null }>("SELECT MAX(seq) AS m FROM messages WHERE kind = 'status'").toArray()[0]?.m ?? 0
      if (seen >= lastStatusSeq) finalSeenAt = now
    }
    this.storage.sql.exec(
      'UPDATE meta SET reporterSeenUpTo = ?, finalSeenAt = ?, lastActivityAt = ? WHERE id = 0',
      seen,
      finalSeenAt,
      now,
    )
    await this.rearm()
    return 'ok'
  }

  /** Forget: destroys the thread, telling the index first. Accepts the
   *  secret of a thread already dying, so a Forget that died mid-flight can
   *  be repeated to finish. */
  async revoke(secretHash: string): Promise<'ok' | 'forbidden'> {
    const meta = this.authorize(secretHash, true)
    if (meta === null) return 'forbidden'
    await this.destroyNotifying(meta.reportId)
    return 'ok'
  }

  /** Marks the thread dying (synchronously, before any await), tells the
   *  index, then wipes. A throwing `onDestroy` clears the mark and
   *  rethrows, leaving a live thread for the retry — but only the call
   *  that SET the mark may clear it: the alarm and a Forget can overlap
   *  (each awaiting its own `onDestroy`), and if the first one's index
   *  call failed while the second was still in flight, clearing the mark
   *  would briefly revive the thread under the second call, letting a
   *  reporter post land and be wiped a moment later. A call that found
   *  the mark already set leaves it to its owner (or to the next alarm,
   *  which is due at once for a dying thread). */
  private async destroyNotifying(reportId: string): Promise<void> {
    const mine =
      this.storage.sql.exec<{ dying: number }>('SELECT dying FROM meta WHERE id = 0').toArray()[0]?.dying === 0
    if (mine) this.storage.sql.exec('UPDATE meta SET dying = 1 WHERE id = 0')
    try {
      await this.onDestroy(reportId)
    } catch (err) {
      if (mine) this.storage.sql.exec('UPDATE meta SET dying = 0 WHERE id = 0')
      throw err
    }
    await this.destroy()
  }

  // -------------------------------------------------------------------
  // Maintainer side (admin routes, follow-up COMMIT)
  // -------------------------------------------------------------------

  async postMaintainer(text: string, now: number): Promise<AppendResult> {
    const meta = this.liveMeta()
    if (meta === null) return { ok: false, reason: 'not-found' }
    if (this.messageCount() + 1 > THREAD_MAX_MESSAGES) return { ok: false, reason: 'full' }
    const seq = this.insertMessage(now, 'maintainer', 'text', text)
    this.storage.sql.exec('UPDATE meta SET lastActivityAt = ? WHERE id = 0', now)
    await this.rearm()
    return { ok: true, seq, ...this.snapshot() }
  }

  /** Sets the status and appends a `system`/`status` message whose text is
   *  `note` when given, else `defaultStatusText`. `version` is kept only for
   *  `fixed`. Moving to a non-final status clears `finalSeenAt`; moving to
   *  a final one leaves the grace clock for the reporter's next `markSeen`
   *  (the new status message is after anything already seen). */
  async setStatus(status: ThreadStatus, version: string | null, note: string | null, now: number): Promise<AppendResult> {
    const meta = this.liveMeta()
    if (meta === null) return { ok: false, reason: 'not-found' }
    if (this.messageCount() + 1 > THREAD_MAX_MESSAGES) return { ok: false, reason: 'full' }
    const statusVersion = status === 'fixed' && version ? version : null
    const text = note && note.trim().length > 0 ? note : defaultStatusText(status, statusVersion)
    const seq = this.insertMessage(now, 'system', 'status', text)
    this.storage.sql.exec(
      'UPDATE meta SET status = ?, statusVersion = ?, statusAt = ?, finalSeenAt = NULL, lastActivityAt = ? WHERE id = 0',
      status,
      statusVersion,
      now,
      now,
    )
    await this.rearm()
    return { ok: true, seq, ...this.snapshot() }
  }

  /** A follow-up attachment landed (follow-up COMMIT). Idempotent per
   *  `childId`: a retried COMMIT finds the existing message and appends
   *  nothing (`appended: false`). Flips `need-info` to `open` (§5.6). On
   *  a `fixed`/`closed` thread the status stays, but `finalSeenAt` is
   *  cleared: the reporter is still sending material, so the 30-day grace
   *  clock stops and starts over from their next `markSeen`. */
  async appendAttachment(childId: string, text: string, now: number): Promise<AppendAttachmentResult> {
    const meta = this.liveMeta()
    if (meta === null) return { ok: false, reason: 'not-found' }
    const existing = this.storage.sql
      .exec<{ seq: number }>("SELECT seq FROM messages WHERE kind = 'attachment' AND attachmentReportId = ?", childId)
      .toArray()[0]
    if (existing !== undefined) return { ok: true, seq: existing.seq, appended: false, ...this.snapshot() }
    if (this.messageCount() + 1 > REPORTER_MESSAGE_CAP) return { ok: false, reason: 'full' }
    const seq = this.insertMessage(now, 'reporter', 'attachment', text, childId)
    if (meta.status === 'need-info') {
      this.storage.sql.exec("UPDATE meta SET status = 'open', statusAt = ?, lastActivityAt = ? WHERE id = 0", now, now)
    } else if (isFinalStatus(meta.status)) {
      this.storage.sql.exec('UPDATE meta SET finalSeenAt = NULL, lastActivityAt = ? WHERE id = 0', now)
    } else {
      this.storage.sql.exec('UPDATE meta SET lastActivityAt = ? WHERE id = 0', now)
    }
    await this.rearm()
    return { ok: true, seq, appended: true, ...this.snapshot() }
  }

  /** Everything the admin detail page shows; moves `adminSeenUpTo` to the
   *  latest seq. `null` (and no storage) when there is no thread. Not
   *  activity (§5.5 lists messages, status changes, polls, and seen). */
  async adminView(now: number): Promise<ThreadAdminView | null> {
    void now
    const meta = this.readMeta()
    if (meta === null) return null
    const latest = this.latestSeq()
    if (latest !== meta.adminSeenUpTo) {
      this.storage.sql.exec('UPDATE meta SET adminSeenUpTo = ? WHERE id = 0', latest)
      await this.rearm()
    }
    return {
      reportId: meta.reportId,
      following: meta.secretHash !== '',
      status: meta.status,
      statusVersion: meta.statusVersion,
      statusAt: meta.statusAt,
      createdAt: meta.createdAt,
      lastActivityAt: meta.lastActivityAt,
      lastPolledAt: meta.lastPolledAt,
      reporterSeenUpTo: meta.reporterSeenUpTo,
      previousAdminSeenUpTo: meta.adminSeenUpTo,
      finalSeenAt: meta.finalSeenAt,
      latest,
      messages: this.messagesAfter(0),
    }
  }

  /** Wipes storage and cancels the alarm — idempotent, and does NOT tell
   *  the index (admin delete removes the row itself). */
  async destroy(): Promise<void> {
    await this.storage.deleteAll()
    await this.storage.deleteAlarm()
  }

  /** The alarm handler's logic. A fire before the current due time (a stale
   *  alarm a later write already moved) re-arms and returns; a fire on a
   *  thread that no longer exists just clears the alarm. Otherwise: tell
   *  the index, then wipe — see the class doc for the order. */
  async alarm(now: number): Promise<void> {
    const meta = this.readMeta()
    if (meta === null) {
      await this.storage.deleteAlarm()
      return
    }
    const due = ThreadStore.alarmTimeFor(meta)
    if (now < due) {
      await this.storage.setAlarm(due)
      return
    }
    await this.destroyNotifying(meta.reportId)
  }
}
