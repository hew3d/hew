/**
 * bug-intake — the private Report Bug intake service (docs/design/report-bug.md
 * §4, §8, the only sections this Worker implements exactly). Written as
 * plain functions over the `BugIntakeEnv` binding interface (`types.ts`),
 * mirroring share-relay's `handlers.ts` split: `handlers.test.ts` runs this
 * against a fake DO namespace (`testSupport/fakeDurableObject.ts`) wrapping
 * the real `ReportStore`/`IndexStore` logic, with no `cloudflare:workers` or
 * `cloudflare:email` import anywhere in this file's dependency graph — see
 * `reportDrop.ts`, `reportIndex.ts`, and `emailMailer.ts` for the thin
 * wrappers that touch those runtime modules, wired in only by `index.ts`.
 *
 * Every response carries `Cache-Control: no-store` (§8) and no CORS headers
 * — the web build posts same-origin under `app.hew3d.com` (and under
 * `beta.hew3d.com`, the pre-release host), and the desktop
 * client isn't a browser, so there is nothing here for CORS to gate.
 *
 * The upload is chunked (§4 "Chunked upload"): the Workers Free plan caps
 * CPU time at 10 ms per request, and copying an upload of tens or hundreds
 * of megabytes through one request doesn't fit — so the client splits it
 * into pieces of at most
 * `PIECE_BYTES` (1.9 MB) and sends START (this handles the first piece and
 * opens the upload), then PIECE for each following piece, then COMMIT. Each
 * of those three is its own exported handler below.
 */

import {
  MAX_UPLOAD_BYTES,
  PIECE_BYTES,
  CONTRACT_FORMAT,
  INDEX_DO_NAME,
  THREAD_MESSAGE_MAX_CHARS,
  THREAD_BODY_MAX_BYTES,
  STATUS_VERSION_MAX_CHARS,
} from './constants.ts'
import { readBodyCapped, TOO_LARGE } from './bytes.ts'
import { decompressAndValidateHead, HeadDecompressor, parseHeadForDisplay } from './headScan.ts'
import { generateReportId, isValidReportId, normalizeReportId } from './id.ts'
import { hashClient } from './clientHash.ts'
import { generateUploadToken, generateFollowSecret, hashToken } from './uploadToken.ts'
import { sendNotification, sendThreadNotification, formatBytes, type RawMailSender } from './email.ts'
import { verifyAccessJwt, type VerifyOptions } from './adminAuth.ts'
import { renderList, renderDetail, renderFormError, escapeHtml, CANNED_REPLIES } from './adminPages.ts'
import { isThreadStatus } from './threadStore.ts'
import type { BugIntakeEnv } from './types.ts'

/** The only origin admin mutation forms are ever legitimately submitted
 *  from — the admin pages themselves are served under this same origin.
 *  Admin pages are only ever served from app.hew3d.com, never from the beta
 *  host, so beta reports are administered there too. */
const ADMIN_ORIGIN = 'https://app.hew3d.com'

function jsonResponse(status: number, body: unknown, extraHeaders?: Record<string, string>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json', 'cache-control': 'no-store', ...extraHeaders },
  })
}

function htmlResponse(status: number, html: string, extraHeaders?: Record<string, string>): Response {
  return new Response(html, {
    status,
    headers: { 'content-type': 'text/html; charset=utf-8', 'cache-control': 'no-store', ...extraHeaders },
  })
}

function notFound(): Response {
  return new Response('Not found', { status: 404, headers: { 'cache-control': 'no-store' } })
}

function parseContentLength(header: string | null): number | null {
  if (header === null) return null
  const n = Number(header)
  return Number.isFinite(n) && n >= 0 ? n : null
}

function indexStub(env: BugIntakeEnv) {
  return env.REPORT_INDEX.get(env.REPORT_INDEX.idFromName(INDEX_DO_NAME))
}

function dropStub(env: BugIntakeEnv, id: string) {
  return env.REPORT_DROP.get(env.REPORT_DROP.idFromName(id))
}

function threadStub(env: BugIntakeEnv, id: string) {
  return env.REPORT_THREAD.get(env.REPORT_THREAD.idFromName(id))
}

/** A thread message's length rule: 1 to `THREAD_MESSAGE_MAX_CHARS`
 *  characters counted as Unicode code points (what a Rust client's
 *  `chars().count()` counts), and not only whitespace. */
function validMessageText(text: unknown): text is string {
  if (typeof text !== 'string' || text.trim().length === 0) return false
  return [...text].length <= THREAD_MESSAGE_MAX_CHARS
}

/** The parent-thread line for a follow-up attachment — "kitchen.skp (46.5
 *  MB)". The file name comes from the follow-up bundle's description,
 *  which clients write as "File for report HEW-XXXX-XXXX: <name>" on its
 *  first line (the head scan reads only `format`/`report`/`system`, never
 *  the `attachment` key itself); the size is the stored compressed bundle,
 *  the one size the Worker knows. A description in any other shape falls
 *  back to "A file". */
export function attachmentLabel(description: string, sizeBytes: number): string {
  const firstLine = description.split(/\r?\n/, 1)[0]
  // A greedy capture trimmed afterwards — not a lazy `(.+?)\s*$`, which
  // backtracks quadratically on a long run of spaces.
  const match = /^File for report HEW-[0-9A-Z]{4}-[0-9A-Z]{4}:(.*)$/i.exec(firstLine)
  const trimmed = match ? match[1].trim() : ''
  const name = trimmed === '' ? 'A file' : [...trimmed].slice(0, 200).join('')
  return `${name} (${formatBytes(sizeBytes)})`
}

