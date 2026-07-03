import { describe, it, expect } from 'vitest'
import { deriveDockContext, dockVerbsFor, dockChipLabel } from './dockLogic'
import type { NodeRef } from './treeModel'

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const group = (id: bigint): NodeRef => ({ kind: 'group', id })
const instance = (id: bigint): NodeRef => ({ kind: 'instance', id })
const sketch = (id: bigint): NodeRef => ({ kind: 'sketch', id })

describe('deriveDockContext', () => {
  it('is "empty" when nothing is selected and no guide is selected', () => {
    expect(deriveDockContext([], null)).toBe('empty')
  })

  it('is null when a guide is selected, regardless of node selection', () => {
    expect(deriveDockContext([], 5n)).toBeNull()
    expect(deriveDockContext([obj(1n)], 5n)).toBeNull()
  })

  it('is "object" for a single selected Object', () => {
    expect(deriveDockContext([obj(1n)], null)).toBe('object')
  })

  it('is "group" for a single selected Group', () => {
    expect(deriveDockContext([group(1n)], null)).toBe('group')
  })

  it('is "instance" for a single selected Instance', () => {
    expect(deriveDockContext([instance(1n)], null)).toBe('instance')
  })

  it('is null for a single selected free-standing sketch (no verb set defined)', () => {
    expect(deriveDockContext([sketch(1n)], null)).toBeNull()
  })

  it('is "multi" for more than one selected node, regardless of kind mix', () => {
    expect(deriveDockContext([obj(1n), obj(2n)], null)).toBe('multi')
    expect(deriveDockContext([obj(1n), group(2n), instance(3n)], null)).toBe('multi')
  })
})

describe('dockVerbsFor', () => {
  it('empty: primary Rectangle, then Line, Circle', () => {
    const verbs = dockVerbsFor('empty')
    expect(verbs.map((v) => v.id)).toEqual(['tool-rectangle', 'tool-line', 'tool-circle'])
  })

  it('object: primary Push/Pull, then Move, Paint, Erase (spec Face row)', () => {
    const verbs = dockVerbsFor('object')
    expect(verbs.map((v) => v.id)).toEqual(['tool-pushpull', 'tool-move', 'tool-paint', 'edit-delete'])
  })

  it('group: primary Edit, then Move, Scale, Ungroup, Erase (spec Component/Group row)', () => {
    const verbs = dockVerbsFor('group')
    expect(verbs.map((v) => v.id)).toEqual(['enter-context', 'tool-move', 'tool-scale', 'ungroup', 'edit-delete'])
  })

  it('instance: primary Edit, then Move, Scale, Make Unique, Explode (spec Component row)', () => {
    const verbs = dockVerbsFor('instance')
    expect(verbs.map((v) => v.id)).toEqual(['enter-context', 'tool-move', 'tool-scale', 'make-unique', 'explode-instance'])
  })

  it('multi: primary Move, then Erase', () => {
    const verbs = dockVerbsFor('multi')
    expect(verbs.map((v) => v.id)).toEqual(['tool-move', 'edit-delete'])
  })

  it('every context stays within the spec cap ("4-6 items max, curated")', () => {
    for (const ctx of ['empty', 'object', 'group', 'instance', 'multi'] as const) {
      expect(dockVerbsFor(ctx).length).toBeLessThanOrEqual(6)
      expect(dockVerbsFor(ctx).length).toBeGreaterThan(0)
    }
  })
})

describe('dockChipLabel', () => {
  it('has a label for every context', () => {
    for (const ctx of ['empty', 'object', 'group', 'instance', 'multi'] as const) {
      expect(dockChipLabel(ctx).length).toBeGreaterThan(0)
    }
  })
})
