/**
 *  — component tests for the dialog chrome.
 *
 * Covers: RecoveryDialog, ImportingOverlay, ImportReportDialog, the STL
 * solid-gating dialog StlExportDialog, the STL import units-chooser
 * StlUnitsDialog, and the "Open on Phone…" handoff dialog PhoneShareDialog.
 * None of these touch WASM or three.js, so no mocks beyond callbacks are
 * needed — except PhoneShareDialog, which calls through
 * `@tauri-apps/api/core`'s `invoke` directly (for `qr_svg`) and uploads/
 * invalidates its encrypted drop via the dynamically-imported
 * `@tauri-apps/plugin-http`'s `fetch` (native HTTP, not the browser's —
 * see that module's doc comment for why); both are mocked for its describe
 * block only, per the `mockInvoke`/`mockFetch` helpers just below the
 * imports.
 *
 * FloatingPanel's tests lived here too until deleted that component
 * (replaced by the permanently docked tray, `TraySection.tsx` — see its own
 * test file).
 */

import { render, screen, fireEvent, waitFor, act } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { RecoveryDialog } from './RecoveryDialog'
import { ImportingOverlay } from './ImportingOverlay'
import { ImportReportDialog } from './ImportReportDialog'
import { StlExportDialog } from './StlExportDialog'
import { StlUnitsDialog } from './StlUnitsDialog'
import { ExportDialog } from './ExportDialog'
import { RescaleConfirmDialog } from './RescaleConfirmDialog'
import { PhoneShareDialog } from './PhoneShareDialog'
import { resetStlImportUnitForTest, setLastStlImportUnit } from '../settings/stlImportUnit'
import type { RecoveryListing } from '../io/recoveryStore'
import type { ImportReport } from '../io/fileHost'

// PhoneShareDialog's external dependencies — hoisted so the vi.mock
// factories below (themselves hoisted above these imports by Vitest) can
// reference mockInvoke/mockFetch without a "used before initialization"
// error. PhoneShareDialog imports `@tauri-apps/plugin-http` dynamically
// (`await import(...)`, for web-bundle hygiene — see its module doc), but
// vi.mock intercepts dynamic imports of a mocked specifier exactly like
// static ones, so mocking it here still reaches that call.
const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
const mockFetch = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/plugin-http', () => ({ fetch: mockFetch }))

/**
 * Every dialog's Escape handler must `stopPropagation()` alongside its
 * `preventDefault()` — otherwise the keydown bubbles document → window and
 * ALSO fires the Viewport's own Escape handling (context-pop or gesture
 * cancel) underneath the dialog. Attaches a window-level spy, fires Escape
 * on `document` (mirroring the dialogs' own `document.addEventListener`
 * registration), and asserts the event never reaches `window`.
 */
function expectEscapeStopsPropagationToWindow(): void {
  const windowKeyDown = vi.fn()
  window.addEventListener('keydown', windowKeyDown)
  try {
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(windowKeyDown).not.toHaveBeenCalled()
  } finally {
    window.removeEventListener('keydown', windowKeyDown)
  }
}

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

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', () => {
    render(
      <RecoveryDialog
        listings={single}
        onRecover={vi.fn()}
        onDiscard={vi.fn()}
        onDismiss={vi.fn()}
      />,
    )
    expectEscapeStopsPropagationToWindow()
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

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', () => {
    render(<ImportReportDialog report={baseReport} onClose={vi.fn()} />)
    expectEscapeStopsPropagationToWindow()
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

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', () => {
    render(<StlExportDialog offenders={offenders} onExport={vi.fn()} onCancel={vi.fn()} />)
    expectEscapeStopsPropagationToWindow()
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
// StlUnitsDialog (STL import units-chooser — DESIGN §5/§7: default
// selection is Millimeters, and the chosen unit_scale threads through to
// onChoose exactly as import_stl expects)
// ---------------------------------------------------------------------------