/** Unconditionally deletes a report's (possibly partial) `ReportDrop` and
 *  releases its `ReportIndex` reservation — the cleanup abandonment needs,
 *  at any point in the upload (docs/design/report-bug.md §4: "A 400 on a
 *  piece or commit abandons the upload at once"). Best-effort and swallows
 *  its own failures: if THIS throws, `ReportIndex`'s idle-upload backstop
 *  (`IDLE_PRUNE_TIMEOUT_MS`) is what eventually frees the reservation
 *  anyway, and surfacing a cleanup error instead of the original failure
 *  would only be more confusing.
 *
 *  UNCONDITIONAL is the important word: this destroys `id`'s `ReportDrop`
 *  no matter whose request triggered the call, with no token check of its
 *  own. Every call site below only reaches this AFTER a token has already
 *  been verified for `id` by the `ReportStore` call whose result triggered
 *  it (`putPiece`'s `'too-large'`/`'bytes-past-total'`, `commit`'s
 *  `'incomplete'`) — or, in `handleStart`, against an id this same request
 *  just generated and failed to ever store anything under, so there is
 *  nothing yet for a token to protect. An oversized `PIECE` body is
 *  DIFFERENT: it fails before any `ReportStore` call ever sees a token, so
 *  it goes through `ReportStore.abandon`/`ReportDrop.abandon` instead,
 *  which checks the token itself — see `handlePutPiece`. */
/** A `ReportDrop`'s alarm (retention or abandonment) for `reportId`, after
 *  the drop has read its id and before it wipes its bytes:
 *  `ReportIndex.bundleExpired`, then — when that removed the row (nobody
 *  follows the report) — the report's `ReportThread` too. Such a thread can
 *  only be a secretless one the admin created by replying or setting a
 *  status (`adminThreadFor`); with its row gone nothing could ever reach or
 *  delete it again. `ReportThread.destroy` is idempotent and writes nothing
 *  for an id with no thread. Throws on failure, so the drop's alarm keeps
 *  its bytes and retries the whole step. */
export async function expireReport(env: BugIntakeEnv, reportId: string): Promise<void> {
  const rowGone = await indexStub(env).bundleExpired(reportId)
  if (rowGone) await threadStub(env, reportId).destroy()
}

