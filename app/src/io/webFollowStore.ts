/**
 * webFollowStore — the web shell's followed-report records
 * (docs/design/follow-a-report.md §4.2): `localStorage["hew.followedReports"]`,
 * same-origin under the app's self-only CSP. A record carries the follow
 * secret; the secret is readable only through `getWebFollowSecret`, used by
 * the web host for its own requests, and never reaches the UI or a log.
 *
 * Storage is untrusted input: corrupt JSON, a non-array, or malformed
 * records read as empty / are dropped, and are not rewritten until the next
 * write.
 */

import { computeState, type FollowedReport, type FollowStatus, type ThreadMessage } from './followPolicy'

export const FOLLOWED_REPORTS_KEY = 'hew.followedReports'
/** Where a corrupt stored value is stashed before it is overwritten. */
export const FOLLOWED_REPORTS_CORRUPT_KEY = 'hew.followedReports.corrupt'
export const LAUNCH_CHECK_AT_KEY = 'hew.followedReports.launchCheckAt'

export interface WebFollowRecord extends FollowedReport {
  secret: string
  /** Highest `seenUpTo` the service has acknowledged (internal; never in the public view). */
  seenSynced: number
}

const STATUSES: ReadonlySet<string> = new Set<FollowStatus>(['open', 'need-info', 'fixed', 'closed', 'gone'])
const MAX_EXCERPT = 200

const listeners = new Set<() => void>()

function num(v: unknown, fallback: number): number {
  return typeof v === 'number' && Number.isFinite(v) ? v : fallback
}

function sanitizeMessage(v: unknown): ThreadMessage | null {
  if (typeof v !== 'object' || v === null) return null
  const m = v as Record<string, unknown>
  if (typeof m.seq !== 'number' || !Number.isFinite(m.seq)) return null
  if (m.author !== 'maintainer' && m.author !== 'reporter' && m.author !== 'system') return null
  if (m.kind !== 'text' && m.kind !== 'status' && m.kind !== 'attachment') return null
  const out: ThreadMessage = { seq: m.seq, at: num(m.at, 0), author: m.author, kind: m.kind, text: typeof m.text === 'string' ? m.text : '' }
  if (typeof m.attachmentReportId === 'string') out.attachmentReportId = m.attachmentReportId
  return out
}

function sanitizeRecord(v: unknown): WebFollowRecord | null {
  if (typeof v !== 'object' || v === null) return null
  const r = v as Record<string, unknown>
  if (typeof r.id !== 'string' || r.id === '' || typeof r.secret !== 'string' || r.secret === '') return null
  const messages = Array.isArray(r.messages) ? r.messages.map(sanitizeMessage).filter((m): m is ThreadMessage => m !== null) : []
  const status = typeof r.status === 'string' && STATUSES.has(r.status) ? (r.status as FollowStatus) : 'open'
  const rec: WebFollowRecord = {
    id: r.id,
    secret: r.secret,
    sentAt: num(r.sentAt, 0),
    appVersion: typeof r.appVersion === 'string' ? r.appVersion : '',
    excerpt: typeof r.excerpt === 'string' ? r.excerpt : '',
    fileName: typeof r.fileName === 'string' ? r.fileName : null,
    status,
    statusVersion: typeof r.statusVersion === 'string' ? r.statusVersion : null,
    statusAt: typeof r.statusAt === 'number' ? r.statusAt : null,
    messages,
    latest: num(r.latest, 0),
    seenUpTo: num(r.seenUpTo, 0),
    lastCheckedAt: typeof r.lastCheckedAt === 'number' ? r.lastCheckedAt : null,
    state: r.state === 'done' ? 'done' : 'following',
    seenSynced: Math.max(0, num(r.seenSynced, 0)),
  }
  return rec
}

/** Every stored record; unreadable storage reads as empty. */
export function loadWebFollowRecords(): WebFollowRecord[] {
  try {
    if (typeof localStorage === 'undefined') return []
    const raw = localStorage.getItem(FOLLOWED_REPORTS_KEY)
    if (raw === null) return []
    const parsed: unknown = JSON.parse(raw)
    const list = Array.isArray(parsed) ? parsed : typeof parsed === 'object' && parsed !== null ? (parsed as { reports?: unknown }).reports : undefined
    if (!Array.isArray(list)) return []
    const seen = new Set<string>()
    const out: WebFollowRecord[] = []
    for (const item of list) {
      const rec = sanitizeRecord(item)
      if (rec !== null && !seen.has(rec.id)) {
        seen.add(rec.id)
        out.push(rec)
      }
    }
    return out
  } catch {
    return []
  }
}

/**
 * Writes the records and tells this tab's listeners (other tabs get the
 * `storage` event). Returns whether the write stuck (read back after
 * `setItem`). An unreadable store is never overwritten; a corrupt value is
 * stashed under `FOLLOWED_REPORTS_CORRUPT_KEY` first.
 */
export function saveWebFollowRecords(records: readonly WebFollowRecord[]): boolean {
  let ok = false
  try {
    let raw: string | null
    try {
      raw = localStorage.getItem(FOLLOWED_REPORTS_KEY)
    } catch {
      return false // read error: do not overwrite what we could not read
    }
    if (raw !== null) {
      try {
        JSON.parse(raw)
      } catch {
        try {
          localStorage.setItem(FOLLOWED_REPORTS_CORRUPT_KEY, raw)
        } catch {
          // best effort
        }
      }
    }
    const json = JSON.stringify(records)
    localStorage.setItem(FOLLOWED_REPORTS_KEY, json)
    ok = localStorage.getItem(FOLLOWED_REPORTS_KEY) === json
  } catch {
    // Storage full or blocked: following degrades to nothing rather than breaking the app.
  }
  notifyWebFollowChanged()
  return ok
}

export function notifyWebFollowChanged(): void {
  listeners.forEach((l) => l())
}

/** Subscribes to changes from this tab and (via `storage`) from other tabs. */
export function subscribeWebFollow(cb: () => void): () => void {
  listeners.add(cb)
  const onStorage = (e: StorageEvent) => {
    if (e.key === null || e.key === FOLLOWED_REPORTS_KEY) cb()
  }
  if (typeof window !== 'undefined') window.addEventListener('storage', onStorage)
  return () => {
    listeners.delete(cb)
    if (typeof window !== 'undefined') window.removeEventListener('storage', onStorage)
  }
}

/** The record shape with the secret removed. */
export function publicReport(rec: WebFollowRecord): FollowedReport {
  const { secret: _secret, seenSynced: _synced, ...rest } = rec
  return { ...rest, messages: rest.messages.map((m) => ({ ...m })) }
}

declare const __HEW_VERSION__: string | undefined

/** Stores a record for a report that just committed with a follow secret. */
export function recordWebFollow(input: { id: string; secret: string; excerpt: string; fileName: string | null }): boolean {
  const rec: WebFollowRecord = {
    id: input.id,
    secret: input.secret,
    sentAt: Date.now(),
    appVersion: typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '',
    excerpt: input.excerpt.slice(0, MAX_EXCERPT),
    fileName: input.fileName,
    status: 'open',
    statusVersion: null,
    statusAt: null,
    messages: [],
    latest: 0,
    seenUpTo: 0,
    lastCheckedAt: null,
    state: 'following',
    seenSynced: 0,
  }
  rec.state = computeState(rec)
  const rest = loadWebFollowRecords().filter((r) => r.id !== rec.id)
  return saveWebFollowRecords([rec, ...rest])
}
