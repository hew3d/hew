/**
 * Top-level React error boundary.
 *
 * Without it, any error thrown during render/commit unmounts the whole tree and
 * leaves a blank white window (observed when using a tool right after a glTF
 * import) — and the only escape is a manual reload that loses unsaved work. This
 * catches the error instead and shows it (so the cause is diagnosable), keeps
 * the autosave snapshot intact so a reload can Recover, and persists the message
 * to `localStorage` so it survives the reload.
 *
 * Note: error boundaries only catch render/lifecycle errors, not errors thrown
 * inside event handlers or async callbacks. A fully-white window is the former,
 * which is exactly what this addresses. A Rust kernel panic is usually the
 * latter: the tool or undo handler that hit it catches the trap, and the app
 * keeps running on the poisoned instance. So the boundary also listens for the
 * wasm panic hook's `KERNEL_PANIC_EVENT` and stops the app right away, while
 * the recording the hook captured is still in memory for Report this crash
 * (docs/design/report-bug.md §2) — the crash screen's `ReportBugDialog`,
 * rendered here in crash mode since the rest of the app tree is gone.
 */
import { Component, type ErrorInfo, type ReactNode } from 'react'
import { t } from '@lingui/core/macro'
import { getEntries } from './log/LogStore'
import { getKernelStop, getPanicCapture, KERNEL_PANIC_EVENT, type KernelStop } from './log/panicCapture'
import { ReportBugDialog } from './panels/ReportBugDialog'
import type { ReportCrashInfo } from './log/reportBundle'

export const LAST_ERROR_KEY = 'hew:lastError'

/** Format the full crash report text — shared by the localStorage record
 *  (LAST_ERROR_KEY, read after reload) and the dialog's Copy button below, so
 *  what a user copies to paste into a bug report always matches what actually
 *  gets persisted. */
function formatCrashReport(error: Error, componentStack: string, recentErrors: readonly string[]): string {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- crash report text for the maintainer, stays English
  return `${new Date().toISOString()}\n${error.message}\n\n${error.stack ?? ''}\n\n${componentStack}\n\n--- recent console errors ---\n${recentErrors.join('\n')}`
}

interface Props {
  children: ReactNode
}

