/**
 * Smoke test that proves the component-test harness is wired: jsdom env,
 * @testing-library/react render, and jest-dom matchers all work together.
 *
 * ErrorBoundary is the ideal subject — it's the one component whose whole job is
 * an observable render branch (children vs. fallback) with no wasm/three.js seam.
 */
import { fireEvent, render, screen } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ErrorBoundary, LAST_ERROR_KEY } from './ErrorBoundary'

describe('ErrorBoundary', () => {
  beforeEach(() => {
    localStorage.clear()
  })
  afterEach(() => {
    vi.restoreAllMocks()
  })

  it('renders children when nothing throws', () => {
    render(
      <ErrorBoundary>
        <p>hello world</p>
      </ErrorBoundary>,
    )
    expect(screen.getByText('hello world')).toBeInTheDocument()
  })

  it('renders the fallback and records the error when a child throws', () => {
    // The thrown error logs to console.error via componentDidCatch; silence it so
    // the test output stays clean (the boundary itself is what we're asserting).
    vi.spyOn(console, 'error').mockImplementation(() => {})

    function Boom(): never {
      throw new Error('kaboom')
    }
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    )

    expect(screen.getByRole('heading', { name: /hew hit an error/i })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /reload/i })).toBeInTheDocument()
    // The fallback surfaces the underlying message and persists it for post-reload.
    expect(screen.getByText(/kaboom/)).toBeInTheDocument()
    expect(localStorage.getItem(LAST_ERROR_KEY)).toContain('kaboom')
  })

  // Regression: the crash dialog's text was silently unselectable/uncopyable
  // — index.css sets `user-select: none` on <body> for the app's native-app
  // feel, and the dialog never opted back in the way LogPanel's entries do.
  // Pin both the direct fix (the fallback opts the whole dialog back into
  // text selection) and the added affordance (a Copy button using the same
  // formatted text the localStorage record gets).
  it('opts the fallback dialog back into text selection', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    function Boom(): never {
      throw new Error('kaboom')
    }
    const { container } = render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    )
    const dialog = container.firstElementChild as HTMLElement
    expect(dialog.style.userSelect).toBe('text')
  })

  it('copies the full crash report — matching the persisted record — via the Copy details button', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.assign(navigator, { clipboard: { writeText } })
    // Seed a recorded kernel panic — this is the actual playtest scenario
    // ("kernel-panic dialog is no longer copyable"), and it also exercises
    // the `recentErrors` line in the copied text, not just the base error
    // message/stack (both empty-`recentErrors` and populated cases must
    // round-trip through the same Copy button correctly).
    localStorage.setItem('hew:lastPanic', 'panicked at crates/kernel/src/ops.rs:123')

    function Boom(): never {
      throw new Error('kaboom')
    }
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    )
    expect(screen.getByText(/kernel panic/)).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: /copy details/i }))

    expect(writeText).toHaveBeenCalledTimes(1)
    const copied = writeText.mock.calls[0][0] as string
    expect(copied).toContain('kaboom')
    expect(copied).toContain('kernel panic — panicked at crates/kernel/src/ops.rs:123')
    // Same shape as the persisted record (formatCrashReport is shared) — not
    // a byte-identical string, since each call stamps its own
    // `new Date().toISOString()`. Strip the leading timestamp line before
    // comparing the rest.
    const stripTimestamp = (s: string) => s.replace(/^.*\n/, '')
    expect(stripTimestamp(copied)).toBe(stripTimestamp(localStorage.getItem(LAST_ERROR_KEY) ?? ''))
    expect(await screen.findByRole('button', { name: /^copied$/i })).toBeInTheDocument()
  })
})