async function abandonUpload(env: BugIntakeEnv, id: string): Promise<void> {
  try {
    await dropStub(env, id).destroy()
  } catch (err) {
    console.error(`bug-intake: failed to delete abandoned drop ${id}:`, err)
  }
  try {
    await indexStub(env).deleteReport(id)
  } catch (err) {
    console.error(`bug-intake: failed to release reservation ${id}:`, err)
  }
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/** `GET /report/` (also `/report`). */
export function handleIdentity(): Response {
  return jsonResponse(200, {
    service: 'hew-bug-intake',
    format: CONTRACT_FORMAT,
    maxBytes: MAX_UPLOAD_BYTES,
    pieceBytes: PIECE_BYTES,
  })
}

// ---------------------------------------------------------------------------
// START — POST /report/
// ---------------------------------------------------------------------------

/** `POST /report/` (also `/report`) — begins an upload with its first
 *  piece. Order, cheapest first: declared `Content-Length` for THIS piece
 *  (`411`/`413`); `Content-Type` (`400`); `Hew-Upload-Length` for the WHOLE
 *  report present, parseable, and within `MAX_UPLOAD_BYTES` (`400`/`413`);
 *  `IP_HASH_SECRET` configured (`503`); the piece body itself, read and
 *  bounded to `PIECE_BYTES` (`413`) and checked against the declared total
 *  (`400`); its decompressed head validated (`400` — `headScan.ts`, only
 *  this piece is ever decompressed). Only once ALL of that passes does the
 *  fused `ReportIndex.reserveReport` call happen (`429`/`507`), and only
 *  once THAT passes does the piece actually get written to a fresh
 *  `ReportDrop`. This order matches docs/design/report-bug.md §4 exactly:
 *  head validation before reservation, because by the time reservation
 *  happens this handler already knows the report's real
 *  `appVersion`/`platform`/description excerpt, so there's no placeholder
 *  metadata to fill in later the way single-request streaming needed. */
export async function handleStart(request: Request, env: BugIntakeEnv): Promise<Response> {
  const declaredPiece = parseContentLength(request.headers.get('content-length'))
  if (declaredPiece === null) {
    return jsonResponse(411, { error: 'length-required' })
  }
  if (declaredPiece > PIECE_BYTES) {
    return jsonResponse(413, { error: 'too-large', maxBytes: MAX_UPLOAD_BYTES })
  }

  const contentType = (request.headers.get('content-type') ?? '').split(';')[0].trim().toLowerCase()
  if (contentType !== 'application/gzip') {
    return jsonResponse(400, { error: 'invalid', message: 'Content-Type must be application/gzip' })
  }

  const declaredTotal = parseContentLength(request.headers.get('hew-upload-length'))
  if (declaredTotal === null || declaredTotal < 1) {
    return jsonResponse(400, { error: 'invalid', message: 'Hew-Upload-Length is missing or malformed' })
  }
  if (declaredTotal > MAX_UPLOAD_BYTES) {
    return jsonResponse(413, { error: 'too-large', maxBytes: MAX_UPLOAD_BYTES })
  }

  if (!env.IP_HASH_SECRET) {
    return jsonResponse(503, { error: 'unavailable' })
  }

  // A follow-up attachment (docs/design/follow-a-report.md §5.6): the
  // parent's follow secret must verify before anything is read or
  // reserved; every failure is the same 403, whether the parent id is
  // malformed, unknown, unfollowed, or the secret is wrong. `Hew-Follow` is
  // ignored on a follow-up, so attachments never get threads of their own.
  const followUpHeader = request.headers.get('hew-follow-up')
  let parentId: string | null = null
  if (followUpHeader !== null) {
    const secret = request.headers.get('hew-follow-secret') ?? ''
    const rawParent = followUpHeader.trim()
    if (secret === '' || !isValidReportId(rawParent)) {
      return jsonResponse(403, { error: 'forbidden' })
    }
    parentId = normalizeReportId(rawParent)
    let verified: boolean
    try {
      verified = await threadStub(env, parentId).verifySecret(await hashToken(secret))
    } catch (err) {
      console.error(`bug-intake: verifySecret failed for follow-up of ${parentId}:`, err)
      return jsonResponse(503, { error: 'unavailable' })
    }
    if (!verified) return jsonResponse(403, { error: 'forbidden' })
    // The thread alone is not enough: the parent must also be a committed
    // report the index still lists, so an attachment never hangs off a
    // row that is gone or was never finished.
    let parentEntry: Awaited<ReturnType<ReturnType<typeof indexStub>['getReport']>>
    try {
      parentEntry = await indexStub(env).getReport(parentId)
    } catch (err) {
      console.error(`bug-intake: getReport failed for follow-up of ${parentId}:`, err)
      return jsonResponse(503, { error: 'unavailable' })
    }
    if (parentEntry === null || !parentEntry.committed) return jsonResponse(403, { error: 'forbidden' })
  }
  const wantsFollow = parentId === null && request.headers.get('hew-follow')?.trim() === '1'

  const piece = await readBodyCapped(request, PIECE_BYTES)
  if (piece === TOO_LARGE) {
    return jsonResponse(413, { error: 'too-large', maxBytes: MAX_UPLOAD_BYTES })
  }
  if (piece.byteLength === 0) {
    return jsonResponse(400, { error: 'invalid', message: 'body is empty' })
  }
  if (piece.byteLength > declaredTotal) {
    return jsonResponse(400, { error: 'invalid', message: 'the first piece is longer than the declared total' })
  }
  // Every piece but the last is exactly PIECE_BYTES (both clients split at
  // that size) — see `ReportStore.putPiece`'s doc for why the protocol
  // insists: it is what bounds how many requests an upload can take.
  if (piece.byteLength < declaredTotal && piece.byteLength !== PIECE_BYTES) {
    return jsonResponse(400, { error: 'invalid', message: `a first piece that is not the whole report must be exactly ${PIECE_BYTES} bytes` })
  }

  const validated = await decompressAndValidateHead(piece, piece.byteLength === declaredTotal)
  if (!validated.ok) {
    return jsonResponse(400, { error: 'invalid', message: validated.message })
  }
  const { description, appVersion, platform } = validated.fields

  const now = Date.now()
  const clientIp = request.headers.get('cf-connecting-ip') ?? ''
  const clientHash = await hashClient(env.IP_HASH_SECRET, clientIp, now)
  const index = indexStub(env)
  const id = generateReportId()

  let reservation: Awaited<ReturnType<typeof index.reserveReport>>
  try {
    reservation = await index.reserveReport(clientHash, now, {
      id,
      receivedAt: now,
      sizeBytes: declaredTotal,
      appVersion,
      platform,
      descriptionPreview: description.slice(0, 200),
      parentId,
    })
  } catch (err) {
    console.error('bug-intake: reserveReport failed:', err)
    return jsonResponse(503, { error: 'unavailable' })
  }

  if (!reservation.ok) {
    if (reservation.reason === 'rate-limited') {
      return jsonResponse(429, { error: 'rate-limited' }, { 'retry-after': String(Math.ceil(reservation.retryAfterMs / 1000)) })
    }
    return jsonResponse(507, { error: 'full' })
  }

  const token = generateUploadToken()
  const tokenHash = await hashToken(token)
  // The follow secret is minted here, not at COMMIT (design D4): a retried
  // COMMIT after a lost 201 then needs no rotation. Only its hash is kept.
  const follow = wantsFollow ? generateFollowSecret() : null
  const followHash = follow === null ? null : await hashToken(follow)
  try {
    await dropStub(env, id).startUpload(id, declaredTotal, tokenHash, piece, {
      description,
      appVersion,
      platform,
      followHash,
      parentId,
    })
  } catch (err) {
    console.error(`bug-intake: startUpload failed for ${id}:`, err)
    await abandonUpload(env, id)
    return jsonResponse(503, { error: 'unavailable' })
  }

  return jsonResponse(201, follow === null ? { id, token, pieceBytes: PIECE_BYTES } : { id, token, pieceBytes: PIECE_BYTES, follow })
}

// ---------------------------------------------------------------------------
// PIECE — PUT /report/<id>/<index>
// ---------------------------------------------------------------------------

/** `PUT /report/<id>/<index>` (index ≥ 1, enforced by the router before
 *  this is called). A piece over `PIECE_BYTES` is read (and thus known to
 *  be oversized) BEFORE `ReportStore.putPiece` ever gets to check the
 *  token, so this handler cannot lean on that call's own token check —
 *  unlike the `'too-large'`/`'bytes-past-total'` outcomes `putPiece` itself
 *  can return below, which it only reaches once ITS token check already
 *  passed. So an oversized body abandons the upload only through
 *  `ReportDrop.abandon`, which checks the presented token ITSELF before
 *  destroying anything: a report id is PUBLIC (a user pastes it into a
 *  GitHub issue), so it must never by itself be enough to delete someone
 *  else's report, committed or not — a tokenless or wrong-token oversized
 *  PUT leaves the target completely untouched (`403`), same as any other
 *  wrong-token request. Only a MATCHING token on a still-uncommitted
 *  upload actually abandons it (`400`); a matching token against a
 *  committed report, or an unknown one, answers `404`/`403` exactly like a
 *  normal-sized piece would. Missing/wrong token on a normal-sized piece —
 *  `ReportDrop.putPiece` compares its hash in constant time — never
 *  abandons anything either, for the same reason. */
export async function handlePutPiece(request: Request, env: BugIntakeEnv, id: string, index: number): Promise<Response> {
  const token = request.headers.get('hew-upload-token') ?? ''

  const piece = await readBodyCapped(request, PIECE_BYTES)
  if (piece === TOO_LARGE) {
    if (token === '') {
      return jsonResponse(403, { error: 'forbidden' })
    }
    const tokenHash = await hashToken(token)
    let abandonResult: Awaited<ReturnType<ReturnType<typeof dropStub>['abandon']>>
    try {
      abandonResult = await dropStub(env, id).abandon(tokenHash)
    } catch (err) {
      console.error(`bug-intake: abandon failed for oversized piece ${id}/${index}:`, err)
      return jsonResponse(503, { error: 'unavailable' })
    }
    if (!abandonResult.ok) {
      if (abandonResult.reason === 'not-found') return jsonResponse(404, { error: 'not-found' })
      return jsonResponse(403, { error: 'forbidden' })
    }
    try {
      await indexStub(env).deleteReport(id)
    } catch (err) {
      console.error(`bug-intake: failed to release reservation ${id}:`, err)
    }
    return jsonResponse(400, { error: 'invalid', message: `piece is over ${PIECE_BYTES} bytes` })
  }

  if (token === '') {
    return jsonResponse(403, { error: 'forbidden' })
  }
  const tokenHash = await hashToken(token)

  let result: Awaited<ReturnType<ReturnType<typeof dropStub>['putPiece']>>
  try {
    result = await dropStub(env, id).putPiece(index, tokenHash, piece)
  } catch (err) {
    console.error(`bug-intake: putPiece failed for ${id}/${index}:`, err)
    return jsonResponse(503, { error: 'unavailable' })
  }

  if (!result.ok) {
    switch (result.reason) {
      case 'not-found':
        return jsonResponse(404, { error: 'not-found' })
      case 'forbidden':
        return jsonResponse(403, { error: 'forbidden' })
      case 'too-large':
        // The token was already verified by `putPiece` itself above (it
        // checks `forbidden` before `too-large`) — unconditional cleanup is
        // safe here, unlike the pre-body-read case above.
        await abandonUpload(env, id)
        return jsonResponse(400, { error: 'invalid', message: `piece is over ${PIECE_BYTES} bytes` })
      case 'bytes-past-total':
        await abandonUpload(env, id)
        return jsonResponse(400, { error: 'invalid', message: 'bytes past the declared total' })
      case 'short-piece':
        // Token verified by `putPiece` before this outcome, like the two
        // above. A middle piece smaller than PIECE_BYTES is never something
        // a real client sends — it is the keepalive an attacker would use
        // to hold a reservation open, so the upload ends here.
        await abandonUpload(env, id)
        return jsonResponse(400, { error: 'invalid', message: `every piece but the last must be exactly ${PIECE_BYTES} bytes` })
      case 'expired':
        await abandonUpload(env, id)
        return jsonResponse(400, { error: 'invalid', message: 'the upload took too long and was abandoned' })
      case 'out-of-order':
        return jsonResponse(409, { error: 'out-of-order', expected: result.expected })
    }
  }

  if (result.stored) {
    try {
      await indexStub(env).touchActivity(id, Date.now())
    } catch (err) {
      // The piece is already safely stored; a failure to record activity
      // only risks the idle-prune backstop cleaning this up early — logged,
      // not surfaced, since the piece itself succeeded.
      console.error(`bug-intake: touchActivity failed for ${id}:`, err)
    }
  }
  return new Response(null, { status: 204, headers: { 'cache-control': 'no-store' } })
}

// ---------------------------------------------------------------------------
// COMMIT — POST /report/<id>/commit
// ---------------------------------------------------------------------------

/** `POST /report/<id>/commit`. Idempotent for the token holder:
 *  `ReportDrop.commit` itself answers `ok` again (with the same fields,
 *  never re-arming anything) for a drop that's already committed, as long
 *  as the token still matches — so a client that lost its `201` response
 *  (a dropped connection, a timeout) and retries lands on the same success
 *  rather than a `404` that would make it think the report never landed.
 *  What must NOT repeat on a retry is the maintainer notification email:
 *  `ReportIndex.commitReport` reports whether IT actually flipped the row
 *  from uncommitted to committed, and the email only goes out when it did
 *  — so a retry after `commitReport` itself failed the first time still
 *  sends exactly one email (this attempt flips it), and a retry after a
 *  fully successful first commit sends none (the row was already
 *  committed). */
export async function handleCommit(request: Request, env: BugIntakeEnv, id: string, mailer: RawMailSender): Promise<Response> {
  const token = request.headers.get('hew-upload-token') ?? ''
  if (token === '') {
    return jsonResponse(403, { error: 'forbidden' })
  }
  const tokenHash = await hashToken(token)

  let result: Awaited<ReturnType<ReturnType<typeof dropStub>['commit']>>
  try {
    result = await dropStub(env, id).commit(tokenHash)
  } catch (err) {
    console.error(`bug-intake: commit failed for ${id}:`, err)
    return jsonResponse(503, { error: 'unavailable' })
  }

  if (!result.ok) {
    if (result.reason === 'not-found') return jsonResponse(404, { error: 'not-found' })
    if (result.reason === 'forbidden') return jsonResponse(403, { error: 'forbidden' })
    // incomplete
    await abandonUpload(env, id)
    return jsonResponse(400, { error: 'invalid', message: 'fewer bytes arrived than declared' })
  }

  const now = Date.now()

  // A followed report's thread is created BEFORE the index row flips, so a
  // failure here is a 503 the client retries and `create` (idempotent)
  // finishes; the row's `following` is set by the same `commitReport` call.
  if (result.followHash !== null && result.parentId === null) {
    try {
      await threadStub(env, id).create(id, result.followHash, now)
    } catch (err) {
      console.error(`bug-intake: thread create failed for ${id}:`, err)
      return jsonResponse(503, { error: 'unavailable' })
    }
  }

  // A follow-up attachment's message on the parent thread, appended before
  // the flip for the same reason; `appendAttachment` is idempotent per
  // child id. A parent thread that ended meanwhile (or is full) leaves the
  // attachment a plain report with its `parentId`, which still lists it
  // under the parent on the admin page.
  let parentStatus: { status: string; statusVersion: string | null } | null = null
  const label = attachmentLabel(result.description, result.sizeBytes)
  if (result.parentId !== null) {
    try {
      const appended = await threadStub(env, result.parentId).appendAttachment(id, label, now)
      if (appended.ok) parentStatus = { status: appended.status, statusVersion: appended.statusVersion }
      else console.log(`bug-intake: attachment ${id} not added to ${result.parentId}'s thread: ${appended.reason}`)
    } catch (err) {
      console.error(`bug-intake: appendAttachment failed for ${id} on ${result.parentId}:`, err)
      return jsonResponse(503, { error: 'unavailable' })
    }
  }

  let outcome: Awaited<ReturnType<ReturnType<typeof indexStub>['commitReport']>>
  try {
    outcome = await indexStub(env).commitReport(id, result.followHash !== null && result.parentId === null)
  } catch (err) {
    // The drop is already committed and its bytes are safe — this only
    // means the admin index doesn't know it yet. See handlers.ts's design
    // note (README "Chunked upload"): there is no two-phase commit across
    // the two DOs here, and this narrow, rare inconsistency window is an
    // accepted tradeoff rather than a full protocol for closing it. A
    // client retry lands here again and (this time, if `commitReport`
    // succeeds) both flips the row and sends the notification email.
    console.error(`bug-intake: ReportIndex.commitReport failed for ${id} after a successful drop commit:`, err)
    return jsonResponse(503, { error: 'unavailable' })
  }

  if (outcome === 'not-found') {
    // Genuinely abnormal: the drop just committed successfully, but
    // `ReportIndex` has no row for it (an admin delete racing this
    // request, say). Never silently answer 201 for a report the index no
    // longer knows about.
    console.error(`bug-intake: ReportIndex has no row for ${id} after a successful drop commit`)
    return jsonResponse(503, { error: 'unavailable' })
  }

  if (outcome === 'flipped' && result.parentId !== null) {
    // A follow-up sends the attachment email INSTEAD of the new-report one
    // (follow-a-report §3.3), and refreshes the parent row's list caches.
    try {
      await indexStub(env).setThreadCache(result.parentId, { reporterActivityAt: now, ...(parentStatus ?? {}) })
    } catch (err) {
      console.error(`bug-intake: setThreadCache failed for ${result.parentId}:`, err)
    }
    await sendThreadNotification(
      env,
      { kind: 'attachment', id: result.parentId, childId: id, label, sizeBytes: result.sizeBytes },
      mailer,
    )
  } else if (outcome === 'flipped') {
    // Best-effort and genuinely never throws (see `sendNotification`'s
    // doc) — the report is already committed and durably stored by this
    // point (§8). Sent only on the call that actually flipped the index
    // row, so a retry never sends a second email.
    await sendNotification(
      env,
      { id, appVersion: result.appVersion, platform: result.platform, description: result.description, sizeBytes: result.sizeBytes },
      mailer,
    )
  }

  return jsonResponse(201, { id })
}

// ---------------------------------------------------------------------------
// Thread routes — /report/<id>/thread (docs/design/follow-a-report.md §5.3)
// ---------------------------------------------------------------------------

function threadForbidden(): Response {
  return jsonResponse(403, { error: 'forbidden' })
}

function rateLimited(retryAfterMs: number): Response {
  return jsonResponse(429, { error: 'rate-limited' }, { 'retry-after': String(Math.max(1, Math.ceil(retryAfterMs / 1000))) })
}

/** The per-client thread-route quota (`THREAD_RATE_WINDOW_MAX` per 10
 *  minutes), counted on `ReportIndex` only AFTER the thread DO accepted the
 *  secret — so a wrong-secret flood costs the index nothing (design D10).
 *  `null` means within the limit. */
async function countThreadRequest(env: BugIntakeEnv, request: Request, now: number): Promise<Response | null> {
  const clientHash = await hashClient(env.IP_HASH_SECRET!, request.headers.get('cf-connecting-ip') ?? '', now)
  const counted = await indexStub(env).countThreadRequest(clientHash, now)
  return counted.ok ? null : rateLimited(counted.retryAfterMs)
}

/** Reads a thread route's small JSON body, or `null` if it is too big or
 *  not a JSON object. */
async function readJsonObject(request: Request): Promise<Record<string, unknown> | null> {
  const body = await readBodyCapped(request, THREAD_BODY_MAX_BYTES)
  if (body === TOO_LARGE) return null
  try {
    const parsed: unknown = JSON.parse(new TextDecoder().decode(body))
    return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed) ? (parsed as Record<string, unknown>) : null
  } catch {
    return null
  }
}

