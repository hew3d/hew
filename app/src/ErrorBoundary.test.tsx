/**
 * Smoke test that proves the component-test harness is wired: jsdom env,
 * @testing-library/react render, and jest-dom matchers all work together.
 *
 * ErrorBoundary is the ideal subject — it's the one component whose whole job is
 * an observable render branch (children vs. fallback) with no wasm/three.js seam.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { ErrorBoundary, LAST_ERROR_KEY } from './ErrorBoundary'
import { clearKernelStop, KERNEL_PANIC_EVENT, recordKernelStop } from './log/panicCapture'

describe('ErrorBoundary', () => {
  beforeEach(() => {
    localStorage.clear()
  })
  afterEach(() => {
    vi.restoreAllMocks()
    delete (globalThis as { __hewLastPanic?: unknown }).__hewLastPanic
    clearKernelStop()
  })

  it('shows a recorded kernel stop in its own words — the out-of-memory open', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <ErrorBoundary>
        <p>still running</p>
      </ErrorBoundary>,
    )
    act(() => {
      recordKernelStop('Out of memory opening Big.skp (137.0 MB); kernel memory 4095.9 MB', {
        headline: 'This model is too large for Hew to open',
        detail: 'Hew ran out of memory opening Big.skp (137.0 MB). Reload to keep working.',
      })
    })

    expect(screen.getByRole('heading', { name: 'This model is too large for Hew to open' })).toBeInTheDocument()
    expect(screen.getByText(/ran out of memory opening Big\.skp/)).toBeInTheDocument()
    expect(screen.queryByText('still running')).not.toBeInTheDocument()
    expect(screen.queryByText(/hew hit an error/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/render error/i)).not.toBeInTheDocument()
    // The underlying-error list names it a stop, not a panic.
    expect(screen.getByText(/kernel stopped — /)).toBeInTheDocument()
    expect(screen.queryByText(/kernel panic — /)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^report this crash$/i })).toBeInTheDocument()
    expect(localStorage.getItem(LAST_ERROR_KEY)).toContain('Out of memory opening Big.skp')
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

  // A kernel panic usually lands in a handler that catches the trap, so no
  // render ever throws — the wasm hook's event has to stop the app instead.
  it('shows the crash screen on the kernel panic event with no render error', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    render(
      <ErrorBoundary>
        <p>still running</p>
      </ErrorBoundary>,
    )
    ;(globalThis as { __hewLastPanic?: unknown }).__hewLastPanic = {
      at: '2026-01-01T00:00:00.000Z',
      message: 'panicked at crates/kernel/src/document.rs:42: UnknownVertex',
      recording: null,
    }
    act(() => {
      window.dispatchEvent(new Event(KERNEL_PANIC_EVENT))
    })

    expect(screen.getByRole('heading', { name: /hew hit an error/i })).toBeInTheDocument()
    expect(screen.queryByText('still running')).not.toBeInTheDocument()
    expect(screen.getByText(/UnknownVertex/)).toBeInTheDocument()
    expect(screen.queryByText(/render error/i)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /^report this crash$/i })).toBeInTheDocument()
    expect(localStorage.getItem(LAST_ERROR_KEY)).toContain('UnknownVertex')
  })

  it('stops listening for the kernel panic event once unmounted', () => {
    const { unmount } = render(
      <ErrorBoundary>
        <p>ok</p>
      </ErrorBoundary>,
    )
    const remove = vi.spyOn(window, 'removeEventListener')
    unmount()
    expect(remove).toHaveBeenCalledWith(KERNEL_PANIC_EVENT, expect.any(Function))
  })

  // The hook console.errors the panic too, and the console capture copies
  // that into LogStore — the list must still name the panic only once.
  it('lists a kernel panic once, not again as its console copy', async () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    const LogStore = await import('./log/LogStore')
    LogStore.clear()
    const message = 'panicked at crates/kernel/src/document.rs:42: UnknownVertex'
    LogStore.append('error', 'console', message)
    LogStore.append('error', 'console', 'recursive use of an object')
    render(
      <ErrorBoundary>
        <p>ok</p>
      </ErrorBoundary>,
    )
    ;(globalThis as { __hewLastPanic?: unknown }).__hewLastPanic = {
      at: '2026-01-01T00:00:00.000Z',
      message,
      recording: null,
    }
    act(() => {
      window.dispatchEvent(new Event(KERNEL_PANIC_EVENT))
    })

    const list = screen.getByText(/UnknownVertex/).textContent ?? ''
    expect(list.split('UnknownVertex')).toHaveLength(2)
    expect(list).toContain('recursive use of an object')
    LogStore.clear()
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

  it('drops the pending reset of the copied mark when it unmounts', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } })
    vi.useFakeTimers()
    try {
      function Boom(): never {
        throw new Error('kaboom')
      }
      const { unmount } = render(
        <ErrorBoundary>
          <Boom />
        </ErrorBoundary>,
      )
      fireEvent.click(screen.getByRole('button', { name: /copy details/i }))
      expect(vi.getTimerCount()).toBe(1)
      unmount()
      // Nothing is left to fire on a boundary that is gone.
      expect(vi.getTimerCount()).toBe(0)
    } finally {
      vi.useRealTimers()
    }
  })

  // componentDidCatch prefers the in-memory panic capture (this page session)
  // over the localStorage record when both exist, since only the capture
  // carries a recording a saved reproducer can use.
  it('prefers the in-memory panic capture over localStorage when both are present', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    localStorage.setItem('hew:lastPanic', 'panicked at crates/kernel/src/ops.rs:123')
    ;(globalThis as { __hewLastPanic?: unknown }).__hewLastPanic = {
      at: '2026-01-01T00:00:00.000Z',
      message: 'panicked at crates/kernel/src/document.rs:42: UnknownVertex',
      recording: '{"version":2,"calls":[],"golden_hash":0}',
    }

    function Boom(): never {
      throw new Error('kaboom')
    }
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    )

    expect(
      screen.getByText(/kernel panic — 2026-01-01T00:00:00\.000Z\s+panicked at crates\/kernel\/src\/document\.rs:42: UnknownVertex/),
    ).toBeInTheDocument()
    expect(screen.queryByText(/ops\.rs:123/)).not.toBeInTheDocument()
  })

  it('falls back to the localStorage panic record when there is no in-memory capture', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    localStorage.setItem('hew:lastPanic', 'panicked at crates/kernel/src/ops.rs:123')

    function Boom(): never {
      throw new Error('kaboom')
    }
    render(
      <ErrorBoundary>
        <Boom />
      </ErrorBoundary>,
    )

    expect(screen.getByText(/kernel panic — panicked at crates\/kernel\/src\/ops\.rs:123/)).toBeInTheDocument()
  })

  describe('Report this crash button', () => {
    function renderBoom() {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      function Boom(): never {
        throw new Error('kaboom')
      }
      render(
        <ErrorBoundary>
          <Boom />
        </ErrorBoundary>,
      )
    }

    it('shows the hint line', () => {
      renderBoom()
      expect(
        screen.getByText(/report this crash sends the steps that led here privately to the hew developer/i),
      ).toBeInTheDocument()
    })

    it('opens ReportBugDialog in crash mode on click, with the model row unavailable', async () => {
      renderBoom()
      expect(screen.queryByRole('dialog', { name: /report this crash/i })).not.toBeInTheDocument()

      fireEvent.click(screen.getByRole('button', { name: /^report this crash$/i }))

      const dialog = await screen.findByRole('dialog', { name: /report this crash/i })
      expect(dialog).toBeInTheDocument()
      // Crash mode: no live scene, so the model row's checkbox is disabled and
      // shows why (reportBundle.ts's ReportRowInfo.disabledReason).
      expect(await screen.findByText(/unavailable after a crash/i)).toBeInTheDocument()
    })

    it('closes the dialog via its own Cancel button', async () => {
      renderBoom()
      fireEvent.click(screen.getByRole('button', { name: /^report this crash$/i }))
      await screen.findByRole('dialog', { name: /report this crash/i })

      fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }))

      expect(screen.queryByRole('dialog', { name: /report this crash/i })).not.toBeInTheDocument()
    })

    it('sources the crash message from the in-memory panic capture when present', async () => {
      vi.spyOn(console, 'error').mockImplementation(() => {})
      function Boom(): never {
        throw new Error('kaboom')
      }
      ;(globalThis as { __hewLastPanic?: unknown }).__hewLastPanic = {
        at: '2026-01-01T00:00:00.000Z',
        message: 'panicked at crates/kernel/src/document.rs:42: UnknownVertex',
        recording: '{"version":2,"calls":["Foo"],"golden_hash":0}',
      }
      render(
        <ErrorBoundary>
          <Boom />
        </ErrorBoundary>,
      )

      fireEvent.click(screen.getByRole('button', { name: /^report this crash$/i }))
      await screen.findByRole('dialog', { name: /report this crash/i })
      // Expand the Recorded steps row's preview — it reflects the panic
      // capture's recording (one step, "Foo"), not "no recording available",
      // proof crashInfo() picked up the in-memory capture rather than
      // falling back to a bare render-error crash block.
      const showButtons = await screen.findAllByRole('button', { name: /^show$/i })
      // Show links wait for the dialog to finish gathering the report.
      await waitFor(() => expect(showButtons[1]).not.toBeDisabled())
      fireEvent.click(showButtons[1]) // rows: system, recording, model, log
      expect(await screen.findByText(/1 step recorded/i)).toBeInTheDocument()
    })
  })
})
