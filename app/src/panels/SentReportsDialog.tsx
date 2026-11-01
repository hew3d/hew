/**
 * SentReportsDialog — Help ▸ Sent Reports… (docs/design/follow-a-report.md
 * §2.2). A modal in the Report Bug dialog's family: the reports this copy of
 * Hew is following on the left, the selected report's thread on the right.
 *
 * Maintainer text is the only text that is ever linkified, and only for
 * `http`/`https` URLs (D18); the reporter's own messages are plain text.
 * Everything React renders is escaped, so nothing here is markup.
 */

import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { useLingui } from '@lingui/react/macro'
import { getFollowHost, compareVersions, unreadCount, type FollowHost, type FollowedReport, type ThreadMessage } from '../io/followHost'
import { ReportError } from '../io/reportClient'
import { buildFollowUpUpload, detectPlatform, type FollowUpFile } from '../log/reportBundle'
import { getFailedOpen } from '../log/failedOpen'
import { isTauri } from '../io/fileHost'
import { isMac, isWindows, isLinux } from '../platform'
import { isComposingKey } from '../input/keyboard'
import { openExternal } from './ReportBugDialog'

export const REPLY_MAX = 4000

export interface SentReportsDialogProps {
  onClose: () => void
  /** Select this report first (the notice bar's View with one report). */
  initialId?: string | null
  /** Present only on a desktop build that carries the updater. */
  onCheckForUpdates?: () => void
  /** Test seam; defaults to `getFollowHost()`. */
  host?: FollowHost
}

/** Splits `text` into plain runs and `http`/`https` URLs. Never matches any
 *  other scheme, so `javascript:` and friends stay inert text. */
const URL_RE = /https?:\/\/[^\s<>"'`]+/gi
const TRAILING_PUNCT = /[.,;:!?\]}]+$/

/** Strips trailing sentence punctuation; a trailing `)` goes only when the
 *  URL has more `)` than `(` (so `(see https://x.y)` loses it but
 *  `https://en.wikipedia.org/wiki/Foo_(bar)` keeps it). */
function trimUrl(raw: string): string {
  let url = raw
  for (;;) {
    const stripped = url.replace(TRAILING_PUNCT, '')
    if (stripped !== url) {
      url = stripped
      continue
    }
    if (url.endsWith(')') && url.split(')').length > url.split('(').length) {
      url = url.slice(0, -1)
      continue
    }
    break
  }
  return url.length > 0 ? url : raw
}

export function linkifySegments(text: string): { text: string; url: string | null }[] {
  const out: { text: string; url: string | null }[] = []
  let last = 0
  for (const m of text.matchAll(URL_RE)) {
    const start = m.index ?? 0
    const raw = trimUrl(m[0])
    let url: string | null = null
    try {
      const u = new URL(raw)
      if (u.protocol === 'http:' || u.protocol === 'https:') url = u.href
    } catch {
      url = null
    }
    if (start > last) out.push({ text: text.slice(last, start), url: null })
    out.push({ text: raw, url })
    last = start + raw.length
  }
  if (last < text.length) out.push({ text: text.slice(last), url: null })
  return out
}

function currentVersion(): string {
  return typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0'
}

function dateOf(ms: number): string {
  const d = new Date(ms)
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

const OVERLAY_STYLE: React.CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'var(--backdrop-dim, rgba(0,0,0,0.6))',
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  zIndex: 2000,
}

const DIALOG_STYLE: React.CSSProperties = {
  background: 'var(--surface-overlay, #2a2a2a)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 6px)',
  boxShadow: 'var(--shadow-palette, 0 8px 32px rgba(0,0,0,0.6))',
  padding: '20px 24px',
  width: '820px',
  maxWidth: '94vw',
  height: '560px',
  maxHeight: '88vh',
  display: 'flex',
  flexDirection: 'column',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '12px',
}

const BODY_STYLE: React.CSSProperties = { display: 'flex', gap: '16px', flex: 1, minHeight: 0 }

