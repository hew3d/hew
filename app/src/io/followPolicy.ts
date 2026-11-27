/**
 * followPolicy — the pure rules of "Follow a bug report"
 * (docs/design/follow-a-report.md §6.3): version compare, unread counting,
 * and when a launch check may run. No I/O, so the web host, the UI, and the
 * tests share one definition. `followHost.ts` re-exports everything here.
 */

export type ThreadAuthor = 'maintainer' | 'reporter' | 'system'
export type ThreadMessageKind = 'text' | 'status' | 'attachment'

export interface ThreadMessage {
  seq: number
  at: number
  author: ThreadAuthor
  kind: ThreadMessageKind
  text: string
  attachmentReportId?: string
}

export type FollowStatus = 'open' | 'need-info' | 'fixed' | 'closed' | 'gone'

export interface FollowedReport {
  id: string
  sentAt: number
  appVersion: string
  excerpt: string
  /** The failed-open file sent with the report, if any. */
  fileName: string | null
  status: FollowStatus
  statusVersion: string | null
  statusAt: number | null
  messages: ThreadMessage[]
  latest: number
  seenUpTo: number
  lastCheckedAt: number | null
  /** `done`: a final status has been shown (or the thread is gone). */
  state: 'following' | 'done'
}

export type CheckReason = 'launch' | 'manual'

export interface CheckSummary {
  checked: number
  withNews: number
  newCount: number
  failed: number
}

export interface CheckResult {
  reports: FollowedReport[]
  summary: CheckSummary
}

/** Launch checks skip a report checked more recently than this. */
export const LAUNCH_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000
/** A report still `following` this long after sending stops being polled at launch. */
export const FOLLOW_MAX_AGE_MS = 365 * 24 * 60 * 60 * 1000
/** Delay between app mount and the launch check. */
export const LAUNCH_CHECK_DELAY_MS = 15_000

/**
 * Compares dotted versions: negative when `a` is older than `b`, 0 when
 * equal, positive when newer. A pre-release (`1.4.0-pre.1`) compares lower
 * than its release (`1.4.0`); pre-release identifiers compare per semver
 * (numeric < alphanumeric, numeric by value). Build metadata is ignored.
 */
export function compareVersions(a: string, b: string): number {
  const parse = (v: string) => {
    const [core = '', pre] = v.trim().replace(/^v/, '').split('+')[0]!.split(/-(.*)/s)
    const nums = core.split('.').map((p) => {
      const n = Number.parseInt(p, 10)
      return Number.isFinite(n) ? n : 0
    })
    return { nums, pre: pre === undefined || pre === '' ? null : pre.split('.') }
  }
  const x = parse(a)
  const y = parse(b)
  const len = Math.max(x.nums.length, y.nums.length, 3)
  for (let i = 0; i < len; i++) {
    const d = (x.nums[i] ?? 0) - (y.nums[i] ?? 0)
    if (d !== 0) return d < 0 ? -1 : 1
  }
  if (x.pre === null && y.pre === null) return 0
  if (x.pre === null) return 1
  if (y.pre === null) return -1
  const n = Math.max(x.pre.length, y.pre.length)
  for (let i = 0; i < n; i++) {
    const p = x.pre[i]
    const q = y.pre[i]
    if (p === undefined) return -1
    if (q === undefined) return 1
    const pn = /^\d+$/.test(p)
    const qn = /^\d+$/.test(q)
    if (pn && qn) {
      const d = Number(p) - Number(q)
      if (d !== 0) return d < 0 ? -1 : 1
    } else if (pn !== qn) {
      return pn ? -1 : 1
    } else if (p !== q) {
      return p < q ? -1 : 1
    }
  }
  return 0
}

/** Whether a message is news to the reporter: from the maintainer or the
 *  system, or a status change (the reporter's own posts are not). */
export function isNewsMessage(m: ThreadMessage): boolean {
  return m.author === 'maintainer' || m.author === 'system' || m.kind === 'status'
}

/** Messages the reporter has not yet viewed. A gone report has none. */
export function unreadCount(report: FollowedReport): number {
  if (report.status === 'gone') return 0
  let n = 0
  for (const m of report.messages) if (m.seq > report.seenUpTo && isNewsMessage(m)) n++
  return n
}

export function hasNews(report: FollowedReport): boolean {
  return unreadCount(report) > 0
}

/** Total unread across reports, for the Help menu's "(n new)". */
export function totalUnread(reports: readonly FollowedReport[]): number {
  return reports.reduce((n, r) => n + unreadCount(r), 0)
}

/** The seq of the last status message, 0 when there is none. */
export function lastStatusSeq(report: Pick<FollowedReport, 'messages'>): number {
  let last = 0
  for (const m of report.messages) if (m.kind === 'status' && m.seq > last) last = m.seq
  return last
}

/** `done` once the reporter has seen a final status (or the thread is gone). */
export function computeState(report: FollowedReport): 'following' | 'done' {
  if (report.status === 'gone') return 'done'
  if ((report.status === 'fixed' || report.status === 'closed') && report.seenUpTo >= lastStatusSeq(report)) return 'done'
  return 'following'
}

/** A `following` report this old is no longer polled automatically (Check now still works). */
export function isPastAutoCheckAge(report: FollowedReport, now: number): boolean {
  return now - report.sentAt >= FOLLOW_MAX_AGE_MS
}

/** Whether the launch check may poll this report right now (§6.3). */
export function shouldCheckAtLaunch(report: FollowedReport, now: number): boolean {
  if (report.state !== 'following' || report.status === 'gone') return false
  if (isPastAutoCheckAge(report, now)) return false
  // A future timestamp (clock moved back) counts as due, like the desktop shell.
  return report.lastCheckedAt === null || report.lastCheckedAt > now || now - report.lastCheckedAt >= LAUNCH_CHECK_INTERVAL_MS
}
