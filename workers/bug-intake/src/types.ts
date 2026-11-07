/**
 * Minimal local shims for the pieces of the Cloudflare Workers Durable
 * Object runtime this project touches — hand-rolled rather than depending
 * on `@cloudflare/workers-types`, same choice share-relay makes (see its
 * `types.ts`) so the unit suite (`node --test`) runs with zero installs.
 * `src/testSupport/fakeDurableObject.ts` implements these interfaces over
 * Node's built-in `node:sqlite`. `wrangler dev`/`deploy` type-check against
 * their own bundled definitions regardless of what's declared here.
 */

import type {
  ThreadView,
  ThreadAdminView,
  ThreadStatus,
  PostReporterResult,
  AppendResult,
  AppendAttachmentResult,
} from './threadStore.ts'

/** A cursor over a `SqlStorage.exec` result set — only `toArray()` is used
 *  here, same subset share-relay declares. */
export interface SqlStorageCursor<T> {
  toArray(): T[]
}

/** Synchronous SQL over a Durable Object's private SQLite database. Column
 *  values are one of `ArrayBuffer | string | number | null`. */
export interface SqlStorage {
  exec<T = Record<string, unknown>>(query: string, ...bindings: unknown[]): SqlStorageCursor<T>
}

/** The subset of `DurableObjectStorage` this Worker calls. `setAlarm`/
 *  `getAlarm`/`deleteAlarm`/`deleteAll` are genuinely asynchronous host calls
 *  in the real runtime, but the runtime's input/output gates still serialize
 *  them against other requests to the same DO id. */
export interface DurableObjectStorage {
  readonly sql: SqlStorage
  setAlarm(scheduledTime: number): void | Promise<void>
  getAlarm(): Promise<number | null>
  deleteAlarm(): void | Promise<void>
  deleteAll(): Promise<void>
}

/** Trimmed down from the real `DurableObjectState` to the one property this
 *  project's DO constructors read. */
export interface DurableObjectState {
  readonly storage: DurableObjectStorage
}

/** Opaque DO identity — never inspected beyond passing it from `idFromName`
 *  to `get`. */
export interface DurableObjectId {
  toString(): string
}

/** What `startUpload` needs beyond the raw bytes/lengths — the streamed
 *  head's validated fields, carried here so `commit` can hand them back for
 *  the notification email without re-reading anything. */
export interface StartFields {
  description: string
  appVersion: string
  platform: string
  /** SHA-256 hex of a follow secret minted at START, if any. */
  followHash?: string | null
  /** The parent report of a follow-up attachment, if any. */
  parentId?: string | null
}

export type PutPieceResult =
  | { ok: true; stored: boolean }
  | { ok: false; reason: 'not-found' }
  | { ok: false; reason: 'forbidden' }
  | { ok: false; reason: 'too-large' }
  | { ok: false; reason: 'short-piece' }
  | { ok: false; reason: 'bytes-past-total' }
  | { ok: false; reason: 'expired' }
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

/** Result of `ReportStore.abandon` — the only authorized way to destroy an
 *  in-progress upload's storage from outside the store itself. A report ID
 *  is PUBLIC (a user pastes it into a GitHub issue), so it must never by
 *  itself authorize deleting anything: `ok: true` only when `tokenHash`
 *  matched the upload's own stored token. `not-found` covers both "never
 *  started" and "already committed" — same collapse `putPiece`/`commit`
 *  make, since the caller can't (and doesn't need to) tell those apart. */
export type AbandonResult = { ok: true } | { ok: false; reason: 'not-found' } | { ok: false; reason: 'forbidden' }

/** The RPC surface `ReportDrop` (`src/reportDrop.ts`) exposes: the chunked
 *  upload state machine (`reportStore.ts`) plus admin readback and
 *  deletion. `startUpload`/`putPiece`/`commit`/`abandon` never see a raw
 *  upload token, only its SHA-256 hex digest — the caller (`handlers.ts`)
 *  hashes it before calling in. */