/** Every `/report/<id>/thread[...]` route. Order: the secret header (a
 *  missing one is 403 with no DO touched), `IP_HASH_SECRET` (503), the
 *  request body where there is one (400 — it says nothing about the
 *  report), then the thread DO (403 for every failure: no thread, a
 *  wrong or revoked secret, an admin-only thread), then the rate count.
 *  The DO call comes first so a rejected secret never writes to the index;
 *  for a reporter post the secret is checked with `verifySecret` before
 *  the count so a rate-limited post stores nothing. */
async function handleThread(
  request: Request,
  env: BugIntakeEnv,
  id: string,
  sub: 'thread' | 'seen',
  mailer: RawMailSender,
): Promise<Response> {
  const secret = request.headers.get('hew-follow-secret') ?? ''
  if (secret === '') return threadForbidden()
  if (!env.IP_HASH_SECRET) return jsonResponse(503, { error: 'unavailable' })
  const secretHash = await hashToken(secret)
  const thread = threadStub(env, id)
  const now = Date.now()

  try {
    if (sub === 'seen') {
      if (request.method !== 'POST') return methodNotAllowed()
      const body = await readJsonObject(request)
      const upTo = body?.upTo
      if (typeof upTo !== 'number' || !Number.isInteger(upTo) || upTo < 0) {
        return jsonResponse(400, { error: 'invalid', message: 'body must be {"upTo": <non-negative integer>}' })
      }
      if ((await thread.markSeen(secretHash, upTo, now)) === 'forbidden') return threadForbidden()
      const limited = await countThreadRequest(env, request, now)
      if (limited) return limited
      return new Response(null, { status: 204, headers: { 'cache-control': 'no-store' } })
    }

    if (request.method === 'GET') {
      const afterParam = new URL(request.url).searchParams.get('after')
      const after = afterParam === null ? 0 : Number(afterParam)
      if (!/^[0-9]+$/.test(afterParam ?? '0') || !Number.isSafeInteger(after)) {
        return jsonResponse(400, { error: 'invalid', message: 'after must be a non-negative integer' })
      }
      const view = await thread.read(secretHash, after, now)
      if (view === 'forbidden') return threadForbidden()
      const limited = await countThreadRequest(env, request, now)
      if (limited) return limited
      return jsonResponse(200, view)
    }

    if (request.method === 'POST') {
      const body = await readJsonObject(request)
      const text = body?.text
      if (!validMessageText(text)) {
        return jsonResponse(400, { error: 'invalid', message: `text must be 1 to ${THREAD_MESSAGE_MAX_CHARS} characters` })
      }
      if (!(await thread.verifySecret(secretHash))) return threadForbidden()
      const limited = await countThreadRequest(env, request, now)
      if (limited) return limited
      const posted = await thread.postReporter(secretHash, text, now)
      if (!posted.ok) {
        if (posted.reason === 'forbidden') return threadForbidden()
        if (posted.reason === 'too-many') return rateLimited(posted.retryAfterMs)
        return jsonResponse(409, { error: 'full' })
      }
      try {
        await indexStub(env).setThreadCache(id, { reporterActivityAt: now, status: posted.status, statusVersion: posted.statusVersion })
      } catch (err) {
        // The message is stored; only the admin list's caches are stale.
        console.error(`bug-intake: setThreadCache failed for ${id}:`, err)
      }
      await sendThreadNotification(env, { kind: 'reply', id, text, reopened: posted.reopened }, mailer)
      return jsonResponse(201, { seq: posted.seq })
    }

    if (request.method === 'DELETE') {
      // Forget. `revoke` tells the index (through the DO's own hook) before
      // wiping. A second call finds no thread: 403, which the client reads
      // as done. Counted like every other accepted request, but never
      // turned into a 429 — the thread is already gone.
      if ((await thread.revoke(secretHash)) === 'forbidden') return threadForbidden()
      try {
        await countThreadRequest(env, request, now)
      } catch (err) {
        console.error('bug-intake: countThreadRequest failed after a revoke:', err)
      }
      return new Response(null, { status: 204, headers: { 'cache-control': 'no-store' } })
    }

    return methodNotAllowed()
  } catch (err) {
    console.error(`bug-intake: thread route failed for ${id}:`, err)
    return jsonResponse(503, { error: 'unavailable' })
  }
}

