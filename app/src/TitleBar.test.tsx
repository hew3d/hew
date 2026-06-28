/**
 *  — component tests for TitleBar.
 *
 * TitleBar is the custom Linux desktop window chrome (draggable bar + resize grips).
 * The @tauri-apps/api/window module is dynamically imported in a useEffect; we
 * mock it so the test never touches a real Tauri API.
 *
 * Component modification note: NONE — TitleBar already carries aria-label on every
 * button, making it testable without data-testid additions.
 */

import { render, screen } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { TitleBar } from './TitleBar'

// Tauri window API is a dynamic import inside a useEffect.  Vitest intercepts
// module resolution, so mocking the static path works even for dynamic imports.
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: vi.fn(() => ({
    isMaximized: vi.fn(() => Promise.resolve(false)),
    onResized: vi.fn(() => Promise.resolve(() => { /* unlisten noop */ })),
    minimize: vi.fn(() => Promise.resolve()),
    toggleMaximize: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
    startResizeDragging: vi.fn(() => Promise.resolve()),
  })),
}))

describe('TitleBar', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('renders the title string', () => {
    render(<TitleBar title="bridge.hew — Hew" />)
    expect(screen.getByText('bridge.hew — Hew')).toBeInTheDocument()
  })

  it('renders a Minimize button', () => {
    render(<TitleBar title="Untitled" />)
    expect(screen.getByRole('button', { name: /minimize/i })).toBeInTheDocument()
  })

  it('renders a Maximize button when not maximized', () => {
    render(<TitleBar title="Untitled" />)
    expect(screen.getByRole('button', { name: /maximize/i })).toBeInTheDocument()
  })

  it('renders a Close button', () => {
    render(<TitleBar title="Untitled" />)
    expect(screen.getByRole('button', { name: /close/i })).toBeInTheDocument()
  })

  it('renders resize grips when not maximized', () => {
    const { container } = render(<TitleBar title="Untitled" />)
    // There are 8 grip divs (N/S/E/W + 4 corners)
    // They are the only divs with a fixed position style
    const fixed = container.querySelectorAll('[style*="position: fixed"]')
    expect(fixed.length).toBe(8)
  })
})
