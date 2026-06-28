/**
 * Smoke test that proves the component-test harness is wired: jsdom env,
 * @testing-library/react render, and jest-dom matchers all work together.
 *
 * ErrorBoundary is the ideal subject — it's the one component whose whole job is
 * an observable render branch (children vs. fallback) with no wasm/three.js seam.
 */
import { render, screen } from '@testing-library/react'
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
})