const LIST_STYLE: React.CSSProperties = {
  width: '250px',
  flexShrink: 0,
  overflowY: 'auto',
  borderRight: '1px solid var(--border-subtle, #3a3a3a)',
  paddingRight: '8px',
}

const ITEM_STYLE: React.CSSProperties = {
  display: 'block',
  width: '100%',
  textAlign: 'left',
  background: 'none',
  border: '1px solid transparent',
  borderRadius: 'var(--radius-control, 4px)',
  padding: '6px 8px',
  marginBottom: '4px',
  cursor: 'pointer',
  color: 'inherit',
  font: 'inherit',
}

const ITEM_SELECTED_STYLE: React.CSSProperties = {
  ...ITEM_STYLE,
  background: 'var(--surface-input, #1c1c1c)',
  border: '1px solid var(--border-strong, #4a4a4a)',
}

const SMALL_STYLE: React.CSSProperties = { fontSize: '11px', color: 'var(--text-tertiary, #999)' }

const PILL_STYLE: React.CSSProperties = {
  fontSize: '10px',
  padding: '1px 6px',
  borderRadius: '8px',
  border: '1px solid var(--border-strong, #4a4a4a)',
  color: 'var(--text-secondary, #ddd)',
  whiteSpace: 'nowrap',
}

const DOT_STYLE: React.CSSProperties = {
  display: 'inline-block',
  width: '8px',
  height: '8px',
  borderRadius: '50%',
  background: 'var(--accent-base, #6a9fe0)',
  marginRight: '6px',
}

const PANE_STYLE: React.CSSProperties = { flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column' }

const MESSAGES_STYLE: React.CSSProperties = {
  flex: 1,
  minHeight: 0,
  overflowY: 'auto',
  margin: '10px 0',
  display: 'flex',
  flexDirection: 'column',
  gap: '8px',
}

const BUBBLE_STYLE: React.CSSProperties = {
  maxWidth: '80%',
  padding: '6px 10px',
  borderRadius: 'var(--radius-control, 6px)',
  background: 'var(--surface-input, #1c1c1c)',
  border: '1px solid var(--border-subtle, #3a3a3a)',
  whiteSpace: 'pre-wrap',
  wordBreak: 'break-word',
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-primary, #eee)',
}

const TEXTAREA_STYLE: React.CSSProperties = {
  width: '100%',
  padding: '7px 8px',
  background: 'var(--surface-input, #1c1c1c)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-body, 12px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  boxSizing: 'border-box',
  resize: 'vertical',
  minHeight: '56px',
}

const SECONDARY_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 14px',
  background: 'var(--surface-input, #444)',
  color: 'var(--text-primary, #eee)',
  border: '1px solid var(--border-strong, transparent)',
  borderRadius: 'var(--radius-control, 4px)',
  fontSize: 'var(--font-size-menu-item, 13px)',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  cursor: 'pointer',
}

const PRIMARY_BUTTON_STYLE: React.CSSProperties = {
  ...SECONDARY_BUTTON_STYLE,
  background: 'var(--accent-base, #3a5e9e)',
  color: 'var(--accent-text-strong, #fff)',
  border: 'none',
}

const LINK_STYLE: React.CSSProperties = {
  color: 'var(--accent-base, #6a9fe0)',
  cursor: 'pointer',
  textDecoration: 'underline',
  background: 'none',
  border: 'none',
  padding: 0,
  font: 'inherit',
}

const ERROR_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--danger-text, #e88)',
  marginTop: '8px',
  whiteSpace: 'pre-wrap',
}

const ROW_STYLE: React.CSSProperties = { display: 'flex', gap: '8px', flexWrap: 'wrap', alignItems: 'center', marginTop: '8px' }

/** Maintainer text with `http`/`https` URLs as links; reporter and system
 *  text always go through `PlainText`. */
function Linkified({ text }: { text: string }) {
  return (
    <>
      {linkifySegments(text).map((seg, i) =>
        seg.url === null ? (
          <Fragment key={i}>{seg.text}</Fragment>
        ) : (
          <a
            key={i}
            href={seg.url}
            target="_blank"
            rel="noopener noreferrer"
            style={LINK_STYLE}
            onClick={(e) => {
              e.preventDefault()
              openExternal(seg.url as string)
            }}
          >
            {seg.text}
          </a>
        ),
      )}
    </>
  )
}

