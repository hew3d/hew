import { describe, it, expect } from 'vitest'
import { deriveDockContext, dockVerbsFor, dockChipLabel, isDockVerbEnabled } from './dockLogic'
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

  // Contract change ("sketches are first-class interactable"): a
  // selected sketch used to return null (no dock at all) — it now gets a
  // dedicated 'sketch' context with its own curated verb set, since sketches
  // gained real Push/Pull/Move/Rotate/Scale/Erase behavior.
  it('is "sketch" for a single selected free-standing sketch', () => {
    expect(deriveDockContext([sketch(1n)], null)).toBe('sketch')
  })

  it('is "multi" for more than one selected node, regardless of kind mix', () => {
    expect(deriveDockContext([obj(1n), obj(2n)], null)).toBe('multi')
    expect(deriveDockContext([obj(1n), group(2n), instance(3n)], null)).toBe('multi')
  })

  it('is "multi" for a sketch mixed into a multi-selection (multi rules unchanged)', () => {
    expect(deriveDockContext([obj(1n), sketch(2n)], null)).toBe('multi')
  })
})

describe('dockVerbsFor', () => {
  it('empty: primary Rectangle, then Line, Circle, Arc', () => {
    const verbs = dockVerbsFor('empty')
    expect(verbs.map((v) => v.id)).toEqual(['tool-rectangle', 'tool-line', 'tool-circle', 'tool-arc'])
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

  it('sketch: primary Push/Pull, then Move, Rotate, Scale, Erase', () => {
    const verbs = dockVerbsFor('sketch')
    expect(verbs.map((v) => v.id)).toEqual([
      'tool-pushpull', 'tool-move', 'tool-rotate', 'tool-scale', 'edit-delete',
    ])
  })

  it('sketch verbs reuse the exact same ids as the other rows (menuActionRef/activeToolId compatibility)', () => {
    const sketchIds = new Set(dockVerbsFor('sketch').map((v) => v.id))
    const objectIds = new Set(dockVerbsFor('object').map((v) => v.id))
    const groupIds = new Set(dockVerbsFor('group').map((v) => v.id))
    // tool-pushpull and edit-delete already exist on the Object row; tool-move
    // and tool-scale already exist on the Group/Instance rows — no new ids
    // were invented for the sketch context.
    expect(sketchIds.has('tool-pushpull')).toBe(true)
    expect(objectIds.has('tool-pushpull')).toBe(true)
    expect(sketchIds.has('edit-delete')).toBe(true)
    expect(objectIds.has('edit-delete')).toBe(true)
    expect(sketchIds.has('tool-scale')).toBe(true)
    expect(groupIds.has('tool-scale')).toBe(true)
  })

  it('every context stays within the spec cap ("4-6 items max, curated")', () => {
    for (const ctx of ['empty', 'object', 'group', 'instance', 'multi', 'sketch'] as const) {
      expect(dockVerbsFor(ctx).length).toBeLessThanOrEqual(6)
      expect(dockVerbsFor(ctx).length).toBeGreaterThan(0)
    }
  })
})

describe('isDockVerbEnabled', () => {
  it('hover-preview (sketch not selected): only Push/Pull is enabled', () => {
    const enabled = dockVerbsFor('sketch').filter((v) => isDockVerbEnabled(v, true))
    expect(enabled.map((v) => v.id)).toEqual(['tool-pushpull'])
  })

  it('with a real selection every verb is enabled, in every context', () => {
    for (const ctx of ['empty', 'object', 'group', 'instance', 'multi', 'sketch'] as const) {
      for (const verb of dockVerbsFor(ctx)) {
        expect(isDockVerbEnabled(verb, false)).toBe(true)
      }
    }
  })
})

describe('dockChipLabel', () => {
  it('has a label for every context', () => {
    for (const ctx of ['empty', 'object', 'group', 'instance', 'multi', 'sketch'] as const) {
      expect(dockChipLabel(ctx).length).toBeGreaterThan(0)
    }
  })

  it('reads "SKETCH" for the sketch context', () => {
    expect(dockChipLabel('sketch')).toBe('SKETCH')
  })
})
