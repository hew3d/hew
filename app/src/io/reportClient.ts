/**
 * reportClient — submits a Help ▸ Report Bug report to the intake service
 * (docs/design/report-bug.md §8, the wire contract this must match exactly;
 * `workers/bug-intake` is the other side, built separately).
 *
 * A report is the gzip of the bundle `reportBundle.ts` builds, uploaded in
 * pieces of `PIECE_BYTES`: a start request carrying the first piece and the
 * total length, each following piece in order, then a commit. Each request
 * stays small because the intake service runs where a request gets 10 ms of
 * CPU (§4). A request that fails with a network error or a 5xx is retried
 * once.
 *
 * Desktop runs that whole upload in the Rust `report_submit` command
 * (report_client.rs) — the same "no arbitrary-URL capability in the
 * webview" posture as `relayClient.ts` — and always can, in every server
 * mode (design §5: a bug report is for Hew's developer, not a self-hosted
 * homelab). Web uploads same-origin under `/report/`, which only makes
 * sense — and is only attempted — when the page's own origin actually
 * serves that route: the production `app.hew3d.com` deployment, the
 * `beta.hew3d.com` deployment (same Worker, its own route), or a local
 * `vite dev` with `HEW_REPORT_PROXY` forwarding it to `wrangler dev` (see
 * `vite.config.ts`). Every other web build (self-hosted, a plain dev
 * server) has nowhere to send to, so `canSend()` is false there and
 * `ReportBugDialog` hides Send report entirely rather than let a submit
 * attempt fail.
 */

import { isTauri } from './fileHost'
import { recordWebFollow } from './webFollowStore'

/** Mirrors `ReportErrorKind` in report_client.rs (serde camelCase), plus
 *  `offline` for a web `fetch` that never reached the network at all —
 *  the desktop command's transport failures already collapse to
 *  `unreachable`/`tls`, but a browser `fetch` throwing a bare `TypeError`
 *  carries no such distinction from Rust to preserve. */
export type ReportErrorKind =
  | 'invalid'
  | 'tooLarge'
  | 'rateLimited'
  | 'full'
  | 'unreachable'
  | 'tls'
  | 'status'
  | 'offline'

export class ReportError extends Error {
  readonly kind: ReportErrorKind
  readonly status: number | undefined
  /** Seconds a 429 asked the client to wait, when it said. */
  readonly retryAfter: number | undefined

  constructor(kind: ReportErrorKind, message: string, status?: number, retryAfter?: number) {
    super(message)
    this.name = 'ReportError'
    this.kind = kind
    this.status = status
    this.retryAfter = retryAfter
  }
}

/* oxlint-disable lingui/no-unlocalized-strings -- ReportError kind identifiers */
const KINDS: ReadonlySet<string> = new Set<ReportErrorKind>([
  'invalid',
  'tooLarge',
  'rateLimited',
  'full',
  'unreachable',
  'tls',
  'status',
  'offline',
])
/* oxlint-enable lingui/no-unlocalized-strings */

/** Turns whatever the Tauri `report_submit` invoke rejected with into a
 *  `ReportError`. Mirrors `relayClient.ts`'s `toRelayError`. */
export function toReportError(err: unknown): ReportError {
  if (err instanceof ReportError) return err
  if (typeof err === 'object' && err !== null && 'kind' in err) {
    const kind = (err as { kind: unknown }).kind
    const message = (err as { message?: unknown }).message
    const status = (err as { status?: unknown }).status
    const retryAfter = (err as { retryAfter?: unknown }).retryAfter
    if (typeof kind === 'string' && KINDS.has(kind)) {
      return new ReportError(
        kind as ReportErrorKind,
        typeof message === 'string' ? message : kind,
        typeof status === 'number' ? status : undefined,
        typeof retryAfter === 'number' ? retryAfter : undefined,
      )
    }
  }
  return new ReportError('unreachable', err instanceof Error ? err.message : String(err))
}

