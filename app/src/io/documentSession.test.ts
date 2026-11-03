import { describe, it, expect } from 'vitest'
import {
  INITIAL_SESSION,
  deriveTitle,
  documentName,
  documentBaseName,
  saveStateLabel,
  afterMutation,
  markNonUndoableDirty,
  NON_UNDOABLE_REASON,
  importedReason,
  applyWriteThroughSave,
  afterSave,
  afterOpen,
  afterImport,
  afterRecovery,
  canRevertToSaved,
  type DocSessionState,
} from './documentSession'
import type { FileRef } from './fileHost'

const mockRef = (name: string): FileRef => ({ name, format: 'hew', handle: null })
const NOW = 1_700_000_000_000
const NO_REASONS = new Set<string>()

describe('documentSession', () => {
  describe('INITIAL_SESSION', () => {
    it('starts clean with no ref', () => {
      expect(INITIAL_SESSION.currentRef).toBeNull()
      expect(INITIAL_SESSION.dirty).toBe(false)
    })

    it('starts with no edit/save history', () => {
      expect(INITIAL_SESSION.lastEditAt).toBeNull()
      expect(INITIAL_SESSION.lastSavedAt).toBeNull()
    })

    it('starts with no non-undoable reasons', () => {
      expect(INITIAL_SESSION.nonUndoableReasons.size).toBe(0)
    })
  })

  describe('deriveTitle', () => {
    it('shows "Untitled" when no ref', () => {
      expect(deriveTitle(INITIAL_SESSION)).toBe('Untitled — Hew')
    })

    it('shows filename when ref present', () => {
      const state: DocSessionState = { currentRef: mockRef('model.hew'), dirty: false, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      expect(deriveTitle(state)).toBe('model.hew — Hew')
    })

    it('prepends "• " when dirty', () => {
      const state: DocSessionState = { currentRef: null, dirty: true, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      expect(deriveTitle(state)).toBe('• Untitled — Hew')
    })

    it('dirty + named ref', () => {
      const state: DocSessionState = { currentRef: mockRef('my-model.hew'), dirty: true, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      expect(deriveTitle(state)).toBe('• my-model.hew — Hew')
    })
  })

  describe('documentName (— bare name for TitleBar/MenuBar)', () => {
    it('shows "Untitled" when no ref or importedName', () => {
      expect(documentName(INITIAL_SESSION)).toBe('Untitled')
    })

    it('shows the ref name, no dirty mark or " — Hew" suffix', () => {
      const state: DocSessionState = { currentRef: mockRef('model.hew'), dirty: true, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      expect(documentName(state)).toBe('model.hew')
    })

    it('falls back to importedName when currentRef is null', () => {
      const next = afterImport('Kitchen.dae', NOW)
      expect(documentName(next)).toBe('Kitchen')
    })
  })

  describe('saveStateLabel (— "Edited just now" indicator)', () => {
    it('is blank for a fresh document with no edit/save history', () => {
      expect(saveStateLabel(INITIAL_SESSION, NOW)).toBe('')
    })

    it('shows "Edited" with no relative time when dirty but lastEditAt is somehow null', () => {
      const state: DocSessionState = { currentRef: null, dirty: true, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      expect(saveStateLabel(state, NOW)).toBe('Edited')
    })

    it('shows "Edited just now" immediately after a mutation', () => {
      const state = afterMutation(INITIAL_SESSION, NOW, false)
      expect(saveStateLabel(state, NOW)).toBe('Edited just now')
    })

    it('shows "Edited N minutes ago" as time passes without a save', () => {
      const state = afterMutation(INITIAL_SESSION, NOW, false)
      expect(saveStateLabel(state, NOW + 5 * 60_000)).toBe('Edited 5 minutes ago')
    })

    it('shows "Saved just now" immediately after a save', () => {
      const state = afterSave(mockRef('model.hew'), NOW, true)
      expect(saveStateLabel(state, NOW)).toBe('Saved just now')
    })

    it('shows "Saved N minutes ago" as time passes after a save', () => {
      const state = afterSave(mockRef('model.hew'), NOW, true)
      expect(saveStateLabel(state, NOW + 2 * 60_000)).toBe('Saved 2 minutes ago')
    })
  })

  describe('afterMutation', () => {
    it('marks dirty when not at the saved mark', () => {
      const next = afterMutation(INITIAL_SESSION, NOW, false)
      expect(next.dirty).toBe(true)
    })

    it('stays clean when at the saved mark and no non-undoable reasons', () => {
      const next = afterMutation(INITIAL_SESSION, NOW, true)
      expect(next.dirty).toBe(false)
      expect(next).toBe(INITIAL_SESSION) // no-op — same reference
    })

    it('stamps lastEditAt on the clean -> dirty transition', () => {
      const next = afterMutation(INITIAL_SESSION, NOW, false)
      expect(next.lastEditAt).toBe(NOW)
    })

    it('returns same object when already dirty (no-op, including lastEditAt)', () => {
      const dirty: DocSessionState = { currentRef: null, dirty: true, lastEditAt: NOW, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      expect(afterMutation(dirty, NOW + 60_000, false)).toBe(dirty)
    })

    it('preserves currentRef', () => {
      const state: DocSessionState = { currentRef: mockRef('foo.hew'), dirty: false, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS }
      const next = afterMutation(state, NOW, false)
      expect(next.currentRef).toEqual(mockRef('foo.hew'))
    })

    it('undo back to the saved depth (atSavedMark: true) cleans the document', () => {
      const edited = afterMutation(INITIAL_SESSION, NOW, false)
      expect(edited.dirty).toBe(true)
      const undone = afterMutation(edited, NOW + 1000, true)
      expect(undone.dirty).toBe(false)
    })

    it('stays dirty at the saved mark when a non-undoable reason is still pending', () => {
      const withReason = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      const next = afterMutation(withReason, NOW + 1000, true)
      expect(next.dirty).toBe(true)
    })
  })

  describe('markNonUndoableDirty', () => {
    it('dirties even though the kernel undo stack is untouched', () => {
      const next = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      expect(next.dirty).toBe(true)
      expect(next.nonUndoableReasons.has(NON_UNDOABLE_REASON.scenesEdited)).toBe(true)
    })

    it('stamps lastEditAt on the clean -> dirty transition', () => {
      const next = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      expect(next.lastEditAt).toBe(NOW)
    })

    it('does not restart the edit clock for a second reason while already dirty', () => {
      const first = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      const second = markNonUndoableDirty(first, NON_UNDOABLE_REASON.libraryMetadata, NOW + 5000)
      expect(second.lastEditAt).toBe(NOW)
      expect(second.nonUndoableReasons.size).toBe(2)
    })

    it('is a no-op for a reason already recorded while already dirty', () => {
      const first = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      const second = markNonUndoableDirty(first, NON_UNDOABLE_REASON.scenesEdited, NOW + 5000)
      expect(second).toBe(first)
    })

    it('survives afterMutation reporting the kernel clean again (still dirty)', () => {
      const withReason = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      const stillDirty = afterMutation(withReason, NOW + 1000, true)
      expect(stillDirty.dirty).toBe(true)
      expect(stillDirty.nonUndoableReasons.has(NON_UNDOABLE_REASON.scenesEdited)).toBe(true)
    })
  })

  describe('afterSave', () => {
    it('keeps a non-undoable reason added while the write was pending', () => {
      const edited = markNonUndoableDirty(INITIAL_SESSION, NON_UNDOABLE_REASON.scenesEdited, NOW)
      const next = afterSave(mockRef('saved.hew'), NOW + 1, false, edited)
      expect(next.dirty).toBe(true)
      expect(next.nonUndoableReasons).toContain(NON_UNDOABLE_REASON.scenesEdited)
      expect(next.lastEditAt).toBe(NOW)
    })
    it('clears dirty flag when atSavedMark is true', () => {
      const ref = mockRef('saved.hew')
      const next = afterSave(ref, NOW, true)
      expect(next.dirty).toBe(false)
    })

    it('sets currentRef to the returned ref', () => {
      const ref = mockRef('saved.hew')
      const next = afterSave(ref, NOW, true)
      expect(next.currentRef).toEqual(ref)
    })

    it('stamps lastSavedAt and clears lastEditAt when clean', () => {
      const next = afterSave(mockRef('saved.hew'), NOW, true)
      expect(next.lastSavedAt).toBe(NOW)
      expect(next.lastEditAt).toBeNull()
    })

    it('clears non-undoable reasons', () => {
      const next = afterSave(mockRef('saved.hew'), NOW, true)
      expect(next.nonUndoableReasons.size).toBe(0)
    })

    it('works for Save As (new name)', () => {
      const newRef = mockRef('new-name.hew')
      const next = afterSave(newRef, NOW, true)
      expect(next.currentRef?.name).toBe('new-name.hew')
      expect(next.dirty).toBe(false)
    })

    it('stays dirty when atSavedMark is false (an edit raced ahead of the write)', () => {
      const next = afterSave(mockRef('saved.hew'), NOW, false)
      expect(next.dirty).toBe(true)
      expect(next.lastEditAt).toBe(NOW)
    })
  })

  describe('afterOpen', () => {
    it('sets ref and clears dirty', () => {
      const ref = mockRef('opened.hew')
      const next = afterOpen(ref, NOW)
      expect(next.currentRef?.name).toBe('opened.hew')
      expect(next.dirty).toBe(false)
    })

    it('stamps lastSavedAt (the doc is in sync with disk as of now) and clears lastEditAt', () => {
      const next = afterOpen(mockRef('opened.hew'), NOW)
      expect(next.lastSavedAt).toBe(NOW)
      expect(next.lastEditAt).toBeNull()
    })

    it('null ref = New document (Untitled)', () => {
      const next = afterOpen(null, NOW)
      expect(next.currentRef).toBeNull()
      expect(next.dirty).toBe(false)
    })

    it('clears non-undoable reasons', () => {
      const next = afterOpen(null, NOW)
      expect(next.nonUndoableReasons.size).toBe(0)
    })
  })

  describe('afterImport', () => {
    it('is dirty with no file handle (currentRef = null)', () => {
      const next = afterImport('Kitchen.dae', NOW)
      expect(next.currentRef).toBeNull()
      expect(next.dirty).toBe(true)
    })

    it('stamps lastEditAt and leaves lastSavedAt null (never saved)', () => {
      const next = afterImport('Kitchen.dae', NOW)
      expect(next.lastEditAt).toBe(NOW)
      expect(next.lastSavedAt).toBeNull()
    })

    it('strips .dae extension from importedName', () => {
      const next = afterImport('Kitchen.dae', NOW)
      expect(next.importedName).toBe('Kitchen')
    })

    it('strips .dae extension case-insensitively', () => {
      const next = afterImport('Model.DAE', NOW)
      expect(next.importedName).toBe('Model')
    })

    it('strips every known extension from importedName, not just .dae', () => {
      expect(afterImport('Part.stl', NOW).importedName).toBe('Part')
      expect(afterImport('House.skp', NOW).importedName).toBe('House')
      expect(afterImport('Scan.GLB', NOW).importedName).toBe('Scan')
      expect(afterImport('Chair.hew', NOW).importedName).toBe('Chair')
    })

    it('keeps an unknown extension and an inner dot', () => {
      expect(afterImport('v1.2 plan.txt', NOW).importedName).toBe('v1.2 plan.txt')
      expect(afterImport('a.skp.dae', NOW).importedName).toBe('a.skp')
    })

    it('title uses importedName when currentRef is null', () => {
      const next = afterImport('Guest House Countertops.dae', NOW)
      expect(deriveTitle(next)).toBe('• Guest House Countertops — Hew')
    })

    it('falls back to "Untitled" when no ref and no importedName', () => {
      // Ensures INITIAL_SESSION / afterOpen(null, now) still works correctly
      expect(deriveTitle({ currentRef: null, dirty: false, lastEditAt: null, lastSavedAt: null, nonUndoableReasons: NO_REASONS })).toBe('Untitled — Hew')
    })

    it('stamps an Imported non-undoable reason (safety net for a zero-content import)', () => {
      const next = afterImport('Kitchen.dae', NOW)
      expect(next.nonUndoableReasons.has(importedReason('Kitchen'))).toBe(true)
    })

    it('stays dirty even if the kernel later reports at_saved_mark true', () => {
      const imported = afterImport('Kitchen.dae', NOW)
      const still = afterMutation(imported, NOW + 1000, true)
      expect(still.dirty).toBe(true)
    })
  })

  describe('afterRecovery', () => {
    it('is dirty via the Recovered document reason', () => {
      const next = afterRecovery(mockRef('crashed.hew'), undefined, NOW)
      expect(next.dirty).toBe(true)
      expect(next.nonUndoableReasons.has(NON_UNDOABLE_REASON.recovered)).toBe(true)
    })

    it('leaves lastSavedAt null — never written to the real file', () => {
      const next = afterRecovery(mockRef('crashed.hew'), undefined, NOW)
      expect(next.lastSavedAt).toBeNull()
    })

    it('carries the recovered lastEditAt through', () => {
      const next = afterRecovery(null, 'Untitled', NOW - 5000)
      expect(next.lastEditAt).toBe(NOW - 5000)
    })

    it('stays dirty even if the kernel reports at_saved_mark true (fresh Scene.load default)', () => {
      const recovered = afterRecovery(mockRef('crashed.hew'), undefined, NOW)
      const still = afterMutation(recovered, NOW + 1000, true)
      expect(still.dirty).toBe(true)
    })
  })
})

describe('applyWriteThroughSave (library stamp write-through)', () => {
  const ref = { name: 'model.hew', format: 'hew' as const, handle: '/tmp/model.hew' }

  it('applies afterSave when the session is untouched since capture', () => {
    const captured = afterSave(ref, 1000, true)
    const next = applyWriteThroughSave(captured, captured, ref, 2000, true)
    expect(next.dirty).toBe(false)
    expect(next).not.toBe(captured)
  })

  it('keeps an edit made during the async save dirty (never clobbers)', () => {
    const captured = afterSave(ref, 1000, true)
    const edited = afterMutation(captured, 1500, false)
    expect(edited.dirty).toBe(true)
    const next = applyWriteThroughSave(edited, captured, ref, 2000, true)
    expect(next).toBe(edited)
    expect(next.dirty).toBe(true)
  })

  it('keeps a replaced session (new/open during the save) untouched', () => {
    const captured = afterSave(ref, 1000, true)
    const replaced = afterImport('Other', 1500)
    const next = applyWriteThroughSave(replaced, captured, ref, 2000, true)
    expect(next).toBe(replaced)
  })

  it('honors atSavedMark: false (an undo-stack edit raced ahead, unseen by the coarser session guard)', () => {
    const captured = afterSave(ref, 1000, true)
    const next = applyWriteThroughSave(captured, captured, ref, 2000, false)
    expect(next.dirty).toBe(true)
  })
})

describe('afterOpen with a document format', () => {
  const skpRef = (format: 'skp2026' | 'skp2017', handle: unknown = '/tmp/Model.skp') => ({
    name: 'Model.skp',
    format,
    handle,
  })

  it('binds the ref (and its format) as a clean document', () => {
    const next = afterOpen(skpRef('skp2026'), NOW)
    expect(next.dirty).toBe(false)
    expect(next.currentRef?.format).toBe('skp2026')
    expect(next.currentRef?.handle).toBe('/tmp/Model.skp')
    expect(next.importedName).toBeUndefined()
    expect(next.nonUndoableReasons.size).toBe(0)
    expect(next.lastSavedAt).toBe(NOW)
  })

  it('titles the document with the .skp file name', () => {
    expect(deriveTitle(afterOpen(skpRef('skp2017'), NOW))).toBe('Model.skp — Hew')
    expect(documentName(afterOpen(skpRef('skp2017'), NOW))).toBe('Model.skp')
  })

  it('a save keeps the format of the ref it is handed', () => {
    const next = afterSave(skpRef('skp2017'), NOW, true)
    expect(next.currentRef?.format).toBe('skp2017')
    expect(next.dirty).toBe(false)
  })

  it('a recovered skp document keeps its format on the ref', () => {
    const next = afterRecovery(skpRef('skp2026'), undefined, NOW)
    expect(next.currentRef?.format).toBe('skp2026')
    expect(next.dirty).toBe(true)
  })
})

describe('documentBaseName', () => {
  it('drops the extension of an opened .skp (never Model.skp.hew)', () => {
    const state = afterOpen({ name: 'Model.skp', format: 'skp2026', handle: null }, NOW)
    expect(documentBaseName(state)).toBe('Model')
  })

  it('drops the extension of an imported name and of a .hew ref', () => {
    expect(documentBaseName(afterImport('Kitchen.dae', NOW))).toBe('Kitchen')
    expect(documentBaseName(afterOpen({ name: 'house.hew', format: 'hew', handle: null }, NOW))).toBe('house')
  })

  it('is Untitled for a blank document', () => {
    expect(documentBaseName(INITIAL_SESSION)).toBe('Untitled')
  })
})

describe('canRevertToSaved', () => {
  const dirty = (handle: unknown, over: Partial<DocSessionState> = {}): DocSessionState => ({
    ...INITIAL_SESSION,
    currentRef: { name: 'a.hew', format: 'hew', handle },
    dirty: true,
    ...over,
  })

  it('is true for a changed document bound to a desktop path or a web file handle', () => {
    expect(canRevertToSaved(dirty('/tmp/a.hew'))).toBe(true)
    expect(canRevertToSaved(dirty({ getFile: () => undefined }))).toBe(true)
  })

  it('is false when clean, with no file, or with no handle to read it back through', () => {
    expect(canRevertToSaved(dirty('/tmp/a.hew', { dirty: false }))).toBe(false)
    expect(canRevertToSaved({ ...INITIAL_SESSION, dirty: true })).toBe(false)
    expect(canRevertToSaved(dirty(null))).toBe(false)
  })
})
