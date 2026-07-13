/**
 *  — component tests for the dialog chrome.
 *
 * Covers: RecoveryDialog, ImportingOverlay, ImportReportDialog, and the
 * STL solid-gating dialog StlExportDialog.
 * None of these touch WASM or three.js, so no mocks beyond callbacks are needed.
 *
 * FloatingPanel's tests lived here too until deleted that component
 * (replaced by the permanently docked tray, `TraySection.tsx` — see its own
 * test file).
 */

import { render, screen, fireEvent } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { RecoveryDialog } from './RecoveryDialog'
import { ImportingOverlay } from './ImportingOverlay'
import { ImportReportDialog } from './ImportReportDialog'
import { StlExportDialog } from './StlExportDialog'
import { ExportDialog } from './ExportDialog'
import type { RecoveryListing } from '../io/recoveryStore'
import type { ImportReport } from '../io/fileHost'

// ---------------------------------------------------------------------------
// RecoveryDialog
// ---------------------------------------------------------------------------

describe('RecoveryDialog', () => {
  const listing = (name: string, ageMs: number): RecoveryListing => ({
    slot: name,
    meta: {
      version: 1,
      name,
      savedAt: Date.now() - ageMs,
      path: null,
    },
  })
  const single = [listing('bridge.hew', 120_000)] // 2 minutes ago

  it('shows the document name', () => {
    render(
      <RecoveryDialog
        listings={single}
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
        listings={single}
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
        listings={single}
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
        listings={single}
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
        listings={single}
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
        listings={single}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    expect(screen.getByRole('dialog', { name: /recover unsaved document/i })).toBeInTheDocument()
  })

  it('lists every document by name with multiple snapshots', () => {
    const multi = [listing('bridge.hew', 120_000), listing('tower.hew', 300_000)]
    render(
      <RecoveryDialog
        listings={multi}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    // With N crashed documents, every one of them is offered — recovery must
    // never silently drop all but the newest.
    expect(screen.getByText(/recover 2 unsaved documents/i)).toBeInTheDocument()
    expect(screen.getByText(/bridge\.hew/)).toBeInTheDocument()
    expect(screen.getByText(/tower\.hew/)).toBeInTheDocument()
  })

  it('labels the buttons Recover All / Discard All with multiple snapshots', () => {
    const multi = [listing('bridge.hew', 120_000), listing('tower.hew', 300_000)]
    const onRecover = vi.fn()
    const onDiscard = vi.fn()
    render(
      <RecoveryDialog
        listings={multi}
        onRecover={onRecover}
        onDiscard={onDiscard}
        onDismiss={vi.fn()}
      />,
    )
    fireEvent.click(screen.getByRole('button', { name: /recover all/i }))
    expect(onRecover).toHaveBeenCalledOnce()
    fireEvent.click(screen.getByRole('button', { name: /discard all/i }))
    expect(onDiscard).toHaveBeenCalledOnce()
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
    warnings: [],
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

  it('shows a SketchUp recovery note in the warnings section', () => {
    const withWarnings: ImportReport = {
      ...baseReport,
      warnings: ['parser recovered from a malformed section: Desync { offset: 0x4a10 }'],
    }
    render(<ImportReportDialog report={withWarnings} onClose={vi.fn()} />)
    expect(screen.getByText(/warnings \(1\)/i)).toBeInTheDocument()
    expect(
      screen.getByText('parser recovered from a malformed section: Desync { offset: 0x4a10 }'),
    ).toBeInTheDocument()
    // No "content may be missing" lead-in — each warning line carries its own
    // context, and the old banner contradicted split notices.
    expect(screen.queryByText(/some content may be missing/i)).not.toBeInTheDocument()
  })

  it('shows a non-manifold split notice (dae/gltf/skp) without contradicting it', () => {
    const withSplit: ImportReport = {
      ...baseReport,
      warnings: [
        "'Roof' is non-manifold; imported as 2 open shells (split at non-manifold edges, geometry unchanged)",
      ],
    }
    render(<ImportReportDialog report={withSplit} onClose={vi.fn()} />)
    expect(screen.getByText(/warnings \(1\)/i)).toBeInTheDocument()
    expect(screen.getByText(/geometry unchanged/)).toBeInTheDocument()
    expect(screen.queryByText(/some content may be missing/i)).not.toBeInTheDocument()
  })

  it('counts every warning in the section header', () => {
    const withBoth: ImportReport = {
      ...baseReport,
      warnings: [
        "'Roof' is non-manifold; imported as 2 open shells (split at non-manifold edges, geometry unchanged)",
        'parser recovered from a malformed section: Desync { offset: 0x4a10 }',
      ],
    }
    render(<ImportReportDialog report={withBoth} onClose={vi.fn()} />)
    expect(screen.getByText(/warnings \(2\)/i)).toBeInTheDocument()
  })

  it('does not show the warnings section when warnings is empty', () => {
    render(<ImportReportDialog report={baseReport} onClose={vi.fn()} />)
    expect(screen.queryByText(/warnings \(\d+\)/i)).not.toBeInTheDocument()
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

// ---------------------------------------------------------------------------
// StlExportDialog ( — solid gating)
// ---------------------------------------------------------------------------

describe('StlExportDialog', () => {
  const offenders = ['Roof', 'Object 7']

  it('shows the non-manifold warning and names every offender', () => {
    render(<StlExportDialog offenders={offenders} onExport={vi.fn()} onCancel={vi.fn()} />)
    expect(
      screen.getByText(/not watertight solids; the STL may not be manifold/i),
    ).toBeInTheDocument()
    expect(screen.getByText('Roof')).toBeInTheDocument()
    expect(screen.getByText('Object 7')).toBeInTheDocument()
  })

  it('calls onExport when "Export Anyway" is clicked', () => {
    const onExport = vi.fn()
    render(<StlExportDialog offenders={offenders} onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /export anyway/i }))
    expect(onExport).toHaveBeenCalledOnce()
  })

  it('calls onCancel when Cancel is clicked', () => {
    const onCancel = vi.fn()
    render(<StlExportDialog offenders={offenders} onExport={vi.fn()} onCancel={onCancel} />)
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }))
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('calls onCancel — never onExport — when Escape is pressed', () => {
    const onExport = vi.fn()
    const onCancel = vi.fn()
    render(<StlExportDialog offenders={offenders} onExport={onExport} onCancel={onCancel} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledOnce()
    expect(onExport).not.toHaveBeenCalled()
  })

  it('has the expected ARIA dialog role and label', () => {
    render(<StlExportDialog offenders={offenders} onExport={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /export stl warning/i })).toBeInTheDocument()
  })

  it('renames itself after the pending format via formatLabel (3MF)', () => {
    render(
      <StlExportDialog
        offenders={offenders}
        formatLabel="3MF"
        onExport={vi.fn()}
        onCancel={vi.fn()}
      />,
    )
    expect(screen.getByRole('dialog', { name: /export 3mf warning/i })).toBeInTheDocument()
    expect(screen.getByText('Export 3MF Anyway?')).toBeInTheDocument()
    expect(
      screen.getByText(/not watertight solids; the 3MF may not be manifold/i),
    ).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// ExportDialog (unified Export… dialog, — replaces the two
// separate "Export…"/"Export STL…" menu entries)
// ---------------------------------------------------------------------------

describe('ExportDialog', () => {
  it('has the expected ARIA dialog role and label', () => {
    render(<ExportDialog onExport={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /export/i })).toBeInTheDocument()
  })

  it('shows every format option in the Format select', () => {
    render(<ExportDialog onExport={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText(/gltf binary \(\.glb\).*y-up, meters/i)).toBeInTheDocument()
    expect(screen.getByText(/stl binary \(\.stl\).*millimeters, for 3d printing/i)).toBeInTheDocument()
    expect(screen.getByText(/3mf \(\.3mf\).*part names and colors/i)).toBeInTheDocument()
  })

  it('defaults to glTF and calls onExport with "glb" when Export is clicked', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('glb', 48)
  })

  it('calls onExport with "stl" after switching the Format select to STL', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'stl' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('stl', 48)
  })

  it('hides the curve-resolution select for glTF and shows it for STL', () => {
    render(<ExportDialog onExport={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.queryByLabelText(/curve resolution/i)).not.toBeInTheDocument()
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'stl' } })
    expect(screen.getByLabelText(/curve resolution/i)).toBeInTheDocument()
  })

  it('passes the chosen STL curve resolution through onExport', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'stl' } })
    fireEvent.change(screen.getByLabelText(/curve resolution/i), { target: { value: '96' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('stl', 96)
  })

  it('offers a stored-facets ("as modeled") resolution choice', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'stl' } })
    fireEvent.change(screen.getByLabelText(/curve resolution/i), { target: { value: '0' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('stl', 0)
  })

  it('calls onExport with "3mf" after switching the Format select to 3MF', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: '3mf' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('3mf', 48)
  })

  it('calls onCancel when Cancel is clicked', () => {
    const onCancel = vi.fn()
    render(<ExportDialog onExport={vi.fn()} onCancel={onCancel} />)
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }))
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('calls onCancel — never onExport — when Escape is pressed', () => {
    const onExport = vi.fn()
    const onCancel = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={onCancel} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledOnce()
    expect(onExport).not.toHaveBeenCalled()
  })
})