export interface ReportDropStub {
  startUpload(reportId: string, declaredTotal: number, tokenHash: string, firstPiece: Uint8Array, fields: StartFields): Promise<void>
  putPiece(index: number, tokenHash: string, data: Uint8Array): Promise<PutPieceResult>
  commit(tokenHash: string): Promise<CommitResult>
  /** Destroys this upload's storage ONLY if `tokenHash` matches — the fix
   *  for an oversized `PUT` piece, which must not be able to delete a
   *  stranger's report just because it knows the (public) report id. */
  abandon(tokenHash: string): Promise<AbandonResult>
  head(): Promise<{ reportId: string; totalBytes: number; pieceCount: number; receivedAt: number } | null>
  read(from: number, count: number): Promise<Uint8Array[]>
  /** A byte `ReadableStream` over every stored piece, in index order,
   *  without the caller ever holding more than one piece in memory —
   *  Workers RPC forwards a byte-oriented stream across the DO boundary
   *  without buffering it whole (see
   *  https://developers.cloudflare.com/workers/runtime-apis/rpc/). Used by
   *  the admin download route instead of reading every piece into memory. */
  readStream(): Promise<ReadableStream<Uint8Array>>
  destroy(): Promise<void>
}

/** What `IndexStore.commitReport` reports back: `'flipped'` only the FIRST
 *  time a given row goes from uncommitted to committed — a retried COMMIT
 *  (after `ReportDrop.commit` itself already answered `ok` for an
 *  already-committed drop) finds the row already committed and gets
 *  `'already-committed'` instead, so the caller (`handlers.ts`) knows not
 *  to send a second notification email. `'not-found'` is the genuinely
 *  abnormal case — the row is gone even though the drop just committed
 *  successfully (an admin delete racing a client's retry, say) — which the
 *  caller must NOT treat the same as `'already-committed'`, since one means
 *  "already handled, say 201" and the other means "something is wrong, say
 *  503". */
export type CommitReportOutcome = 'flipped' | 'already-committed' | 'not-found'

/** The RPC surface `ReportIndex` (`src/reportIndex.ts`) exposes — the single
 *  instance backing the admin list, the storage ceiling, and the per-client
 *  and per-day counters (`indexStore.ts`). `reserveReport` fuses the rate
 *  limit check, the storage-ceiling check, and reserving the report's bytes
 *  into one call specifically so no two concurrent submissions can both
 *  pass a ceiling check meant to admit only one of them (`indexStore.ts`'s
 *  doc comment on `reserveReport` has the full argument); a successful
 *  reservation must be followed by exactly one of `commitReport` (the
 *  upload finished) or `deleteReport` (it was abandoned, at any point). */
export interface ReportIndexStub {
  reserveReport(
    clientHash: string,
    now: number,
    entry: ReportReservation,
  ): Promise<{ ok: true } | { ok: false; reason: 'rate-limited'; retryAfterMs: number } | { ok: false; reason: 'full' }>
  /** `following`: COMMIT just created this report's thread. */
  commitReport(id: string, following?: boolean): Promise<CommitReportOutcome>
  touchActivity(id: string, now: number): Promise<void>
  listReports(): Promise<ReportEntry[]>
  getReport(id: string): Promise<ReportEntry | null>
  deleteReport(id: string): Promise<void>
  setTriaged(id: string, triaged: boolean): Promise<void>
  /** `'thread'` stops short of the cap (`EMAIL_THREAD_RESERVE`). */
  reserveEmail(now: number, purpose?: 'report' | 'thread'): Promise<boolean>
  /** A `ReportDrop` alarm (retention or abandonment) fired for `id`.
   *  Resolves `true` when the row is gone afterwards. */
  bundleExpired(id: string): Promise<boolean>
  /** `id`'s `ReportThread` was destroyed (alarm or Forget). */
  threadEnded(id: string): Promise<void>
  setThreadCache(id: string, patch: ThreadCachePatch): Promise<void>
  markAdminViewed(id: string, now: number): Promise<void>
  listChildren(parentId: string): Promise<ReportEntry[]>
  countThreadRequest(clientHash: string, now: number): Promise<{ ok: true } | { ok: false; retryAfterMs: number }>
}

/** See `indexStore.ts`. */
export interface ThreadCachePatch {
  status?: string
  statusVersion?: string | null
  reporterActivityAt?: number
}