export function SentReportsDialog({ onClose, initialId = null, onCheckForUpdates, host: hostProp }: SentReportsDialogProps) {
  const { t } = useLingui()
  const host = useMemo(() => hostProp ?? getFollowHost(), [hostProp])
  const available = host.available()
  const [reports, setReports] = useState<FollowedReport[] | null>(null)
  const [selectedId, setSelectedId] = useState<string | null>(initialId)
  const [checking, setChecking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [reply, setReply] = useState('')
  const [replying, setReplying] = useState(false)
  const [attachPercent, setAttachPercent] = useState<number | null>(null)
  const [attaching, setAttaching] = useState(false)
  const [attachedName, setAttachedName] = useState<string | null>(null)
  const [confirmForget, setConfirmForget] = useState(false)
  const dialogRef = useRef<HTMLDivElement>(null)
  const fileInputRef = useRef<HTMLInputElement>(null)
  const mountedRef = useRef(true)
  /** Set synchronously when an upload/reply starts, so a second click cannot start another. */
  const busyRef = useRef(false)
  const attachFocusedRef = useRef(false)
  const selectedRef = useRef(selectedId)
  selectedRef.current = selectedId
  const failedOpen = getFailedOpen()
  const failedName = failedOpen?.name ?? ''

  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const errorText = useCallback(
    (err: unknown): string => {
      if (err instanceof ReportError) {
        switch (err.kind) {
          case 'offline':
          case 'unreachable':
            return t`Couldn’t reach the report service. Check your internet connection and try again.`
          case 'rateLimited': {
            const secs = err.retryAfter
            return secs !== undefined && secs > 0
              ? t`Too many requests right now. Try again in ${secs} seconds.`
              : t`Too many requests right now. Try again in a few minutes.`
          }
          case 'full':
            return t`The report service is temporarily full. Try again later.`
          case 'tooLarge':
            return t`That file is too large to send.`
          case 'tls':
            return t`The report service’s certificate isn’t trusted by this computer.`
          case 'invalid':
            return t`The report service didn’t accept that.`
          case 'status':
            return t`Unexpected response from the report service.`
        }
      }
      return err instanceof Error ? err.message : String(err)
    },
    [t],
  )

  const refresh = useCallback(async () => {
    try {
      const list = await host.list()
      if (!mountedRef.current) return
      // Newest first.
      setReports([...list].sort((a, b) => b.sentAt - a.sentAt))
    } catch (err) {
      if (mountedRef.current) {
        setReports((r) => r ?? [])
        setError(errorText(err))
      }
    }
  }, [host, errorText])

  // Opening the dialog refreshes every followed report.
  useEffect(() => {
    if (!available) {
      setReports([])
      return
    }
    let cancelled = false
    void (async () => {
      await refresh()
      if (cancelled) return
      setChecking(true)
      try {
        await host.check('manual')
      } catch (err) {
        if (!cancelled && mountedRef.current) setError(errorText(err))
      }
      if (cancelled || !mountedRef.current) return
      setChecking(false)
      await refresh()
    })()
    const unsub = host.subscribe(() => void refresh())
    return () => {
      cancelled = true
      unsub()
    }
  }, [host, available, refresh, errorText])

  // Default selection: the asked-for report, else one with news, else newest.
  useEffect(() => {
    if (reports === null || reports.length === 0) return
    if (selectedId !== null && reports.some((r) => r.id === selectedId)) return
    const pick = reports.find((r) => unreadCount(r) > 0) ?? reports[0]!
    setSelectedId(pick.id)
  }, [reports, selectedId])

  const selected = reports?.find((r) => r.id === selectedId) ?? null

  // Viewing a thread marks everything in it seen.
  // Keep the newest message in view when the thread opens or grows.
  const messagesRef = useRef<HTMLDivElement | null>(null)
  const messageCount = selected?.messages.length ?? 0
  useEffect(() => {
    const el = messagesRef.current
    if (el !== null) el.scrollTop = el.scrollHeight
  }, [selectedId, messageCount])
  const seenKey = selected === null ? null : `${selected.id}:${selected.latest}:${selected.seenUpTo}`
  useEffect(() => {
    // Also when nothing new arrived: an earlier seen mark may never have
    // reached the service, and the host resends it (a no-op when in sync).
    if (selected === null || selected.status === 'gone') return
    void host.markSeen(selected.id, selected.latest).catch(() => {})
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seenKey, host])

  useEffect(() => {
    setReply('')
    setConfirmForget(false)
    setAttachedName(null)
    setError(null)
  }, [selectedId])

  const busy = replying || attaching

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        if (confirmForget) setConfirmForget(false)
        else if (!attaching) onClose()
        return
      }
      if (e.key === 'Tab' && dialogRef.current !== null) {
        const focusables = Array.from(
          dialogRef.current.querySelectorAll<HTMLElement>('button:not([disabled]), textarea:not([disabled]), a[href], input:not([disabled]):not([type="file"])'),
        )
        if (focusables.length === 0) return
        const first = focusables[0]!
        const last = focusables[focusables.length - 1]!
        const active = document.activeElement
        if (!dialogRef.current.contains(active)) {
          e.preventDefault()
          first.focus()
        } else if (e.shiftKey && active === first) {
          e.preventDefault()
          last.focus()
        } else if (!e.shiftKey && active === last) {
          e.preventDefault()
          first.focus()
        }
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [onClose, attaching, confirmForget])

  useEffect(() => {
    dialogRef.current?.focus()
  }, [])

  const handleCheckNow = () => {
    if (selected === null) return
    setChecking(true)
    setError(null)
    void host
      .check('manual', selected.id)
      .then(() => refresh())
      .catch((err: unknown) => {
        if (mountedRef.current) setError(errorText(err))
      })
      .finally(() => {
        if (mountedRef.current) setChecking(false)
      })
  }

  const handleSendReply = () => {
    if (selected === null || reply.trim() === '' || busy || busyRef.current) return
    const id = selected.id
    busyRef.current = true
    setReplying(true)
    setError(null)
    void host
      .post(id, reply.trim())
      .then(() => {
        if (!mountedRef.current) return
        if (selectedRef.current === id) setReply('')
        return refresh()
      })
      .catch((err: unknown) => {
        if (mountedRef.current && selectedRef.current === id) setError(errorText(err))
      })
      .finally(() => {
        busyRef.current = false
        if (mountedRef.current) setReplying(false)
      })
  }

  /** `claimed`: the caller already took the busy flag (a picked file is read first). */
  const sendFile = (file: FollowUpFile, name: string, id: string, claimed = false) => {
    if (!claimed) {
      if (busyRef.current) return
      busyRef.current = true
      setAttaching(true)
    }
    const current = () => mountedRef.current && selectedRef.current === id
    setAttachPercent(null)
    setAttachedName(null)
    setError(null)
    void (async () => {
      try {
        const upload = await buildFollowUpUpload({
          reportId: id,
          note: reply.trim(),
          file,
          appVersion: currentVersion(),
          platform: detectPlatform(isTauri, isMac, isWindows, isLinux),
        })
        if (upload.gzip === null) {
          if (current()) setError(upload.unavailableReason ?? t`This file can’t be sent.`)
          return
        }
        await host.attach(id, upload.gzip, (p) => {
          if (mountedRef.current && p.totalBytes > 0) setAttachPercent(Math.floor((p.sentBytes * 100) / p.totalBytes))
        })
        if (!mountedRef.current) return
        if (current()) {
          setAttachedName(name)
          setReply('')
        }
        await host.check('manual', id).catch(() => {})
        await refresh()
      } catch (err) {
        if (current()) setError(errorText(err))
      } finally {
        busyRef.current = false
        if (mountedRef.current) {
          setAttaching(false)
          setAttachPercent(null)
        }
      }
    })()
  }

  const handlePicked = (e: React.ChangeEvent<HTMLInputElement>) => {
    const f = e.target.files?.[0]
    e.target.value = ''
    if (f === undefined || selected === null || busyRef.current) return
    const id = selected.id
    // Claim the busy state before awaiting, so a second pick cannot start another upload.
    busyRef.current = true
    setAttaching(true)
    void f
      .arrayBuffer()
      .then((buf) => sendFile({ kind: 'attachment', name: f.name, label: f.type || f.name, bytes: new Uint8Array(buf) }, f.name, id, true))
      .catch(() => {
        busyRef.current = false
        if (mountedRef.current) {
          setAttaching(false)
          if (selectedRef.current === id) setError(t`Couldn’t read that file.`)
        }
      })
  }

  const handleForget = () => {
    if (selected === null) return
    const id = selected.id
    setConfirmForget(false)
    void host
      .forget(id)
      .then(() => {
        if (!mountedRef.current) return
        setSelectedId(null)
        return refresh()
      })
      .catch((err: unknown) => {
        if (mountedRef.current) setError(errorText(err))
      })
  }

  const statusLabel = (r: FollowedReport): string => {
    switch (r.status) {
      case 'open':
        return t`Open`
      case 'need-info':
        return t`Needs your help`
      case 'fixed': {
        const v = r.statusVersion ?? ''
        return v === '' ? t`Fixed` : t`Fixed in Hew ${v}`
      }
      case 'closed':
        return t`Closed`
      case 'gone':
        return t`No longer available`
    }
  }

  const renderNudge = (r: FollowedReport): ReactNode => {
    if (r.status !== 'fixed' || r.statusVersion === null) return null
    const running = currentVersion()
    if (compareVersions(running, r.statusVersion) >= 0) return null
    return (
      <div data-testid="sent-report-update-nudge" style={{ marginTop: '4px', fontSize: 'var(--font-size-body, 12px)' }}>
        {t`You’re on ${running}.`}{' '}
        {isTauri ? (
          onCheckForUpdates !== undefined ? (
            <button type="button" style={SECONDARY_BUTTON_STYLE} data-testid="sent-report-check-updates" onClick={onCheckForUpdates}>
              {t`Check for Updates…`}
            </button>
          ) : (
            <span>{t`Update Hew through your package manager.`}</span>
          )
        ) : (
          <button type="button" style={SECONDARY_BUTTON_STYLE} data-testid="sent-report-reload" onClick={() => window.location.reload()}>
            {t`Reload Hew`}
          </button>
        )}
      </div>
    )
  }

  const renderHeader = (r: FollowedReport): ReactNode => {
    const date = dateOf(r.sentAt)
    const version = r.appVersion
    let line: string
    switch (r.status) {
      case 'open':
        line = r.messages.some((m) => m.author === 'maintainer')
          ? t`Sent ${date} from Hew ${version}.`
          : t`Sent ${date} from Hew ${version}. No reply yet.`
        break
      case 'need-info':
        line = t`Hew’s developer asked for something. See below.`
        break
      case 'fixed':
        line = t`Sent ${date} from Hew ${version}.`
        break
      case 'closed': {
        const note = [...r.messages].reverse().find((m) => m.kind === 'status')?.text ?? ''
        line = note === '' ? t`Sent ${date} from Hew ${version}.` : note
        break
      }
      case 'gone':
        line = t`This report is no longer available from the service. Hew has stopped checking it.`
        break
    }
    return (
      <div data-testid="sent-report-header">
        <div style={{ ...HEADING_STYLE, marginBottom: '2px', fontSize: '14px' }}>
          {statusLabel(r)} <span style={SMALL_STYLE}>{r.id}</span>
        </div>
        <div style={{ fontSize: 'var(--font-size-body, 12px)' }}>{line}</div>
        {renderNudge(r)}
      </div>
    )
  }

  const renderMessage = (m: ThreadMessage) => {
    if (m.author === 'system' || m.kind === 'status') {
      return (
        <div key={m.seq} data-testid="sent-report-message" data-author="system" style={{ ...SMALL_STYLE, textAlign: 'center' }}>
          {m.text}
        </div>
      )
    }
    const mine = m.author === 'reporter'
    const body = m.text === '' && m.kind === 'attachment' ? t`File attached` : m.text
    return (
      <div
        key={m.seq}
        data-testid="sent-report-message"
        data-author={m.author}
        style={{ alignSelf: mine ? 'flex-end' : 'flex-start', maxWidth: '100%', display: 'flex', flexDirection: 'column', alignItems: mine ? 'flex-end' : 'flex-start' }}
      >
        <span style={SMALL_STYLE}>{mine ? t`You` : t`Hew’s developer`} · {dateOf(m.at)}</span>
        <div style={BUBBLE_STYLE}>{mine ? body : <Linkified text={body} />}</div>
      </div>
    )
  }

  const canReply = selected !== null && selected.status !== 'gone'
  return (
    <div style={OVERLAY_STYLE} onClick={() => !attaching && onClose()}>
      <div
        ref={dialogRef}
        tabIndex={-1}
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={t`Sent Reports`}
        data-testid="sent-reports-dialog"
      >
        <div style={HEADING_STYLE}>{t`Sent Reports`}</div>

        {reports !== null && reports.length === 0 ? (
          <div data-testid="sent-reports-empty" style={{ fontSize: 'var(--font-size-body, 12px)', lineHeight: 1.5, flex: 1 }}>
            <p>
              {t`No reports from this computer are being followed. When you send one with “Let me know what happens” ticked, it appears here along with any reply.`}
            </p>
            {!available && <p data-testid="sent-reports-unavailable">{t`This build can’t send reports, so there is nothing to follow.`}</p>}
            {error !== null && (
              <div style={ERROR_STYLE} role="alert" data-testid="sent-report-error">
                {error}
              </div>
            )}
          </div>
        ) : (
          <div style={BODY_STYLE}>
            <div style={LIST_STYLE} data-testid="sent-reports-list" role="listbox" aria-label={t`Sent reports`}>
              {(reports ?? []).map((r) => {
                const unread = unreadCount(r)
                return (
                  <button
                    key={r.id}
                    type="button"
                    role="option"
                    aria-selected={r.id === selectedId}
                    style={r.id === selectedId ? ITEM_SELECTED_STYLE : ITEM_STYLE}
                    data-testid={`sent-report-item-${r.id}`}
                    data-unread={unread > 0 ? 'true' : 'false'}
                    disabled={busy}
                    onClick={() => setSelectedId(r.id)}
                  >
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: '6px' }}>
                      <span style={{ fontWeight: 600, color: 'var(--text-primary, #eee)' }}>
                        {unread > 0 && <span style={DOT_STYLE} data-testid={`sent-report-dot-${r.id}`} aria-label={t`Unread`} />}
                        {r.id}
                      </span>
                      <span style={PILL_STYLE} data-testid={`sent-report-pill-${r.id}`}>
                        {statusLabel(r)}
                      </span>
                    </div>
                    <div style={SMALL_STYLE}>{dateOf(r.sentAt)}</div>
                    <div style={{ fontSize: '12px', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{r.excerpt}</div>
                  </button>
                )
              })}
            </div>

            <div style={PANE_STYLE}>
              {selected === null ? null : (
                <>
                  {renderHeader(selected)}
                  <div ref={messagesRef} style={MESSAGES_STYLE} data-testid="sent-report-messages" aria-live="polite">
                    {selected.messages.length === 0 && <div style={SMALL_STYLE}>{t`No messages yet.`}</div>}
                    {[...selected.messages].sort((a, b) => a.seq - b.seq).map(renderMessage)}
                  </div>

                  {canReply && (
                    <>
                      <textarea
                        style={TEXTAREA_STYLE}
                        data-testid="sent-report-reply-input"
                        aria-label={t`Your reply`}
                        placeholder={t`Write a reply`}
                        value={reply}
                        maxLength={REPLY_MAX}
                        onChange={(e) => setReply(e.target.value)}
                        disabled={busy}
                      />
                      <div style={ROW_STYLE}>
                        <button
                          type="button"
                          style={PRIMARY_BUTTON_STYLE}
                          data-testid="sent-report-send-reply"
                          disabled={busy || reply.trim() === ''}
                          onClick={handleSendReply}
                        >
                          {replying ? t`Sending…` : t`Send reply`}
                        </button>
                        {failedOpen !== null && (
                          <button
                            type="button"
                            style={PRIMARY_BUTTON_STYLE}
                            data-testid="sent-report-attach-failed"
                            disabled={busy || confirmForget}
                            ref={(el) => {
                              // Focus it once when it first appears (the dialog itself has focus on mount).
                              if (el !== null && !attachFocusedRef.current) {
                                attachFocusedRef.current = true
                                el.focus()
                              }
                            }}
                            onClick={() => sendFile({ kind: 'failedFile', failed: failedOpen }, failedOpen.name, selected.id)}
                          >
                            {t`Attach ${failedName} (the file that didn’t open)`}
                          </button>
                        )}
                        <button
                          type="button"
                          style={SECONDARY_BUTTON_STYLE}
                          data-testid="sent-report-attach"
                          disabled={busy || confirmForget}
                          onClick={() => fileInputRef.current?.click()}
                        >
                          {attaching ? (attachPercent === null ? t`Sending…` : t`Sending… ${attachPercent}%`) : t`Attach a file…`}
                        </button>
                        <input
                          ref={fileInputRef}
                          type="file"
                          hidden
                          data-testid="sent-report-attach-input"
                          aria-label={t`Choose a file to attach`}
                          onChange={handlePicked}
                        />
                      </div>
                      {attachedName !== null && (
                        <div style={SMALL_STYLE} role="status" data-testid="sent-report-attached">
                          {t`Sent ${attachedName}.`}
                        </div>
                      )}
                    </>
                  )}

                  <div style={ROW_STYLE}>
                    <button
                      type="button"
                      style={SECONDARY_BUTTON_STYLE}
                      data-testid="sent-report-check-now"
                      disabled={checking || selected.status === 'gone'}
                      onClick={handleCheckNow}
                    >
                      {checking ? t`Checking…` : t`Check now`}
                    </button>
                    <button
                      type="button"
                      style={SECONDARY_BUTTON_STYLE}
                      data-testid="sent-report-forget"
                      disabled={busy}
                      onClick={() => setConfirmForget(true)}
                    >
                      {t`Forget this report…`}
                    </button>
                  </div>

                  {confirmForget && (
                    <div role="alertdialog" aria-label={t`Forget this report?`} data-testid="sent-report-forget-confirm" style={{ ...ERROR_STYLE, color: 'inherit' }}>
                      <div>
                        {t`Forget this report? Hew stops checking for replies and the thread is removed from the service. The report itself stays with Hew’s developer.`}
                      </div>
                      <div style={ROW_STYLE}>
                        <button type="button" style={SECONDARY_BUTTON_STYLE} data-testid="sent-report-forget-cancel" disabled={busy} onClick={() => setConfirmForget(false)}>
                          {t`Cancel`}
                        </button>
                        <button type="button" style={PRIMARY_BUTTON_STYLE} data-testid="sent-report-forget-confirm-button" disabled={busy} onClick={handleForget}>
                          {t`Forget`}
                        </button>
                      </div>
                    </div>
                  )}
                </>
              )}
              {error !== null && (
                <div style={ERROR_STYLE} role="alert" data-testid="sent-report-error">
                  {error}
                </div>
              )}
            </div>
          </div>
        )}

        <div style={{ ...ROW_STYLE, justifyContent: 'flex-end', marginTop: '12px' }}>
          <button type="button" style={SECONDARY_BUTTON_STYLE} data-testid="sent-reports-close" onClick={onClose} disabled={attaching}>
            {t`Close`}
          </button>
        </div>
      </div>
    </div>
  )
}

declare const __HEW_VERSION__: string | undefined
