import { describe, it, expect } from 'vitest'
import * as THREE from 'three'
import { DrawPlaneCueLayer } from './DrawPlaneCueLayer'
import { axisDrawPlane, groundDrawPlane } from '../tools/drawPlane'
import type { DrawPlane } from '../tools/drawPlane'

describe('DrawPlaneCueLayer', () => {
  it('update(null) leaves the group empty', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update(null)
    expect(layer.group.children).toHaveLength(0)
  })

  it('update() with a cue adds exactly one LineSegments mesh, depth-test disabled', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update({ plane: axisDrawPlane(0, [1, 2, 3]), through: [1, 2, 3] })
    expect(layer.group.children).toHaveLength(1)
    const mesh = layer.group.children[0] as THREE.LineSegments
    expect(mesh).toBeInstanceOf(THREE.LineSegments)
    const mat = mesh.material as THREE.LineBasicMaterial
    expect(mat.depthTest).toBe(false)
    expect(mat.transparent).toBe(true)
  })

  it('a second update() replaces the previous mesh rather than accumulating', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update({ plane: axisDrawPlane(0, [1, 2, 3]), through: [1, 2, 3] })
    layer.update({ plane: axisDrawPlane(1, [4, 5, 6]), through: [4, 5, 6] })
    expect(layer.group.children).toHaveLength(1)
  })

  it('update(null) after a cue clears the group', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update({ plane: axisDrawPlane(0, [1, 2, 3]), through: [1, 2, 3] })
    layer.update(null)
    expect(layer.group.children).toHaveLength(0)
  })

  it('clear() disposes and empties the group', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update({ plane: axisDrawPlane(0, [1, 2, 3]), through: [1, 2, 3] })
    layer.clear()
    expect(layer.group.children).toHaveLength(0)
  })

  it('an axis-aligned plane normal (X) colors the patch the red axis color', () => {
    const layer = new DrawPlaneCueLayer()
    layer.update({ plane: axisDrawPlane(0, [1, 2, 3]), through: [1, 2, 3] })
    const mesh = layer.group.children[0] as THREE.LineSegments
    const mat = mesh.material as THREE.LineBasicMaterial
    // Dark-theme red axis color (default theme in the test environment) —
    // just assert it's NOT the neutral gray, the theme-specific hex is
    // covered by axisColors.test.ts.
    expect(mat.color.getHex()).not.toBe(0x888888)
  })

  it('a tilted (non-axis-aligned) plane normal colors the patch neutral gray', () => {
    const tilted: DrawPlane = {
      origin: [0, 0, 0],
      normal: [1, 1, 1].map((c) => c / Math.sqrt(3)) as [number, number, number],
      u: [1, -1, 0].map((c) => c / Math.sqrt(2)) as [number, number, number],
      v: [1, 1, -2].map((c) => c / Math.sqrt(6)) as [number, number, number],
      ground: false,
    }
    const layer = new DrawPlaneCueLayer()
    layer.update({ plane: tilted, through: [0, 0, 0] })
    const mesh = layer.group.children[0] as THREE.LineSegments
    const mat = mesh.material as THREE.LineBasicMaterial
    expect(mat.color.getHex()).toBe(0x888888)
  })

  it('the patch geometry is centered on `through` (vertex positions bound the through point)', () => {
    const layer = new DrawPlaneCueLayer()
    const through: [number, number, number] = [10, -5, 2]
    layer.update({ plane: groundDrawPlane(), through })
    // groundDrawPlane() is ground so this branch is unreachable through the
    // real tools, but the layer itself is total — exercise it directly to
    // confirm the geometry centers on `through` regardless of ground-ness
    // (the ground-gating lives in `drawPlaneCue`, not this rendering layer).
    const mesh = layer.group.children[0] as THREE.LineSegments
    const pos = mesh.geometry.getAttribute('position')
    let sumX = 0, sumY = 0, sumZ = 0
    for (let i = 0; i < pos.count; i++) {
      sumX += pos.getX(i)
      sumY += pos.getY(i)
      sumZ += pos.getZ(i)
    }
    expect(sumX / pos.count).toBeCloseTo(through[0], 6)
    expect(sumY / pos.count).toBeCloseTo(through[1], 6)
    expect(sumZ / pos.count).toBeCloseTo(through[2], 6)
  })
})