// ---------------------------------------------------------------------------
// Admin
// ---------------------------------------------------------------------------

/** Verifies the Access JWT on the request, or `null` on any failure — see
 *  `adminAuth.ts` for why every failure mode collapses to the same
 *  outcome. `authOptions` is threaded through from `handleRequest` purely so
 *  tests can inject a fake JWKS fetch and a fixed clock; production
 *  (`index.ts`) never passes it, so `verifyAccessJwt` falls back to its
 *  real default cache and `Date.now()`. */
async function requireAdmin(request: Request, env: BugIntakeEnv, authOptions: VerifyOptions): Promise<boolean> {
  const token = request.headers.get('cf-access-jwt-assertion') ?? ''
  const payload = await verifyAccessJwt(token, env.ACCESS_TEAM_DOMAIN, env.ACCESS_AUD, authOptions)
  return payload !== null
}

function forbidden(): Response {
  return new Response('Forbidden', { status: 403, headers: { 'cache-control': 'no-store' } })
}

function methodNotAllowed(): Response {
  return new Response(null, { status: 405, headers: { 'cache-control': 'no-store' } })
}

/** A post-mutation redirect (`Response.redirect` sets no headers of its
 *  own beyond `Location`) — every response from this Worker carries
 *  `Cache-Control: no-store`, redirects included. */
