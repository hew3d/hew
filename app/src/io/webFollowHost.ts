/**
 * webFollowHost — the web shell's `FollowHost` (docs/design/follow-a-report.md
 * §6.1, §6.3, §6.4): records in localStorage (`webFollowStore.ts`), the
 * network through `followClient.ts`. The secret never leaves this module
 * except in the `Hew-Follow-Secret` header of a same-origin request.
 */

import type { FollowHost } from './followHost'
import {
  LAUNCH_CHECK_DELAY_MS,
  LAUNCH_CHECK_INTERVAL_MS,
  computeState,
  isNewsMessage,
  shouldCheckAtLaunch,
  type CheckReason,
  type CheckResult,
  type CheckSummary,
  type FollowStatus,
  type ThreadMessage,
} from './followPolicy'
import { deleteThread, fetchThread, type ThreadView, LAUNCH_TIMEOUT_MS, MANUAL_TIMEOUT_MS, postThreadMessage, postThreadSeen } from './followClient'
import { ReportError, uploadFollowUpWeb, type ReportProgress } from './reportClient'
import {
  LAUNCH_CHECK_AT_KEY,
  loadWebFollowRecords,
  notifyWebFollowChanged,
  publicReport,
  saveWebFollowRecords,
  subscribeWebFollow,
  type WebFollowRecord,
} from './webFollowStore'

const KNOWN_STATUS: ReadonlySet<string> = new Set<FollowStatus>(['open', 'need-info', 'fixed', 'closed'])

/** A 403 means "gone" only when the service said exactly `{"error":"forbidden"}`
 *  (followClient.errorFromResponse sets that message only for that body). */
function isForbidden(err: unknown): boolean {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- compared with followClient's developer-facing message
  return err instanceof ReportError && err.kind === 'status' && err.status === 403 && err.message === 'forbidden'
}

/** Merges a thread view's messages (by seq) and status into a record. */
export function mergeThread(
  rec: WebFollowRecord,
  view: { status: string; statusVersion: string | null; statusAt: number | null; latest: number; messages: ThreadMessage[] },
  now: number,
): { rec: WebFollowRecord; newCount: number } {
  const bySeq = new Map<number, ThreadMessage>(rec.messages.map((m) => [m.seq, m]))
  let newCount = 0
  for (const m of view.messages) {
    if (!bySeq.has(m.seq)) {
      bySeq.set(m.seq, m)
      if (isNewsMessage(m) && m.seq > rec.seenUpTo) newCount++
    }
  }
  const messages = [...bySeq.values()].sort((a, b) => a.seq - b.seq)
  // A view older than what is stored (a slow concurrent refresh) must not roll the status back.
  const fresh = view.latest >= rec.latest
  const next: WebFollowRecord = {
    ...rec,
    messages,
    latest: Math.max(rec.latest, view.latest, messages.length > 0 ? messages[messages.length - 1]!.seq : 0),
    status: fresh && KNOWN_STATUS.has(view.status) ? (view.status as FollowStatus) : rec.status,
    statusVersion: fresh ? view.statusVersion : rec.statusVersion,
    statusAt: fresh ? view.statusAt : rec.statusAt,
    lastCheckedAt: now,
  }
  next.state = computeState(next)
  return { rec: next, newCount }
}

function goneRecord(rec: WebFollowRecord, now: number): WebFollowRecord {
  return { ...rec, status: 'gone', state: 'done', lastCheckedAt: now }
}

/**
 * Re-reads the stored record by id (after any await, so concurrent changes
 * are not clobbered), applies `fn`, and saves. A record forgotten meanwhile
 * is not resurrected. Returns the stored record (or undefined if gone).
 */
function mutate(id: string, fn: (cur: WebFollowRecord) => WebFollowRecord): WebFollowRecord | undefined {
  const all = loadWebFollowRecords()
  const cur = all.find((r) => r.id === id)
  if (cur === undefined) return undefined
  const next = fn(cur)
  if (next !== cur) saveWebFollowRecords(all.map((r) => (r.id === id ? next : r)))
  return next
}

type Fetched = { kind: 'view'; view: ThreadView } | { kind: 'gone' } | { kind: 'failed' }

async function fetchOne(rec: WebFollowRecord, timeoutMs: number): Promise<Fetched> {
  try {
    return { kind: 'view', view: await fetchThread(rec.id, rec.secret, rec.latest, timeoutMs) }
  } catch (err) {
    return isForbidden(err) ? { kind: 'gone' } : { kind: 'failed' }
  }
}

/** Applies a fetch result onto the freshly re-read stored record. */
function applyFetched(id: string, fetched: Fetched): { rec: WebFollowRecord | undefined; newCount: number } {
  if (fetched.kind === 'failed') return { rec: loadWebFollowRecords().find((r) => r.id === id), newCount: 0 }
  const now = Date.now()
  let newCount = 0
  const rec = mutate(id, (cur) => {
    if (fetched.kind === 'gone') return goneRecord(cur, now)
    const merged = mergeThread(cur, fetched.view, now)
    newCount = merged.newCount
    return merged.rec
  })
  return { rec, newCount }
}

/** Sends the seen mark when the service has not acknowledged it yet. */
async function syncSeen(id: string): Promise<void> {
  const rec = loadWebFollowRecords().find((r) => r.id === id)
  if (rec === undefined || rec.status === 'gone' || rec.seenSynced >= rec.seenUpTo) return
  const sent = rec.seenUpTo
  try {
    await postThreadSeen(id, rec.secret, sent)
    mutate(id, (cur) => (cur.seenSynced >= sent ? cur : { ...cur, seenSynced: sent }))
  } catch (err) {
    if (isForbidden(err)) applyFetched(id, { kind: 'gone' })
    // Any other failure: the local mark stands; the next markSeen / open resends.
  }
}

