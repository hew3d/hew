/**
 * ReportBugDialog — Help ▸ Report Bug… (docs/design/report-bug.md §2). One
 * dialog, four entry points: the Help menu, the command palette, the
 * native Help menu, and the crash screen's "Report this crash" (rendered
 * inside `ErrorBoundary` in crash mode — `crash` prop non-null — since the
 * app tree is gone by then).
 *
 * The dialog paints first and gathers after: `reportBundle.ts`'s
 * `gatherReport` reads the recording, model, and log once per opening, and
 * after a large import that blocks the page in stretches of a second or
 * two. Until it finishes the dialog says "Gathering details…" and the
 * checklist, Save to file…, and Send report wait. Ticking rows then
 * recomputes the checklist from the gathered content at once; only the
 * compressed size is measured again, in the background, and Send waits for
 * that. Send and Save paint their busy state before building, and build
 * with the text as it is at the click.
 *
 * `canSend()` (io/reportClient.ts) gates whether Send report exists at
 * all: false on a self-hosted web build or a plain dev server (design §5),
 * where the dialog explains why and offers only Save to file… and the
 * GitHub issue link.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { msg, t } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'
import { Trans, useLingui } from '@lingui/react/macro'
import { formatNumber } from '../i18n/format'
import {
  compressReportUpload,
  describeReport,
  detectPlatform,
  encodeReportFile,
  gatherReport,
  gatherSystemInfo,
  measureUpload,
  scrubHomeDir,
  type ReportableScene,
  type ReportContent,
  type ReportSendMeasure,
  type ReportCrashInfo,
  type ReportFlags,
  type ReportRowKey,
} from '../log/reportBundle'
import { canSend, submitReport, ReportError, type ReportErrorKind } from '../io/reportClient'
import { getDebugMode } from '../settings/debugMode'
import { isMac, isWindows, isLinux } from '../platform'
import { isTauri, makeFileHost } from '../io/fileHost'
import { isComposingKey } from '../input/keyboard'

const GITHUB_NEW_ISSUE_URL = 'https://github.com/hew3d/hew/issues/new'
const PRIVACY_URL = 'https://hew3d.com/privacy/bug-reports/'

/** Keep a GitHub issue URL well under browsers' ~8000-char practical limit. */
const MAX_ISSUE_URL_LENGTH = 6000

/** Remembers whether "Let me know what happens" is ticked (design §6.5). */
export const FOLLOW_PREF_KEY = 'hew.report.follow'

function loadFollowPref(): boolean {
  try {
    return localStorage.getItem(FOLLOW_PREF_KEY) !== '0'
  } catch {
    return true
  }
}

function saveFollowPref(on: boolean): void {
  try {
    localStorage.setItem(FOLLOW_PREF_KEY, on ? '1' : '0')
  } catch {
    /* best effort */
  }
}

const DESCRIPTION_MIN = 10
const DESCRIPTION_MAX = 10_000

const ROW_ORDER: { key: ReportRowKey; label: MessageDescriptor }[] = [
  { key: 'system', label: msg`App version and system` },
  { key: 'recording', label: msg`Recorded steps` },
  { key: 'imports', label: msg`Imported files` },
  { key: 'failedFile', label: msg`File that didn’t open` },
  { key: 'model', label: msg`Model file` },
  { key: 'log', label: msg`Diagnostic log` },
  { key: 'input', label: msg`Raw input events` },
]

type SendState =
  | { kind: 'idle' }
  | { kind: 'sending' }
  | { kind: 'sent'; id: string; following: boolean }
  | { kind: 'failed'; kind_: ReportErrorKind; message: string }

type SaveState =
  | { kind: 'idle' }
  | { kind: 'saving' }
  | { kind: 'saved'; display: string; revealPath: string | null }
  | { kind: 'failed' }

export interface ReportBugDialogProps {
  /** Null in crash mode, or while the kernel is still loading. */
  scene: ReportableScene | null
  documentName: string
  /** Non-null renders the dialog in crash mode: the model row is
   *  unavailable, `recording` prefers this capture, and a `crash` block is
   *  added to the bundle. */
  crash: ReportCrashInfo | null
  onClose: () => void
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
  width: '480px',
  maxWidth: '92vw',
  maxHeight: '86vh',
  overflowY: 'auto',
  fontFamily: 'var(--font-family-ui, system-ui, sans-serif)',
  color: 'var(--text-secondary, #ddd)',
}