describe('StlUnitsDialog', () => {
  beforeEach(() => {
    resetStlImportUnitForTest()
  })

  it('defaults to Millimeters', () => {
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={vi.fn()} onCancel={vi.fn()} />)
    const mm = screen.getByRole('radio', { name: /millimeters/i })
    expect(mm).toBeChecked()
  })

  it('shows the file name and the units prompt', () => {
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText(/bracket\.stl/)).toBeInTheDocument()
    expect(screen.getByText(/don.t record their units/i)).toBeInTheDocument()
  })

  it('calls onChoose with unit_scale 0.001 for the default Millimeters selection', () => {
    const onChoose = vi.fn()
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={onChoose} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /^import$/i }))
    expect(onChoose).toHaveBeenCalledWith(0.001, 'mm')
  })

  it('threads the chosen unit through to onChoose (Inches -> 0.0254)', () => {
    const onChoose = vi.fn()
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={onChoose} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('radio', { name: /inches/i }))
    fireEvent.click(screen.getByRole('button', { name: /^import$/i }))
    expect(onChoose).toHaveBeenCalledWith(0.0254, 'in')
  })

  it('preselects the last choice made this session', () => {
    setLastStlImportUnit('m')
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('radio', { name: /^meters$/i })).toBeChecked()
  })

  it('calls onCancel when Cancel is clicked', () => {
    const onCancel = vi.fn()
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={vi.fn()} onCancel={onCancel} />)
    fireEvent.click(screen.getByRole('button', { name: /cancel/i }))
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('calls onCancel — never onChoose — when Escape is pressed', () => {
    const onChoose = vi.fn()
    const onCancel = vi.fn()
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={onChoose} onCancel={onCancel} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledOnce()
    expect(onChoose).not.toHaveBeenCalled()
  })

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', () => {
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={vi.fn()} onCancel={vi.fn()} />)
    expectEscapeStopsPropagationToWindow()
  })

  it('has the expected ARIA dialog role and label', () => {
    render(<StlUnitsDialog fileName="bracket.stl" onChoose={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /stl import units/i })).toBeInTheDocument()
  })
})

// ---------------------------------------------------------------------------
// RescaleConfirmDialog (Tape Measure's "resize the model?" confirmation,
// design tool-parity §3)
// ---------------------------------------------------------------------------

