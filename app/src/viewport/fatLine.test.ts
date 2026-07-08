/**
 * Tests for the fat-line material registry (fatLine.ts).
 *
 * `updateFatLineResolutions` used to `scene.traverse()` the ENTIRE scene graph
 * every rendered frame to find `LineSegments2` materials — thousands of nodes
 * walked per orbit frame on a large document. It now updates an explicit
 * registry of live fat-line materials (populated by `makeFatSegments`,
 * emptied by `disposeFatSegments`) and is only called when the canvas
 * resolution actually changes (mount/resize).
 */

import { describe, it, expect, vi, afterEach } from 'vitest'
import * as THREE from 'three'
import type { LineMaterial } from 'three/examples/jsm/lines/LineMaterial.js'
import { makeFatSegments, disposeFatSegments, updateFatLineResolutions } from './fatLine'

const SEGMENT = [0, 0, 0, 1, 0, 0]

/** Lines created per test, disposed afterwards so the module-level registry
 * never leaks entries across tests. */
let lines: THREE.Object3D[] = []

function make(): ReturnType<typeof makeFatSegments> {
  const line = makeFatSegments(SEGMENT, { color: 0xffffff, widthPx: 2 })
  lines.push(line)
  return line
}

function resolutionOf(line: THREE.Object3D): THREE.Vector2 {
  return ((line as THREE.Mesh).material as LineMaterial).resolution
}

afterEach(() => {
  for (const line of lines) disposeFatSegments(line)
  lines = []
  vi.restoreAllMocks()
})

describe('fatLine — material registry', () => {
  it('updates registered materials on a resolution change WITHOUT traversing any scene graph', () => {
    const line = make()
    // A busy scene: the old implementation walked every one of these nodes
    // per rendered frame. The registry path must not touch the graph at all.
    const scene = new THREE.Scene()
    scene.add(line)
    for (let i = 0; i < 50; i++) scene.add(new THREE.Group())

    const traverseSpy = vi.spyOn(THREE.Object3D.prototype, 'traverse')
    updateFatLineResolutions(800, 600)

    expect(traverseSpy).not.toHaveBeenCalled()
    expect(resolutionOf(line).x).toBe(800)
    expect(resolutionOf(line).y).toBe(600)
  })

  it('updates every registered material, not just the most recent', () => {
    const a = make()
    const b = make()

    updateFatLineResolutions(1024, 768)

    expect(resolutionOf(a).x).toBe(1024)
    expect(resolutionOf(b).x).toBe(1024)
  })

  it('a line built after a resize is born at the last known resolution (correct on its first frame)', () => {
    updateFatLineResolutions(1920, 1080)

    const line = make()

    expect(resolutionOf(line).x).toBe(1920)
    expect(resolutionOf(line).y).toBe(1080)
  })

  it('a disposed line is dropped from the registry and stops receiving updates', () => {
    const line = make()
    updateFatLineResolutions(640, 480)
    expect(resolutionOf(line).x).toBe(640)

    disposeFatSegments(line)
    updateFatLineResolutions(333, 222)

    // Stale material untouched — it is no longer registered.
    expect(resolutionOf(line).x).toBe(640)
  })
})
