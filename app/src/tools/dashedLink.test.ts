/**
 * DashedLink — the dashed line Move, Rotate, and Scale draw between their
 * two working points: black, or the color of the drawing axis it lies on.
 */

import { describe, expect, it } from 'vitest'
import * as THREE from 'three'
import { DashedLink, LINK_COLOR } from './dashedLink'
import { WORLD_DRAWING_AXES, type DrawingAxes } from './drawingAxes'
import { axisColorsForTheme } from '../viewport/axisColors'
import { getResolvedTheme } from '../settings/theme'

const colorOf = (link: DashedLink): number => (link.line.material as THREE.LineDashedMaterial).color.getHex()
const axisColor = (axis: 0 | 1 | 2): number => {
  // Through a material, so color management matches the link's own path.
  const m = new THREE.LineBasicMaterial({ color: axisColorsForTheme(getResolvedTheme())[axis] })
  return m.color.getHex()
}

describe('DashedLink', () => {
  it('runs between the two points, always on top', () => {
    const link = new DashedLink(() => WORLD_DRAWING_AXES)
    link.set([1, 2, 3], [4, 6, 3])
    const pos = link.line.geometry.getAttribute('position')
    expect([pos.getX(0), pos.getY(0), pos.getZ(0), pos.getX(1), pos.getY(1), pos.getZ(1)]).toEqual([1, 2, 3, 4, 6, 3])
    expect((link.line.material as THREE.Material).depthTest).toBe(false)
  })

  it('is black off the axes', () => {
    const link = new DashedLink(() => WORLD_DRAWING_AXES)
    link.set([0, 0, 0], [1, 1, 0])
    expect(colorOf(link)).toBe(LINK_COLOR)
  })

  it('takes the color of the axis it lies on, either direction', () => {
    const link = new DashedLink(() => WORLD_DRAWING_AXES)
    link.set([1, 1, 1], [5, 1, 1])
    expect(colorOf(link)).toBe(axisColor(0))
    link.set([1, 1, 1], [1, -3, 1])
    expect(colorOf(link)).toBe(axisColor(1))
    link.set([1, 1, 1], [1, 1, 0])
    expect(colorOf(link)).toBe(axisColor(2))
  })

  it('only ON the axis: a line merely close to it stays black', () => {
    const link = new DashedLink(() => WORLD_DRAWING_AXES)
    link.set([0, 0, 0], [10, 0.01, 0]) // ~0.06° off X
    expect(colorOf(link)).toBe(LINK_COLOR)
  })

  it('follows moved drawing axes, not world X/Y/Z', () => {
    const s = Math.SQRT1_2
    const turned: DrawingAxes = { origin: [0, 0, 0], x: [s, s, 0], y: [-s, s, 0], z: [0, 0, 1] }
    const link = new DashedLink(() => turned)
    link.set([0, 0, 0], [2, 2, 0])
    expect(colorOf(link)).toBe(axisColor(0))
    link.set([0, 0, 0], [2, 0, 0]) // world X, off the turned frame
    expect(colorOf(link)).toBe(LINK_COLOR)
  })

  it('holds its dash a constant size on screen: farther away, longer in world units', () => {
    const link = new DashedLink(() => WORLD_DRAWING_AXES)
    link.set([0, 0, 0], [1, 0, 0])
    const camera = new THREE.PerspectiveCamera()
    const wpp = (dist: number): number => dist * 0.001
    const mat = link.line.material as THREE.LineDashedMaterial

    camera.position.set(0.5, 0, 10)
    link.fitDash(camera, wpp)
    const near = mat.dashSize
    camera.position.set(0.5, 0, 40)
    link.fitDash(camera, wpp)
    expect(mat.dashSize).toBeCloseTo(near * 4)
    expect(mat.gapSize).toBeLessThan(mat.dashSize)
  })
})
