/**
 *  — component tests for the dialog and floating-panel chrome.
 *
 * Covers: FloatingPanel, RecoveryDialog, ImportingOverlay, ImportReportDialog.
 * None of these touch WASM or three.js, so no mocks beyond callbacks are needed.
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi, beforeEach } from 'vitest'
import { FloatingPanel } from './FloatingPanel'
import { RecoveryDialog } from './RecoveryDialog'
import { ImportingOverlay } from './ImportingOverlay'
import { ImportReportDialog } from './ImportReportDialog'
import type { RecoverySnapshot } from '../io/recoveryStore'
import type { ImportReport } from '../io/fileHost'

// ---------------------------------------------------------------------------
// FloatingPanel
// ---------------------------------------------------------------------------

describe('FloatingPanel', () => {
  const baseProps = {
    panelId: 'test-panel',
    title: 'My Panel',
    defaultPosition: { x: 20, y: 40 },
    width: 240,
    onClose: vi.fn(),
    zIndex: 10,
    onFocus: vi.fn(),
  }

  beforeEach(() => {
    vi.clearAllMocks()
    localStorage.clear()
  })

  it('renders the title text', () => {
    render(<FloatingPanel {...baseProps}><p>body</p></FloatingPanel>)
    expect(screen.getByText('My Panel')).toBeInTheDocument()
  })

  it('renders children in the panel body', () => {
    render(<FloatingPanel {...baseProps}><p>Panel content</p></FloatingPanel>)
    expect(screen.getByText('Panel content')).toBeInTheDocument()
  })

  it('calls onClose when the × close button is clicked', () => {
    const onClose = vi.fn()
    render(
      <FloatingPanel {...baseProps} onClose={onClose}><p>body</p></FloatingPanel>,
    )
    fireEvent.click(screen.getByTitle('Close panel'))
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('positions the panel at the default coordinates', () => {
    render(
      <FloatingPanel {...baseProps} defaultPosition={{ x: 55, y: 99 }}>
        <p>body</p>
      </FloatingPanel>,
    )
    const panel = screen.getByTestId('floating-panel')
    expect(panel.style.left).toBe('55px')
    expect(panel.style.top).toBe('99px')
  })

  it('restores position from localStorage when a saved position exists', () => {
    localStorage.setItem('hew.panel.test-panel.pos', JSON.stringify({ x: 77, y: 33 }))
    render(<FloatingPanel {...baseProps}><p>body</p></FloatingPanel>)
    const panel = screen.getByTestId('floating-panel')
    // The stored position overrides the default
    expect(panel.style.left).toBe('77px')
    expect(panel.style.top).toBe('33px')
  })

  it('applies the zIndex to the panel root', () => {
    render(<FloatingPanel {...baseProps} zIndex={42}><p>body</p></FloatingPanel>)
    const panel = screen.getByTestId('floating-panel')
    expect(panel.style.zIndex).toBe('42')
  })

  it('calls onFocus when the panel is clicked (pointer-down capture)', () => {
    const onFocus = vi.fn()
    render(
      <FloatingPanel {...baseProps} onFocus={onFocus}><p>body</p></FloatingPanel>,
    )
    fireEvent.pointerDown(screen.getByTestId('floating-panel'))
    expect(onFocus).toHaveBeenCalled()
  })
})

// ---------------------------------------------------------------------------
// RecoveryDialog
// ---------------------------------------------------------------------------

describe('RecoveryDialog', () => {
  const snapshot: RecoverySnapshot = {
    bytes: new Uint8Array([]),
    meta: {
      version: 1,
      name: 'bridge.hew',
      savedAt: Date.now() - 120_000, // 2 minutes ago
      path: null,
    },
  }

  it('shows the document name', () => {
    render(
      <RecoveryDialog
        snapshot={snapshot}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    expect(screen.getByText(/bridge\.hew/)).toBeInTheDocument()
  })

  it('shows a heading about recovering the document', () => {
    render(
      <RecoveryDialog
        snapshot={snapshot}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    expect(screen.getByText(/recover unsaved document/i)).toBeInTheDocument()
  })

  it('calls onRecover when the Recover button is clicked', () => {
    const onRecover = vi.fn()
    render(
      <RecoveryDialog
        snapshot={snapshot}
        onRecover={onRecover}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /recover/i }))
    expect(onRecover).toHaveBeenCalledOnce()
  })

  it('calls onDiscard when the Discard button is clicked', () => {
    const onDiscard = vi.fn()
    render(
      <RecoveryDialog
        snapshot={snapshot}
        onRecover={vi.fn()}
        onDiscard={onDiscard}
        onDismiss={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /discard/i }))
    expect(onDiscard).toHaveBeenCalledOnce()
  })

  it('calls onDismiss — NOT onDiscard — when Escape is pressed', () => {
    const onDiscard = vi.fn()
    const onDismiss = vi.fn()
    render(
      <RecoveryDialog
        snapshot={snapshot}
        onRecover={vi.fn()}
        onDiscard={onDiscard}
        onDismiss={onDismiss}
      />,
    )
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onDismiss).toHaveBeenCalledOnce()
    // Escape must NEVER clear the snapshot — that would destroy recoverable work
    expect(onDiscard).not.toHaveBeenCalled()
  })

  it('has the expected ARIA dialog role and label', () => {
    render(
      <RecoveryDialog
        snapshot={snapshot}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    expect(screen.getByRole('dialog', { name: /recover unsaved document/i })).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// ImportingOverlay
// ---------------------------------------------------------------------------

describe('ImportingOverlay', () => {
  it('shows the name of the file being imported', () => {
    render(<ImportingOverlay fileName="theater.dae" />)
    expect(screen.getByText(/theater\.dae/)).toBeInTheDocument()
  })

  it('has an aria-live region and a status element', () => {
    render(<ImportingOverlay fileName="model.glb" />)
    // The card carries role="status" aria-busy="true"
    const statusEl = screen.getByRole('status')
    expect(statusEl).toBeInTheDocument()
    expect(statusEl).toHaveAttribute('aria-busy', 'true')
  })

  it('shows a hint about large files', () => {
    render(<ImportingOverlay fileName="model.glb" />)
    expect(screen.getByText(/large files/i)).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// ImportReportDialog
// ---------------------------------------------------------------------------

describe('ImportReportDialog', () => {
  const baseReport: ImportReport = {
    objects_created: 5,
    watertight: 4,
    leaky: 1,
    skipped: [],
    textures_missing: [],
  }

  it('shows the object count summary', () => {
    render(<ImportReportDialog report={baseReport} onClose={vi.fn()} />)
    expect(screen.getByText('5')).toBeInTheDocument()
    expect(screen.getByText(/objects imported/i)).toBeInTheDocument()
    expect(screen.getByText(/4 solid/i)).toBeInTheDocument()
    expect(screen.getByText(/1 leaky/i)).toBeInTheDocument()
  })

  it('shows "No objects" message when nothing was imported', () => {
    const emptyReport: ImportReport = { ...baseReport, objects_created: 0, watertight: 0, leaky: 0 }
    render(<ImportReportDialog report={emptyReport} onClose={vi.fn()} />)
    expect(screen.getByText(/no objects were created/i)).toBeInTheDocument()
  })

  it('does not show the leaky section when leaky = 0', () => {
    const noLeaky: ImportReport = { ...baseReport, watertight: 5, leaky: 0 }
    render(<ImportReportDialog report={noLeaky} onClose={vi.fn()} />)
    expect(screen.queryByText(/leaky/i)).not.toBeInTheDocument()
  })

  it('shows the skipped-meshes section when skipped is non-empty', () => {
    const withSkipped: ImportReport = {
      ...baseReport,
      skipped: [{ name: 'BadMesh', reason: 'zero-area triangles' }],
    }
    render(<ImportReportDialog report={withSkipped} onClose={vi.fn()} />)
    expect(screen.getByText('BadMesh')).toBeInTheDocument()
    expect(screen.getByText('zero-area triangles')).toBeInTheDocument()
    expect(screen.getByText(/skipped meshes \(1\)/i)).toBeInTheDocument()
  })

  it('shows the missing-textures section when textures_missing is non-empty', () => {
    const withMissing: ImportReport = {
      ...baseReport,
      textures_missing: ['textures/wood.png'],
    }
    render(<ImportReportDialog report={withMissing} onClose={vi.fn()} />)
    expect(screen.getByText('textures/wood.png')).toBeInTheDocument()
    expect(screen.getByText(/missing textures \(1\)/i)).toBeInTheDocument()
  })

  it('calls onClose when the OK button is clicked', () => {
    const onClose = vi.fn()
    render(<ImportReportDialog report={baseReport} onClose={onClose} />)
    fireEvent.click(screen.getByRole('button', { name: /ok/i }))
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('calls onClose when Escape is pressed', () => {
    const onClose = vi.fn()
    render(<ImportReportDialog report={baseReport} onClose={onClose} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
  })

  it('has the expected ARIA dialog role and label', () => {
    render(<ImportReportDialog report={baseReport} onClose={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /import report/i })).toBeInTheDocument()
  })
})
