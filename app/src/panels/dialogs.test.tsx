/**
 *  — component tests for the dialog chrome.
 *
 * Covers: RecoveryDialog, ImportingOverlay, ImportReportDialog, the STL
 * solid-gating dialog StlExportDialog, the STL import units-chooser
 * StlUnitsDialog, and the "Open on Phone…" handoff dialog PhoneShareDialog.
 * None of these touch WASM or three.js, so no mocks beyond callbacks are
 * needed — except PhoneShareDialog, which calls through
 * `@tauri-apps/api/core`'s `invoke` for everything it does against the
 * shell: `qr_svg`, and the four Rust relay commands (`relay_identity`,
 * `relay_put`, `relay_peek`, `relay_delete` — `io/relayClient.ts`) that
 * replaced the old `@tauri-apps/plugin-http` fetches. `invoke` is mocked
 * once for the whole file (`mockInvoke` below) and routed by command name
 * per test; the server setting (`settings/server.ts`) is mocked so a test
 * can pick cloud or self-hosted without a Rust backend.
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
// reference them without a "used before initialization" error. The relay
// client imports `@tauri-apps/api/core` dynamically (`await import(...)`,
// for web-bundle hygiene), but vi.mock intercepts dynamic imports of a
// mocked specifier exactly like static ones, so mocking it here still
// reaches those calls. `mockServerSetting` is what `getServerSetting()`
// resolves to (the real `effectiveOrigin` stays in play).
const mockInvoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke: mockInvoke }))
const mockServerSetting = vi.hoisted(() => ({
  current: { mode: 'cloud', origin: 'https://app.hew3d.com', uploadKey: '' } as {
    mode: 'cloud' | 'self-hosted'
    origin: string
    uploadKey: string
  },
}))
vi.mock('../settings/server', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../settings/server')>()
  return {
    ...actual,
    getServerSetting: () => Promise.resolve(mockServerSetting.current),
  }
})

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
    expect(screen.getByText(/svg line drawing \(\.svg\)/i)).toBeInTheDocument()
  })

  it('defaults to glTF and calls onExport with "glb" when Export is clicked', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('glb', 48, undefined)
  })

  it('SVG reveals view/scale/hidden-lines options and passes them to onExport (1:1 default)', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'svg' } })
    expect(screen.getByLabelText(/^view$/i)).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText(/^view$/i), { target: { value: 'top' } })
    fireEvent.click(screen.getByLabelText(/hidden lines dashed/i))
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledTimes(1)
    const [format, , svg] = onExport.mock.calls[0]
    expect(format).toBe('svg')
    expect(svg).toMatchObject({ view: 'top', hiddenDashed: true, includeDimensions: true })
    expect(svg.scale.paperMeters / svg.scale.modelMeters).toBeCloseTo(1, 9)
  })

  it('calls onExport with "stl" after switching the Format select to STL', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'stl' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('stl', 48, undefined)
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
    expect(onExport).toHaveBeenCalledWith('stl', 96, undefined)
  })

  it('offers a stored-facets ("as modeled") resolution choice', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'stl' } })
    fireEvent.change(screen.getByLabelText(/curve resolution/i), { target: { value: '0' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('stl', 0, undefined)
  })

  it('calls onExport with "3mf" after switching the Format select to 3MF', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: '3mf' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('3mf', 48, undefined)
  })

  it('calls onExport with "usdz" after switching the Format select to USDZ', () => {
    const onExport = vi.fn()
    render(<ExportDialog onExport={onExport} onCancel={vi.fn()} />)
    fireEvent.change(screen.getByLabelText(/format/i), { target: { value: 'usdz' } })
    fireEvent.click(screen.getByRole('button', { name: /^export$/i }))
    expect(onExport).toHaveBeenCalledWith('usdz', 48, undefined)
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
// workers/share-relay/README.md) — asks the relay who it is, encrypts +
// uploads on mount via the Rust relay commands, renders the QR `qr_svg`
// (mocked `invoke`) returns, and best-effort invalidates the drop on unmount.
// ---------------------------------------------------------------------------

describe('PhoneShareDialog', () => {
  const sampleDoc = { bytes: new Uint8Array([1, 2, 3]), name: 'Bench.hew' }
  const sampleToken = 'a'.repeat(22)
  const sampleQrSvg = '<svg>qr</svg>'
  const CLOUD = { mode: 'cloud' as const, origin: 'https://app.hew3d.com', uploadKey: '' }
  const SELF = { mode: 'self-hosted' as const, origin: 'https://hew.example.org', uploadKey: 'k' }
  const identity = (over: Partial<Record<string, unknown>> = {}) => ({
    origin: 'https://app.hew3d.com',
    service: 'hew-relay',
    contract: 1,
    maxBytes: 32 * 1024 * 1024,
    ttlMs: 10 * 60 * 1000,
    auth: 'none',
    ...over,
  })

  /** A typed Rust `RelayError` as `invoke` rejects with it (a plain object). */
  const relayError = (kind: string, message = kind, status?: number) => ({ kind, message, status })

  type Handlers = Partial<{
    relay_identity: () => unknown
    relay_put: (bytes: Uint8Array) => unknown
    relay_peek: (token: string) => unknown
    relay_delete: (token: string) => unknown
  }>

  /** Routes `mockInvoke` by command name. Defaults: identity answers as the
   *  open cloud relay, PUT succeeds with `sampleToken`, HEAD says present,
   *  DELETE succeeds, `qr_svg` renders. Each is overridable per test; a
   *  handler may return a value, a Promise, or throw/reject. */
  function mockRelay(handlers: Handlers = {}): void {
    mockInvoke.mockImplementation(async (cmd: string, args?: unknown) => {
      switch (cmd) {
        case 'qr_svg':
          return sampleQrSvg
        case 'relay_identity':
          return handlers.relay_identity ? handlers.relay_identity() : identity()
        case 'relay_put':
          return handlers.relay_put ? handlers.relay_put(args as Uint8Array) : { token: sampleToken }
        case 'relay_peek':
          return handlers.relay_peek ? handlers.relay_peek((args as { token: string }).token) : 'present'
        case 'relay_delete':
          return handlers.relay_delete ? handlers.relay_delete((args as { token: string }).token) : undefined
        default:
          throw new Error(`unexpected invoke: ${cmd}`)
      }
    })
  }

  const callsTo = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd)

  beforeEach(() => {
    mockInvoke.mockReset()
    mockServerSetting.current = CLOUD
  })

  it('encrypts the document and PUTs ciphertext (not plaintext) as a raw byte body', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await waitFor(() => expect(callsTo('relay_put')).toHaveLength(1))

    const uploaded = callsTo('relay_put')[0][1] as Uint8Array
    expect(uploaded).toBeInstanceOf(Uint8Array)
    // Never the plaintext bytes verbatim, and long enough to be
    // IV(12) + ciphertext(3) + GCM tag(16) = 31 bytes, not 3.
    expect(uploaded).not.toEqual(sampleDoc.bytes)
    expect(uploaded.byteLength).toBe(31)
    // No URL anywhere in the call — the Rust side owns the origin.
    expect(JSON.stringify(callsTo('relay_put')[0].slice(1))).not.toMatch(/https?:/)
  })

  it('asks the relay who it is before uploading', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await waitFor(() => expect(callsTo('relay_put')).toHaveLength(1))
    expect(callsTo('relay_identity')).toHaveLength(1)
  })

  it('asks the shell to render a QR for a #recv= URL on the app origin, carrying the token', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('qr_svg', expect.anything()))

    const { text } = callsTo('qr_svg')[0][1] as { text: string }
    expect(text.startsWith(`https://app.hew3d.com/#recv=${sampleToken}.`)).toBe(true)
    // The name segment is last and urlencoded — "Bench.hew" survives intact
    // (as %2E-free literal dots, per shareCrypto.ts's fragment grammar).
    expect(text.endsWith('.Bench.hew')).toBe(true)
  })

  it('points the QR at the self-hosted origin when one is configured, and names the server', async () => {
    mockServerSetting.current = SELF
    mockRelay({ relay_identity: () => identity({ origin: SELF.origin }) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    const url = await screen.findByLabelText<HTMLInputElement>(/handoff url/i)
    expect(url.value.startsWith(`https://hew.example.org/#recv=${sampleToken}.`)).toBe(true)
    expect(screen.getByTestId('phone-share-server')).toHaveTextContent('hew.example.org')
  })

  it('shows no server line for the Hew cloud', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await screen.findByLabelText(/handoff url/i)
    expect(screen.queryByTestId('phone-share-server')).not.toBeInTheDocument()
  })

  it('shows a starting state before the relay answers', () => {
    mockRelay({ relay_identity: () => new Promise(() => { /* never resolves in this test */ }) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(screen.getByText(/starting/i)).toBeInTheDocument()
  })

  it('renders the QR image and the URL as selectable text once ready', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    const url = await screen.findByLabelText<HTMLInputElement>(/handoff url/i)
    expect(url.value).toContain('https://app.hew3d.com/#recv=')
    const img = screen.getByAltText(/qr code/i)
    expect(img.getAttribute('src')).toContain('data:image/svg+xml')
    expect(decodeURIComponent(img.getAttribute('src') ?? '')).toContain(sampleQrSvg)
    expect(screen.getByText(/scan from shop mode/i)).toBeInTheDocument()
  })

  it('shows a clear message when the cloud relay is unreachable', async () => {
    mockRelay({ relay_identity: () => Promise.reject(relayError('unreachable')) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/could not reach the share server/i)
    expect(callsTo('relay_put')).toHaveLength(0)
  })

  it('names the self-hosted server when it is unreachable, and points at Settings ▸ Advanced', async () => {
    mockServerSetting.current = SELF
    mockRelay({ relay_identity: () => Promise.reject(relayError('unreachable')) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent(/could not reach hew\.example\.org/i)
    expect(alert).toHaveTextContent(/settings ▸ advanced/i)
  })

  it('explains an untrusted certificate', async () => {
    mockServerSetting.current = SELF
    mockRelay({ relay_identity: () => Promise.reject(relayError('tls')) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/certificate isn't trusted by this computer/i)
  })

  it('explains a rejected upload key (401 on PUT)', async () => {
    mockServerSetting.current = SELF
    mockRelay({
      relay_identity: () => identity({ auth: 'bearer' }),
      relay_put: () => Promise.reject(relayError('unauthorized', 'unauthorized', 401)),
    })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/rejected the upload key.*settings ▸ advanced/i)
  })

  it('refuses to upload without a key when the server requires one, without calling PUT', async () => {
    mockServerSetting.current = { ...SELF, uploadKey: '' }
    mockRelay({ relay_identity: () => identity({ auth: 'bearer' }) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/upload key/i)
    expect(callsTo('relay_put')).toHaveLength(0)
  })

  it('explains a full relay (503 relay full)', async () => {
    mockRelay({ relay_put: () => Promise.reject(relayError('full', 'full', 503)) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/relay is full/i)
  })

  it('degrades to the mirrored constants and still uploads when the identity route is missing (notARelay)', async () => {
    // A proxy that forwards /relay/drop but not GET /relay/, or an older
    // relay: the design's fallback — the upload proceeds with the mirrored
    // cap/TTL rather than failing on the identity probe alone.
    mockServerSetting.current = SELF
    mockRelay({ relay_identity: () => Promise.reject(relayError('notARelay')) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await screen.findByLabelText(/handoff url/i)
    expect(callsTo('relay_put')).toHaveLength(1)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('explains a server that answers the upload with a non-relay shape (notARelay from PUT)', async () => {
    mockServerSetting.current = SELF
    mockRelay({ relay_put: () => Promise.reject(relayError('notARelay')) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/isn’t serving a hew relay/i)
  })

  it('shows a clear message on a 413 (document too large)', async () => {
    mockRelay({ relay_put: () => Promise.reject(relayError('tooLarge', 'too large', 413)) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/too large/i)
  })

  it("uses the relay's own size cap from the identity route, not the mirrored constant", async () => {
    // A self-hosted relay running --max-bytes 2 refuses our 3-byte document
    // before any upload happens.
    mockRelay({ relay_identity: () => identity({ maxBytes: 2 }) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/too large/i)
    expect(callsTo('relay_put')).toHaveLength(0)
  })

  it('shows a clear message on an unexpected status', async () => {
    mockRelay({ relay_put: () => Promise.reject(relayError('status', 'unexpected status 500', 500)) })
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(await screen.findByRole('alert')).toHaveTextContent(/status 500/i)
  })

  it('shows an empty-document message and never talks to the relay when getDocument returns null', () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => null} onClose={vi.fn()} />)
    expect(screen.getByRole('alert')).toHaveTextContent(/nothing to share/i)
    expect(mockInvoke).not.toHaveBeenCalled()
  })

  it('best-effort DELETEs the drop on unmount via the Close button, once a token exists', async () => {
    mockRelay()
    const onClose = vi.fn()
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
    await screen.findByLabelText(/handoff url/i)
    fireEvent.click(screen.getByRole('button', { name: /close/i }))
    expect(onClose).toHaveBeenCalledOnce()
    // The dialog itself doesn't unmount on onClose (App.tsx's conditional
    // render owns that) — simulate the parent reacting to it, mirroring
    // every other close trigger below.
    unmount()
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('relay_delete', { token: sampleToken }))
  })

  it('best-effort DELETEs the drop on unmount via Escape', async () => {
    mockRelay()
    const onClose = vi.fn()
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
    await screen.findByLabelText(/handoff url/i)
    fireEvent.keyDown(document, { key: 'Escape' })
    expect(onClose).toHaveBeenCalledOnce()
    unmount()
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('relay_delete', { token: sampleToken }))
  })

  it('best-effort DELETEs the drop on unmount via the backdrop click', async () => {
    mockRelay()
    const onClose = vi.fn()
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
    await screen.findByLabelText(/handoff url/i)
    fireEvent.click(screen.getByRole('dialog', { name: /open on phone/i }).parentElement as HTMLElement)
    expect(onClose).toHaveBeenCalledOnce()
    unmount()
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('relay_delete', { token: sampleToken }))
  })

  it('never DELETEs on unmount if the upload never got a token (still in flight)', async () => {
    mockRelay({ relay_put: () => new Promise(() => { /* never resolves */ }) })
    const { unmount } = render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    // Let the mount effect's own identity → encrypt → PUT actually start
    // before unmounting — otherwise this can race the effect itself,
    // depending on exactly which pending microtask the test runner
    // schedules next.
    await waitFor(() => expect(callsTo('relay_put')).toHaveLength(1))
    unmount()
    await new Promise((resolve) => setTimeout(resolve, 10))
    expect(callsTo('relay_delete')).toHaveLength(0)
  })

  it('stops Escape from bubbling to window (so it does not also fire the Viewport handler)', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    await screen.findByLabelText(/handoff url/i)
    expectEscapeStopsPropagationToWindow()
  })

  it('has the expected ARIA dialog role and label', async () => {
    mockRelay()
    render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
    expect(screen.getByRole('dialog', { name: /open on phone/i })).toBeInTheDocument()
    await screen.findByLabelText(/handoff url/i) // let the pending upload settle before the test ends
  })

  // ---------------------------------------------------------------------
  // Pickup polling — QR auto-close on pickup. `shouldAdvanceTime` keeps the
  // fake clock ticking in step with real time (so the plain async work that
  // reaches `ready` — identity/encrypt/PUT/qr_svg, none of it timer-based —
  // still resolves and `findByLabelText` doesn't hang), while
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

    it('HEAD-polls the token every 2s once ready — never a GET, which would consume the drop', async () => {
      mockRelay()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={vi.fn()} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000)
      })
      expect(mockInvoke).toHaveBeenCalledWith('relay_peek', { token: sampleToken })
      // Still showing the QR — "present" means "still there", not pickup.
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()
    })

    it('shows a success confirmation and auto-closes when a poll says gone before the TTL (pickup)', async () => {
      let peeks = 0
      mockRelay({
        relay_peek: () => {
          peeks += 1
          // First tick: still there. Second tick: gone (picked up).
          return peeks === 1 ? 'present' : 'gone'
        },
      })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // present — still polling
      })
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // gone — picked up
      })
      expect(screen.getByText(/opened on your phone/i)).toBeInTheDocument()
      expect(screen.queryByLabelText(/handoff url/i)).not.toBeInTheDocument()
      expect(onClose).not.toHaveBeenCalled()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(1500) // the auto-dismiss delay
      })
      expect(onClose).toHaveBeenCalledOnce()

      // Polling itself has stopped — no further peeks once picked up.
      const peeksAtClose = callsTo('relay_peek').length
      await act(async () => {
        await vi.advanceTimersByTimeAsync(4000)
      })
      expect(callsTo('relay_peek').length).toBe(peeksAtClose)
    })

    it('leaves the QR up (no phantom pickup) when the FIRST poll says gone — a relay without the HEAD peek route', async () => {
      // Regression: a deployed relay that predates the HEAD peek answers 404
      // to every HEAD. The desktop just created this drop, so a first-look
      // 404 cannot be a real pickup — it must NOT falsely close a valid QR
      // (the phantom "Opened on your phone" a stale deploy produced).
      mockRelay({ relay_peek: () => 'gone' })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)

      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000) // several poll ticks, all gone
      })
      // The QR stays; no phantom success, no expiry, no auto-close.
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()
      expect(screen.queryByText(/opened on your phone/i)).not.toBeInTheDocument()
      expect(screen.queryByText(/expired/i)).not.toBeInTheDocument()
      expect(onClose).not.toHaveBeenCalled()
    })

    it('shows an expired message (no auto-close) when the gone arrives at/after the TTL', async () => {
      const start = Date.now()
      mockRelay({ relay_peek: () => (Date.now() - start >= DROP_TTL_MS ? 'gone' : 'present') })
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

    it("uses the relay's own TTL from the identity route for the expiry call", async () => {
      // A self-hosted relay with --ttl-secs 30: a gone at 20 s is expiry
      // (30 s − 12 s margin), not a pickup — under the 10-minute mirrored
      // constant it would have read as "opened on your phone".
      const start = Date.now()
      mockRelay({
        relay_identity: () => identity({ ttlMs: 30_000 }),
        relay_peek: () => (Date.now() - start >= 20_000 ? 'gone' : 'present'),
      })
      const onClose = vi.fn()
      render(<PhoneShareDialog getDocument={() => sampleDoc} onClose={onClose} />)
      await screen.findByLabelText(/handoff url/i)
      await act(async () => {
        await vi.advanceTimersByTimeAsync(24_000)
      })
      expect(screen.getByText(/expired/i)).toBeInTheDocument()
      expect(screen.queryByText(/opened on your phone/i)).not.toBeInTheDocument()
      expect(onClose).not.toHaveBeenCalled()
    })

    it('keeps polling through a network error mid-poll instead of closing or erroring', async () => {
      let peeks = 0
      mockRelay({
        relay_peek: () => {
          peeks += 1
          if (peeks === 1) return Promise.reject(relayError('unreachable')) // a missed tick
          return peeks === 2 ? 'present' : 'gone'
        },
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
        await vi.advanceTimersByTimeAsync(2000) // present — still there
      })
      expect(screen.getByLabelText(/handoff url/i)).toBeInTheDocument()

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2000) // gone — picked up
      })
      expect(screen.getByText(/opened on your phone/i)).toBeInTheDocument()
    })
  })
})
