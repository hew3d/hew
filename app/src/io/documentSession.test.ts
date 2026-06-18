import { describe, it, expect } from 'vitest'
import {
  INITIAL_SESSION,
  deriveTitle,
  afterMutation,
  afterSave,
  afterOpen,
  type DocSessionState,
} from './documentSession'
import type { FileRef } from './fileHost'

const mockRef = (name: string): FileRef => ({ name, handle: null })

describe('documentSession', () => {
  describe('INITIAL_SESSION', () => {
    it('starts clean with no ref', () => {
      expect(INITIAL_SESSION.currentRef).toBeNull()
      expect(INITIAL_SESSION.dirty).toBe(false)
    })
  })

  describe('deriveTitle', () => {
    it('shows "Untitled" when no ref', () => {
      expect(deriveTitle(INITIAL_SESSION)).toBe('Untitled — Hew')
    })

    it('shows filename when ref present', () => {
      const state: DocSessionState = { currentRef: mockRef('model.hew'), dirty: false }
      expect(deriveTitle(state)).toBe('model.hew — Hew')
    })

    it('prepends "• " when dirty', () => {
      const state: DocSessionState = { currentRef: null, dirty: true }
      expect(deriveTitle(state)).toBe('• Untitled — Hew')
    })

    it('dirty + named ref', () => {
      const state: DocSessionState = { currentRef: mockRef('my-model.hew'), dirty: true }
      expect(deriveTitle(state)).toBe('• my-model.hew — Hew')
    })
  })

  describe('afterMutation', () => {
    it('marks dirty', () => {
      const next = afterMutation(INITIAL_SESSION)
      expect(next.dirty).toBe(true)
    })

    it('returns same object when already dirty (no-op)', () => {
      const dirty: DocSessionState = { currentRef: null, dirty: true }
      expect(afterMutation(dirty)).toBe(dirty)
    })

    it('preserves currentRef', () => {
      const state: DocSessionState = { currentRef: mockRef('foo.hew'), dirty: false }
      const next = afterMutation(state)
      expect(next.currentRef).toEqual(mockRef('foo.hew'))
    })
  })

  describe('afterSave', () => {
    it('clears dirty flag', () => {
      const ref = mockRef('saved.hew')
      const next = afterSave(ref)
      expect(next.dirty).toBe(false)
    })

    it('sets currentRef to the returned ref', () => {
      const ref = mockRef('saved.hew')
      const next = afterSave(ref)
      expect(next.currentRef).toEqual(ref)
    })

    it('works for Save As (new name)', () => {
      const newRef = mockRef('new-name.hew')
      const next = afterSave(newRef)
      expect(next.currentRef?.name).toBe('new-name.hew')
      expect(next.dirty).toBe(false)
    })
  })

  describe('afterOpen', () => {
    it('sets ref and clears dirty', () => {
      const ref = mockRef('opened.hew')
      const next = afterOpen(ref)
      expect(next.currentRef?.name).toBe('opened.hew')
      expect(next.dirty).toBe(false)
    })

    it('null ref = New document (Untitled)', () => {
      const next = afterOpen(null)
      expect(next.currentRef).toBeNull()
      expect(next.dirty).toBe(false)
    })
  })
})
