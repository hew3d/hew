/**
 * highlightSetsFor — what a set of nodes lights up, shared by the selection
 * highlight and the transform tools' hover highlight.
 */

import { describe, expect, it, vi } from 'vitest'
import { highlightSetsFor, type HighlightDeps } from './highlightSets'
import type { NodeRef } from '../panels/treeModel'
import { ctxGround, draftRef } from '../tools/planeUnits'

const draft = (key: string): NodeRef => draftRef('draft-island', ctxGround(), key)

function deps(groups: Record<string, NodeRef[]> = {}, drafts: Record<string, number[]> = {}): HighlightDeps {
  return {
    groupMembers: vi.fn((id: bigint) => groups[id.toString()] ?? []),
    draftGeometry: vi.fn((node: NodeRef) => Float32Array.from(drafts[String((node as { key?: string }).key)] ?? [])),
  }
}

describe('highlightSetsFor', () => {
  it('nothing lights nothing', () => {
    const sets = highlightSetsFor([], deps())
    expect(sets).toEqual({ objects: [], instances: [], sketches: [], draftSegments: new Float32Array(0) })
  })

  it('objects, instances, and sketches light themselves', () => {
    const sets = highlightSetsFor(
      [{ kind: 'object', id: 1n }, { kind: 'instance', id: 2n }, { kind: 'sketch', id: 3n }],
      deps(),
    )
    expect(sets.objects).toEqual([1n])
    expect(sets.instances).toEqual([2n])
    expect(sets.sketches).toEqual([3n])
  })

  it('a group expands to every leaf object and instance, through nested groups', () => {
    const sets = highlightSetsFor([{ kind: 'group', id: 10n }], deps({
      '10': [{ kind: 'object', id: 1n }, { kind: 'group', id: 11n }],
      '11': [{ kind: 'object', id: 2n }, { kind: 'instance', id: 3n }],
    }))
    expect(sets.objects).toEqual([1n, 2n])
    expect(sets.instances).toEqual([3n])
  })

  it('drawn shapes light their line work, concatenated in node order', () => {
    const d = deps({}, { aa: [0, 0, 0, 1, 0, 0], bb: [5, 5, 0, 6, 5, 0] })
    const sets = highlightSetsFor([draft('aa'), draft('bb')], d)
    expect(Array.from(sets.draftSegments)).toEqual([0, 0, 0, 1, 0, 0, 5, 5, 0, 6, 5, 0])
    expect(sets.objects).toEqual([])
  })
})