describe('RescaleConfirmDialog', () => {
  const props = { currentDistance: 2, typedDistance: 3, factor: 1.5, scope: null }

  it('shows the measured distance, typed distance, and scale factor', () => {
    render(<RescaleConfirmDialog {...props} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByText(/2 m/)).toBeInTheDocument()
    expect(screen.getByText(/3 m/)).toBeInTheDocument()
    expect(screen.getByText(/1\.5000/)).toBeInTheDocument()
  })

  it('calls onConfirm when Resize is clicked', () => {
    const onConfirm = vi.fn()
    render(<RescaleConfirmDialog {...props} onConfirm={onConfirm} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /resize/i }))
    expect(onConfirm).toHaveBeenCalledOnce()
  })

  it('calls onCancel when Cancel is clicked', () => {
    const onCancel = vi.fn()
    render(<RescaleConfirmDialog {...props} onConfirm={vi.fn()} onCancel={onCancel} />)
    fireEvent.click(screen.getByRole('button', { name: /^cancel$/i }))
    expect(onCancel).toHaveBeenCalledOnce()
  })

  it('calls onCancel — never onConfirm — when Escape is pressed', () => {
    const onConfirm = vi.fn()
    const onCancel = vi.fn()
    render(<RescaleConfirmDialog {...props} onConfirm={onConfirm} onCancel={onCancel} />)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onCancel).toHaveBeenCalledOnce()
    expect(onConfirm).not.toHaveBeenCalled()
  })

  it('has the expected ARIA dialog role and label', () => {
    render(<RescaleConfirmDialog {...props} onConfirm={vi.fn()} onCancel={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /resize the model/i })).toBeInTheDocument()
  })

  // The adversarial-review finding this dialog's Escape handler now fixes:
  // Escape used to double-dispatch — this dialog's own document-level
  // listener cancels (correctly), and the SAME event then bubbled to a
  // window-level listener (the Viewport's onKeyDown in production), which
  // routed it to the now-idle TapeMeasureTool and cleared its idlePlaneLock
  // out from under the cancel that had just run. `stopPropagation` in the
  // dialog's handler must stop the event before it ever reaches window.
  it('stops Escape from propagating to a window-level listener (the Viewport, in production)', () => {
    const onCancel = vi.fn()
    const windowListener = vi.fn()
    window.addEventListener('keydown', windowListener)
    try {
      render(<RescaleConfirmDialog {...props} onConfirm={vi.fn()} onCancel={onCancel} />)
      fireEvent.keyDown(document, { key: 'Escape' })
      expect(onCancel).toHaveBeenCalledOnce()
      // If this fires, the fix regressed: the SAME Escape reached window too,
      // which in production means TapeMeasureTool.onKey ran a SECOND time
      // and clobbered the idlePlaneLock the dialog's own cancel preserved.
      expect(windowListener).not.toHaveBeenCalled()
    } finally {
      window.removeEventListener('keydown', windowListener)
    }
  })

  // Scoped variant (docs/design/group-session.md's "Tape Measure scoped
  // rescale"): a group/component session open at arm time swaps the
  // whole-model copy for one naming the innermost frame, without touching
  // the whole-model copy above at all (`scope: null` is untouched by any of
  // this — see the tests above, unchanged).
  describe('scoped (a session frame open)', () => {
    const groupScope = { ...props, scope: { label: 'Group 1', isComponent: false } }
    const componentScope = { ...props, scope: { label: 'Widget', isComponent: true } }

    it('names the group frame instead of "the model"', () => {
      render(<RescaleConfirmDialog {...groupScope} onConfirm={vi.fn()} onCancel={vi.fn()} />)
      expect(screen.getByRole('dialog', { name: 'Resize Group 1' })).toBeInTheDocument()
      expect(screen.getByText(/Resize Group 1 so it becomes/)).toBeInTheDocument()
      expect(screen.queryByText(/the whole model/)).not.toBeInTheDocument()
      expect(screen.queryByText(/the model\?/)).not.toBeInTheDocument()
    })

    it('mentions every copy resizing for a component frame', () => {
      render(<RescaleConfirmDialog {...componentScope} onConfirm={vi.fn()} onCancel={vi.fn()} />)
      expect(screen.getByRole('dialog', { name: 'Resize Widget' })).toBeInTheDocument()
      expect(screen.getByText(/every.*copy of it/)).toBeInTheDocument()
    })

    it('still shows the measured/typed distances and factor', () => {
      render(<RescaleConfirmDialog {...groupScope} onConfirm={vi.fn()} onCancel={vi.fn()} />)
      expect(screen.getByText(/2 m/)).toBeInTheDocument()
      expect(screen.getByText(/3 m/)).toBeInTheDocument()
      expect(screen.getByText(/1\.5000/)).toBeInTheDocument()
    })

    it('Confirm/Cancel/Escape still resolve the same way as the whole-model variant', () => {
      const onConfirm = vi.fn()
      const onCancel = vi.fn()
      render(<RescaleConfirmDialog {...groupScope} onConfirm={onConfirm} onCancel={onCancel} />)
      fireEvent.click(screen.getByRole('button', { name: /resize/i }))
      expect(onConfirm).toHaveBeenCalledOnce()
      fireEvent.keyDown(document, { key: 'Escape' })
      expect(onCancel).toHaveBeenCalledOnce()
    })
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
    expect(screen.getByText(/usdz \(\.usdz\).*ar quick look/i)).toBeInTheDocument()
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

  it('calls onExport with "usdz" after switching the Format select to USDZ', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'usdz' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('usdz', 48)
  })

  it('hides the curve-resolution select for USDZ, like glTF and 3MF', () => {
    render(<ExportDialog onExport={vi.fn()} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'usdz' } })
    expect(screen.queryByLabelText(/curve resolution/i)).not.toBeInTheDocument()
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

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', () => {
    render(<ExportDialog onExport={vi.fn()} onCancel={vi.fn()} />)
    expectEscapeStopsPropagationToWindow()
  })
})

// ---------------------------------------------------------------------------
// PhoneShareDialog (File ▸ Open on Phone…, docs/design/shop-mode.md §4,
// workers/share-relay/README.md) — encrypts+uploads on mount via `fetch`,
// renders the QR `qr_svg` (mocked `invoke`) returns, and best-effort
// invalidates the drop with a DELETE on unmount.
// ---------------------------------------------------------------------------