/** The cloud web origins whose zone routes `/report/` to the intake Worker —
 *  the only web origins uploads are ever attempted from outside a proxied
 *  dev server. Exact `location.origin` matches: scheme and host both count. */
const CLOUD_APP_ORIGINS: readonly string[] = ['https://app.hew3d.com', 'https://beta.hew3d.com']

/** Set by `vite.config.ts`'s `define` when `HEW_REPORT_PROXY` was configured
 *  for this dev server — see that file for why this needs to be a
 *  build-time flag rather than a runtime env read (the browser has no
 *  `process.env`). Undefined (falsy) outside a Vite build (e.g. plain
 *  `tsc`/vitest), matching the `__HEW_VERSION__` fallback convention. */
declare const __HEW_REPORT_DEV_PROXY__: boolean | undefined

/**
 * Whether this build/runtime can actually send a report anywhere. Desktop:
 * always (every server mode posts to the cloud intake service). Web: only
 * a Hew cloud app origin (production or beta), or a dev server with the proxy configured.
 * Everything else (a self-hosted web build, `vite dev` with no proxy) has
 * no route to the intake service, so the dialog must not offer Send report.
 */
export function canSend(): boolean {
  if (isTauri) return true
  if (typeof location !== 'undefined' && CLOUD_APP_ORIGINS.includes(location.origin)) return true
  return typeof __HEW_REPORT_DEV_PROXY__ !== 'undefined' && __HEW_REPORT_DEV_PROXY__
}

export interface ReportSubmitResult {
  id: string
  /** True when the report is now being followed (the reporter asked for it
   *  and the service minted a secret). False for a plain send, for a
   *  follow-up attachment, and against a service that predates Follow. */
  following: boolean
}

/** What the Report Bug dialog passes to follow the report it sends. */
export interface ReportFollowOptions {
  follow: boolean
  /** The first line of the description, for the Sent Reports list. */
  excerpt: string
  /** The failed-open file sent with the report, if any. */
  fileName: string | null
}

/** How much of the compressed report has been uploaded. */
export interface ReportProgress {
  sentBytes: number
  totalBytes: number
}

/** The upload piece size, one storage row on the intake service (§8). */
export const PIECE_BYTES = 1_900_000

/** One request carries at most one piece; two minutes covers that on a slow
 *  uplink and still ends a stalled connection. */
const REQUEST_TIMEOUT_MS = 120_000

/** Maps one of §8's JSON error bodies + HTTP status onto a typed kind —
 *  the web-`fetch` twin of report_client.rs's `ReportError::from_status`.
 *  Falls back to `status` for anything that doesn't match the documented
 *  shape, which includes a piece's 403/404/409. */
/* oxlint-disable lingui/no-unlocalized-strings -- ReportError.message is developer-facing; ReportBugDialog maps `kind` to its own localized copy */
function reportErrorFromResponse(status: number, body: unknown): ReportError {
  const error = typeof body === 'object' && body !== null ? (body as { error?: unknown }).error : undefined
  switch (`${status} ${typeof error === 'string' ? error : ''}`) {
    case '400 invalid':
      return new ReportError('invalid', 'the report was rejected as invalid', status)
    case '413 too-large':
      return new ReportError('tooLarge', 'the report is too large to send', status)
    case '429 rate-limited':
      return new ReportError('rateLimited', 'too many reports sent recently', status)
    case '507 full':
      return new ReportError('full', 'the report service is temporarily full', status)
    default:
      if (status >= 500) return new ReportError('unreachable', 'the report service is unavailable', status)
      return new ReportError('status', `unexpected status ${status}`, status)
  }
}
/* oxlint-enable lingui/no-unlocalized-strings */

/** Runs a raw-body Tauri command that uploads a report in pieces, mapping
 *  its rejection to a `ReportError` and relaying `report-progress` events.
 *  The gzip bytes ride the invoke as a RAW body (the same convention
 *  `relayClient.ts`'s `relayPut` uses), never a JSON array — a report of
 *  hundreds of megabytes must not be serialized as hundreds of millions of
 *  numbers. Options a raw body cannot carry ride in request `headers`. */