export function createWebFollowHost(): FollowHost {
  const find = (id: string) => loadWebFollowRecords().find((r) => r.id === id)

  async function check(reason: CheckReason, id?: string): Promise<CheckResult> {
    const now = Date.now()
    let targets = loadWebFollowRecords()
    if (id !== undefined) targets = targets.filter((r) => r.id === id)
    else targets = targets.filter((r) => (reason === 'launch' ? shouldCheckAtLaunch(r, now) : r.state === 'following' && r.status !== 'gone'))
    const summary: CheckSummary = { checked: 0, withNews: 0, newCount: 0, failed: 0 }
    const timeout = reason === 'launch' ? LAUNCH_TIMEOUT_MS : MANUAL_TIMEOUT_MS
    for (const target of targets) {
      const fetched = await fetchOne(target, timeout)
      summary.checked++
      if (fetched.kind === 'failed') summary.failed++
      else {
        const res = applyFetched(target.id, fetched)
        summary.newCount += res.newCount
        if (res.newCount > 0) summary.withNews++
        // A seen mark that never got through (a network blip) is retried here,
        // so the service learns the final status was seen.
        if (fetched.kind === 'view') await syncSeen(target.id)
      }
    }
    // A done report is never fetched again, but its final seen mark may
    // still be unacknowledged (the blip happened on the last view). The
    // service's 30-day clock starts only from that mark (design §5.5), so a
    // launch check resends it without fetching anything.
    if (reason === 'launch' && id === undefined) {
      for (const rec of loadWebFollowRecords()) {
        if (rec.state === 'done' && rec.status !== 'gone' && rec.seenSynced < rec.seenUpTo) await syncSeen(rec.id)
      }
    }
    return { reports: loadWebFollowRecords().map(publicReport), summary }
  }

  return {
    available: () => true,
    list: async () => loadWebFollowRecords().map(publicReport).sort((a, b) => b.sentAt - a.sentAt),
    check,
    async post(id, text) {
      const rec = find(id)
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- developer-facing
      if (rec === undefined) throw new ReportError('status', 'unknown report', 403)
      try {
        await postThreadMessage(id, rec.secret, text)
      } catch (err) {
        if (isForbidden(err)) applyFetched(id, { kind: 'gone' })
        throw err
      }
      // Our own message (and any reopen) comes back through the normal merge.
      const res = applyFetched(id, await fetchOne(rec, MANUAL_TIMEOUT_MS))
      return publicReport(res.rec ?? rec)
    },
    async markSeen(id, upTo) {
      const next = mutate(id, (cur) => {
        const seenUpTo = Math.max(cur.seenUpTo, Math.min(upTo, cur.latest))
        const updated: WebFollowRecord = { ...cur, seenUpTo }
        updated.state = computeState(updated)
        return seenUpTo !== cur.seenUpTo || updated.state !== cur.state ? updated : cur
      })
      if (next === undefined) return null
      await syncSeen(id)
      const out = find(id)
      return out === undefined ? null : publicReport(out)
    },
    async attach(id, bundleGzip, onProgress?: (p: ReportProgress) => void) {
      const rec = find(id)
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- developer-facing
      if (rec === undefined) throw new ReportError('status', 'unknown report', 403)
      const result = await uploadFollowUpWeb(id, rec.secret, bundleGzip, onProgress)
      applyFetched(id, await fetchOne(rec, MANUAL_TIMEOUT_MS))
      return result
    },
    async forget(id) {
      const rec = find(id)
      if (rec === undefined) return
      try {
        await deleteThread(id, rec.secret)
      } catch {
        // Best effort: the local record goes either way (a stale server
        // thread expires on its own).
      }
      saveWebFollowRecords(loadWebFollowRecords().filter((r) => r.id !== id))
    },
    subscribe: subscribeWebFollow,
  }
}

/**
 * Starts the once-per-launch check (§6.4): 15 seconds from now, skipped when
 * the page has already launched a check within 6 hours (a reload is not a
 * new launch) or the browser is offline. Call once when the app mounts;
 * returns a cancel function.
 */
export function startWebLaunchCheck(host: FollowHost, delayMs: number = LAUNCH_CHECK_DELAY_MS): () => void {
  const timer = setTimeout(() => {
    try {
      if (typeof navigator !== 'undefined' && navigator.onLine === false) return
      if (loadWebFollowRecords().length === 0) return
      const last = Number(localStorage.getItem(LAUNCH_CHECK_AT_KEY))
      const now = Date.now()
      if (Number.isFinite(last) && last > 0 && last <= now && now - last < LAUNCH_CHECK_INTERVAL_MS) return
    } catch {
      return
    }
    // The stamp is written after the check, and only when it got somewhere
    // (something was fetched, or nothing was due), so an online-but-failed
    // check is retried on the next reload instead of six hours later.
    void host.check('launch').then(
      (res) => {
        const { checked, failed } = res.summary
        if (checked === 0 || failed < checked) {
          try {
            localStorage.setItem(LAUNCH_CHECK_AT_KEY, String(Date.now()))
          } catch {
            // Storage unavailable: the next launch checks again.
          }
        }
        notifyWebFollowChanged()
      },
      () => {},
    )
  }, delayMs)
  return () => clearTimeout(timer)
}
