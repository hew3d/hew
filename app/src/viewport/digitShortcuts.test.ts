import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'

// The Viewport key handler is too large to mount in a unit test, so this
// guards the source: digits are typed values, never tool shortcuts.
describe('digit keys are not tool shortcuts', () => {
  const src = readFileSync(new URL('./Viewport.tsx', import.meta.url), 'utf8')

  it('no digit key switches a tool', () => {
    for (const d of ['1', '2', '3', '4', '5', '6']) {
      expect(src).not.toMatch(new RegExp(`ev\\.key === '${d}'\\)\\s*\\{\\s*switchToolRef`))
    }
  })

  it('Space and the letters are the keymap\'s: the viewport hands them to its dispatcher, not to a tool', () => {
    // Tool shortcuts are keymap commands (keymap/defaults.ts), run by the
    // app's dispatcher; the viewport's key handler only keeps them away
    // from the active tool's typed-value buffer.
    expect(src).toMatch(/commandForChord\(chordToString\(chord\)\) !== undefined/)
    expect(src).not.toMatch(/switchToolRef\.current\?\.\('Rectangle'\)/)
  })
})