interface State {
  error: Error | null
  /** What stopped the app: a render/lifecycle throw, or a kernel panic the
   *  wasm hook reported (no render error exists to show as the symptom). */
  source: 'render' | 'panic'
  /** Set when the kernel stopped on a trap rather than a panic (an
   *  out-of-memory abort mid-open): the screen's heading and explanation
   *  come from here instead of the generic internal-error copy. */
  stop: KernelStop | null
  componentStack: string
  /** Recent captured console errors — includes the kernel `panicked at …` line
   *  that poisons the wasm instance (the thrown error is only the symptom). */
  recentErrors: string[]
  /** Brief "Copied" confirmation on the copy-details button, mirroring
   *  LogPanel's own copy-button feedback. */
  copied: boolean
  /** Whether the "Report this crash" dialog (ReportBugDialog, crash mode) is open. */
  reportOpen: boolean
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = {
    error: null,
    source: 'render',
    stop: null,
    componentStack: '',
    recentErrors: [],
    copied: false,
    reportOpen: false,
  }

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error, source: 'render' }
  }

  componentDidMount() {
    window.addEventListener(KERNEL_PANIC_EVENT, this.handleKernelPanic)
  }

  /** The pending reset of the "copied" mark, cleared on unmount so it
   *  never fires on a boundary that is gone. */
  private copiedTimer: number | null = null

  componentWillUnmount() {
    window.removeEventListener(KERNEL_PANIC_EVENT, this.handleKernelPanic)
    if (this.copiedTimer !== null) window.clearTimeout(this.copiedTimer)
    this.copiedTimer = null
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- console.error label for developers
    this.recordCrash(error, info.componentStack ?? '', 'Hew crashed during render:')
  }

  /** The wasm panic hook's event: stop the app now rather than wait for a
   *  render to trip over the poisoned instance, which may never happen. */
  private handleKernelPanic = () => {
    if (this.state.error !== null) return
    const stop = getKernelStop()
    const error = new Error(getPanicCapture()?.message ?? 'kernel panic')
    this.setState({ error, source: 'panic', stop })
    // oxlint-disable-next-line lingui/no-unlocalized-strings -- console.error label for developers
    this.recordCrash(error, '', stop !== null ? 'Hew stopped:' : 'Hew stopped on a kernel panic:')
  }

  private recordCrash(error: Error, stack: string, label: string) {
    // The original failure (often a Rust kernel panic) poisons the wasm instance
    // *before* the render trap we caught here. Prefer the in-memory panic
    // capture (this page session, carries the recording a reproducer needs) —
    // fall back to the wasm panic hook's localStorage record (bypasses the JS
    // console capture, survives a reload) only when there is no capture, e.g.
    // after a reload where the poisoned instance no longer exists in memory.
    const recentErrors: string[] = []
    // The hook also console.errors the panic, which the console capture copies
    // into LogStore; skip that copy so one panic isn't listed twice.
    let panicMessage: string | null = null
    const capture = getPanicCapture()
    if (capture !== null) {
      panicMessage = capture.message
      // A stop recorded from the page (recordKernelStop) is a trap, not a
      // panic; label it as what it is.
      const kind = getKernelStop() !== null ? 'kernel stopped' : 'kernel panic'
      recentErrors.push(`${kind} — ${capture.at}\n${capture.message}`)
    } else {
      try {
        const panic = localStorage.getItem('hew:lastPanic')
        if (panic !== null) {
          panicMessage = panic.slice(panic.indexOf('\n') + 1)
          // oxlint-disable-next-line lingui/no-unlocalized-strings -- crash report text for the maintainer, stays English
          recentErrors.push(`kernel panic — ${panic}`)
        }
      } catch {
        /* ignore */
      }
    }
    recentErrors.push(
      ...getEntries()
        .filter((e) => e.level === 'error' && e.message !== panicMessage)
        .slice(-8)
        .map((e) => e.message),
    )

    try {
      localStorage.setItem(LAST_ERROR_KEY, formatCrashReport(error, stack, recentErrors))
    } catch {
      /* ignore storage failures */
    }
    // eslint-disable-next-line no-console
    console.error(label, error, stack)
    this.setState({ componentStack: stack, recentErrors })
  }

  private handleCopy = () => {
    const { error, componentStack, recentErrors } = this.state
    if (error === null) return
    void navigator.clipboard?.writeText(formatCrashReport(error, componentStack, recentErrors))
    this.setState({ copied: true })
    if (this.copiedTimer !== null) window.clearTimeout(this.copiedTimer)
    this.copiedTimer = window.setTimeout(() => {
      this.copiedTimer = null
      this.setState({ copied: false })
    }, 1200)
  }

  /** Builds the `ReportBugDialog` crash-mode input: the in-memory panic
   *  capture when there is one (carries the recording), else the
   *  localStorage panic record (message only, no recording — survives a
   *  reload but the poisoned instance doesn't), else this is a plain render
   *  error with no kernel panic at all, so there's only the caught error
   *  itself to report. */
  private crashInfo(): ReportCrashInfo {
    const capture = getPanicCapture()
    if (capture !== null) return capture
    try {
      const panic = localStorage.getItem('hew:lastPanic')
      if (panic !== null) {
        const split = panic.indexOf('\n')
        return {
          at: split === -1 ? new Date().toISOString() : panic.slice(0, split),
          message: split === -1 ? panic : panic.slice(split + 1),
          recording: null,
        }
      }
    } catch {
      /* ignore — falls through to the render-error case below */
    }
    return {
      at: new Date().toISOString(),
      // oxlint-disable-next-line lingui/no-unlocalized-strings -- fallback text for the maintainer's bug report
      message: this.state.error?.message ?? 'unknown error',
      recording: null,
    }
  }

  private handleOpenReport = () => this.setState({ reportOpen: true })

  render() {
    const { error, source, stop, componentStack, recentErrors, copied, reportOpen } = this.state
    if (error === null) return this.props.children

    return (
      <div
        style={{
          position: 'fixed',
          inset: 0,
          background: 'var(--surface-window, #1a1a1a)',
          color: 'var(--text-primary, #eee)',
          font: '13px/1.5 system-ui, sans-serif',
          padding: '32px',
          overflow: 'auto',
          zIndex: 100000,
          // Native-app feel disables text selection app-wide by default
          // (index.css) — opt this whole dialog back in, so the error
          // details (a diagnosis/bug-report aid) stay selectable and
          // copyable like the Debug Log panel's entries already are.
          WebkitUserSelect: 'text',
          userSelect: 'text',
        }}
      >
        <h2 style={{ margin: '0 0 8px' }}>{stop !== null ? stop.headline : t`Hew hit an error`}</h2>
        <p style={{ color: 'var(--text-secondary, #bbb)', marginTop: 0 }}>
          {stop !== null
            ? stop.detail
            : t`The app stopped to avoid a blank window. Reload to recover — your most recent autosave will be offered.`}
        </p>
        <button
          onClick={() => location.reload()}
          style={{
            padding: '6px 16px',
            fontSize: 13,
            background: 'var(--accent-base, #3a5e9e)',
            color: '#fff',
            border: 'none',
            borderRadius: 4,
            cursor: 'pointer',
            marginRight: 8,
            marginBottom: 16,
          }}
        >
          {t`Reload`}
        </button>
        <button
          onClick={this.handleCopy}
          style={{
            padding: '6px 16px',
            fontSize: 13,
            background: 'var(--surface-input, #111)',
            color: 'var(--text-primary, #eee)',
            border: '1px solid var(--border-strong, #333)',
            borderRadius: 4,
            cursor: 'pointer',
            marginRight: 8,
            marginBottom: 16,
          }}
        >
          {copied ? t`Copied` : t`Copy details`}
        </button>
        <button
          onClick={this.handleOpenReport}
          style={{
            padding: '6px 16px',
            fontSize: 13,
            background: 'var(--surface-input, #111)',
            color: 'var(--text-primary, #eee)',
            border: '1px solid var(--border-strong, #333)',
            borderRadius: 4,
            cursor: 'pointer',
            marginBottom: 16,
          }}
        >
          {t`Report this crash`}
        </button>
        <p style={{ color: 'var(--text-secondary, #bbb)', marginTop: 0, marginBottom: 16 }}>
          {t`Report this crash sends the steps that led here privately to the Hew developer, or saves them to a file.`}
        </p>
        {reportOpen && (
          <ReportBugDialog
            scene={null}
            documentName=""
            crash={this.crashInfo()}
            onClose={() => this.setState({ reportOpen: false })}
          />
        )}
        {recentErrors.length > 0 && (
          <>
            <div style={{ color: 'var(--text-secondary, #bbb)', margin: '4px 0' }}>
              {t`Underlying error(s) — the first is usually the real cause:`}
            </div>
            <pre
              style={{
                whiteSpace: 'pre-wrap',
                background: 'var(--surface-input, #111)',
                border: '1px solid var(--danger-base, #533)',
                borderRadius: 4,
                padding: 12,
                color: 'var(--danger-base, #fbb)',
                fontSize: 12,
                marginBottom: 16,
              }}
            >
              {recentErrors.join('\n\n')}
            </pre>
          </>
        )}
        {source === 'render' && (
          <>
            <div style={{ color: 'var(--text-secondary, #bbb)', margin: '4px 0' }}>{t`Render error (symptom):`}</div>
            <pre
              style={{
                whiteSpace: 'pre-wrap',
                background: 'var(--surface-input, #111)',
                border: '1px solid var(--border-strong, #333)',
                borderRadius: 4,
                padding: 12,
                color: 'var(--danger-base, #f88)',
                fontSize: 12,
              }}
            >
              {error.message}
              {'\n\n'}
              {error.stack}
              {componentStack ? `\n\n${componentStack}` : ''}
            </pre>
          </>
        )}
      </div>
    )
  }
}
