/**
 * Every tunable value this Worker uses. Per docs/design/report-bug.md §8:
 * account-specific and personal values are Worker secrets (`types.ts`'s
 * `BugIntakeEnv`); everything else — retention, size caps, rate limits — is
 * a constant here, not configuration, so `wrangler.toml` never needs a
 * `[vars]` block.
 */

/** The whole compressed report (`Hew-Upload-Length`) can't exceed this —
 *  1 GiB. It's also `GET /report/`'s reported `maxBytes`. Cloudflare's
 *  100 MB request body limit does not bound it: the upload arrives in
 *  `PIECE_BYTES` pieces, one request each. What does bound it is
 *  `STORE_CEILING_BYTES` below, which a few reports at this cap would fill;
 *  the cap is set for the largest SketchUp file a report has to carry (a
 *  model that runs the kernel out of memory is hundreds of megabytes), and
 *  the ceiling is accepted as the price. */
export const MAX_UPLOAD_BYTES = 1024 * 1024 * 1024

/** The Workers FREE plan caps CPU time at a hard, non-configurable 10 ms
 *  per request (docs/design/report-bug.md §4 "Chunked upload"), and a
 *  Worker isolate has only 128 MB of memory — copying a report of tens of
 *  megabytes, let alone one near `MAX_UPLOAD_BYTES`, through a SINGLE
 *  request fits neither. So the client splits the
 *  compressed report into pieces of at most this many bytes and uploads
 *  each in its own request (`POST /report/` for the first, `PUT
 *  /report/<id>/<index>` for the rest): every request handles exactly ONE
 *  piece — read it, store it as one `ReportDrop` row, respond — which is
 *  cheap enough in CPU time to stay on Free, and small enough (comfortably
 *  under the 2 MB SQLite row/BLOB cap) to store as-is with no
 *  rechunking and no multi-piece batching. A 90 MiB report is about 50
 *  requests; one at the 1 GiB cap is about 565. */
export const PIECE_BYTES = 1_900_000

/** How much DECOMPRESSED head data `headScan.ts`'s `HeadDecompressor` reads
 *  before it stops decompressing — comfortably past `format`, all of
 *  `report` (including a `description` at its 10,000-character maximum),
 *  and `system`, for any legitimate bundle. Only the FIRST piece is ever
 *  decompressed (docs/design/report-bug.md §4: "decompresses only the
 *  start of that piece") — `report`/`system` always land well inside a
 *  single 1.9 MB piece, so nothing later in the upload is ever touched by
 *  this. A head that doesn't balance within this budget is invalid rather
 *  than read further. */
export const HEAD_MAX_DECOMPRESSED_BYTES = 256 * 1024

/** How long a stored report survives before its `ReportDrop`'s alarm wipes
 *  it (docs/design/report-bug.md §4 "Retention: 90 days"). The admin page
 *  can delete a report sooner; this is only the outer bound. Distinct from
 *  `ABANDON_TIMEOUT_MS` below, which governs an IN-PROGRESS upload, not a
 *  finished report. */
export const RETENTION_DAYS = 90
export const RETENTION_MS = RETENTION_DAYS * 24 * 60 * 60 * 1000

/** `ReportIndex` refuses a new reservation once the sum of already-stored
 *  (and reserved-but-uncommitted) reports' `sizeBytes` would pass this
 *  ceiling (507) — the account's 5 GB of free Durable Object storage is
 *  shared with share-relay's ten-minute drops, so this Worker claims a
 *  fixed slice rather than the whole thing. 3 GiB fits about 34 reports the
 *  size of the design's 59.9 MB worked example, but only 3 at the 1 GiB
 *  `MAX_UPLOAD_BYTES` cap: a run of reports carrying huge files, or a
 *  deliberate one, fills the store until the admin page deletes them or
 *  retention does. Accepted — the reports that matter most are the ones
 *  with a file that would not open, and those files are big. */
export const STORE_CEILING_BYTES = 3 * 1024 * 1024 * 1024

/** A client may reserve at most one maximum-size report's worth of bytes
 *  across its unfinished uploads. Completed reports still count against
 *  the global ceiling, but no longer hold this temporary budget. This
 *  prevents a few small START pieces from one client reserving the entire
 *  store before any of their declared report bytes have actually arrived. */
export const CLIENT_PENDING_BYTES = MAX_UPLOAD_BYTES

/** A `ReportDrop` mid-upload arms its alarm for this long after every piece
 *  (and after the start piece) — if no new piece or commit arrives before
 *  it fires, the alarm treats the upload as abandoned: it deletes the
 *  partial drop and tells `ReportIndex` to release the reservation.
 *  Re-armed on every piece, so a genuinely slow (but still active) upload
 *  is never cut off mid-stream — only a truly stalled one is. */