export async function invokeUpload<T>(
  command: string,
  gzipBytes: Uint8Array,
  headers: Record<string, string> | undefined,
  onProgress?: (progress: ReportProgress) => void,
): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  let unlisten: (() => void) | undefined
  if (onProgress !== undefined) {
    try {
      const { listen } = await import('@tauri-apps/api/event')
      unlisten = await listen<ReportProgress>('report-progress', (event) => onProgress(event.payload))
    } catch {
      // Progress is cosmetic; the upload doesn't depend on it.
    }
  }
  try {
    return await (headers === undefined
      ? invoke<T>(command, gzipBytes as never)
      : invoke<T>(command, gzipBytes as never, { headers }))
  } catch (err) {
    throw toReportError(err)
  } finally {
    unlisten?.()
  }
}

/** Desktop path: Rust runs the piece-by-piece upload (and stores the follow
 *  record itself when `follow` was asked for). */
async function submitDesktop(
  gzipBytes: Uint8Array,
  onProgress?: (progress: ReportProgress) => void,
  follow?: ReportFollowOptions,
): Promise<ReportSubmitResult> {
  const headers =
    follow === undefined
      ? undefined
      : { 'hew-options': encodeURIComponent(JSON.stringify({ follow: follow.follow, excerpt: follow.excerpt, fileName: follow.fileName })) }
  const result = await invokeUpload<{ id: string; following?: boolean }>('report_submit', gzipBytes, headers, onProgress)
  return { id: result.id, following: result.following === true }
}

/** One `fetch` with the per-request timeout. A thrown error (offline, DNS,
 *  aborted by the timeout) comes back as `null` instead of a response. */
const responseTimers = new WeakMap<Response, ReturnType<typeof setTimeout>>()

async function fetchOnce(url: string, init: RequestInit): Promise<Response | null> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), REQUEST_TIMEOUT_MS)
  try {
    const response = await fetch(url, { ...init, signal: controller.signal })
    // The timer stays armed until the body has been read (`readJson`), so a
    // stalled body cannot hang forever; a body nobody reads is aborted at
    // the deadline, which is harmless.
    responseTimers.set(response, timer)
    return response
  } catch {
    clearTimeout(timer)
    return null
  }
}

/** §8's retry rule: a network error or a 5xx is retried once. Every step
 *  tolerates a repeat: the service accepts a retry of the last piece, and a
 *  commit repeated with the upload's token answers 201 again. */
async function fetchWithRetry(url: string, init: RequestInit): Promise<Response> {
  const first = await fetchOnce(url, init)
  if (first !== null && first.status < 500) return first
  const second = await fetchOnce(url, init)
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- developer-facing ReportError.message; the dialog localizes by kind
  if (second === null) throw new ReportError('offline', 'could not reach the report service')
  return second
}

async function readJson(response: Response): Promise<unknown> {
  try {
    return await response.json()
  } catch {
    return null
  } finally {
    const timer = responseTimers.get(response)
    if (timer !== undefined) clearTimeout(timer)
  }
}

/** One piece of the upload as a request body. `gzip()` allocates a plain
 *  ArrayBuffer, so the view is safe to hand to `fetch` as-is. */
function pieceBody(bytes: Uint8Array, start: number, end: number): Uint8Array<ArrayBuffer> {
  return bytes.subarray(start, end) as Uint8Array<ArrayBuffer>
}

/** Extra START headers for a web upload (follow requests). */
type StartHeaders = Record<string, string>

/** The chunked upload loop: start, pieces, commit, all same-origin under
 *  `/report/`. Resolves with the new id and the START body (so a caller can
 *  read `follow`) only after COMMIT answered 201. */
