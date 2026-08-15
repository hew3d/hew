import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { CueLayer } from './CueLayer'
import type { Snap } from '../tools/types'

const onAxisSnap: Snap = { x: 1, y: 2, z: 3, kind: 'on-axis', direction: [1, 0, 0] }
const onEdgeSnap: Snap = { x: 1, y: 2, z: 3, kind: 'on-edge', direction: [0, 1, 0] }
const endpointSnap: Snap = { x: 1, y: 2, z: 3, kind: 'endpoint' } // no direction

describe('CueLayer', () => {
  it('update(null) leaves the group empty', () => {
    const layer = new CueLayer()
    layer.update(null)
    expect(layer.group.children).toHaveLength(0)
  })

  it('a snap with no direction adds no guide line, regardless of kind', () => {
    const layer = new CueLayer()
    layer.update(endpointSnap)
    expect(layer.group.children).toHaveLength(0)
  })

  it('a directional snap draws a guide line by default (suppressAxisLine omitted)', () => {
    const layer = new CueLayer()
    layer.update(onAxisSnap)
    expect(layer.group.children).toHaveLength(1)
    expect(layer.group.children[0]).toBeInstanceOf(THREE.LineSegments)
  })

  it('suppressAxisLine=false behaves identically to omitting it', () => {
    const layer = new CueLayer()
    layer.update(onAxisSnap, false)
    expect(layer.group.children).toHaveLength(1)
  })

  describe('suppressAxisLine=true (shop-mode round-3 playtest finding 2)', () => {
    it('an on-axis snap draws NO guide line — Shop Mode shows no world axes to reference', () => {
      const layer = new CueLayer()
      layer.update(onAxisSnap, true)
      expect(layer.group.children).toHaveLength(0)
    })

    it('a non-axis directional snap (e.g. on-edge) still draws its guide line — it references real, visible geometry', () => {
      const layer = new CueLayer()
      layer.update(onEdgeSnap, true)
      expect(layer.group.children).toHaveLength(1)
    })

    it('toggling the flag on a second update replaces rather than accumulates', () => {
      const layer = new CueLayer()
      layer.update(onAxisSnap, false)
      expect(layer.group.children).toHaveLength(1)
      layer.update(onAxisSnap, true)
      expect(layer.group.children).toHaveLength(0)
    })
  })

  it('clear() empties the group without disposing update()-able state', () => {
    const layer = new CueLayer()
    layer.update(onAxisSnap)
    layer.clear()
    expect(layer.group.children).toHaveLength(0)
    // Still usable afterward.
    layer.update(onEdgeSnap)
    expect(layer.group.children).toHaveLength(1)
  })
})