export const ABANDON_TIMEOUT_MS = 10 * 60 * 1000

/** The absolute lifetime of an UNCOMMITTED upload, measured from its START
 *  regardless of activity — `ABANDON_TIMEOUT_MS` and `IDLE_PRUNE_TIMEOUT_MS`
 *  below only measure silence, and a reservation holds its full declared
 *  size against `STORE_CEILING_BYTES` from the moment START accepts it. An
 *  upload still uncommitted this long after it started is abandoned (its
 *  alarm is never armed past this bound, `putPiece` refuses the next piece
 *  as `expired`, and the index prune drops it) so a client cannot hold a
 *  reservation open indefinitely by trickling pieces. Eight hours is far
 *  past the slowest legitimate upload: `MAX_UPLOAD_BYTES` takes about two
 *  and a half hours on a 1 Mbit/s uplink, and `ABANDON_TIMEOUT_MS` ends a
 *  stalled one long before this does. */
export const UPLOAD_LIFETIME_MS = 8 * 60 * 60 * 1000

/** `ReportIndex`'s own backstop, independent of any one `ReportDrop`'s
 *  alarm: any uncommitted row whose `lastActivity` is older than this gets
 *  pruned (its bytes given back to the ceiling, and its drop destroyed) the
 *  next time a reservation is attempted. This is ACTIVITY time, not age
 *  since the upload started — a legitimate upload near the cap takes longer
 *  than this to finish piece by piece, but it keeps touching `lastActivity`
 *  the whole time, so it's never at risk; only an upload that's genuinely
 *  gone quiet is. Longer than `ABANDON_TIMEOUT_MS` on purpose: the
 *  per-drop alarm is the primary cleanup path, and this only needs to catch
 *  what it somehow missed. */
export const IDLE_PRUNE_TIMEOUT_MS = 15 * 60 * 1000

/** Per-client submission limits, keyed on the daily-salted IP hash
 *  (`clientHash.ts`). Both windows are enforced; either can trip the 429. */
export const RATE_LIMIT_WINDOW_MS = 10 * 60 * 1000
export const RATE_LIMIT_WINDOW_MAX = 5
export const RATE_LIMIT_DAY_MAX = 20

/** Notification emails are capped account-wide, not per client — a burst of
 *  distinct-IP spam should not turn into fifty-plus emails just because each
 *  sender only submitted once. Reports past the cap still store and appear
 *  in the admin list; only the email is skipped. */
export const EMAIL_DAY_MAX = 50

/** `system.appVersion` / `system.platform` length bound. Both are stored
 *  verbatim in the `ReportIndex` row for every report (the admin list and
 *  the email subject), and that row's bytes are NOT what `STORE_CEILING_BYTES`
 *  counts (`sizeBytes` is the compressed upload), so without a bound a
 *  submitter could park ~250 KiB of uncounted text per report in the single
 *  index DO. Real values are short (`1.1.0`, `desktop-macos`). */
export const SYSTEM_FIELD_MAX_CHARS = 128

/** `report.description` length bounds (docs/design/report-bug.md §2's "10 to
 *  10,000 characters", restated in §8's 400 case). */
export const DESCRIPTION_MIN_CHARS = 10
export const DESCRIPTION_MAX_CHARS = 10_000

/** The only bundle format this Worker accepts (docs/design/report-bug.md §3). */
export const BUNDLE_FORMAT = 1

/** `GET /report/`'s identity payload version (§8). */
export const CONTRACT_FORMAT = 1

/** The single `ReportIndex` Durable Object's `idFromName` key — there is
 *  exactly one instance account-wide, so the name is a fixed constant
 *  rather than derived from anything request-specific. */
export const INDEX_DO_NAME = 'index'

/** How long a fetched Access JWKS is trusted before being re-fetched
 *  (`adminAuth.ts`) — "cache keys briefly" per the task brief: long enough
 *  to avoid a JWKS fetch on every admin request, short enough that a key
 *  rotation on Cloudflare's side takes effect quickly. */
export const JWKS_CACHE_MS = 5 * 60 * 1000

/** An upload token is this many random bytes, base64url-encoded (43
 *  characters, no padding) — docs/design/report-bug.md §8. It authorizes
 *  only its own upload's pieces and commit; the Worker never stores it
 *  directly, only its SHA-256 (`uploadToken.ts`). */
export const UPLOAD_TOKEN_BYTES = 32