async function uploadWeb(
  gzipBytes: Uint8Array,
  onProgress: ((progress: ReportProgress) => void) | undefined,
  extraStartHeaders: StartHeaders,
): Promise<{ id: string; startJson: unknown }> {
  const total = gzipBytes.byteLength
  const firstEnd = Math.min(PIECE_BYTES, total)

  const start = await fetchWithRetry('/report/', {
    method: 'POST',
    headers: { 'Content-Type': 'application/gzip', 'Hew-Upload-Length': String(total), ...extraStartHeaders },
    body: pieceBody(gzipBytes, 0, firstEnd),
  })
  const startJson = await readJson(start)
  if (start.status !== 201) throw reportErrorFromResponse(start.status, startJson)
  const id = typeof startJson === 'object' && startJson !== null ? (startJson as { id?: unknown }).id : undefined
  const token = typeof startJson === 'object' && startJson !== null ? (startJson as { token?: unknown }).token : undefined
  if (typeof id !== 'string' || typeof token !== 'string') {
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- developer-facing ReportError.message; the dialog localizes by kind
    throw new ReportError('status', 'malformed response from the report service')
  }
  onProgress?.({ sentBytes: firstEnd, totalBytes: total })

  const base = `/report/${encodeURIComponent(id)}`
  let offset = firstEnd
  for (let index = 1; offset < total; index++) {
    const end = Math.min(offset + PIECE_BYTES, total)
    const response = await fetchWithRetry(`${base}/${index}`, {
      method: 'PUT',
      headers: { 'Content-Type': 'application/gzip', 'Hew-Upload-Token': token },
      body: pieceBody(gzipBytes, offset, end),
    })
    if (response.status !== 204) throw reportErrorFromResponse(response.status, await readJson(response))
    offset = end
    onProgress?.({ sentBytes: offset, totalBytes: total })
  }

  const commit = await fetchWithRetry(`${base}/commit`, {
    method: 'POST',
    headers: { 'Hew-Upload-Token': token },
  })
  if (commit.status === 201) return { id, startJson }
  throw reportErrorFromResponse(commit.status, await readJson(commit))
}

/** Web path. Only ever attempted when `canSend()` is true (the caller's
 *  responsibility — this does not re-check). The follow record is stored
 *  only after COMMIT answered 201, so a failed send is never followed. */
async function submitWeb(
  gzipBytes: Uint8Array,
  onProgress?: (progress: ReportProgress) => void,
  follow?: ReportFollowOptions,
): Promise<ReportSubmitResult> {
  const wantsFollow = follow?.follow === true
  const { id, startJson } = await uploadWeb(gzipBytes, onProgress, wantsFollow ? { 'Hew-Follow': '1' } : {})
  const secret = wantsFollow && typeof startJson === 'object' && startJson !== null ? (startJson as { follow?: unknown }).follow : undefined
  if (follow !== undefined && typeof secret === 'string' && secret !== '') {
    const stored = recordWebFollow({ id, secret, excerpt: follow.excerpt, fileName: follow.fileName })
    return { id, following: stored }
  }
  return { id, following: false }
}

/** Web follow-up attachment: a normal upload whose START names the parent
 *  report and proves the secret. */
export async function uploadFollowUpWeb(
  parentId: string,
  secret: string,
  gzipBytes: Uint8Array,
  onProgress?: (progress: ReportProgress) => void,
): Promise<ReportSubmitResult> {
  const { id } = await uploadWeb(gzipBytes, onProgress, { 'Hew-Follow-Up': parentId, 'Hew-Follow-Secret': secret })
  return { id, following: false }
}

/** Submit a report — the gzip of the bundle `reportBundle.ts` builds
 *  (`send.gzip`) — to the intake service, reporting upload progress as it
 *  goes. Rejects with a `ReportError` naming the specific problem; callers
 *  should check `canSend()` first, since a build with nowhere to send has no
 *  reason to attempt this at all. */
export function submitReport(
  gzipBytes: Uint8Array,
  onProgress?: (progress: ReportProgress) => void,
  follow?: ReportFollowOptions,
): Promise<ReportSubmitResult> {
  return isTauri ? submitDesktop(gzipBytes, onProgress, follow) : submitWeb(gzipBytes, onProgress, follow)
}
