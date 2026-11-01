/**
 * followClient — same-origin `fetch` client for the thread routes
 * (docs/design/follow-a-report.md §5.3/§5.4). Web shell only; the desktop
 * shell does this in Rust. Every request carries `Hew-Follow-Secret`; the
 * secret is never logged or put in an error message. Failures are
 * `ReportError`s with reportClient's kinds; a 403 is `status` 403 (the
 * thread is gone for this reporter).
 */

import { ReportError } from './reportClient'
import type { ThreadMessage } from './followPolicy'

export const LAUNCH_TIMEOUT_MS = 10_000
export const MANUAL_TIMEOUT_MS = 20_000

export interface ThreadView {
  format: number
  status: string
  statusVersion: string | null
  statusAt: number | null
  latest: number
  messages: ThreadMessage[]
  now: number
}

/* oxlint-disable lingui/no-unlocalized-strings -- ReportError.message is developer-facing; the UI maps `kind` to localized copy */
function errorFromResponse(response: Response, body: unknown): ReportError {
  const status = response.status
  const error = typeof body === 'object' && body !== null ? (body as { error?: unknown }).error : undefined
  if (status === 400) return new ReportError('invalid', 'the message was rejected as invalid', status)
  if (status === 429) {
    const ra = Number(response.headers.get('Retry-After'))
    return new ReportError('rateLimited', 'too many requests', status, Number.isFinite(ra) && ra > 0 ? ra : undefined)
  }
  if (status === 409) return new ReportError('full', 'the thread is full', status)
  if (status >= 500) return new ReportError('unreachable', 'the report service is unavailable', status)
  return new ReportError('status', typeof error === 'string' && error === 'forbidden' ? 'forbidden' : `unexpected status ${status}`, status)
}

async function request(path: string, secret: string, init: RequestInit, timeoutMs: number): Promise<{ response: Response; body: unknown }> {
  const controller = new AbortController()
  const timer = setTimeout(() => controller.abort(), timeoutMs)
  let response: Response
  try {
    response = await fetch(path, {
      ...init,
      headers: { ...(init.headers as Record<string, string> | undefined), 'Hew-Follow-Secret': secret },
      cache: 'no-store',
      signal: controller.signal,
    })
  } catch {
    clearTimeout(timer)
    throw new ReportError('offline', 'could not reach the report service')
  }
  // The timer stays armed until the body has been read, so a stalled body aborts too.
  let body: unknown = null
  try {
    if (response.status !== 204) {
      try {
        body = await response.json()
      } catch {
        body = null
      }
    }
  } finally {
    clearTimeout(timer)
  }
  return { response, body }
}
/* oxlint-enable lingui/no-unlocalized-strings */

const threadPath = (id: string) => `/report/${encodeURIComponent(id)}/thread`

function parseThread(body: unknown): ThreadView {
  const b = (typeof body === 'object' && body !== null ? body : {}) as Record<string, unknown>
  const messages = Array.isArray(b.messages) ? (b.messages as ThreadMessage[]).filter((m) => typeof m === 'object' && m !== null && typeof m.seq === 'number') : []
  if (typeof b.status !== 'string' || typeof b.latest !== 'number') {
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- developer-facing
    throw new ReportError('status', 'malformed response from the report service')
  }
  return {
    format: typeof b.format === 'number' ? b.format : 1,
    status: b.status,
    statusVersion: typeof b.statusVersion === 'string' ? b.statusVersion : null,
    statusAt: typeof b.statusAt === 'number' ? b.statusAt : null,
    latest: b.latest,
    messages,
    now: typeof b.now === 'number' ? b.now : Date.now(),
  }
}

export async function fetchThread(id: string, secret: string, after: number, timeoutMs: number): Promise<ThreadView> {
  const query = `?after=${Math.max(0, Math.floor(after))}` // oxlint-disable-line lingui/no-unlocalized-strings -- URL query
  const { response, body } = await request(threadPath(id) + query, secret, { method: 'GET' }, timeoutMs)
  if (response.status !== 200) throw errorFromResponse(response, body)
  return parseThread(body)
}

export async function postThreadMessage(id: string, secret: string, text: string): Promise<number> {
  const { response, body } = await request(
    threadPath(id),
    secret,
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ text }) },
    MANUAL_TIMEOUT_MS,
  )
  if (response.status !== 201) throw errorFromResponse(response, body)
  const seq = typeof body === 'object' && body !== null ? (body as { seq?: unknown }).seq : undefined
  return typeof seq === 'number' ? seq : 0
}

export async function postThreadSeen(id: string, secret: string, upTo: number): Promise<void> {
  const { response, body } = await request(
    `${threadPath(id)}/seen`,
    secret,
    { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ upTo }) },
    MANUAL_TIMEOUT_MS,
  )
  if (response.status !== 204) throw errorFromResponse(response, body)
}

export async function deleteThread(id: string, secret: string): Promise<void> {
  const { response, body } = await request(threadPath(id), secret, { method: 'DELETE' }, MANUAL_TIMEOUT_MS)
  if (response.status !== 204) throw errorFromResponse(response, body)
}
