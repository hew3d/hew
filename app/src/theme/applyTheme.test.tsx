/**
 * `readAppliedTheme` needs a real `document` global, so this file runs under
 * jsdom (`.test.tsx` per vitest.config.ts's environmentMatchGlobs) even
 * though it renders no React — mirrors TextBillboard.dom.test.tsx's same
 * reasoning.
 */
import { describe, it, expect, afterEach } from 'vitest'
import { readAppliedTheme } from './applyTheme'

describe('readAppliedTheme', () => {
  afterEach(() => {
    delete document.documentElement.dataset.theme
  })

  it('reflects document.documentElement.dataset.theme — the same attribute initThemeSync keeps current for both an explicit Settings > Theme change and an OS-level prefers-color-scheme flip under \'auto\'', () => {
    document.documentElement.dataset.theme = 'dark'
    expect(readAppliedTheme()).toBe('dark')

    // Simulate a flip WITHOUT going through any theme-setting API at all —
    // this is exactly what an OS-level prefers-color-scheme change looks
    // like from this function's point of view: applyTheme's own matchMedia
    // listener (not exercised here) would perform this same dataset write,
    // and readAppliedTheme just re-reads it.
    document.documentElement.dataset.theme = 'light'
    expect(readAppliedTheme()).toBe('light')
  })

  it('falls back to dark for any value other than the literal "light" (including unset)', () => {
    document.documentElement.dataset.theme = 'something-unexpected'
    expect(readAppliedTheme()).toBe('dark')

    delete document.documentElement.dataset.theme
    expect(readAppliedTheme()).toBe('dark')
  })

  it('never calls matchMedia — a DOM attribute read only, safe to call every render frame', () => {
    const original = window.matchMedia
    window.matchMedia = (() => {
      throw new Error('readAppliedTheme must not query matchMedia')
    }) as typeof window.matchMedia
    document.documentElement.dataset.theme = 'light'
    try {
      expect(() => readAppliedTheme()).not.toThrow()
      expect(readAppliedTheme()).toBe('light')
    } finally {
      window.matchMedia = original
    }
  })
})
