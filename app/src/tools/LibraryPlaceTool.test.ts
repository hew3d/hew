import { describe, expect, it } from 'vitest'
import { parseInsertResult } from './LibraryPlaceTool'

describe('parseInsertResult', () => {
  it('parses the wasm report shape (root ids as decimal strings)', () => {
    const result = parseInsertResult({
      rootKinds: [2, 0],
      rootIds: ['4294967297', '12'],
      definitionsAdded: 1,
      definitionsReused: 0,
      materialsAdded: 2,
      materialsReused: 1,
      objectsAdded: 3,
      guidesAdded: 0,
      worldSketchesSkipped: 1,
      annotationsSkipped: 2,
    })
    expect(result.rootKinds).toEqual([2, 0])
    expect(result.rootIds).toEqual([4294967297n, 12n])
    expect(result.definitionsReused).toBe(0)
    expect(result.worldSketchesSkipped).toBe(1)
    expect(result.annotationsSkipped).toBe(2)
  })

  it('tolerates a malformed report without throwing', () => {
    const result = parseInsertResult(undefined)
    expect(result.rootKinds).toEqual([])
    expect(result.rootIds).toEqual([])
    expect(result.definitionsReused).toBe(0)
    expect(result.drawn).toEqual([])
    expect(result.drawingUnchanged).toBe(false)
  })

  it("parses a paste's drawn sketches (v1.5 decisions D3)", () => {
    const result = parseInsertResult({
      rootKinds: [],
      rootIds: [],
      drawn: [
        { plane: [0, 0, 0, 0, 0, 1], detached: null, anchors: [[[0, 0, 0], [1, 0, 0]]] },
        { plane: [0, 0, 1, 0, 0, 1], detached: '4294967298', anchors: [] },
        { plane: [0, 0], detached: null, anchors: [] },
      ],
      drawingUnchanged: true,
    })
    expect(result.drawn).toEqual([
      { plane: [0, 0, 0, 0, 0, 1], detached: null, anchors: [[[0, 0, 0], [1, 0, 0]]] },
      { plane: [0, 0, 1, 0, 0, 1], detached: 4294967298n, anchors: [] },
    ])
    expect(result.drawingUnchanged).toBe(true)
  })
})