function redirectNoStore(location: string, status: 303): Response {
  return new Response(null, { status, headers: { location, 'cache-control': 'no-store' } })
}

const ADMIN_CSP = "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'"

function adminHtml(status: number, html: string): Response {
  return htmlResponse(status, html, { 'content-security-policy': ADMIN_CSP })
}

/** Rejects a mutating admin request whose `Origin` isn't the admin page's
 *  own origin — defense in depth alongside Cloudflare Access, same posture
 *  as `adminAuth.ts`'s fail-closed JWT check. Missing `Origin` is also
 *  rejected: every browser that can submit these forms sends one. */
function originAllowed(request: Request): boolean {
  return request.headers.get('origin') === ADMIN_ORIGIN
}

async function handleAdminList(env: BugIntakeEnv): Promise<Response> {
  const reports = await indexStub(env).listReports()
  return adminHtml(200, renderList(reports))
}

/** Reads and decompresses just piece 0 of a report's HEAD, for the detail
 *  page — never the whole bundle: unlike the old `readFullBundle`/`gunzip`
 *  path (up to ~2x the report's memory, and a full decompression that
 *  could run well past a report's compressed size — the design's worked
 *  example, 210.9 MB from 59.9 MB), this reads one stored row and
 *  decompresses at most `HeadDecompressor`'s bounded budget, the same one
 *  `handleStart`'s own head scan uses. `isFinalPiece` mirrors
 *  `decompressAndValidateHead`'s call in `handleStart`: `true` only when
 *  the whole report is a single piece, so a genuinely truncated (more
 *  pieces exist) piece 0 is never handed to the decompressor as if it were
 *  gzip's true end (see `HeadDecompressor.finish`'s doc for why that
 *  distinction matters). Returns `null` for a report with no piece 0 at
 *  all, or if decompression errors; never throws. */
async function readHeadForDisplay(env: BugIntakeEnv, id: string): Promise<string | null> {
  const stub = dropStub(env, id)
  const head = await stub.head()
  if (head === null) return null
  const pieces = await stub.read(0, 1)
  if (pieces.length === 0) return null
  const decompressor = new HeadDecompressor()
  await decompressor.write(pieces[0])
  const { text, errored } = await decompressor.finish(head.pieceCount === 1)
  return errored ? null : text
}