const HEADING_STYLE: React.CSSProperties = {
  fontSize: '15px',
  fontWeight: 600,
  color: 'var(--text-primary, #eee)',
  marginBottom: '14px',
}

const LABEL_STYLE: React.CSSProperties = {
  display: 'block',
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-tertiary, #ccc)',
  marginBottom: '6px',
  marginTop: '14px',
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
}

const INPUT_STYLE: React.CSSProperties = {
  ...TEXTAREA_STYLE,
  resize: undefined,
}

const HINT_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: 'var(--text-tertiary, #999)',
  marginTop: '4px',
}

const ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  alignItems: 'flex-start',
  gap: '8px',
  padding: '6px 0',
  borderBottom: '1px solid var(--border-subtle, #3a3a3a)',
}

const ROW_LABEL_STYLE: React.CSSProperties = {
  flex: 1,
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-primary, #eee)',
}

const ROW_SIZE_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: 'var(--text-tertiary, #999)',
}

const SHOW_LINK_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: 'var(--accent-base, #6a9fe0)',
  cursor: 'pointer',
  background: 'none',
  border: 'none',
  padding: 0,
  marginLeft: '8px',
  textDecoration: 'underline',
}

const PREVIEW_STYLE: React.CSSProperties = {
  whiteSpace: 'pre-wrap',
  wordBreak: 'break-word',
  background: 'var(--surface-input, #1c1c1c)',
  border: '1px solid var(--border-strong, #4a4a4a)',
  borderRadius: 'var(--radius-control, 4px)',
  padding: '8px',
  fontSize: '11px',
  fontFamily: 'var(--font-family-mono, monospace)',
  maxHeight: '160px',
  overflow: 'auto',
  marginTop: '4px',
  width: '100%',
}

const NOTE_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: 'var(--text-tertiary, #999)',
  margin: '12px 0 0',
  lineHeight: 1.5,
}

