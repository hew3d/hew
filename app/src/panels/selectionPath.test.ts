import { describe, expect, it } from 'vitest'
import { ancestorsOf, buildSelectionPath, frameClickAction, hoverLine, pathAbove, type SelectionPathInput } from './selectionPath'
import { nodeEq, nodeKey, type NodeRef } from './treeModel'

const g = (id: number): NodeRef => ({ kind: 'group', id: BigInt(id) })
const o = (id: number): NodeRef => ({ kind: 'object', id: BigInt(id) })
const i = (id: number): NodeRef => ({ kind: 'instance', id: BigInt(id) })

// Kitchen(g1) > Base(g2) > Cabinet(g3) > Door(o10), Carcass(o11)
// Island(g4) > Shaker(i20, definition member object 30)
// Floor(o12) top level
const PARENT = new Map<string, NodeRef>([
  [nodeKey(g(2)), g(1)],
  [nodeKey(g(3)), g(2)],
  [nodeKey(o(10)), g(3)],
  [nodeKey(o(11)), g(3)],
  [nodeKey(i(20)), g(4)],
])
const NAMES = new Map<string, string>([
  [nodeKey(g(1)), 'Kitchen'], [nodeKey(g(2)), 'Base Cabinets'], [nodeKey(g(3)), 'Cabinet'],
  [nodeKey(g(4)), 'Island'], [nodeKey(o(10)), 'Door'], [nodeKey(o(11)), 'Carcass'],
  [nodeKey(o(12)), 'Floor'], [nodeKey(i(20)), 'Shaker Door'],
  [nodeKey({ kind: 'sketch', id: 5n }), 'Sketch 1'],
])

function input(over: Partial<SelectionPathInput> = {}): SelectionPathInput {
  return {
    sessionStack: [],
    activeContext: [],
    selected: [],
    hitLeaf: null,
    parentOf: (n) => PARENT.get(nodeKey(n)),
    sessionMembers: null,
    labelFor: (n) => NAMES.get(nodeKey(n)) ?? `?${n.kind}`,
    memberName: (id) => (id === 30n ? 'Door Panel' : undefined),
    draftOwner: () => null,
    ...over,
  }
}
const labels = (c: ReturnType<typeof buildSelectionPath>) => c.map((x) => x.label)
const states = (c: ReturnType<typeof buildSelectionPath>) => c.map((x) => x.state)