async function handleAdminDetail(env: BugIntakeEnv, id: string): Promise<Response> {
  const index = indexStub(env)
  const entry = await index.getReport(id)
  // A reservation still uploading (or one abandoned and not yet pruned) is
  // treated exactly like an unknown id — it isn't a real report yet.
  if (entry === null || !entry.committed) return adminHtml(404, `<p>No report ${escapeHtml(id)}.</p>`)
  let head = null
  if (!entry.bundleDeleted) {
    try {
      const text = await readHeadForDisplay(env, id)
      if (text !== null) head = parseHeadForDisplay(text)
    } catch (err) {
      console.error(`bug-intake: failed to read/decompress report ${id}'s head for detail view:`, err)
    }
  }
  // A follow-up attachment has no thread of its own (its parent's page is
  // where the conversation is), so its page shows only the "↳ for" link.
  if (entry.parentId !== null) return adminHtml(200, renderDetail(entry, head))
  const now = Date.now()
  const thread = await threadStub(env, id).adminView(now)
  await index.markAdminViewed(id, now)
  const children = await index.listChildren(id)
  return adminHtml(200, renderDetail(entry, head, { thread, children }))
}

/** Reads an admin form body (`application/x-www-form-urlencoded`). */
async function readForm(request: Request): Promise<URLSearchParams> {
  const body = await readBodyCapped(request, THREAD_BODY_MAX_BYTES)
  if (body === TOO_LARGE) return new URLSearchParams()
  return new URLSearchParams(new TextDecoder().decode(body))
}

/** The thread a maintainer action writes to: the report's own, or — for a
 *  report nobody follows (or whose thread ended) — one created now with no
 *  secret, so the reply or status is "kept with it" (§3.2) but can reach
 *  no one. `null` for an unknown, uncommitted, or follow-up row. */
async function adminThreadFor(env: BugIntakeEnv, id: string, now: number) {
  const entry = await indexStub(env).getReport(id)
  if (entry === null || !entry.committed || entry.parentId !== null) return null
  const thread = threadStub(env, id)
  await thread.create(id, '', now) // a no-op when the thread exists
  return thread
}

async function handleAdminReply(request: Request, env: BugIntakeEnv, id: string): Promise<Response> {
  if (!originAllowed(request)) return forbidden()
  const form = await readForm(request)
  const canned = form.get('canned')
  let text: string | null
  if (canned !== null && canned !== '') {
    text = Object.hasOwn(CANNED_REPLIES, canned) ? CANNED_REPLIES[canned].text : null
    if (text === null) return adminHtml(400, renderFormError(id, 'Unknown canned reply.'))
  } else {
    text = form.get('text')
    if (!validMessageText(text)) {
      return adminHtml(400, renderFormError(id, `A reply must be 1 to ${THREAD_MESSAGE_MAX_CHARS} characters.`))
    }
  }
  const now = Date.now()
  const thread = await adminThreadFor(env, id, now)
  if (thread === null) return notFound()
  const posted = await thread.postMaintainer(text, now)
  if (!posted.ok && posted.reason === 'not-found') return notFound()
  if (!posted.ok) return adminHtml(409, renderFormError(id, 'This thread is full.'))
  return redirectNoStore(`${ADMIN_ORIGIN}/report/admin/${id}`, 303)
}

async function handleAdminStatus(request: Request, env: BugIntakeEnv, id: string): Promise<Response> {
  if (!originAllowed(request)) return forbidden()
  const form = await readForm(request)
  const status = form.get('status')
  if (!isThreadStatus(status)) return adminHtml(400, renderFormError(id, 'Pick a status.'))
  const version = (form.get('version') ?? '').trim()
  if (status === 'fixed' && version === '') return adminHtml(400, renderFormError(id, 'Fixed needs the version it is fixed in.'))
  if ([...version].length > STATUS_VERSION_MAX_CHARS) return adminHtml(400, renderFormError(id, 'That version is too long.'))
  const note = (form.get('note') ?? '').trim()
  if ([...note].length > THREAD_MESSAGE_MAX_CHARS) return adminHtml(400, renderFormError(id, 'That note is too long.'))
  const now = Date.now()
  const thread = await adminThreadFor(env, id, now)
  if (thread === null) return notFound()
  const set = await thread.setStatus(status, status === 'fixed' ? version : null, note === '' ? null : note, now)
  if (!set.ok && set.reason === 'not-found') return notFound()
  if (!set.ok) return adminHtml(409, renderFormError(id, 'This thread is full.'))
  // The status is already stored on the thread; a failed cache copy only
  // leaves the list's badge stale. Answering 5xx here would invite a retry
  // that writes a second status message.
  try {
    await indexStub(env).setThreadCache(id, { status: set.status, statusVersion: set.statusVersion })
  } catch (err) {
    console.error(`bug-intake: setThreadCache failed for ${id}:`, err)
  }
  return redirectNoStore(`${ADMIN_ORIGIN}/report/admin/${id}`, 303)
}

async function handleAdminDownload(env: BugIntakeEnv, id: string): Promise<Response> {
  const entry = await indexStub(env).getReport(id)
  if (entry === null || !entry.committed || entry.bundleDeleted) return notFound()
  const stub = dropStub(env, id)
  const dropHead = await stub.head()
  if (dropHead === null) return notFound()
  // Streamed straight out of the `ReportDrop`, one piece row at a time
  // (`ReportStore.readStream`) — never read into memory whole. Served
  // exactly as stored — still gzip-compressed (docs/design/report-bug.md
  // §4 "Admin page"); the maintainer's tooling decompresses it, same as the
  // client did before upload.
  const stream = await stub.readStream()
  return new Response(stream, {
    status: 200,
    headers: {
      'content-type': 'application/gzip',
      'content-disposition': `attachment; filename="${id}.json.gz"`,
      'content-length': String(dropHead.totalBytes),
      'cache-control': 'no-store',
    },
  })
}

