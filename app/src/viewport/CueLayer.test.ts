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

  describe('the line where two objects\' faces meet (meetingLine)', () => {
    const line: [[number, number, number], [number, number, number]] = [
      [1, 0.6, 0.5],
      [1, 1.4, 0.5],
    ]
    const ends = (ls: THREE.LineSegments): number[] =>
      Array.from((ls.geometry.getAttribute('position') as THREE.BufferAttribute).array as Float32Array)

    it('draws the whole stretch between its two ends, in the line kind\'s colour', () => {
      const layer = new CueLayer()
      layer.update({ x: 1, y: 0.8, z: 0.5, kind: 'on-intersection-line', meetingLine: line })
      expect(layer.group.children).toHaveLength(1)
      const drawn = layer.group.children[0] as THREE.LineSegments
      expect(drawn).toBeInstanceOf(THREE.LineSegments)
      expect(ends(drawn)).toEqual([1, 0.6, 0.5, 1, 1.4, 0.5].map((v) => Math.fround(v)))
      expect((drawn.material as THREE.LineBasicMaterial).color.getHex()).toBe(0xe05a00)
      // Drawn over the faces it lies on, which would otherwise hide it.
      expect((drawn.material as THREE.LineBasicMaterial).depthTest).toBe(false)
    })

    it('draws the line under its midpoint too, in the midpoint\'s colour', () => {
      const layer = new CueLayer()
      layer.update({ x: 1, y: 1, z: 0.5, kind: 'midpoint', meetingLine: line })
      expect(layer.group.children).toHaveLength(1)
      const drawn = layer.group.children[0] as THREE.LineSegments
      expect((drawn.material as THREE.LineBasicMaterial).color.getHex()).toBe(0x00cccc)
    })

    it('draws nothing for a line of no length', () => {
      const layer = new CueLayer()
      layer.update({ x: 1, y: 1, z: 0.5, kind: 'on-intersection-line', meetingLine: [[1, 1, 0.5], [1, 1, 0.5]] })
      expect(layer.group.children).toHaveLength(0)
    })

    it('a snap without a meeting line draws no such line', () => {
      const layer = new CueLayer()
      layer.update({ x: 1, y: 1, z: 0.5, kind: 'on-face' })
      expect(layer.group.children).toHaveLength(0)
    })

    it('the next update replaces the line rather than adding to it', () => {
      const layer = new CueLayer()
      layer.update({ x: 1, y: 0.8, z: 0.5, kind: 'on-intersection-line', meetingLine: line })
      layer.update({ x: 1, y: 0.9, z: 0.5, kind: 'on-intersection-line', meetingLine: line })
      expect(layer.group.children).toHaveLength(1)
      layer.update(null)
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