describe('buildSelectionPath', () => {
  it('shows nothing for no selection, a multi-selection, or a lone top-level object', () => {
    expect(buildSelectionPath(input())).toEqual([])
    expect(buildSelectionPath(input({ selected: [o(10), o(11)] }))).toEqual([])
    expect(buildSelectionPath(input({ selected: [o(12)] }))).toEqual([])
  })

  it('a nested selection made outside the viewport ends at the selected node', () => {
    const c = buildSelectionPath(input({ selected: [g(3)] }))
    expect(labels(c)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet'])
    expect(states(c)).toEqual(['above', 'above', 'above', 'selected'])
    // Outliner-made selection: no hit leaf, so no crumbs below.
    expect(c.every((x) => x.state !== 'below')).toBe(true)
  })

  it('a viewport click adds the crumbs below the selection down to the hit leaf', () => {
    const c = buildSelectionPath(input({ selected: [g(1)], hitLeaf: { object: 10n } }))
    expect(labels(c)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet', 'Door'])
    expect(states(c)).toEqual(['above', 'selected', 'below', 'below', 'below'])
  })

  it('drops a stale hit leaf that is not under the selection', () => {
    const c = buildSelectionPath(input({ selected: [g(4)], hitLeaf: { object: 10n } }))
    expect(labels(c)).toEqual([])
  })

  it('shows a two-crumb path for a nested object and hides at one', () => {
    expect(labels(buildSelectionPath(input({ selected: [g(1)] })))).toEqual([])
    expect(labels(buildSelectionPath(input({ selected: [g(2)] })))).toEqual(['Model', 'Kitchen', 'Base Cabinets'])
  })

  it('ends a definition-member hit with a non-clickable italic member crumb', () => {
    const c = buildSelectionPath(input({ selected: [g(4)], hitLeaf: { object: 30n, instance: 20n } }))
    expect(labels(c)).toEqual(['Model', 'Island', 'Shaker Door', 'Door Panel'])
    const member = c[c.length - 1]
    expect(member.kind).toBe('member')
    expect(member.state).toBe('member')
    expect(member.node).toBeNull()
    expect(member.enterNode).toEqual(i(20))
    // The instance crumb before it is the deepest selectable one.
    expect(c[c.length - 2].node).toEqual(i(20))
  })

  it('names an unnamed member "Part"', () => {
    const c = buildSelectionPath(input({ selected: [g(4)], hitLeaf: { object: 31n, instance: 20n } }))
    expect(c[c.length - 1].label).toBe('Part')
  })

  it('an open component shows its hit part as a normal crumb, not a member', () => {
    const c = buildSelectionPath(input({
      sessionStack: [{ node: i(20), label: 'Shaker Door' }],
      selected: [i(20)],
      hitLeaf: { object: 30n, instance: 20n },
    }))
    expect(c.some((x) => x.kind === 'member')).toBe(false)
  })

  it('an open session reads as editing frames and replaces the pill with no selection', () => {
    const c = buildSelectionPath(input({ sessionStack: [{ node: g(3), label: 'Cabinet' }] }))
    expect(labels(c)).toEqual(['Model', 'Cabinet'])
    expect(c[1].frameDepth).toBe(0)
    expect(c[0].frameDepth).toBeNull()
  })

  it('splices the open stack in front of a selection under it (parent links erased by the frame)', () => {
    // Inside Kitchen > Base > Cabinet: the frame erased Door/Carcass links.
    const erased = new Map(PARENT)
    erased.delete(nodeKey(o(10)))
    erased.delete(nodeKey(o(11)))
    const c = buildSelectionPath(input({
      sessionStack: [
        { node: g(1), label: 'Kitchen' }, { node: g(2), label: 'Base Cabinets' }, { node: g(3), label: 'Cabinet' },
      ],
      parentOf: (n) => erased.get(nodeKey(n)),
      sessionMembers: [o(10), o(11)],
      selected: [o(10)],
    }))
    expect(labels(c)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet', 'Door'])
    expect(c.map((x) => x.frameDepth)).toEqual([null, 0, 1, 2, null])
    expect(states(c)).toEqual(['above', 'above', 'above', 'above', 'selected'])
    // Frames are labeled from the captured frame label, not a scene query.
    expect(pathAbove(o(10), {
      sessionStack: [{ node: g(1), label: 'Kitchen' }, { node: g(2), label: 'Base Cabinets' }, { node: g(3), label: 'Cabinet' }],
      parentOf: (n) => erased.get(nodeKey(n)),
      sessionMembers: [o(10), o(11)],
    })).toEqual([g(1), g(2), g(3)])
  })

  it('does not duplicate a frame that the parent walk also reaches', () => {
    const c = buildSelectionPath(input({
      sessionStack: [{ node: g(1), label: 'Kitchen' }],
      selected: [g(3)],
    }))
    expect(labels(c)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet'])
  })

  it('marks a selected frame itself as the selected crumb', () => {
    const c = buildSelectionPath(input({ sessionStack: [{ node: g(1), label: 'Kitchen' }], selected: [g(1)] }))
    expect(states(c)).toEqual(['above', 'selected'])
  })

  it('a top-level object selected while a session is open sits after a divider, not inside the frame', () => {
    const c = buildSelectionPath(input({
      sessionStack: [{ node: g(3), label: 'Cabinet' }],
      sessionMembers: [o(10), o(11)],
      selected: [o(12)],
    }))
    expect(labels(c)).toEqual(['Model', 'Cabinet', '', 'Floor'])
    expect(c[2].kind).toBe('divider')
    expect(c[2].node).toBeNull()
    expect(c[2].frameDepth).toBeNull()
    expect(c[2].enterNode).toBeNull()
    expect(states(c)).toEqual(['above', 'above', 'above', 'selected'])
  })

  it('a nested object outside the open group keeps its own chain after the divider', () => {
    const c = buildSelectionPath(input({
      sessionStack: [{ node: g(4), label: 'Island' }],
      sessionMembers: [i(20)],
      selected: [o(10)],
    }))
    expect(labels(c)).toEqual(['Model', 'Island', '', 'Kitchen', 'Base Cabinets', 'Cabinet', 'Door'])
  })

  it('a selection under an outer frame drops the deeper frames that are not on its chain', () => {
    // Kitchen (g1) and Island (g4, not inside Kitchen) are open; Base
    // Cabinets is under Kitchen only, so the Island frame is dropped.
    const c = buildSelectionPath(input({
      sessionStack: [{ node: g(1), label: 'Kitchen' }, { node: g(4), label: 'Island' }],
      sessionMembers: [i(20)],
      selected: [g(2)],
    }))
    expect(labels(c)).toEqual(['Model', 'Kitchen', 'Base Cabinets'])
    expect(c.some((x) => x.kind === 'divider')).toBe(false)
    expect(c.map((x) => x.frameDepth)).toEqual([null, 0, null])
  })

  it('a multi-selection inside a session shows just Model and the frames', () => {
    const c = buildSelectionPath(input({
      sessionStack: [{ node: g(3), label: 'Cabinet' }], selected: [o(10), o(11)],
    }))
    expect(labels(c)).toEqual(['Model', 'Cabinet'])
  })

  it('a drawn selection ends with the drawing label after its owner chain', () => {
    const line: NodeRef = { kind: 'draft-edge', id: 1n, ctx: '{}', key: 'k' }
    const c = buildSelectionPath(input({
      selected: [line],
      draftOwner: () => o(10),
      labelFor: (n) => (n.kind === 'draft-edge' ? 'Line' : (NAMES.get(nodeKey(n)) ?? '?')),
    }))
    expect(labels(c)).toEqual(['Model', 'Kitchen', 'Base Cabinets', 'Cabinet', 'Door', 'Line'])
    expect(states(c)).toEqual(['above', 'above', 'above', 'above', 'above', 'selected'])
    expect(c[c.length - 1].enterNode).toBeNull()
  })

  it('a drawing on the ground or in a free sketch reads Model › Sketch N › drawing and passes the two-crumb rule', () => {
    const sketch: NodeRef = { kind: 'sketch', id: 5n }
    const line: NodeRef = { kind: 'draft-edge', id: 1n, ctx: '{}', key: 'k' }
    const c = buildSelectionPath(input({
      selected: [line],
      draftOwner: () => sketch,
      labelFor: (n) => (n.kind === 'draft-edge' ? 'Line' : (NAMES.get(nodeKey(n)) ?? '?')),
    }))
    expect(labels(c)).toEqual(['Model', 'Sketch 1', 'Line'])
  })

  it('a drawing with no resolvable owner is a lone drawing crumb and is hidden', () => {
    const line: NodeRef = { kind: 'draft-edge', id: 1n, ctx: '{}', key: 'k' }
    expect(buildSelectionPath(input({ selected: [line] }))).toEqual([])
  })

  it('wires double-click targets: groups, instances, objects enter; Model does not', () => {
    const c = buildSelectionPath(input({ selected: [g(1)], hitLeaf: { object: 10n } }))
    expect(c[0].enterNode).toBeNull()
    expect(c[1].enterNode).toEqual(g(1))
    expect(nodeEq(c[4].enterNode as NodeRef, o(10))).toBe(true)
  })
})

describe('ancestorsOf', () => {
  it('walks outermost first and reports no frames at top level', () => {
    const r = ancestorsOf(o(10), (n) => PARENT.get(nodeKey(n)), 0, null)
    expect(r.ancestors).toEqual([g(1), g(2), g(3)])
    expect(r.underFrames).toBe(false)
  })
})

describe('hoverLine', () => {
  const parentOf = (n: NodeRef) => PARENT.get(nodeKey(n))
  const labelFor = (n: NodeRef) => NAMES.get(nodeKey(n)) ?? '?'
  const memberName = (id: bigint) => (id === 30n ? 'Door Panel' : undefined)

  it('names the leaf and its direct parent', () => {
    expect(hoverLine({ object: 10n }, parentOf, labelFor, memberName)).toBe('Door · in Cabinet')
  })
  it('names a top-level object bare', () => {
    expect(hoverLine({ object: 12n }, parentOf, labelFor, memberName)).toBe('Floor')
  })
  it('reads a definition member as "<member> · in <instance>"', () => {
    expect(hoverLine({ object: 30n, instance: 20n }, parentOf, labelFor, memberName)).toBe('Door Panel · in Shaker Door')
  })
  it('inside an open component the part reads by its own label, not Part', () => {
    const lf = (n: NodeRef) => (n.kind === 'object' && n.id === 31n ? 'Handle' : (NAMES.get(nodeKey(n)) ?? '?'))
    expect(hoverLine({ object: 31n, instance: 20n }, parentOf, lf, memberName, (id) => id === 20n))
      .toBe('Handle · in Shaker Door')
    // Not open: the closed-definition fallback stays.
    expect(hoverLine({ object: 31n, instance: 20n }, parentOf, lf, memberName, () => false))
      .toBe('Part · in Shaker Door')
  })
  it('is null when the snap names no object', () => {
    expect(hoverLine({}, parentOf, labelFor, memberName)).toBeNull()
  })
  it('an instance alone (a line drawn inside it) reads as the instance in its parent', () => {
    expect(hoverLine({ instance: 20n }, parentOf, labelFor, memberName)).toBe('Shaker Door · in Island')
  })
})

describe('frameClickAction', () => {
  it('is a no-op on the innermost frame and closes down to any outer one', () => {
    expect(frameClickAction(2, 3, 0)).toBe('noop')
    expect(frameClickAction(0, 3, 0)).toBe('close-to')
    expect(frameClickAction(2, 2, 1)).toBe('noop')
    expect(frameClickAction(1, 2, 1)).toBe('close-to')
    expect(frameClickAction(0, 2, 1)).toBe('close-to')
  })
})
