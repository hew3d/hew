import { describe, expect, it } from 'vitest'
import { DOCUMENT_BACKDROP_COLORS_ENABLED, documentBackdropColors } from './documentBackdrop'
import type { ViewStyleColors } from './Viewport'

const loud: ViewStyleColors = { background: [255, 0, 255], sky: [0, 255, 0], ground: [255, 255, 0] }

describe('documentBackdropColors', () => {
  it('is off: a loud document or scene style gives Hew’s own theme look', () => {
    expect(DOCUMENT_BACKDROP_COLORS_ENABLED).toBe(false)
    expect(documentBackdropColors(loud)).toBeNull()
    expect(documentBackdropColors(null)).toBeNull()
  })
})