async function handleAdminDelete(request: Request, env: BugIntakeEnv, id: string): Promise<Response> {
  if (!originAllowed(request)) return forbidden()
  // The thread goes first, so the reporter's key stops working the same
  // moment (follow-a-report §3.2); the row is deleted here directly, so
  // the thread's own index hook is not needed on this path.
  await threadStub(env, id).destroy()
  await dropStub(env, id).destroy()
  const index = indexStub(env)
  await index.deleteReport(id)
  // A report's follow-up attachments are the reporter's material for THIS
  // report: a deletion request quoting the ID means the file they attached
  // too, and a child left behind would only carry a "for <parent>" link to
  // a page that no longer exists. Children have no threads of their own.
  for (const child of await index.listChildren(id)) {
    await dropStub(env, child.id).destroy()
    await index.deleteReport(child.id)
  }
  return redirectNoStore(`${ADMIN_ORIGIN}/report/admin/`, 303)
}

async function handleAdminTriage(request: Request, env: BugIntakeEnv, id: string): Promise<Response> {
  if (!originAllowed(request)) return forbidden()
  const index = indexStub(env)
  const entry = await index.getReport(id)
  if (entry === null || !entry.committed) return notFound()
  await index.setTriaged(id, !entry.triaged)
  return redirectNoStore(`${ADMIN_ORIGIN}/report/admin/${id}`, 303)
}

async function routeAdmin(
  request: Request,
  env: BugIntakeEnv,
  rest: string,
  authOptions: VerifyOptions,
): Promise<Response> {
  if (!(await requireAdmin(request, env, authOptions))) return forbidden()

  if (rest === '' || rest === '/') {
    if (request.method !== 'GET') return methodNotAllowed()
    return handleAdminList(env)
  }

  const segments = rest.replace(/^\//, '').split('/')
  const rawId = segments[0]
  if (!isValidReportId(rawId)) return notFound()
  const id = normalizeReportId(rawId)

  if (segments.length === 1) {
    if (request.method !== 'GET') return methodNotAllowed()
    return handleAdminDetail(env, id)
  }
  if (segments.length === 2) {
    const action = segments[1]
    if (action === 'download' && request.method === 'GET') return handleAdminDownload(env, id)
    if (action === 'delete' && request.method === 'POST') return handleAdminDelete(request, env, id)
    if (action === 'triage' && request.method === 'POST') return handleAdminTriage(request, env, id)
    if (action === 'reply' && request.method === 'POST') return handleAdminReply(request, env, id)
    if (action === 'status' && request.method === 'POST') return handleAdminStatus(request, env, id)
  }
  return notFound()
}

// ---------------------------------------------------------------------------
// Top-level dispatch
// ---------------------------------------------------------------------------

const ADMIN_PREFIX = '/report/admin'
const REPORT_PREFIX = '/report/'

async function handleRequestInner(
  request: Request,
  env: BugIntakeEnv,
  mailer: RawMailSender,
  authOptions: VerifyOptions,
  pathname: string,
): Promise<Response> {
  if (pathname === '/report' || pathname === '/report/') {
    if (request.method === 'GET') return handleIdentity()
    if (request.method === 'POST') return handleStart(request, env)
    return methodNotAllowed()
  }

  if (pathname === ADMIN_PREFIX || pathname.startsWith(`${ADMIN_PREFIX}/`)) {
    const rest = pathname.slice(ADMIN_PREFIX.length)
    return routeAdmin(request, env, rest, authOptions)
  }

  if (pathname.startsWith(REPORT_PREFIX)) {
    const rest = pathname.slice(REPORT_PREFIX.length) // "<id>/<index-or-commit>"
    const segments = rest.split('/')
    if (segments.length === 3 && segments[1] === 'thread' && segments[2] === 'seen') {
      // Every thread-route failure is 403, a malformed id included (no
      // public route says whether an id exists or is well formed).
      if (!isValidReportId(segments[0])) return threadForbidden()
      return handleThread(request, env, normalizeReportId(segments[0]), 'seen', mailer)
    }
    if (segments.length === 2 && segments[1] === 'thread') {
      if (!isValidReportId(segments[0])) return threadForbidden()
      return handleThread(request, env, normalizeReportId(segments[0]), 'thread', mailer)
    }
    if (segments.length === 2) {
      const [rawId, action] = segments
      if (isValidReportId(rawId)) {
        const id = normalizeReportId(rawId)
        if (action === 'commit') {
          if (request.method === 'POST') return handleCommit(request, env, id, mailer)
          return methodNotAllowed()
        }
        if (/^[1-9][0-9]*$/.test(action)) {
          if (request.method === 'PUT') return handlePutPiece(request, env, id, Number(action))
          return methodNotAllowed()
        }
      }
    }
  }

  return notFound()
}

/** The single entry point `index.ts`'s `fetch` calls. Every route handler
 *  above already catches the Durable Object errors it can anticipate; this
 *  wrapper is the backstop for anything that slips past those — a bug in
 *  code that didn't anticipate a particular failure, not a substitute for
 *  handling errors closer to where they happen. §8 requires every failure
 *  to end in `{"error":"unavailable"}` with `Cache-Control: no-store`, not
 *  a raw, uncaught-exception response with neither; an admin-path request
 *  gets a plain-text 503 instead of that JSON shape (nothing on
 *  `/report/admin/*` speaks the §8 JSON contract), but the same status and
 *  no-store header. */
export async function handleRequest(
  request: Request,
  env: BugIntakeEnv,
  mailer: RawMailSender,
  authOptions: VerifyOptions = {},
): Promise<Response> {
  const pathname = new URL(request.url).pathname
  try {
    return await handleRequestInner(request, env, mailer, authOptions, pathname)
  } catch (err) {
    console.error('bug-intake: unhandled error', err)
    if (pathname === ADMIN_PREFIX || pathname.startsWith(`${ADMIN_PREFIX}/`)) {
      return new Response('Service unavailable', { status: 503, headers: { 'cache-control': 'no-store' } })
    }
    return jsonResponse(503, { error: 'unavailable' })
  }
}
