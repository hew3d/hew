/**
 * FollowNoticeBar — the dismissible bar shown when a check finds news on a
 * followed report (docs/design/follow-a-report.md §2.3, D15). The kernel
 * error banner's shape, but not sticky, and not a toast. "Later" hides it
 * for this launch and marks nothing seen.
 */

import { useEffect, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { getFollowHost, hasNews, type FollowHost, type FollowedReport } from '../io/followHost'

export interface FollowNoticeBarProps {
  /** View the reports; `id` is set when exactly one report has news. */
  onView: (id: string | null) => void
  /** Test seam; defaults to `getFollowHost()`. */
  host?: FollowHost
}

const BAR_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  borderBottom: '1px solid var(--border-strong, #4a4a4a)',
  color: 'var(--text-primary, #eee)',
  padding: '8px 16px',
  display: 'flex',
  alignItems: 'center',
  gap: '12px',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  fontSize: '13px',
  flexShrink: 0,
  zIndex: 200,
}

const BUTTON_STYLE: React.CSSProperties = {
  padding: '4px 14px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: '13px',
  fontFamily: 'inherit',
  cursor: 'pointer',
}

const PRIMARY_STYLE: React.CSSProperties = {
  ...BUTTON_STYLE,
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
}

/** The followed reports, kept current across host changes. */
export function useFollowedReports(host?: FollowHost): FollowedReport[] {
  const [reports, setReports] = useState<FollowedReport[]>([])
  useEffect(() => {
    const h = host ?? getFollowHost()
    let cancelled = false
    const load = () => {
      void h
        .list()
        .then((list) => {
          if (!cancelled) setReports(Array.isArray(list) ? list : [])
        })
        .catch(() => {})
    }
    load()
    const unsub = h.subscribe(load)
    return () => {
      cancelled = true
      unsub()
    }
  }, [host])
  return reports
}

export function FollowNoticeBar({ onView, host }: FollowNoticeBarProps) {
  const { t } = useLingui()
  const reports = useFollowedReports(host)
  const [later, setLater] = useState(false)
  const withNews = reports.filter(hasNews)
  if (later || withNews.length === 0) return null
  const count = withNews.length
  const only = withNews[0]!
  const onlyId = only.id
  // "Reply" only when a maintainer wrote text; status changes and system notes are "an update".
  const isReply = (r: FollowedReport) => r.messages.some((m) => m.seq > r.seenUpTo && m.author === 'maintainer' && m.kind === 'text')
  const allUpdates = withNews.every((r) => !isReply(r))
  const allReplies = withNews.every(isReply)
  return (
    <div style={BAR_STYLE} role="status" data-testid="follow-notice-bar">
      <span style={{ flex: 1 }}>
        {count === 1 ? (
          isReply(only) ? (
            <Trans>
              Your bug report {onlyId} has a reply from Hew’s developer.
            </Trans>
          ) : (
            <Trans>
              Your bug report {onlyId} has an update from Hew’s developer.
            </Trans>
          )
        ) : allReplies ? (
          t`${count} of your bug reports have replies.`
        ) : allUpdates ? (
          t`${count} of your bug reports have updates.`
        ) : (
          t`${count} of your bug reports have news.`
        )}
      </span>
      <button type="button" style={PRIMARY_STYLE} data-testid="follow-notice-view" onClick={() => onView(count === 1 ? only.id : null)}>
        {t`View`}
      </button>
      <button type="button" style={BUTTON_STYLE} data-testid="follow-notice-later" onClick={() => setLater(true)}>
        {t`Later`}
      </button>
    </div>
  )
}
