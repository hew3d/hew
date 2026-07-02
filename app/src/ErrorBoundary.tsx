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
 * which is exactly what this addresses.
 */
import { Component, type ErrorInfo, type ReactNode } from 'react'
import { getEntries } from './log/LogStore'

export const LAST_ERROR_KEY = 'hew:lastError'

interface Props {
  children: ReactNode
}

interface State {
  error: Error | null
  componentStack: string
  /** Recent captured console errors — includes the kernel `panicked at …` line
   *  that poisons the wasm instance (the thrown error is only the symptom). */
  recentErrors: string[]
}

export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, componentStack: '', recentErrors: [] }

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error }
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    const stack = info.componentStack ?? ''
    // The original failure (often a Rust kernel panic) poisons the wasm instance
    // *before* the render trap we caught here. The wasm panic hook records it to
    // localStorage (it bypasses the JS console capture), so read that first, then
    // append any recent captured console errors.
    const recentErrors: string[] = []
    try {
      const panic = localStorage.getItem('hew:lastPanic')
      if (panic !== null) recentErrors.push(`kernel panic — ${panic}`)
    } catch {
      /* ignore */
    }
    recentErrors.push(
      ...getEntries()
        .filter((e) => e.level === 'error')
        .slice(-8)
        .map((e) => e.message),
    )

    try {
      localStorage.setItem(
        LAST_ERROR_KEY,
        `${new Date().toISOString()}\n${error.message}\n\n${error.stack ?? ''}\n\n${stack}\n\n--- recent console errors ---\n${recentErrors.join('\n')}`,
      )
    } catch {
      /* ignore storage failures */
    }
    // eslint-disable-next-line no-console
    console.error('Hew crashed during render:', error, stack)
    this.setState({ componentStack: stack, recentErrors })
  }

  render() {
    const { error, componentStack, recentErrors } = this.state
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
        }}
      >
        <h2 style={{ margin: '0 0 8px' }}>Hew hit an error</h2>
        <p style={{ color: 'var(--text-secondary, #bbb)', marginTop: 0 }}>
          The app stopped to avoid a blank window. Reload to recover — your most
          recent autosave will be offered.
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
            marginBottom: 16,
          }}
        >
          Reload
        </button>
        {recentErrors.length > 0 && (
          <>
            <div style={{ color: 'var(--text-secondary, #bbb)', margin: '4px 0' }}>
              Underlying error(s) — the first is usually the real cause:
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
        <div style={{ color: 'var(--text-secondary, #bbb)', margin: '4px 0' }}>Render error (symptom):</div>
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
      </div>
    )
  }
}
