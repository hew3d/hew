# bug-intake

A small, private Cloudflare Worker behind Hew's Report Bug dialog. The
dialog (`docs/design/report-bug.md`) builds a JSON bundle — a description,
optional recorded steps, optional imported files, an optional model file, a
scrubbed log tail — gzip-compresses it, and uploads the compressed bytes
here in a series of small chunked requests. This Worker assembles the
upload, emails the maintainer once it's complete, and shows the report on a
Cloudflare-Access-protected admin page. Nothing here is public: there is no
way to read a report back by its own ID, and the admin routes answer 403 to
anyone Access hasn't already let through.

**Why chunked, not one request**: a real 59 MiB SketchUp import produces a
210.9 MB JSON bundle that gzips to 59.9 MB, and the compressed cap is 90
MiB. The Workers **Free** plan — which this Worker deliberately stays on —
caps CPU time at a hard, non-configurable **10 ms per request**; copying a
60–90 MiB upload through a single request doesn't fit that budget no matter
how the copying is arranged (streamed or not — see "Chunked upload" below
for the actual numbers). So the client splits the compressed bundle into
pieces of at most 1.9 MB and uploads each in its own request instead.

This is the second Worker beside `share-relay`, and deliberately mirrors
its structure and storage posture — see that Worker's `README.md` for the
fuller argument (SQLite-backed Durable Objects, why not R2, the free-tier
limits and why they fail closed) before reading the report-bug-specific
parts below.

## API

The full wire contract is `docs/design/report-bug.md` §8, which the app
lane also codes against — this Worker implements it exactly; do not extend
it without updating that document first. An upload is three steps: **START**
(the first piece), any number of **PIECE** requests (every piece after the
first), then **COMMIT**.

| Route | Method | Purpose | Response |
|---|---|---|---|
| `/report/` (also `/report`) | `GET` | identity | `200 {"service":"hew-bug-intake","format":1,"maxBytes":1073741824,"pieceBytes":1900000}` |
| `/report/` (also `/report`) | `POST` | **START** — begins an upload with its first piece | `201 {"id":"HEW-XXXX-XXXX","token":"<43-char base64url>","pieceBytes":1900000}` / `400` invalid (including a first piece that is neither the whole report nor exactly `pieceBytes`) / `411` no Content-Length / `413` too large / `429` rate limited / `507` full / `5xx` unavailable |
| `/report/<id>/<index>` (`index` ≥ 1) | `PUT` | **PIECE** — uploads one following piece | `204` stored (or an accepted idempotent retry) / `400` invalid (piece too large, past the declared total, a non-final piece that is not exactly `pieceBytes`, or an upload older than `UPLOAD_LIFETIME_MS` — all WITH the correct upload token — **abandons the upload**) / `403` forbidden (missing/wrong token — never abandons anything, even an oversized piece) / `404` not found (unknown, committed, or already abandoned) / `409 {"error":"out-of-order","expected":<n>}` / `5xx` unavailable |
| `/report/<id>/commit` | `POST` | **COMMIT** — finishes the upload | `201 {"id":"HEW-XXXX-XXXX"}` — idempotent for the token holder: a retry against an already-committed report answers `201` again (with no second notification email), not `404` / `400` invalid (fewer bytes arrived than declared — **abandons the upload**) / `403` forbidden / `404` not found / `5xx` unavailable |
| `/report/admin/` | `GET` | the reports list (HTML), Access-protected | |
| `/report/admin/<id>` | `GET` | one report's detail (HTML) — reads and decompresses only piece 0's HEAD, never the whole bundle | |
| `/report/admin/<id>/download` | `GET` | the exact stored, still-**compressed** bundle bytes, streamed straight out of the `ReportDrop` as `<id>.json.gz`, `Content-Disposition: attachment` | |
| `/report/admin/<id>/delete` | `POST` | deletes the report, its thread, and its follow-up attachments; redirects to the list | |
| `/report/admin/<id>/triage` | `POST` | toggles the triaged flag; redirects to the detail page | |
| `/report/<id>/thread?after=<seq>` | `GET` | a followed report's thread (header `Hew-Follow-Secret`) — see "Following a report" | `200` view / `400` bad `after` / `403` / `429` / `5xx` |
| `/report/<id>/thread` | `POST` | a reporter message, JSON `{"text":"…"}` | `201 {"seq":<n>}` / `400` / `403` / `409 {"error":"full"}` / `429` / `5xx` |
| `/report/<id>/thread/seen` | `POST` | JSON `{"upTo":<seq>}` | `204` / `400` / `403` / `429` / `5xx` |
| `/report/<id>/thread` | `DELETE` | Forget: revoke the secret and destroy the thread | `204` / `403` (also the answer to a second call) / `5xx` |
| `/report/admin/<id>/reply` | `POST` | form `text`, or `canned=ask-file` / `canned=ask-steps`; redirects to the detail page | |
| `/report/admin/<id>/status` | `POST` | form `status` (`open`/`need-info`/`fixed`/`closed`), `version` (required for `fixed`), `note`; redirects to the detail page | |

Every response carries `Cache-Control: no-store`; none carry CORS headers —
the web build posts same-origin under `app.hew3d.com` (and under
`beta.hew3d.com`), and the desktop client isn't a browser, so there is nothing
for CORS to gate here (unlike `share-relay`, which serves a phone's browser
directly).

**START** (`POST /report/`) validates, in order (cheapest first): the
declared `Content-Length` for THIS piece (`411`/`413`), `Content-Type:
application/gzip` (`400`), the `Hew-Upload-Length` header — the WHOLE
report's declared total compressed size — present, parseable, and within
`maxBytes` (`400`/`413`), that `IP_HASH_SECRET` is configured (`503`), the
piece body itself read and bounded to `pieceBytes` (`413`) and checked
against the declared total (`400`), and its decompressed HEAD validated
(`400` — see "Head validation" below; only this one piece is ever
decompressed). Only once all of that passes does the fused `ReportIndex`
call happen (rate limit `429`, storage ceiling `507`), and only once THAT
passes does the piece get written to a fresh `ReportDrop` and a fresh
upload token minted and returned.