const DESTINATION_STYLE: React.CSSProperties = {
  fontSize: '11px',
  color: 'var(--text-tertiary, #999)',
  margin: '14px 0 0',
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

const BUTTON_ROW_STYLE: React.CSSProperties = {
  display: 'flex',
  justifyContent: 'flex-end',
  gap: '10px',
  marginTop: '18px',
}

const SECONDARY_BUTTON_STYLE: React.CSSProperties = {
  padding: '6px 16px',
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

const PRIMARY_BUTTON_DISABLED_STYLE: React.CSSProperties = {
  ...PRIMARY_BUTTON_STYLE,
  opacity: 0.5,
  cursor: 'default',
}

const SECONDARY_BUTTON_DISABLED_STYLE: React.CSSProperties = {
  ...SECONDARY_BUTTON_STYLE,
  opacity: 0.5,
  cursor: 'default',
}

const ERROR_STYLE: React.CSSProperties = {
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--danger-text, #e88)',
  marginTop: '14px',
  whiteSpace: 'pre-wrap',
}

const SUCCESS_STYLE: React.CSSProperties = {
  marginTop: '14px',
  fontSize: 'var(--font-size-body, 12px)',
  color: 'var(--text-secondary, #ddd)',
}

const DEFAULT_FLAGS: ReportFlags = {
  system: true,
  recording: true,
  imports: true,
  failedFile: true,
  model: true,
  log: true,
  input: true,
}

const EMPTY_FIELDS = { description: '', expected: '', contact: '' }

/** Resolves once the browser has had a chance to paint the current state:
 *  the next animation frame, then a task. A timer stands in where frames
 *  don't run (a hidden tab, tests). */
function afterPaint(): Promise<void> {
  return new Promise((resolve) => {
    if (typeof requestAnimationFrame !== 'function') {
      setTimeout(resolve, 0)
      return
    }
    const fallback = setTimeout(resolve, 100)
    requestAnimationFrame(() => {
      clearTimeout(fallback)
      setTimeout(resolve, 0)
    })
  })
}

function formatBytes(n: number): string {
  if (n < 1024) return t`${n} B`
  if (n < 1024 * 1024) {
    const kb = formatNumber(n / 1024, { fractionDigits: 1 })
    return t`${kb} KB`
  }
  const mb = formatNumber(n / (1024 * 1024), { fractionDigits: 1 })
  return t`${mb} MB`
}

/** Open an external URL the OS-appropriate way — the opener plugin on
 *  desktop (App.tsx's user-guide link does the same), a new tab on the web. */
/** The first 200 code points of `s`; never cuts an emoji's surrogate pair in half. */
export function excerptOf(s: string): string {
  return Array.from(s).slice(0, 200).join('')
}

export function openExternal(url: string): void {
  if (isTauri) {
    void import('@tauri-apps/plugin-opener')
      .then(({ openUrl }) => openUrl(url))
      .catch(() => {
        /* best effort */
      })
  } else {
    window.open(url, '_blank', 'noopener')
  }
}

/** Builds the "Also open a public GitHub issue" / "Open a public GitHub
 *  issue" URL: description, version, platform, and — after a successful
 *  Send — the report ID; after a Save to file… instead, `id` is null and
 *  the footer asks the reporter to attach the saved file, since there's no
 *  ID to correlate it by. Never an attachment either way — GitHub's
 *  new-issue URL has no way to carry one. Truncates the description first
 *  if the encoded URL would run long.
 *
 * Truncates by CODE POINT (`Array.from`), not by UTF-16 unit (`.slice`): a
 * description with emoji or other astral-plane characters is a surrogate
 * PAIR in a JS string, and a unit-based slice can cut between the two
 * halves, leaving a lone unpaired surrogate that `encodeURIComponent`
 * throws a `URIError` on ("malformed URI sequence") — turning "also file a
 * GitHub issue" into a crash. */
/* oxlint-disable lingui/no-unlocalized-strings -- the GitHub issue title, body footer, and URL are written for the maintainer and stay English */
function githubIssueUrl(description: string, version: string, platform: string, id: string | null): string {
  const title = id !== null ? `Bug report ${id}` : 'Bug report'
  const footer =
    id !== null
      ? `\n\n---\nVersion: ${version}\nPlatform: ${platform}\nReport ID: ${id}`
      : `\n\n---\nVersion: ${version}\nPlatform: ${platform}\n\nPlease attach the saved report file to this issue.`
  const base = `${GITHUB_NEW_ISSUE_URL}?title=${encodeURIComponent(title)}&body=`
  const budget = MAX_ISSUE_URL_LENGTH - base.length - encodeURIComponent(footer).length
  const codePoints = Array.from(description)
  let bodyPoints = codePoints
  while (encodeURIComponent(bodyPoints.join('')).length > Math.max(0, budget) && bodyPoints.length > 0) {
    bodyPoints = bodyPoints.slice(0, Math.floor(bodyPoints.length * 0.9))
  }
  let body = bodyPoints.join('')
  if (bodyPoints.length < codePoints.length) body = `${body.trimEnd()}\n\n[truncated]`
  return `${base}${encodeURIComponent(body + footer)}`
}
/* oxlint-enable lingui/no-unlocalized-strings */

function reportErrorMessage(err: unknown): { kind: ReportErrorKind; message: string } {
  if (err instanceof ReportError) {
    switch (err.kind) {
      case 'invalid':
        return { kind: err.kind, message: t`The report was rejected as invalid — check the description length.` }
      case 'tooLarge':
        return { kind: err.kind, message: t`This report is too large to send.` }
      case 'rateLimited':
        return { kind: err.kind, message: t`Too many reports sent recently — try again in a few minutes.` }
      case 'full':
        return { kind: err.kind, message: t`The report service is temporarily full — try again later.` }
      case 'tls':
        return { kind: err.kind, message: t`The report service’s certificate isn’t trusted by this computer.` }
      case 'offline':
        return { kind: err.kind, message: t`Could not reach the report service — check your internet connection.` }
      case 'unreachable':
        return { kind: err.kind, message: t`The report service is unavailable right now.` }
      case 'status': {
        const status = err.status
        return { kind: err.kind, message: status ? t`Unexpected response from the report service (${status}).` : t`Unexpected response from the report service.` }
      }
    }
  }
  return { kind: 'unreachable', message: err instanceof Error ? err.message : String(err) }
}

export function ReportBugDialog({ scene, documentName, crash, onClose }: ReportBugDialogProps) {
  const { t, i18n } = useLingui()
  const [description, setDescription] = useState('')
  const [expected, setExpected] = useState('')
  const [contact, setContact] = useState('')
  const [flags, setFlags] = useState<ReportFlags>(DEFAULT_FLAGS)
  const [expanded, setExpanded] = useState<Set<ReportRowKey>>(new Set())
  const [sendState, setSendState] = useState<SendState>({ kind: 'idle' })
  // Percent of the compressed report uploaded so far, or null before the
  // first piece lands; a large report goes up in many pieces (design §4).
  const [sendPercent, setSendPercent] = useState<number | null>(null)
  const [follow, setFollow] = useState(loadFollowPref)
  // Say what happened to a save: a file written with no confirmation is the
  // "where did it go?" problem this dialog replaces. A cancelled picker
  // returns silently to the form (`idle`) rather than reporting anything —
  // the user chose not to save, that's not a failure. `revealPath` is the
  // raw (unscrubbed) desktop path `revealItemInDir` needs; null on web,
  // where there's no folder to reveal.
  const [saveState, setSaveState] = useState<SaveState>({ kind: 'idle' })
  const [homeDir, setHomeDir] = useState<string | null>(null)
  // Web has no home directory to look up at all, so it's ready immediately;
  // desktop starts NOT ready until the async homeDir() lookup below settles
  // (resolved OR failed) — see that effect for why Send/Save must wait on it.
  const [homeDirReady, setHomeDirReady] = useState(!isTauri)
  // Everything the report can contain, gathered once per opening (module doc
  // comment). Null while gathering.
  const [content, setContent] = useState<ReportContent | null>(null)
  // The compressed size of what Send would upload, for the checklist `key`
  // it was measured with.
  const [measure, setMeasure] = useState<(ReportSendMeasure & { key: string }) | null>(null)
  // Bumped to measure again after Save to file cancelled a measurement.
  const [measureRun, setMeasureRun] = useState(0)
  const measureAbort = useRef<AbortController | null>(null)

  // Guards every setState in an async chain (handleSend/handleSaveToFile)
  // against firing after this dialog has already unmounted — e.g. the user
  // closed it, or the whole app did, while a submit was still in flight.
  const mountedRef = useRef(true)
  useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  const debugMode = useMemo(() => getDebugMode(), [])
  const platform = useMemo(() => detectPlatform(isTauri, isMac, isWindows, isLinux), [])
  const system = useMemo(
    () => gatherSystemInfo(typeof __HEW_VERSION__ !== 'undefined' ? __HEW_VERSION__ : '0.0.0', platform),
    [platform],
  )
  const sendAvailable = useMemo(() => canSend(), [])
  const checklistKey = JSON.stringify(flags)

  // The desktop home directory, resolved once (design §8's "Clients" —
  // `homeDir()` from `@tauri-apps/api/path`). Null on web (nothing to
  // scrub) or if it can't be resolved. Gathering waits for it, and Send
  // and Save wait for gathering, so nothing can go out before the log is
  // scrubbed.
  useEffect(() => {
    if (!isTauri) return
    let cancelled = false
    void import('@tauri-apps/api/path')
      .then(({ homeDir }) => homeDir())
      .then((dir) => {
        if (!cancelled) setHomeDir(dir)
      })
      .catch(() => {
        /* leave null — scrubbing is best-effort */
      })
      .finally(() => {
        if (!cancelled) setHomeDirReady(true)
      })
    return () => {
      cancelled = true
    }
  }, [])

  // Builds run one at a time. After a large import a build holds a few
  // hundred megabytes while it runs, and ticking rows or pressing Send while
  // an earlier build is still going would otherwise stack them.
  const buildQueue = useRef<Promise<unknown>>(Promise.resolve())
  const enqueueBuild = <T,>(task: () => Promise<T>): Promise<T> => {
    const run = buildQueue.current.then(
      () => task(),
      () => task(),
    )
    // Settle to nothing either way: holding `run` itself would keep the last
    // build's result (a whole report file or upload) alive in this ref.
    buildQueue.current = run.then(
      () => undefined,
      () => undefined,
    )
    return run
  }

  // Gather once, after the dialog has painted "Gathering details…". Closing
  // the dialog mid-gather stops it at its next step.
  useEffect(() => {
    if (!homeDirReady) return
    const controller = new AbortController()
    void (async () => {
      await afterPaint()
      if (controller.signal.aborted) return
      try {
        const gathered = await enqueueBuild(() =>
          gatherReport({ scene, documentName, platform, debugMode, crash, homeDir, signal: controller.signal }),
        )
        if (!controller.signal.aborted) setContent(gathered)
      } catch {
        /* closed while gathering */
      }
    })()
    return () => controller.abort()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [homeDirReady])

  // Measure the compressed size for the checklist on screen. A newer
  // checklist cancels the measurement still running for the old one.
  useEffect(() => {
    if (content === null || !sendAvailable) return
    // A row unticked and ticked again before its new measurement ran still
    // has this checklist's result; don't compress the whole report again.
    if (measure !== null && measure.key === checklistKey) return
    const controller = new AbortController()
    measureAbort.current = controller
    const key = checklistKey
    enqueueBuild(async () => {
      // Let the checklist this measurement is for paint first.
      await afterPaint()
      if (controller.signal.aborted) return
      const result = await measureUpload(content, { fields: EMPTY_FIELDS, flags, signal: controller.signal })
      if (!controller.signal.aborted && mountedRef.current) setMeasure({ ...result, key })
    }).catch(() => {
      /* cancelled by a newer checklist, a save, or closing the dialog */
    })
    return () => controller.abort()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [content, checklistKey, measureRun])

  // While sending or saving, closing the dialog any of the three usual ways
  // (Escape, overlay click, Cancel) would lose the report ID the moment it
  // arrives (send) or interrupt a write partway through (save) — so all
  // three are suppressed for that one window, not just Cancel's own button.
  const sending = sendState.kind === 'sending'
  const savingFile = saveState.kind === 'saving'
  const busy = sending || savingFile

  const handleKeyDown = useCallback(
    (e: KeyboardEvent) => {
      if (isComposingKey(e)) return
      if (e.key === 'Escape') {
        e.preventDefault()
        e.stopPropagation()
        if (!busy) onClose()
      }
    },
    [onClose, busy],
  )
  useEffect(() => {
    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [handleKeyDown])

  const gathering = content === null
  const currentMeasure = measure !== null && measure.key === checklistKey ? measure : null
  const described = useMemo(
    () => (content === null ? null : describeReport(content, flags, currentMeasure?.dropped ?? [])),
    [content, flags, currentMeasure],
  )

  const trimmedDescription = description.trim()
  const trimmedLength = trimmedDescription.length
  const descriptionLength = description.length
  const descriptionValid = trimmedDescription.length >= DESCRIPTION_MIN && description.length <= DESCRIPTION_MAX
  // The measurement says up front when Send can't go ahead (too large even
  // after dropping rows, or no compression). Send compresses again with the
  // real text before uploading, so this only gates the button.
  const canSubmit =
    descriptionValid && !busy && !gathering && currentMeasure !== null && currentMeasure.unavailableReason === null
  const canSave = !busy && !gathering

  const approxSize = formatBytes(currentMeasure?.compressedBytes ?? 0)
  const statusNote = gathering
    ? t`Gathering details…`
    : sending
      ? sendPercent === null
        ? t`Compressing the report…`
        : t`Uploading the report…`
      : savingFile
        ? t`Saving the report…`
        : !sendAvailable
        ? null
        : currentMeasure === null
          ? t`Measuring the compressed size…`
          : (currentMeasure.unavailableReason ??
            t`About ${approxSize} to send, compressed.`)

  const toggleRow = (key: ReportRowKey) => setFlags((f) => ({ ...f, [key]: !f[key] }))
  const toggleExpanded = (key: ReportRowKey) =>
    setExpanded((s) => {
      const next = new Set(s)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  const handleSend = () => {
    if (!canSubmit || content === null) return
    setSendState({ kind: 'sending' })
    setSendPercent(null)
    // Send needs this checklist's measured size, so anything still measuring
    // is redundant, and would hold the upload behind it in the build queue.
    measureAbort.current?.abort()
    const options = { fields: { description, expected, contact }, flags }
    void (async () => {
      try {
        // Show "Sending…" before compressing, which blocks in stretches.
        await afterPaint()
        const upload = await enqueueBuild(() => compressReportUpload(content, options))
        if (upload.gzip === null) {
          if (mountedRef.current) {
            setSendState({
              kind: 'failed',
              // oxlint-disable-next-line lingui/no-unlocalized-strings -- error-kind identifier, not copy
              kind_: 'tooLarge',
              message: upload.unavailableReason ?? t`This report can’t be sent. Save it to a file instead.`,
            })
          }
          return
        }
        const firstLine = description.trim().split(/\r?\n/, 1)[0] ?? ''
        const result = await submitReport(
          upload.gzip,
          (p) => {
            if (mountedRef.current && p.totalBytes > 0) {
              setSendPercent(Math.floor((p.sentBytes * 100) / p.totalBytes))
            }
          },
          {
            follow,
            excerpt: excerptOf(firstLine),
            fileName: flags.failedFile && content.failedOpen != null ? content.failedOpen.name : null,
          },
        )
        if (mountedRef.current) setSendState({ kind: 'sent', id: result.id, following: result.following })
      } catch (err) {
        if (!mountedRef.current) return
        const { kind, message } = reportErrorMessage(err)
        setSendState({ kind: 'failed', kind_: kind, message })
      }
    })()
  }

  const handleSaveToFile = () => {
    if (!canSave || content === null) return
    setSaveState({ kind: 'saving' })
    // A size measurement still running would hold the save behind it in the
    // build queue, so cancel it. If the size on screen was still pending, it
    // is measured again when the dialog returns to the form.
    measureAbort.current?.abort()
    const cancelledMeasure = currentMeasure === null
    const remeasure = () => {
      if (cancelledMeasure && mountedRef.current) setMeasureRun((n) => n + 1)
    }
    const options = { fields: { description, expected, contact }, flags }
    // Build AND write live inside one try/catch, so every possible outcome —
    // a build hiccup, a cancelled picker, a write failure, or success — sets
    // a new saveState and none of them can leave the dialog stuck on
    // "Saving…" forever (the playtest-found bug: a slow/hung write left the
    // form showing "Saving…" with no way out).
    void (async () => {
      try {
        await afterPaint()
        const bytes = await enqueueBuild(() => encodeReportFile(content, options))
        const name = `hew-bug-report-${new Date().toISOString().replace(/[:.]/g, '-')}.json`
        const savedTo = await makeFileHost().exportBinary(bytes, name, {
          description: t`Hew Bug Report`,
          ext: 'json',
          mime: 'application/json',
        })
        if (!mountedRef.current) return
        if (savedTo === null) {
          setSaveState({ kind: 'idle' })
          remeasure()
          return
        }
        const display = isTauri ? scrubHomeDir(savedTo, homeDir) : t`Saved as "${savedTo}".`
        setSaveState({ kind: 'saved', display, revealPath: isTauri ? savedTo : null })
      } catch {
        if (!mountedRef.current) return
        setSaveState({ kind: 'failed' })
        remeasure()
      }
    })()
  }

  const handleRevealSaved = () => {
    if (saveState.kind !== 'saved' || saveState.revealPath === null) return
    void import('@tauri-apps/plugin-opener')
      .then(({ revealItemInDir }) => revealItemInDir(saveState.revealPath as string))
      .catch(() => {
        /* best effort */
      })
  }

  /** "Show in Finder" (macOS) / "Show in Explorer" (Windows) / "Show in
   *  folder" (Linux) — the OS-conventional wording for revealing a file. */
  const revealLabel = isMac ? t`Show in Finder` : isWindows ? t`Show in Explorer` : t`Show in folder`

  const handleOpenGithubIssueForSave = () => {
    openExternal(githubIssueUrl(description, system.appVersion, system.platform, null))
  }

  const [copied, setCopied] = useState(false)
  const sentId = sendState.kind === 'sent' ? sendState.id : ''
  const handleCopyId = () => {
    if (sendState.kind !== 'sent') return
    void navigator.clipboard?.writeText(sendState.id)
    setCopied(true)
    window.setTimeout(() => setCopied(false), 1200)
  }

  const handleOpenGithubIssue = () => {
    if (sendState.kind !== 'sent') return
    openExternal(githubIssueUrl(description, system.appVersion, system.platform, sendState.id))
  }

  // Raw input is listed only in Debug Mode, Imported files only when a
  // recorded step actually embeds a file, and the file that didn't open only
  // when there is one.
  const rowsToShow = ROW_ORDER.filter(
    (r) =>
      (r.key !== 'input' || debugMode) &&
      (r.key !== 'imports' || (described?.importedFiles.length ?? 0) > 0) &&
      (r.key !== 'failedFile' || content?.failedOpen != null),
  )

  return (
    <div style={OVERLAY_STYLE} onClick={() => !busy && onClose()}>
      <div
        style={DIALOG_STYLE}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-modal="true"
        aria-label={crash !== null ? t`Report this crash` : t`Report Bug`}
      >
        <div style={HEADING_STYLE}>{crash !== null ? t`Report this crash` : t`Report a Bug`}</div>

        {/* Once sent (or saved), the form gives way to the confirmation: a
            filled-in form above a below-the-fold success line reads as
            "nothing happened", the problem this dialog exists to fix. */}
        {sendState.kind !== 'sent' && saveState.kind !== 'saved' && (
          <>
            <label style={LABEL_STYLE} htmlFor="report-bug-description">
              {t`What happened?`}
            </label>
            <textarea
              id="report-bug-description"
              style={{ ...TEXTAREA_STYLE, minHeight: '72px' }}
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              autoFocus
              maxLength={DESCRIPTION_MAX}
              disabled={busy}
            />
            <div style={HINT_STYLE}>
              {description.length === 0
                ? t`${DESCRIPTION_MIN}–${DESCRIPTION_MAX} characters`
                : !descriptionValid
                  ? t`Needs at least ${DESCRIPTION_MIN} characters (${trimmedLength} so far)`
                  : t`${descriptionLength} / ${DESCRIPTION_MAX}`}
            </div>

            <label style={LABEL_STYLE} htmlFor="report-bug-expected">
              {t`What did you expect instead?`}
            </label>
            <textarea
              id="report-bug-expected"
              style={{ ...TEXTAREA_STYLE, minHeight: '48px' }}
              value={expected}
              onChange={(e) => setExpected(e.target.value)}
              // The intake service reads the whole `report` object from the
              // first 256 KiB of the decompressed report, so every text field
              // is capped, not just the description.
              maxLength={DESCRIPTION_MAX}
              disabled={busy}
            />

            <label style={LABEL_STYLE} htmlFor="report-bug-contact">
              {t`Email for follow-up (optional)`}
            </label>
            <input
              id="report-bug-contact"
              type="email"
              style={INPUT_STYLE}
              value={contact}
              onChange={(e) => setContact(e.target.value)}
              maxLength={254}
              disabled={busy}
            />

            <label style={{ ...LABEL_STYLE, marginBottom: 0 }}>{t`What’s included`}</label>
            <div>
              {rowsToShow.map(({ key, label }) => {
                const rowLabel = i18n._(label)
                const row = described?.rows[key]
                const disabled = row?.disabledReason != null
                const checked = flags[key] && !disabled
                return (
                  <div key={key} style={ROW_STYLE}>
                    <input
                      type="checkbox"
                      checked={checked}
                      // Waits for gathering, and locks while sending or
                      // saving: those build from the checklist at the click.
                      disabled={gathering || disabled || busy}
                      onChange={() => toggleRow(key)}
                      aria-label={rowLabel}
                      style={{ marginTop: '2px' }}
                    />
                    <div style={{ flex: 1 }}>
                      <div style={{ display: 'flex', alignItems: 'center' }}>
                        <span style={ROW_LABEL_STYLE}>{rowLabel}</span>
                        <span style={ROW_SIZE_STYLE}>{row ? formatBytes(row.bytes) : ''}</span>
                        <button
                          type="button"
                          style={SHOW_LINK_STYLE}
                          onClick={() => toggleExpanded(key)}
                          disabled={row === undefined}
                        >
                          {expanded.has(key) ? t`Hide` : t`Show`}
                        </button>
                      </div>
                      {disabled && <div style={HINT_STYLE}>{row?.disabledReason}</div>}
                      {!disabled && checked && sendAvailable && row?.droppedFromSend === true && (
                        <div style={HINT_STYLE}>
                          {key === 'failedFile'
                            ? t`Too large to send; its name, size, and error go instead. Save to file includes the file itself.`
                            : t`Too large to send; still included in Save to file.`}
                        </div>
                      )}
                      {expanded.has(key) && row && <div style={PREVIEW_STYLE}>{row.preview}</div>}
                    </div>
                  </div>
                )
              })}
            </div>

            {statusNote !== null && (
              <p style={NOTE_STYLE} aria-live="polite">
                {statusNote}
              </p>
            )}

            <p style={NOTE_STYLE}>
              <Trans>
                Recorded steps and the model file contain your model’s geometry. Imported files are the files you
                imported or opened and the textures and library items you added, and the file that didn’t open is
                that file, all byte for byte. Untick these to send only the description, system details, and log.
              </Trans>
            </p>

            {/* A build that can't send has no destination to name; the line
                below it explains the file-and-issue route instead. */}
            {sendAvailable && (
              <p style={DESTINATION_STYLE}>
                {t`Sent privately to the Hew developer. Not posted publicly.`}{' '}
                <button type="button" style={LINK_STYLE} onClick={() => openExternal(PRIVACY_URL)}>
                  {t`Learn more`}
                </button>
              </p>
            )}

            {!sendAvailable && sendState.kind === 'idle' && (
              <p style={NOTE_STYLE}>
                <Trans>
                  This build can’t send reports directly. Save the file below and attach it to a GitHub issue
                  instead.
                </Trans>
              </p>
            )}
          </>
        )}

        {sendAvailable && sendState.kind !== 'sent' && saveState.kind !== 'saved' && (
          <label style={{ ...ROW_STYLE, borderBottom: 'none', marginTop: '14px', cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={follow}
              disabled={busy}
              data-testid="report-follow-checkbox"
              onChange={(e) => {
                setFollow(e.target.checked)
                saveFollowPref(e.target.checked)
              }}
              style={{ marginTop: '2px' }}
            />
            <span style={{ flex: 1 }}>
              <span style={{ ...ROW_LABEL_STYLE, display: 'block', fontWeight: 600 }}>
                {t`Let me know what happens with this report`}
              </span>
              <span style={{ ...HINT_STYLE, display: 'block' }}>
                {t`Hew keeps a private key for this report on this computer and checks for a reply when it starts. No account, no email. You can stop any time under Help ▸ Sent Reports.`}
              </span>
            </span>
          </label>
        )}

        {sendState.kind === 'failed' && (
          <div style={ERROR_STYLE} role="alert">
            {sendState.message}
          </div>
        )}

        {saveState.kind === 'failed' && (
          <div style={ERROR_STYLE} role="alert">
            {t`Couldn’t save the file.`}
          </div>
        )}

        {sendState.kind === 'sent' && (
          <div style={SUCCESS_STYLE} role="status">
            <div>
              <Trans>
                Report sent — ID <code>{sentId}</code>
              </Trans>{' '}
              <button type="button" style={LINK_STYLE} onClick={handleCopyId}>
                {copied ? t`Copied` : t`Copy`}
              </button>
            </div>
            {sendState.following && (
              <div style={{ marginTop: '8px' }} data-testid="report-follow-confirmation">
                <Trans>
                  You’ll find replies and the report’s status under <strong>Help ▸ Sent Reports…</strong>
                </Trans>
              </div>
            )}
            <div style={{ marginTop: '8px' }}>
              <button type="button" style={SECONDARY_BUTTON_STYLE} onClick={handleOpenGithubIssue}>
                {t`Also open a public GitHub issue`}
              </button>
            </div>
          </div>
        )}

        {/* Save to file… replaces the form the same way a successful Send
            does (module doc comment on the sendState block above) — a
            written file with no confirmation is exactly the "where did it
            go?" bug this dialog exists to fix. */}
        {saveState.kind === 'saved' && (
          <div style={SUCCESS_STYLE} role="status">
            <div>{t`Report saved.`}</div>
            <div style={{ marginTop: '4px', wordBreak: 'break-all' }}>{saveState.display}</div>
            <div style={{ marginTop: '8px', display: 'flex', gap: '10px', flexWrap: 'wrap' }}>
              {saveState.revealPath !== null && (
                <button type="button" style={SECONDARY_BUTTON_STYLE} onClick={handleRevealSaved}>
                  {revealLabel}
                </button>
              )}
              <button type="button" style={SECONDARY_BUTTON_STYLE} onClick={handleOpenGithubIssueForSave}>
                {t`Open a public GitHub issue`}
              </button>
            </div>
          </div>
        )}

        {sendState.kind !== 'sent' && saveState.kind !== 'saved' && (
          <div style={BUTTON_ROW_STYLE}>
            <button
              type="button"
              style={busy ? SECONDARY_BUTTON_DISABLED_STYLE : SECONDARY_BUTTON_STYLE}
              onClick={onClose}
              disabled={busy}
            >
              {t`Cancel`}
            </button>
            <button
              type="button"
              style={canSave ? SECONDARY_BUTTON_STYLE : SECONDARY_BUTTON_DISABLED_STYLE}
              onClick={handleSaveToFile}
              disabled={!canSave}
            >
              {savingFile ? t`Saving…` : t`Save to file…`}
            </button>
            {sendAvailable && (
              <button
                type="button"
                style={canSubmit ? PRIMARY_BUTTON_STYLE : PRIMARY_BUTTON_DISABLED_STYLE}
                onClick={handleSend}
                disabled={!canSubmit}
              >
                {sending ? (sendPercent === null ? t`Sending…` : t`Sending… ${sendPercent}%`) : t`Send report`}
              </button>
            )}
          </div>
        )}

        {(sendState.kind === 'sent' || saveState.kind === 'saved') && (
          <div style={BUTTON_ROW_STYLE}>
            <button type="button" style={SECONDARY_BUTTON_STYLE} onClick={onClose}>
              {t`Close`}
            </button>
          </div>
        )}
      </div>
    </div>
  )
}

declare const __HEW_VERSION__: string | undefined