/** One row of the admin list / detail metadata. `sizeBytes` is the raw
 *  bundle's byte length, used both for display and for the storage ceiling
 *  (`indexStore.ts` sums this column rather than keeping a separate
 *  counter, so deleting a report's row is the only bookkeeping a delete
 *  needs). `committed` is false from the moment `reserveReport` accepts a
 *  submission until `commitReport` runs after its bundle is fully written —
 *  `listReports` never returns an uncommitted row, and admin routes that
 *  read one via `getReport` treat it as not-yet-existing. */
export interface ReportEntry {
  id: string
  receivedAt: number
  appVersion: string
  platform: string
  descriptionPreview: string
  sizeBytes: number
  triaged: boolean
  committed: boolean
  /** Display caches for the admin list (`indexStore.ts`'s `ReportRow`). */
  following: boolean
  status: string
  statusVersion: string | null
  reporterActivityAt: number | null
  adminViewedAt: number | null
  parentId: string | null
  bundleDeleted: boolean
}

/** What `reserveReport` needs at reservation time — before a byte of the
 *  bundle has gone anywhere, so before `triaged`/`committed` mean anything. */
export type ReportReservation = Pick<
  ReportEntry,
  'id' | 'receivedAt' | 'appVersion' | 'platform' | 'descriptionPreview' | 'sizeBytes'
> & { parentId?: string | null }

/** The RPC surface `ReportThread` (`src/reportThread.ts`) exposes — one
 *  followed report's thread (docs/design/follow-a-report.md §5.1). Every
 *  reporter-facing method takes the presented follow secret's SHA-256 hex,
 *  never the raw secret. Shapes are defined in `threadStore.ts`. */
export interface ReportThreadStub {
  create(reportId: string, secretHash: string, now: number): Promise<void>
  verifySecret(secretHash: string): Promise<boolean>
  read(secretHash: string, afterSeq: number, now: number): Promise<ThreadView | 'forbidden'>
  postReporter(secretHash: string, text: string, now: number): Promise<PostReporterResult>
  markSeen(secretHash: string, upTo: number, now: number): Promise<'ok' | 'forbidden'>
  revoke(secretHash: string): Promise<'ok' | 'forbidden'>
  postMaintainer(text: string, now: number): Promise<AppendResult>
  setStatus(status: ThreadStatus, version: string | null, note: string | null, now: number): Promise<AppendResult>
  appendAttachment(childId: string, text: string, now: number): Promise<AppendAttachmentResult>
  adminView(now: number): Promise<ThreadAdminView | null>
  destroy(): Promise<void>
}

/** The subset of `DurableObjectNamespace<T>` this Worker calls. */
export interface DurableObjectNamespace<T> {
  idFromName(name: string): DurableObjectId
  get(id: DurableObjectId): T
}

/** `send_email` binding surface (`cloudflare:email`'s `SendEmail` binding
 *  type, trimmed to the one method `email.ts` calls). */
export interface SendEmailBinding {
  send(message: unknown): Promise<void>
}

/** The Worker's env bindings. Every account-specific or personal value is a
 *  Worker secret (never a `[vars]` entry — see `wrangler.toml`'s header
 *  comment and README.md's "Configuration"); tunables live in
 *  `constants.ts` instead. */
export interface BugIntakeEnv {
  REPORT_DROP: DurableObjectNamespace<ReportDropStub>
  REPORT_INDEX: DurableObjectNamespace<ReportIndexStub>
  REPORT_THREAD: DurableObjectNamespace<ReportThreadStub>
  NOTIFY: SendEmailBinding
  /** The maintainer's verified Email Routing destination address. Unset:
   *  submissions still store, email is skipped, and the skip is logged once
   *  per request (never repeated retries). */
  NOTIFY_TO?: string
  /** The sender address on a domain with Email Routing enabled. Unset has
   *  the same skip-and-log behavior as a missing `NOTIFY_TO`. */
  NOTIFY_FROM?: string
  /** `<team>.cloudflareaccess.com` — both the JWKS host and the expected
   *  `iss`. Unset makes every `/report/admin/*` request 403. */
  ACCESS_TEAM_DOMAIN?: string
  /** The Access application's audience tag, checked against the JWT's
   *  `aud`. Unset makes every `/report/admin/*` request 403. */
  ACCESS_AUD?: string
  /** Salts the daily client-IP hash (`clientHash.ts`). Unset refuses every
   *  submission with 503 rather than ever hashing an IP unsalted. */
  IP_HASH_SECRET?: string
}