**PIECE** (`PUT /report/<id>/<index>`) accepts exactly the next expected
index (stores it, `204`), silently accepts an identical retry of the last
stored index without storing it twice (also `204` — the client's own retry
after a dropped response), and refuses anything else as `409
out-of-order` naming the index it actually expects. A piece over
`pieceBytes`, one that would push the upload past its declared total, a
piece that is not the LAST one yet is not exactly `pieceBytes` long (both
clients split at that size; a smaller or empty middle piece is only ever a
keepalive — see "Abandonment"), or any piece arriving more than
`UPLOAD_LIFETIME_MS` after START, is a `400` that **abandons the upload**
— but ONLY once the presented
`Hew-Upload-Token` has been checked against it: report IDs are public (a
user pastes one into a GitHub issue), so an ID alone must never be enough
to destroy a report. A missing or wrong token on an oversized piece is a
`403` that leaves the target completely untouched — the same as a
missing/wrong token on a normal-sized piece — whether the target is still
uploading or already committed; only a MATCHING token against a
still-uncommitted upload actually abandons it. (`src/reportStore.ts`'s
`abandon`, exposed as `ReportDrop.abandon`, is what checks the token before
any of this destroys anything.)

**COMMIT** (`POST /report/<id>/commit`) checks every declared byte arrived
(`400` + abandon if not), marks the report committed, sends the maintainer
notification (best-effort, never fails the commit — see "Notification
email"), and returns `201`. It is **idempotent for the token holder**: a
second COMMIT against an already-committed report, with the same token,
answers `201` again with the same fields rather than `404` — a client that
lost its first `201` response (a dropped connection, a timeout) and retries
must land on the same success, not be told the report doesn't exist. Only
the FIRST call that actually flips the report from uncommitted to committed
sends the notification email, so a retry never sends a second one; a wrong
token against an already-committed report is `403`, never a free read of
its fields.

IDs are `HEW-` plus 8 Crockford base32 characters (`src/id.ts`) from
`crypto.getRandomValues`, formatted `HEW-XXXX-XXXX` everywhere — the START
response, the admin URL, the DO name, the index's primary key. An ID
identifies a report; it authorizes nothing (no public route reads by ID) —
that's what the upload token, minted fresh per upload and never persisted
in the clear, is for.

## Following a report

The design is `docs/design/follow-a-report.md` (§5 is the service). A
report sent with the request header `Hew-Follow: 1` on START gets a
**follow secret**: 32 random bytes, base64url, 43 characters, minted by the
same generator as the upload token and returned once as `"follow"` in the
START `201` body. The Worker keeps only its SHA-256 hex, first in the
drop's meta row and then, once COMMIT succeeds, in a third Durable Object,
**`ReportThread`** (one per followed report, `src/reportThread.ts` over
`src/threadStore.ts`). Minting at START means a retried COMMIT after a lost
`201` needs no new secret: the client already holds it, and thread creation
is idempotent. The secret travels only in the `Hew-Follow-Secret` header,
is compared in constant time, and is never logged or emailed.

**The thread** is a single ordered timeline (`seq`): maintainer replies,
reporter messages, follow-up attachments, and status changes
(`open`/`need-info`/`fixed`/`closed`, each a `system` message). Storage is
lazy exactly like `ReportStore`: a request naming an id with no thread
reads `sqlite_master` and persists nothing. **Every** thread-route failure
— no secret, wrong or revoked secret, unknown or malformed id, a report
nobody follows — is the same `403 {"error":"forbidden"}`, so no public
route says whether a report exists.

**GET thread** answers
`{"format":1,"status":"…","statusVersion":"1.3.2"|null,"statusAt":<ms>,"latest":<seq>,"messages":[…],"now":<ms>}`;
each message is `{"seq","at","author":"maintainer"|"reporter"|"system","kind":"text"|"status"|"attachment","text"}`
plus `"attachmentReportId"` on attachments only. Polling is a write (it
records `lastPolledAt` for retention). A reporter message on a `fixed` or
`closed` thread reopens it (status `open` plus a `system` line); on a
`need-info` thread it moves the status to `open`, as an attachment does.

**Limits** (`src/constants.ts`): messages 1–4,000 characters (code
points); 200 messages per thread, of which the last 10 are kept for the
maintainer — a reporter message (two slots when it reopens the thread) or
a follow-up attachment message is `409 full` once it would take the thread
past 190, while maintainer replies and status changes may use all 200, so a
reporter can never fill a thread the maintainer still has to answer or
close; 20 reporter messages per thread per UTC
day (`429` with `Retry-After` to midnight); 60 thread-route requests per
client hash per 10 minutes across all threads, counted in `ReportIndex`'s
`rate_window` under a `thread:` key prefix **only after** the thread DO has
accepted the secret, so a wrong-secret flood never writes to the index. A
reporter post checks the secret with `verifySecret` before counting, so a
rate-limited post stores nothing; a `GET` or `seen` over the limit has
already been served by the thread DO and answers `429` anyway; `DELETE` is
never turned into a `429`.

**Follow-up attachments** reuse the whole upload protocol. START carries
`Hew-Follow-Up: <parentId>` and `Hew-Follow-Secret: <parent's secret>`; the
parent thread's `verifySecret` runs, and the parent's index row must exist
and be committed, before the body is read or anything is reserved (`403`
otherwise), `Hew-Follow` is ignored (attachments never get
threads), and the new row records `parentId`. COMMIT appends an
`attachment` message to the parent thread (idempotent per child id, so a
retried COMMIT adds nothing) whose text is `"<name> (<size>)"`: the name is
read from the bundle description's first line, which clients write as
`File for report HEW-XXXX-XXXX: <name>`, and the size is the stored
compressed bundle; another description shape gives `"A file (<size>)"`.
It flips a `need-info` parent to `open`; on a `fixed`/`closed` parent the
status stays but the 30-day grace clock stops; it starts over from the
reporter's next `seen`. A parent thread already full, or gone,
leaves the attachment a plain child report with no thread message. It
sends the attachment email
**instead of** the new-report email. A follow-up counts against the
ordinary submission limits and the storage ceiling like any report.

**Retention.** The thread's alarm is re-armed on every write to the
earlier of 30 days after the reporter's client acknowledged a final status
(`seen` through the last status message while `fixed`/`closed`) and 365
days after the last activity (any message, status change, poll, or seen).
Forget, admin delete, and that alarm destroy it; the alarm and Forget tell
`ReportIndex` first (`threadEnded`, which also resets the row's cached
status to `open`) and only then wipe. Before that call they mark the
thread `dying`, so a post that reaches the DO while the call is in flight
is refused (`403`, or not-found for the maintainer) rather than written
into a thread about to vanish; a failed call clears the mark so the retry
starts clean, and a mark left by a destroy that died mid-flight makes the
alarm due at once (Forget also accepts the secret to finish it). The bundle keeps its
own 90-day alarm, but its retention now calls `ReportIndex.bundleExpired`:
a followed, committed row stays with `bundleDeleted = 1` and `sizeBytes =
0` (the ceiling frees) so the maintainer can still answer and set "fixed
in"; any other row is deleted as before, and when the row goes, the
maintainer-only thread a reply or status on an unfollowed report created
is destroyed with it (`expireReport` in `src/handlers.ts`; a no-op for a
report with no thread). When the thread later ends, a
bundle-less row is deleted and a live one just stops showing as followed.

**Index caches.** `reports` gains `following`, `status`, `statusVersion`,
`reporterActivityAt`, `adminViewedAt`, `parentId`, and `bundleDeleted`,
added to an existing table by `ALTER TABLE … ADD COLUMN` guarded by
`PRAGMA table_info` (`ReportDrop`'s `meta` gains `followHash`/`parentId`
the same way). They are display caches; the thread is the source of truth.

**Admin.** The list gains Status and Following columns, a **new** marker
(the reporter wrote or attached something since the maintainer last opened
the report), and a "↳ for HEW-…" marker on follow-up rows. The detail page
shows Following yes/no, the thread (every message escaped, the
maintainer's own included), a reply form with two canned-reply submit
buttons, a status form, and the report's attachments; opening it moves the
thread's admin cursor and the row's `adminViewedAt`. A reply or status on a
report nobody follows creates a thread with no secret, kept for the
maintainer only. A bundle-less row shows its metadata and thread with no
download. Delete destroys the thread first, so the reporter's key stops
working at once, and then deletes the report's follow-up attachments with
it (they are the reporter's material for that report; a deletion request
quoting the ID covers them). A status change whose list-cache update fails still
answers `303` (the thread has the status; only the badge is stale), so a
retried form never writes a second status message. Still no script on any
admin page.

**Email.** Two more subjects share the 50-a-day counter and the
best-effort rule, but stop at 35 (`EMAIL_THREAD_RESERVE` = 15 is kept for
new-report emails, which may use the whole cap), so a chatty thread never
starves the notification for a new report: `[Hew bug report] HEW-… — reply from the reporter` (the
message text in the body) and `[Hew bug report] HEW-… — file attached
(46.5 MB)` (the size in the subject, the reporter-chosen file name only in
the body).

## Chunked upload

**The numbers.** The Workers Free plan caps CPU time at a hard,
non-configurable **10 ms per request** — not wall-clock time (I/O waits on
`fetch`/RPC calls don't count), but actual CPU execution does, and copying
bytes around is real CPU work. A single request handling a 60–90 MiB
upload — reading the body, decompressing or re-chunking it, issuing the
Durable Object writes — does easily tens of milliseconds of CPU work in the
worst case; there is no way to arrange that copying (streamed or buffered)
to fit under 10 ms. Splitting the upload into many small requests instead
means each request only ever touches one piece — small enough to read,
validate, and write in comfortably under a millisecond of CPU time — with
the client, not this Worker, paying for the wall-clock time of issuing
~50 requests for a 90 MiB report.

**The piece size**: `PIECE_BYTES` = 1,900,000 bytes (`src/constants.ts`).
Chosen to sit comfortably under the 2 MB SQLite row/BLOB cap a Durable
Object's storage enforces, so every piece is written as **exactly one row**
with no rechunking and no multi-piece batching on the write side — a piece
arrives, gets read whole (`src/bytes.ts`'s `readBodyCapped`, trivially safe
at this size), and is inserted as one `INSERT` with one bound BLOB
parameter. A 90 MiB report is about 50 requests.

**Head validation** happens exactly once, synchronously, inside the START
handler, against ONLY the first piece — `src/headScan.ts`'s
`decompressAndValidateHead` runs the first piece through a
`DecompressionStream('gzip')` capped at `HEAD_MAX_DECOMPRESSED_BYTES` (256
KiB — comfortably past `format`, all of `report` including a `description`
at its 10,000-character maximum, and `system`, for any legitimate bundle),
then hands the result to `validateHead`: a string/bracket-aware scanner (so
a description containing `{`, `}`, or `,` inside a quoted string never
confuses it) that checks the decompressed text begins `{"format":1,
"report":{…}` in that exact key order, that `report.description` is
10–10,000 characters, and — if a `system` key immediately follows `report`
— that it's an object whose `appVersion`/`platform` are at most
`SYSTEM_FIELD_MAX_CHARS` (128) characters: both are stored verbatim in the
`ReportIndex` row, whose bytes the storage ceiling does not count, so an
unbounded value there would be uncounted storage. `system` is **optional**: the dialog lets a user
untick "App version and system" entirely, and a report submitted without
it is still accepted, with `appVersion`/`platform` recorded as empty
strings (the admin page and the notification email display "unknown").
Because this all happens at START, before `ReportIndex.reserveReport` is
ever called, the reservation's `appVersion`/`platform`/description preview
are the REAL values from the very first call — there's no placeholder
metadata for COMMIT to fill in later. Every piece after the first is never
decompressed or inspected by this Worker at all: `report`/`system` always
land well inside the first 1.9 MB piece, so nothing later in a multi-piece
upload is ever touched by the head scan.

**Bounding decompression matters even for a single, already-size-capped
piece**: a small, highly compressible piece could otherwise decompress to
something huge (a gzip bomb) — `HeadDecompressor` stops accepting
decompressed output once it hits its cap regardless of how much compressed
input is still coming, which is what actually keeps this bounded.

## Storage

Three Durable Object classes, all SQLite-backed, all bound in
`wrangler.toml` with `new_sqlite_classes` migrations (`v1`: `ReportDrop`,
`ReportIndex`; `v2`: `ReportThread`, described under "Following a
report") — no R2 bucket, same free-tier-fails-closed posture as
`share-relay`'s `ShareDrop`:

- **`ReportDrop`** (`src/reportDrop.ts`, `src/reportStore.ts`) — one
  instance per report, running its own small state machine over two
  tables: a single `meta` row (the declared total, the hashed upload
  token, `nextIndex`, `receivedBytes`, `lastActivity`, and — set once at
  START, from the head-validated fields — `description`/`appVersion`/
  `platform`) and a `piece(idx, data, length)` table with exactly one row
  per uploaded piece, written exactly once each, in the order they arrive.
  No rechunking, no batching — a piece is already small enough
  (`PIECE_BYTES`, comfortably under SQLite's 2 MB row/BLOB cap) to write as
  a single bound parameter. Reading a report back never holds more than one
  piece in memory either: the admin **download** route streams
  (`ReportStore.readStream`, exposed as `ReportDrop.readStream`) a byte
  `ReadableStream` that pulls one piece row at a time straight out of
  SQLite — Workers RPC forwards a byte-oriented stream across the Durable
  Object boundary without buffering it whole — and the admin **detail**
  route reads only piece 0 (`ReportStore.read(0, 1)`) to render its HEAD.
  Unlike `share-relay`'s one-shot `ShareDrop`, a report is read (list
  preview, detail render, download) as many times as the admin page wants —
  there is no consume/claim step — until an admin delete or an alarm wipes
  it.
- **`ReportIndex`** (`src/reportIndex.ts`, `src/indexStore.ts`) — a single
  instance, `idFromName("index")`. One row per report (id, received time,
  app version, platform, the first 200 characters of the description,
  size, triaged flag, `lastActivity`, and a `committed` flag — see
  "Ceiling" below) backs the admin list; the storage ceiling is a
  `SUM(sizeBytes)` over that same table, not a separate counter, so
  deleting a report's row is the entire "give the space back" step.
  Per-client rate-limit counters and the daily email counter live here too
  (below) — the docs don't say whether the `ratelimit` binding is
  available on the free plan, so this Worker doesn't depend on it.

**Retention**: 90 days (`RETENTION_DAYS`, `src/constants.ts`). `COMMIT`
re-arms `ReportDrop`'s Durable Object alarm for `receivedAt + RETENTION_MS`
(reusing the same alarm slot the upload used for abandonment, rather than
cancelling and re-scheduling); when it fires, the bytes are wiped and
`ReportIndex` is told to drop the corresponding row. The admin page can
delete a report sooner.

**Ceiling**: 3 GiB (`STORE_CEILING_BYTES`) of the account's shared 5 GB
free Durable Object storage — `share-relay`'s ten-minute drops use the
rest. (That fits about 34 reports the size of the design's 59.9 MB worked
example, or 32 at the 90 MiB cap.) The rate-limit check, the ceiling check,
and reserving a report's DECLARED total are ONE fused Durable Object call,
`ReportIndex.reserveReport` — not a read-only check followed by a later
write, and made against the declared `Hew-Upload-Length` at START, before
the rest of the upload has even begun. That fusion is what makes the
ceiling actually hold under concurrency: `reserveReport` inserts an
*uncommitted* `reports` row the instant it accepts the START request,
before a second piece has arrived, so the reservation counts against every
other concurrent caller's ceiling check immediately rather than only once
the (potentially 50-request) upload eventually finishes. `handleCommit`
calls `ReportIndex.commitReport(id)` once `ReportDrop.commit` succeeds;
every abandonment path (a `400` on a piece or commit, an immediate cleanup
call) calls `deleteReport(id)` instead to release the reservation —
`listReports()` and the admin detail/download/triage routes only ever
treat a committed row as a real report. `POST /report/` returns `507` once
a reservation would pass the ceiling.

## Abandonment

An upload that stops partway — the user closes the dialog, the network
drops, the client crashes — must not hold its reserved bytes against the
ceiling forever, nor leave an orphaned `ReportDrop` around indefinitely.
Two independent mechanisms, deliberately overlapping:

- **The `ReportDrop`'s own alarm** (`ABANDON_TIMEOUT_MS`, 10 minutes) is
  the primary path: armed at START and re-armed on every newly stored
  piece, so it always reflects time since the *last activity*, not time
  since the upload began. If it fires while the upload is still
  uncommitted, `alarm()` reads the report ID, tells `ReportIndex` to delete
  the corresponding row, THEN wipes the drop (`destroy()`) — that order,
  not the reverse, matters: if `ReportIndex.deleteReport` fails (a
  transient RPC error) after the drop were already destroyed, a retried
  alarm would find no `head()` left, skip the `ReportIndex` call entirely,
  and orphan that row forever (it would keep counting against the storage
  ceiling with nothing left to ever free it). Doing the index delete first
  means a retry after ITS failure still finds the same `head()` and tries
  again — `deleteReport` is idempotent, so a retry after it actually
  succeeded (but `destroy()` never ran) is harmless too. This is the exact
  same order COMMIT's retention alarm eventually runs too; `alarm()` needs
  no branch on which case it is.
- **`ReportIndex`'s idle-upload backstop**
  (`findAndDeleteIdleUncommittedIds`, `IDLE_PRUNE_TIMEOUT_MS`, 15 minutes)
  runs opportunistically before every `reserveReport` call, independent of
  any one drop's alarm — it exists for whatever that alarm somehow missed
  (a crash between the alarm firing and its own cleanup completing, for
  instance). It's keyed on `lastActivity`, **not** age since the upload
  started: a legitimate, still-progressing 90 MiB upload can genuinely
  take longer than 15 minutes end to end on a slow connection, but it
  keeps touching `lastActivity` on every piece the whole time, so it's
  never at risk of being pruned mid-upload — only an upload that has
  actually gone quiet is. `IDLE_PRUNE_TIMEOUT_MS` is longer than
  `ABANDON_TIMEOUT_MS` on purpose, since the per-drop alarm is meant to be
  the one that actually fires first in the normal case.

- **An absolute lifetime** (`UPLOAD_LIFETIME_MS`, two hours since START),
  independent of activity, because both mechanisms above measure only
  silence while an uncommitted upload holds its whole declared size
  against the storage ceiling from the moment START accepts it. Without
  it, a caller could hold a 90 MiB reservation open forever by sending
  one trivial piece every nine minutes — and with 20 STARTs a day per
  client hash, fill the 3 GiB ceiling for everyone in two days. Three
  places enforce it: the drop's alarm is never armed later than
  `receivedAt + UPLOAD_LIFETIME_MS`; `ReportStore.putPiece` refuses a
  piece after that instant as `expired` (a `400` that abandons); and the
  index prune also drops uncommitted rows whose `receivedAt` is past it.
  The companion rule that every piece but the last is exactly `pieceBytes`
  (`short-piece`, also a `400` that abandons) caps an upload at
  `ceil(declaredTotal / pieceBytes)` requests, so there is no cheap
  keepalive to send in the first place. Two hours is far beyond the
  slowest legitimate upload: 90 MiB in 1.9 MB pieces at the clients'
  two-minute per-request timeout is about 100 minutes.

An explicit `400` on a piece or commit (oversized piece, bytes past the
declared total, a short or late piece, an incomplete commit) doesn't wait
for either of these —
`handlers.ts`'s `abandonUpload` runs immediately, deleting the drop and
releasing the reservation as part of answering that same request.

START itself has no idempotency of its own — each call generates a fresh
report ID and reservation, so a client that loses its `201` response (a
dropped connection, a timeout) and retries START creates a genuinely
SECOND reservation under a different ID, not a resumed one. That second
reservation is an ordinary in-progress upload like any other: if the
client never comes back to it, its own `ABANDON_TIMEOUT_MS` alarm (or the
idle-upload backstop) releases it the same way it would any other
abandoned upload — nothing about a START retry needs special-casing.

## Abuse controls

All in `ReportIndex` (`src/indexStore.ts`), all counted with a handful of
upserted rows rather than one row per event — row writes are the free
tier's least generous quota (100k/day account-wide):

- **Per-client limits**: 5 submissions per **fixed** 10-minute window
  (`bucket = floor(now / RATE_LIMIT_WINDOW_MS)`, not a sliding window —
  so up to 10 can legitimately land across one window boundary, 5 in the
  tail of one bucket and 5 in the head of the next) and 20 per UTC day,
  keyed on `SHA-256(IP_HASH_SECRET + UTC date + CF-Connecting-IP)`
  (`src/clientHash.ts`), checked once at START — a "submission" is a
  whole upload attempt, not a per-piece cost. The raw IP is **never**
  stored anywhere — only this salted, date-scoped hash, which changes
  every UTC day on its own (the date rides the hash input), so "one row
  per client per day" falls out for free in the `rate_day` table. Missing
  `IP_HASH_SECRET` refuses every START with `503` rather than ever hashing
  an IP unsalted. **Abandoning an upload never gives its rate-limit slot
  back** — only its storage reservation is released (`deleteReport`); the
  `rate_window`/`rate_day` counters `reserveReport` increments are never
  decremented by an abandonment, START retry, or admin delete. This is
  deliberate: if abandonment freed a slot, a client could START-then-
  immediately-abandon in a loop to bypass the per-window/per-day caps
  entirely.
- **Email cap**: 50 notifications a day, account-wide (`EMAIL_DAY_MAX`),
  independent of the per-client counters — many distinct-IP spammers each
  submitting once should not multiply into many emails. Thread-event
  emails stop at `EMAIL_DAY_MAX - EMAIL_THREAD_RESERVE` (35). Reports past the
  cap still store and appear in the admin list; only the email is skipped.
- Both rate tables prune stale rows opportunistically on write, not on a
  schedule, so they stay small without this DO needing its own alarm
  (which would collide with "the alarm" meaning abandonment/retention,
  `ReportDrop`'s job, everywhere else in this codebase).
- **A guessed id costs nothing durable**: `PUT`/`commit` (and an admin
  route) naming a well-formed id that was never started still instantiate
  that id's `ReportDrop`, but `ReportStore` creates its tables only in
  `startUpload` — every other method first asks `sqlite_master` whether
  the schema exists and answers "not found" without creating it. The
  earlier constructor-runs-`CREATE TABLE` shape left a few KiB of empty
  schema behind per such request, forever (no alarm, no cleanup), which
  an unauthenticated caller could have minted by the hundred thousand
  against the account-wide Durable Object storage and daily row-write
  quotas shared with share-relay. share-relay's `DropStore` follows the
  same rule.

Turnstile is out of scope here — see `docs/design/report-bug.md` §4 for why
(the desktop path can't run a browser challenge; the web path would need to
loosen the app's `connect-src 'self'` CSP).

## Notification email

One email per stored report, sent from COMMIT once the report is durably
committed, through the `send_email` binding (`NOTIFY` in `wrangler.toml`,
no `destination_address`: the recipient comes from the `NOTIFY_TO` secret
at send time, and Cloudflare still restricts delivery to a verified Email
Routing destination address regardless).

`src/email.ts` builds the raw MIME message by hand — one `text/plain` part,
no HTML, no attachments (they're private, and outbound mail is capped well
above what a plain-text report needs) — using the legacy
`EmailMessage`/`cloudflare:email` API rather than the newer
`env.NOTIFY.send({...})` convenience surface, which belongs to a separate
"Email Sending" product with its own independent domain onboarding; this
design's `send_email` binding plus an Email Routing verified destination
(deploy checklist, below) is the one `docs/design/report-bug.md` §8
specifies. No dependency was added for this: a single-part UTF-8 text
message is a handful of header lines, cheaper to write by hand than to
pull in `mimetext` for. Source:
<https://developers.cloudflare.com/email-routing/email-workers/send-email-workers/>.

`appVersion`/`platform` are attacker-controlled JSON fields that ride the
Subject header — `sanitizeHeaderValue` strips CR/LF from them before they
land there. `system` is optional (a user can untick "App version and
system" entirely), so both can arrive as empty strings; the email shows
"unknown" for either rather than a blank field. The description rides the
body verbatim; it needs no CR/LF treatment, because a MIME parser never
resumes reading headers after the blank line that ends them, so nothing in
the body can inject a header regardless of what it contains. The size the
email reports is the COMPRESSED (declared total) size — this Worker never
decompresses a whole report just to notify about it.

Missing `NOTIFY_TO`/`NOTIFY_FROM` skips the email and logs once; a send
that throws (bad credentials, Cloudflare-side failure, whatever) is caught
and logged, never surfaced to the submitter — the report is already
durably committed by the time this runs, so a notification failure changes
nothing about whether the commit succeeded. `sendNotification`'s whole body
runs inside one try/catch, not just the mail-send call — the
`ReportIndex.reserveEmail` call it makes first is a Durable Object RPC like
any other and can fail the same ways.

`src/email.ts` itself never imports `cloudflare:email` — that import lives
in `src/emailMailer.ts` alone, wired in only by `src/index.ts`. This is the
same split `share-relay` uses for `ShareDrop`/`DropStore` (a class that
`extends DurableObject` needs `cloudflare:workers`, which bare `node --test`
can't resolve): `sendNotification` takes an injected `RawMailSender`
instead, so the unit suite exercises the real skip/cap/error-handling logic
with a fake mailer, with zero risk of a stray import breaking `node --test`.

## Admin page

Server-rendered HTML under `/report/admin/*`, no client-side JavaScript at
all. Every request is verified against the `Cf-Access-Jwt-Assertion` header
Cloudflare Access attaches (`src/adminAuth.ts`): RS256 over WebCrypto
against the team's JWKS
(`https://<ACCESS_TEAM_DOMAIN>/cdn-cgi/access/certs`, cached a few minutes
per team domain), checking `iss`, that `aud` contains `ACCESS_AUD`, and
expiry. This is defense in depth on top of Access itself already fronting
the route — **every** failure mode (missing header, bad signature, wrong
issuer/audience, expired, unset `ACCESS_*` secrets) collapses to the same
bare `403`, deliberately: there is no way to distinguish "not logged in"
from "misconfigured" from the response.

Every report-derived value that reaches an admin page — the description,
`appVersion`/`platform`, system info, the description preview in the list
— is attacker-controlled (a submitter writes the whole JSON body), so
`src/adminPages.ts`'s `escapeHtml` runs on every one of them before it
reaches a template string; there is no exception. A strict
`Content-Security-Policy` (`default-src 'none'; style-src 'unsafe-inline';
form-action 'self'; frame-ancestors 'none'`) rides every admin response as a
second layer, not a substitute for escaping. Mutating routes (`delete`,
`triage`) also check the request's `Origin` header against
`https://app.hew3d.com` — the admin pages' own origin — before doing anything.
Admin pages are only ever served from `app.hew3d.com`, so reports sent from
the beta host are administered there too. An upload still in progress (or
abandoned and not yet pruned) is `404` in the detail/download routes and
simply absent from the list — it isn't a real report yet.

Neither admin route reassembles a report's pieces into one buffer any
more — the old approach (read every stored piece into an array, concatenate
them, and for detail fully `gunzip` the result) cost up to roughly 2x the
report's size in isolate memory (as much as ~180 MiB for a report near the
90 MiB cap) and far more than the 10 ms/request CPU budget this Worker
otherwise stays under everywhere else:

- The **detail** page reads and decompresses ONLY piece 0
  (`ReportStore.read(0, 1)`, then `headScan.ts`'s `HeadDecompressor` —
  the same bounded decompressor `handleStart`'s own head validation uses)
  and parses it with `headScan.ts`'s `parseHeadForDisplay`, a tolerant
  sibling of `validateHead` that reuses its bracket-aware scanner but never
  rejects anything — a malformed, truncated, or older-format head still
  renders whatever prefix parses. It shows `description`/`expected`/
  `contact`, the optional `system` block, and an optional `crash` block (the
  client writes `crash` immediately after `system`, or after `report` when
  `system` is absent), plus the report's stored compressed size — and says
  that any recorded steps, imported files, the model file, and the
  diagnostic log are in the downloaded bundle, rather than claiming whether
  each one is included (piece 0 alone can't answer that for a multi-piece
  report). If the head can't be read or decompressed at all, the page still
  renders, just without those fields.
- The **download** route streams the exact stored, still-compressed bytes
  straight out of the `ReportDrop` (`ReportStore.readStream`, a byte
  `ReadableStream` that pulls one piece row at a time) as `<id>.json.gz`
  with `Content-Type: application/gzip` and a `Content-Length` set from the
  stored total — the Worker never holds more than one piece of a report in
  memory at once, for a report of any size up to `MAX_UPLOAD_BYTES`.

## Testing

```sh
cd workers/bug-intake
npm test   # node --test src/*.test.ts
```

No install step needed to run the tests themselves (Node's built-in test
runner and native TypeScript stripping, `.node-version` pins 26.7.0);
`npm install` is only needed for `wrangler dev`/`deploy`. Mirrors
`share-relay`'s approach throughout:

- `src/testSupport/fakeDurableObject.ts` is a `node:sqlite`-backed fake DO
  namespace/storage stack (copied from `share-relay`'s file of the same
  name — see its header comment for the BLOB-boundary and
  `deleteAll`-is-`DROP`-not-`DELETE` details). `ReportStore`/`IndexStore`
  (the framework-free logic behind `ReportDrop`/`ReportIndex`) satisfy the
  DO stub interfaces directly, so tests wrap them with no DO-class
  boilerplate at all.
- `src/testSupport/testEnv.ts`'s `buildTestEnv()` is the whole Worker
  in-process: `handleRequest` over fake namespaces for all three DO
  classes (wired like the real wrappers: the thread's destroy hook calls
  `ReportIndex.threadEnded`), a recording mailer, and a test Access JWKS
  plus signed JWT (`makeTestAccess`) passed as `VerifyOptions`. It also
  exposes each DO's storage and alarm time and fires alarms on demand
  (`runThreadAlarm`, `runDropAlarm`). `src/follow.test.ts` uses it; an
  end-to-end harness can put a small HTTP server in front of `handle`.
- `src/handlers.test.ts` drives `handleRequest` end to end against that
  fake namespace, with real gzip'd pieces built via `CompressionStream`
  (the same runtime API the Worker uses, not a shortcut): every status in
  the API table above for START, PIECE, and COMMIT — the full validation
  order, ID/token format, rate limiting (both windows), the storage
  ceiling, that the client IP never appears anywhere reachable from the
  index — a real start → many-pieces → commit flow including a genuine
  ~90 MiB upload asserted to land as exactly one stored row per piece (no
  rechunking), an idempotent piece retry, out-of-order with the expected
  index named, wrong/missing token as `403`, an unknown/committed/
  abandoned upload as `404`, bytes past the declared total and a short
  commit both `400`-ing and abandoning the upload (reservation released,
  drop deleted), the alarm's own logic (tell `ReportIndex` FIRST, then
  destroy) releasing a stale upload, the index's idle-prune backstop keying
  on activity time rather than upload age, and the full admin surface
  (auth, list/detail/download, Origin-checked mutations) using a real
  RS256-signed JWT generated in-process with WebCrypto against a fake JWKS
  endpoint. Also covered: a tokenless or wrong-token oversized `PUT`
  against either a committed report or an in-flight upload leaves it
  completely untouched (`403`/`404`, never abandoned) while a matching
  token still abandons it; `COMMIT`'s idempotency for the token holder (a
  second commit with the right token answers `201` again with no second
  email, a wrong token on an already-committed report is `403`, and a
  retry after `ReportIndex.commitReport` fails once still lands — `503`
  then `201`, the row committed, exactly one email); download streaming
  compared byte-for-byte against the uploaded gzip across several pieces;
  and detail rendering from just the head for a multi-piece report
  (including its `crash` block) and still rendering a page when the head
  can't be read. Also: using envs whose `REPORT_INDEX`/`REPORT_DROP` stub
  is swapped for one that deliberately throws on a named method, that
  every Durable Object failure across the START/PIECE/COMMIT and admin
  paths maps to `503 {"error":"unavailable"}` rather than an uncaught
  exception, including at `handleRequest`'s own top-level catch-all for
  whatever a closer try/catch didn't anticipate.
- `src/headScan.test.ts` covers `validateHead`'s scanner directly (correct
  key order, a description at each length boundary, string-aware scanning,
  `system` present vs. entirely absent vs. present-but-incomplete-fields
  vs. truncated exactly at the report/system boundary, wrong key order, a
  non-`1` format, adversarial/truncated input that must fail cleanly
  rather than throw) and `HeadDecompressor`/`decompressAndValidateHead`
  directly: decompression fed in one piece, bad gzip flagged without
  throwing (including the write/cancel race this exposed — the cap being
  reached mid-write must never be misreported as "bad gzip"), and the cap
  stopping decompression at exactly `HEAD_MAX_DECOMPRESSED_BYTES`. Also
  covers `parseHeadForDisplay` (the admin detail page's tolerant, display-
  only sibling of `validateHead`): description/expected/contact and the
  system block, a `crash` block whether it follows `system` or (`system`
  absent) `report` directly, a head missing every optional field, and that
  it never throws on adversarial or truncated input, returning `null` or a
  partial result instead.
- `src/adminAuth.test.ts` covers JWT verification specifically: valid,
  wrong `aud`, wrong `iss`, expired, missing, unset secrets, unknown `kid`,
  a signature from the wrong key, a tampered payload, a non-RS256 `alg`,
  and the JWKS cache's fetch/expiry behavior.
- `src/reportStore.test.ts` covers the per-upload state machine directly:
  a multi-piece upload round-tripped exactly at the 90 MiB boundary
  (piece by piece, proving one row per piece with no rechunking), reads
  being non-destructive, every `putPiece`/`commit` outcome (stored,
  idempotent retry, retry with a mismatched length rejected, skip-ahead
  and far-behind both rejected as out-of-order with the right expected
  index, wrong token, oversized piece, bytes past the declared total,
  not-found for both "never started" and "already committed", incomplete
  commit), that a second `commit` with the right token is idempotent
  (same fields, no second alarm-arm) while a wrong token on an
  already-committed drop is forbidden, `abandon`'s own token check (matches
  → destroyed, wrong token or already-committed → survives untouched,
  never-started → not-found), `readStream`'s byte-for-byte output at both
  single- and multi-piece sizes, and that the alarm is armed at start,
  re-armed on every piece, and re-armed for `RETENTION_MS` from the
  ORIGINAL `receivedAt` (not from commit time) once committed.
- `src/indexStore.test.ts` is where `reserveReport`'s fused rate-limit +
  ceiling + reservation logic is tested most directly: two reservations
  racing near the ceiling (only one may be admitted), that an uncommitted
  reservation's bytes count against the ceiling exactly like a committed
  one, that committing doesn't double-count, that deleting (releasing) one
  gives its bytes back, plus a dedicated activity-tracking suite:
  `reserveReport` stamps `lastActivity`, `touchActivity` extends how long
  a row survives the idle prune (and is a no-op once committed),
  `findAndDeleteIdleUncommittedIds` deletes only what's actually idle and
  returns their ids, a committed row is never pruned however old, and —
  the key distinction the idle-prune design depends on — a row with
  *recent* activity is never pruned even if it was *started* long ago. Also
  covers `commitReport`'s own idempotency outcome (`'flipped'` the first
  time, `'already-committed'` on repeat, `'not-found'` for an unknown id).
- `src/adminPages.test.ts` (HTML-escaping/XSS, "unknown" for an absent
  `system`, the downloaded-bundle note replacing the old included/not-
  included claims) and `src/email.test.ts` (header-injection stripping,
  "unknown" for an absent `system`, skip/cap/failure behavior, including
  that a `reserveEmail` Durable Object failure is caught the same as a
  mailer failure) round out the rest.

There is no black-box conformance suite here the way `share-relay` has one
against `hew-relay` — there is no second implementation of this contract
to keep honest, only the app lane coding against the same §8 document.

The ~90 MiB test cases in `src/reportStore.test.ts` and
`src/handlers.test.ts` genuinely allocate, base64-encode, and gzip tens of
megabytes of random data and upload it piece by piece — real work, not a
mock — so the full suite takes a few seconds rather than the sub-second run
a purely logic-level suite would.

## Deploy checklist (maintainer)

**Upgrading an existing deployment to following (migration `v2`)**: a
plain `npx wrangler deploy` from this directory applies `[[migrations]]
tag = "v2"` (`new_sqlite_classes = ["ReportThread"]`) and binds
`REPORT_THREAD`. No new secrets. Existing reports keep working: the new
index and drop columns are added in place on first use. Then check each
new route from a real client (a report sent with following on, a reply
from the admin page, the client's next check, an attachment, a status
change, Forget).

Manual, one-time-per-environment, same posture as `share-relay`'s checklist
(that Worker's `README.md` has the fuller version of steps 1–3 below). This
Worker runs entirely on the Workers **Free** plan — the whole point of the
chunked-upload design above is staying under Free's 10 ms/request CPU cap,
so there is no Paid-plan requirement anywhere in this checklist.

1. **Install dependencies**: `cd workers/bug-intake && npm install`.
2. **Authenticate**: `npx wrangler login` (or `CLOUDFLARE_API_TOKEN`).
3. **Enable Email Routing** on the `hew3d.com` zone (dashboard → the zone →
   **Email** → **Email Routing**) if it isn't already (`share-relay` never
   needed it), and verify the maintainer's own destination address there —
   the `send_email` binding can only deliver to a verified address, and
   `NOTIFY_TO` must be that address.
4. **First deploy**: `npx wrangler deploy`. Registers the Worker, applies
   the `new_sqlite_classes = ["ReportDrop", "ReportIndex"]` migration, and
   binds `REPORT_DROP`/`REPORT_INDEX`/`NOTIFY` — all from `wrangler.toml`.
   The route (step 6) makes it reachable.
5. **Set every secret** — `wrangler.toml` deliberately has no `[vars]`
   block (a committed one would publish the maintainer's address in this
   public repository, and unlike secrets, `[vars]` don't survive
   `wrangler deploy`):
   ```sh
   npx wrangler secret put NOTIFY_TO            # the verified destination from step 3
   npx wrangler secret put NOTIFY_FROM          # a sender address on hew3d.com
   npx wrangler secret put ACCESS_TEAM_DOMAIN   # <team>.cloudflareaccess.com
   npx wrangler secret put ACCESS_AUD           # the Access application's AUD tag, from step 7
   npx wrangler secret put IP_HASH_SECRET       # any long random value; submissions 503 without it
   ```
6. **Wire the route**: `wrangler.toml`'s `[[routes]]` block
   (`app.hew3d.com/report/*` on zone `hew3d.com`, and
   `beta.hew3d.com/report/*` for the beta host) takes effect on the next
   deploy — it requires `hew3d.com` to already be a Cloudflare-managed zone,
   and `app.hew3d.com` to already carry the Pages-hosted web app. Redeploy
   (`npx wrangler deploy`) after adding it if it wasn't there for step 4. Any
   later change to the routes needs the same redeploy. Verify the Workers
   route wins over the Pages custom domain on this hostname the same way
   `share-relay`'s checklist does: `curl -s https://app.hew3d.com/report/`
   should return the identity JSON, not the web app's HTML.
7. **Create a Cloudflare Access application** for
   `app.hew3d.com/report/admin/*` (Zero Trust dashboard → **Access** →
   **Applications** → **Add an application** → **Self-hosted**), with the
   maintainer as the only allowed identity. Note its **Application Audience
   (AUD) Tag** — that's `ACCESS_AUD` from step 5. `src/adminAuth.ts` fails
   every admin request closed (`403`) independent of Access's own gate, so
   a misconfigured Access application never silently exposes reports.
8. **Verify**:
   ```sh
   curl -s https://app.hew3d.com/report/
   # {"service":"hew-bug-intake","format":1,"maxBytes":1073741824,"pieceBytes":1900000}

   # The body of each request must be real gzip. With a local bundle.json:
   gzip -c bundle.json > bundle.json.gz
   curl -s -i -X POST https://app.hew3d.com/report/ \
     -H 'Content-Type: application/gzip' \
     -H "Hew-Upload-Length: $(stat -f%z bundle.json.gz)" \
     --data-binary @bundle.json.gz
   # 201 {"id":"HEW-...-...","token":"...","pieceBytes":1900000} for a bundle
   # under one piece — it's both START and the whole upload in this case.
   # Then finish it:
   curl -s -i -X POST https://app.hew3d.com/report/HEW-XXXX-XXXX/commit \
     -H 'Hew-Upload-Token: <token from above>'
   # 201 {"id":"HEW-XXXX-XXXX"}; confirm the notification email arrives at NOTIFY_TO

   # Then, signed into Access as the maintainer:
   open https://app.hew3d.com/report/admin/
   ```
   For anything larger than one piece, split the gzip file into
   1,900,000-byte pieces and `PUT` each to `/report/<id>/<index>` (starting
   at index 1) with `Hew-Upload-Token` before calling `commit` — exactly
   what the real client does. Delete the smoke-test report from the admin
   page afterward.

`share-relay`'s `DASHBOARD-SETUP.md` items (the zone's one free Rate
Limiting Rule, the billing tripwire) are already spent by that Worker;
this one adds no dashboard-only configuration beyond the Access
application in step 7.
