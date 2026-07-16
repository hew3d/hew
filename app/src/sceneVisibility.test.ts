import { describe, it, expect } from 'vitest'
import {
  isSceneVisiblyEmpty,
  isLoneVisibleSketchScene,
  type HiddenLeafIds,
  type VisibilitySceneView,
} from './sceneVisibility'

/**
 * The first-sketch auto-zoom edge (App.handleDocumentChanged) is defined on
 * VISIBLE emptiness, not kernel emptiness: a document whose only solid is
 * hidden (eye or tag) renders a blank viewport, and a small rectangle drawn
 * into that blank view must reframe exactly like one drawn into a truly
 * empty document — the tiny-unframed-shape failure the feature exists to
 * prevent. These tests pin that hide-then-draw scenario.
 */

interface StubNode {
  kind: string
  id: bigint
}

function stubScene(opts: {
  objects?: bigint[]
  instances?: bigint[]
  groups?: bigint[]
  sketches?: bigint[]
  members?: Map<bigint, StubNode[]>
}): VisibilitySceneView {
  const members = opts.members ?? new Map<bigint, StubNode[]>()
  return {
    object_ids: () => opts.objects ?? [],
    instance_ids: () => opts.instances ?? [],
    group_ids: () => opts.groups ?? [],
    sketch_ids: () => opts.sketches ?? [],
    group_members: (g: bigint) => members.get(g) ?? [],
  }
}

const NOTHING_HIDDEN: HiddenLeafIds = { objects: new Set(), instances: new Set() }

describe('isSceneVisiblyEmpty', () => {
  it('a kernel-empty scene is visibly empty', () => {
    expect(isSceneVisiblyEmpty(stubScene({}), NOTHING_HIDDEN)).toBe(true)
  })

  it('a visible object makes the scene non-empty', () => {
    const scene = stubScene({ objects: [1n] })
    expect(isSceneVisiblyEmpty(scene, NOTHING_HIDDEN)).toBe(false)
  })

  it('a scene whose only object is hidden IS visibly empty (hide-then-draw pre-state)', () => {
    const scene = stubScene({ objects: [1n] })
    const hidden: HiddenLeafIds = { objects: new Set([1n]), instances: new Set() }
    expect(isSceneVisiblyEmpty(scene, hidden)).toBe(true)
  })

  it('a hidden instance is invisible; a visible one is not', () => {
    const scene = stubScene({ instances: [7n] })
    expect(isSceneVisiblyEmpty(scene, NOTHING_HIDDEN)).toBe(false)
    const hidden: HiddenLeafIds = { objects: new Set(), instances: new Set([7n]) }
    expect(isSceneVisiblyEmpty(scene, hidden)).toBe(true)
  })

  it('a group counts through its leaves: all-hidden leaves = visibly empty', () => {
    // Group 10 holds object 1 and a nested group 11 holding instance 7.
    const members = new Map<bigint, StubNode[]>([
      [10n, [{ kind: 'object', id: 1n }, { kind: 'group', id: 11n }]],
      [11n, [{ kind: 'instance', id: 7n }]],
    ])
    const scene = stubScene({ groups: [10n, 11n], members })

    expect(isSceneVisiblyEmpty(scene, NOTHING_HIDDEN)).toBe(false)
    const allHidden: HiddenLeafIds = { objects: new Set([1n]), instances: new Set([7n]) }
    expect(isSceneVisiblyEmpty(scene, allHidden)).toBe(true)
    const partHidden: HiddenLeafIds = { objects: new Set([1n]), instances: new Set() }
    expect(isSceneVisiblyEmpty(scene, partHidden)).toBe(false)
  })

  it('a sketch is always visible content (sketches cannot be hidden)', () => {
    const scene = stubScene({ sketches: [3n] })
    expect(isSceneVisiblyEmpty(scene, NOTHING_HIDDEN)).toBe(false)
  })
})

describe('isLoneVisibleSketchScene (the reframe edge)', () => {
  it('fires for one sketch in a truly empty document', () => {
    const scene = stubScene({ sketches: [3n] })
    expect(isLoneVisibleSketchScene(scene, NOTHING_HIDDEN)).toBe(true)
  })

  it('fires for one sketch when every solid is hidden — the hide-then-draw scenario', () => {
    // Create + extrude an object, tag-hide it (viewport visually blank),
    // then draw a small rectangle: the reframe must fire.
    const scene = stubScene({ objects: [1n], sketches: [3n] })
    const hidden: HiddenLeafIds = { objects: new Set([1n]), instances: new Set() }
    expect(isLoneVisibleSketchScene(scene, hidden)).toBe(true)
  })

  it('does not fire while any solid is visible', () => {
    const scene = stubScene({ objects: [1n], sketches: [3n] })
    expect(isLoneVisibleSketchScene(scene, NOTHING_HIDDEN)).toBe(false)
  })

  it('does not fire for more than one sketch', () => {
    const scene = stubScene({ sketches: [3n, 4n] })
    expect(isLoneVisibleSketchScene(scene, NOTHING_HIDDEN)).toBe(false)
  })
})