describe('PhoneShareDialog', () => {
  const sampleDoc = { bytes: new Uint8Array([1, 2, 3]), name: 'Bench.hew' }
  const sampleToken = 'a'.repeat(22)
  const sampleQrSvg = '<svg>qr</svg>'

  /** Wires `mockFetch` (the module-level `@tauri-apps/plugin-http` mock) to
   *  the normal happy path: any PUT to /drop succeeds with `sampleToken`,
   *  any DELETE succeeds with 204. */
  function mockSuccessfulRelay(): void {
    mockFetch.mockImplementation(async (url: string, init?: RequestInit) => {
      if (init?.method === 'PUT') {
        return new Response(JSON.stringify({ token: sampleToken }), { status: 200 })
      }
      if (init?.method === 'DELETE') {
        return new Response(null, { status: 204 })
      }
      throw new Error(`unexpected fetch: ${init?.method ?? 'GET'} ${url}`)
    })
  }

  beforeEach(() => {
    mockInvoke.mockReset()
    mockInvoke.mockResolvedValue(sampleQrSvg)
    mockFetch.mockReset()
  })

  it('encrypts the document and PUTs ciphertext (not plaintext) to the share-relay /drop endpoint', async () => {
    mockSuccessfulRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await waitFor(() => expect(mockFetch).toHaveBeenCalled())

    const [url, init] = mockFetch.mock.calls[0] as [string, RequestInit]
    expect(url).toBe('https://share.hew3d.com/drop')
    expect(init.method).toBe('PUT')
    const uploaded = new Uint8Array(init.body as ArrayBuffer)
    // Never the plaintext bytes verbatim, and long enough to be
    // IV(12) + ciphertext(3) + GCM tag(16) = 31 bytes, not 3.
    expect(uploaded).not.toEqual(sampleDoc.bytes)
    expect(uploaded.byteLength).toBe(31)
  })

  it('asks the shell to render a QR for a #recv= URL on the app origin, carrying the token', async () => {
    mockSuccessfulRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('qr_svg', expect.anything()))

    const { text } = mockInvoke.mock.calls[0][1] as { text: string }
    expect(text.startsWith(`https://app.hew3d.com/#recv=${sampleToken}.`)).toBe(true)
    // The name segment is last and urlencoded — "Bench.hew" survives intact
    // (as %2E-free literal dots, per shareCrypto.ts's fragment grammar).
    expect(text.endsWith('.Bench.hew')).toBe(true)
  })

  it('shows a starting state before the upload resolves', () => {
    mockFetch.mockReturnValue(new Promise(() => { /* never resolves in this test */ }))
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(screen.getByText(/starting/i)).toBeInTheDocument()
  })

  it('renders the QR image and the URL as selectable text once ready', async () => {
    mockSuccessfulRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    const url = await screen.findByLabelText<HTMLInputElement>(/handoff url/i)
    expect(url.value).toContain('https://app.hew3d.com/#recv=')
    const img = screen.getByAltText(/qr code/i)
    expect(img.getAttribute('src')).toContain('data:image/svg+xml')
    expect(decodeURIComponent(img.getAttribute('src') ?? '')).toContain(sampleQrSvg)
    expect(screen.getByText(/scan from shop mode/i)).toBeInTheDocument()
  })

  it('shows a clear message when the upload is offline (fetch throws a TypeError)', async () => {
    mockFetch.mockRejectedValue(new TypeError('Failed to fetch'))
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/could not reach the share server/i)
  })

  it('shows a clear message on a 413 (document too large)', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 413 }))
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/too large/i)
  })

  it('shows a clear message on a non-200/413 response', async () => {
    mockFetch.mockResolvedValue(new Response(null, { status: 500 }))
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/status 500/i)
  })

  it('shows an empty-document message and never calls fetch when getDocument returns null', () => {
    render(<PhoneShareDialog getDocument={() => null} onClose={vi.fn()} />)
    expect(screen.getByRole('alert')).toHaveTextContent(/nothing to share/i)
    expect(mockFetch).not.toHaveBeenCalled()
  })

  it('best-effort DELETEs the drop on unmount via the Close button, once a token exists', async () => {
    mockSuccessfulRelay()
    const onClose = vi.fn()
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
    await screen.findByLabelText(/handoff url/i)
    fireEvent.click(screen.getByRole('button', { name: /close/i }))
    expect(onClose).toHaveBeenCalledOnce()
    // The dialog itself doesn't unmount on onClose (App.tsx's conditional
    // render owns that) — simulate the parent reacting to it, mirroring
    // every other close trigger below.
    unmount()
    await waitFor(() =>
      expect(mockFetch).toHaveBeenCalledWith(`https://share.hew3d.com/drop/${sampleToken}`, {
        method: 'DELETE',
      }),
    )
  })

  it('best-effort DELETEs the drop on unmount via Escape', async () => {
    mockSuccessfulRelay()
    const onClose = vi.fn()
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
    await screen.findByLabelText(/handoff url/i)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
    unmount()
    await waitFor(() =>
      expect(mockFetch).toHaveBeenCalledWith(`https://share.hew3d.com/drop/${sampleToken}`, {
        method: 'DELETE',
      }),
    )
  })

  it('best-effort DELETEs the drop on unmount via the backdrop click', async () => {
    mockSuccessfulRelay()
    const onClose = vi.fn()
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
    await screen.findByLabelText(/handoff url/i)
    fireEvent.click(screen.getByRole('dialog', { name: /open on phone/i }).parentElement as HTMLElement)
    expect(onClose).toHaveBeenCalledOnce()
    unmount()
    await waitFor(() =>
      expect(mockFetch).toHaveBeenCalledWith(`https://share.hew3d.com/drop/${sampleToken}`, {
        method: 'DELETE',
      }),
    )
  })

  it('never DELETEs on unmount if the upload never got a token (still in flight)', async () => {
    mockFetch.mockReturnValue(new Promise(() => { /* never resolves */ }))
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    // Let the mount effect's own encrypt-then-PUT actually start before
    // unmounting — otherwise this can race the effect itself, depending on
    // exactly which pending microtask the test runner schedules next.
    await new Promise((resolve) => setTimeout(resolve, 10))
    unmount()
    await new Promise((resolve) => setTimeout(resolve, 10))
    expect(mockFetch).not.toHaveBeenCalledWith(expect.stringContaining('/drop/'), expect.anything())
  })

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', async () => {
    mockSuccessfulRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await screen.findByLabelText(/handoff url/i)
    expectEscapeStopsPropagationToWindow()
  })

  it('has the expected ARIA dialog role and label', async () => {
    mockSuccessfulRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /open on phone/i })).toBeInTheDocument()
    await screen.findByLabelText(/handoff url/i) // let the pending upload settle before the test ends
  })

  // ---------------------------------------------------------------------
  // Pickup polling — QR auto-close on pickup. `shouldAdvanceTime` keeps the
  // fake clock ticking in step with real time (so the plain async work that
  // reaches `ready` — encrypt/PUT/qr_svg, none of it timer-based — still
  // resolves and `findByLabelText` doesn't hang), while
  // `vi.advanceTimersByTimeAsync` fast-forwards the 2s poll interval itself
  // without the test actually waiting on it.
  // ---------------------------------------------------------------------

  describe('pickup polling', () => {
    const DROP_TTL_MS = 10 * 60 * 1000 // mirrors dropStore.ts's TTL_MS

    beforeEach(() => {
      vi.useFakeTimers({ shouldAdvanceTime: true })
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    it('HEAD-polls /drop/<token> every 2s once ready — never GET, which would consume the drop', async () => {
      mockFetch.mockImplementation(async (url: string, init?: RequestInit) => {
        if (init?.method === 'PUT') return new Response(JSON.stringify({ token: sampleToken }), { status: 200 })
        if (init?.method === 'HEAD') return new Response(null, { status: 200 })
        throw new Error(`unexpected fetch: ${init?.method ?? 'GET'} ${url}`)
      })
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000)
      })
      expect(mockFetch).toHaveBeenCalledWith(`https://share.hew3d.com/drop/${sampleToken}`, {
        method: 'HEAD',
      })
      // Still showing the QR — a 200 means "still there", not pickup.
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()
    })

    it('shows a success confirmation and auto-closes when a poll 404s before the TTL (pickup)', async () => {
      let headCalls = 0
      mockFetch.mockImplementation(async (url: string, init?: RequestInit) => {
        if (init?.method === 'PUT') return new Response(JSON.stringify({ token: sampleToken }), { status: 200 })
        if (init?.method === 'HEAD') {
          headCalls += 1
          // First tick: still there. Second tick: gone (picked up).
          return new Response(null, { status: headCalls === 1 ? 200 : 404 })
        }
        if (init?.method === 'DELETE') return new Response(null, { status: 204 })
        throw new Error(`unexpected fetch: ${init?.method ?? 'GET'} ${url}`)
      })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // 200 — still polling
      })
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // 404 — picked up
      })
      expect(screen.getByText(/opened on your phone/i)).toBeInTheDocument()
      expect(screen.queryByLabelText(/handoff url/i)).not.toBeInTheDocument()
      expect(onClose).not.toHaveBeenCalled()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(1500) // the auto-dismiss delay
      })
      expect(onClose).toHaveBeenCalledOnce()

      // Polling itself has stopped — no further HEAD calls once picked up.
      const headCallsAtClose = mockFetch.mock.calls.filter(
        (c) => (c[1] as RequestInit | undefined)?.method === 'HEAD',
      ).length
      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000)
      })
      expect(
        mockFetch.mock.calls.filter((c) => (c[1] as RequestInit | undefined)?.method === 'HEAD').length,
      ).toBe(headCallsAtClose)
    })

    it('leaves the QR up (no phantom pickup) when the FIRST poll 404s — a relay without the HEAD peek route', async () => {
      // Regression: a deployed relay that predates the HEAD peek answers 404
      // to every HEAD. The desktop just created this drop, so a first-look
      // 404 cannot be a real pickup — it must NOT falsely close a valid QR
      // (the phantom "Opened on your phone" a stale deploy produced).
      mockFetch.mockImplementation(async (url: string, init?: RequestInit) => {
        if (init?.method === 'PUT') return new Response(JSON.stringify({ token: sampleToken }), { status: 200 })
        if (init?.method === 'HEAD') return new Response(null, { status: 404 }) // stale worker: no HEAD route
        if (init?.method === 'DELETE') return new Response(null, { status: 204 })
        throw new Error(`unexpected fetch: ${init?.method ?? 'GET'} ${url}`)
      })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000) // several poll ticks, all 404
      })
      // The QR stays; no phantom success, no expiry, no auto-close.
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()
      expect(screen.queryByText(/opened on your phone/i)).not.toBeInTheDocument()
      expect(screen.queryByText(/expired/i)).not.toBeInTheDocument()
      expect(onClose).not.toHaveBeenCalled()
    })

    it('shows an expired message (no auto-close) when the 404 arrives at/after the TTL', async () => {
      const start = Date.now()
      mockFetch.mockImplementation(async (url: string, init?: RequestInit) => {
        if (init?.method === 'PUT') return new Response(JSON.stringify({ token: sampleToken }), { status: 200 })
        if (init?.method === 'HEAD') {
          const elapsed = Date.now() - start
          return new Response(null, { status: elapsed >= DROP_TTL_MS ? 404 : 200 })
        }
        if (init?.method === 'DELETE') return new Response(null, { status: 204 })
        throw new Error(`unexpected fetch: ${init?.method ?? 'GET'} ${url}`)
      })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(DROP_TTL_MS + 2000)
      })
      expect(screen.getByText(/expired.*reopen to try again/i)).toBeInTheDocument()

      // Unlike a pickup, expiry never auto-dismisses.
      await act(async () => {
        await vi.advanceTimersByTimeAsync(5000)
      })
      expect(onClose).not.toHaveBeenCalled()
    }, 20_000)

    it('keeps polling through a network error mid-poll instead of closing or erroring', async () => {
      let headCalls = 0
      mockFetch.mockImplementation(async (url: string, init?: RequestInit) => {
        if (init?.method === 'PUT') return new Response(JSON.stringify({ token: sampleToken }), { status: 200 })
        if (init?.method === 'HEAD') {
          headCalls += 1
          if (headCalls === 1) throw new TypeError('network unreachable') // a missed tick
          return new Response(null, { status: headCalls === 2 ? 200 : 404 })
        }
        if (init?.method === 'DELETE') return new Response(null, { status: 204 })
        throw new Error(`unexpected fetch: ${init?.method ?? 'GET'} ${url}`)
      })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // transport error — skipped tick
      })
      // Still up, not closed and not showing an error — the QR stays valid.
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
      expect(onClose).not.toHaveBeenCalled()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // 200 — still there
      })
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // 404 — picked up
      })
      expect(screen.getByText(/opened on your phone/i)).toBeInTheDocument()
    })
  })
})
