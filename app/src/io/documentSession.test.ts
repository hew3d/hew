import { describe, it, expect } from 'vitest'
import {
  INITIAL_SESSION,
  deriveTitle,
  documentName,
  saveStateLabel,
  afterMutation,
  afterSave,
  afterOpen,
  afterImport,
  type DocSessionState,
} from './documentSession'
import type { FileRef } from './fileHost'

const mockRef = (name: string): FileRef => ({ name, handle: null })
const NOW = 1_700_000_000_000

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
  })

  describe('deriveTitle', () => {
    it('shows "Untitled" when no ref', () => {
      expect(deriveTitle(INITIAL_SESSION)).toBe('Untitled — Hew')
    })

    it('shows filename when ref present', () => {
      const state: DocSessionState = { currentRef: mockRef('model.hew'), dirty: false, lastEditAt: null, lastSavedAt: null }
      expect(deriveTitle(state)).toBe('model.hew — Hew')
    })

    it('prepends "• " when dirty', () => {
      const state: DocSessionState = { currentRef: null, dirty: true, lastEditAt: null, lastSavedAt: null }
      expect(deriveTitle(state)).toBe('• Untitled — Hew')
    })

    it('dirty + named ref', () => {
      const state: DocSessionState = { currentRef: mockRef('my-model.hew'), dirty: true, lastEditAt: null, lastSavedAt: null }
      expect(deriveTitle(state)).toBe('• my-model.hew — Hew')
    })
  })

  describe('documentName (— bare name for TitleBar/MenuBar)', () => {
    it('shows "Untitled" when no ref or importedName', () => {
      expect(documentName(INITIAL_SESSION)).toBe('Untitled')
    })

    it('shows the ref name, no dirty mark or " — Hew" suffix', () => {
      const state: DocSessionState = { currentRef: mockRef('model.hew'), dirty: true, lastEditAt: null, lastSavedAt: null }
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
      const state: DocSessionState = { currentRef: null, dirty: true, lastEditAt: null, lastSavedAt: null }
      expect(saveStateLabel(state, NOW)).toBe('Edited')
    })

    it('shows "Edited just now" immediately after a mutation', () => {
      const state = afterMutation(INITIAL_SESSION, NOW)
      expect(saveStateLabel(state, NOW)).toBe('Edited just now')
    })

    it('shows "Edited N minutes ago" as time passes without a save', () => {
      const state = afterMutation(INITIAL_SESSION, NOW)
      expect(saveStateLabel(state, NOW + 5 * 60_000)).toBe('Edited 5 minutes ago')
    })

    it('shows "Saved just now" immediately after a save', () => {
      const state = afterSave(mockRef('model.hew'), NOW)
      expect(saveStateLabel(state, NOW)).toBe('Saved just now')
    })

    it('shows "Saved N minutes ago" as time passes after a save', () => {
      const state = afterSave(mockRef('model.hew'), NOW)
      expect(saveStateLabel(state, NOW + 2 * 60_000)).toBe('Saved 2 minutes ago')
    })
  })

  describe('afterMutation', () => {
    it('marks dirty', () => {
      const next = afterMutation(INITIAL_SESSION, NOW)
      expect(next.dirty).toBe(true)
    })

    it('stamps lastEditAt on the clean -> dirty transition', () => {
      const next = afterMutation(INITIAL_SESSION, NOW)
      expect(next.lastEditAt).toBe(NOW)
    })

    it('returns same object when already dirty (no-op, including lastEditAt)', () => {
      const dirty: DocSessionState = { currentRef: null, dirty: true, lastEditAt: NOW, lastSavedAt: null }
      expect(afterMutation(dirty, NOW + 60_000)).toBe(dirty)
    })

    it('preserves currentRef', () => {
      const state: DocSessionState = { currentRef: mockRef('foo.hew'), dirty: false, lastEditAt: null, lastSavedAt: null }
      const next = afterMutation(state, NOW)
      expect(next.currentRef).toEqual(mockRef('foo.hew'))
    })
  })

  describe('afterSave', () => {
    it('clears dirty flag', () => {
      const ref = mockRef('saved.hew')
      const next = afterSave(ref, NOW)
      expect(next.dirty).toBe(false)
    })

    it('sets currentRef to the returned ref', () => {
      const ref = mockRef('saved.hew')
      const next = afterSave(ref, NOW)
      expect(next.currentRef).toEqual(ref)
    })

    it('stamps lastSavedAt and clears lastEditAt', () => {
      const next = afterSave(mockRef('saved.hew'), NOW)
      expect(next.lastSavedAt).toBe(NOW)
      expect(next.lastEditAt).toBeNull()
    })

    it('works for Save As (new name)', () => {
      const newRef = mockRef('new-name.hew')
      const next = afterSave(newRef, NOW)
      expect(next.currentRef?.name).toBe('new-name.hew')
      expect(next.dirty).toBe(false)
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

    it('title uses importedName when currentRef is null', () => {
      const next = afterImport('Guest House Countertops.dae', NOW)
      expect(deriveTitle(next)).toBe('• Guest House Countertops — Hew')
    })

    it('falls back to "Untitled" when no ref and no importedName', () => {
      // Ensures INITIAL_SESSION / afterOpen(null, now) still works correctly
      expect(deriveTitle({ currentRef: null, dirty: false, lastEditAt: null, lastSavedAt: null })).toBe('Untitled — Hew')
    })
  })
})
