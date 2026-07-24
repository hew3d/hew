/**
 * Direct unit tests for the shared `EditContext` machinery in types.ts:
 * `editContextEq` (the abort-vs-no-op decision every `setEditContext`
 * implementation is supposed to make) and `toolHasArmedGesture` (the
 * Viewport's Escape-routing decision, component-edit-parity.md phase A2).
 * Neither had a direct test before — every prior exercise of `editContextEq`
 * was indirect, through a specific tool's `setEditContext` unit tests.
 */
import { describe, it, expect } from 'vitest'
import { editContextEq, toolHasArmedGesture, type EditContext, type Tool } from './types'

describe('editContextEq', () => {
  it('top === top regardless of any other field', () => {
    expect(editContextEq({ kind: 'top' }, { kind: 'top' })).toBe(true)
  })

  it('different kinds are never equal', () => {
    const object: EditContext = { kind: 'object', id: 1n }
    const group: EditContext = { kind: 'group', id: 1n }
    const instance: EditContext = { kind: 'instance', id: 1n, component: 2n }
    expect(editContextEq(object, group)).toBe(false)
    expect(editContextEq(object, { kind: 'top' })).toBe(false)
    expect(editContextEq(group, instance)).toBe(false)
  })

  it('object/group contexts compare by id', () => {
    expect(editContextEq({ kind: 'object', id: 5n }, { kind: 'object', id: 5n })).toBe(true)
    expect(editContextEq({ kind: 'object', id: 5n }, { kind: 'object', id: 6n })).toBe(false)
    expect(editContextEq({ kind: 'group', id: 5n }, { kind: 'group', id: 5n })).toBe(true)
    expect(editContextEq({ kind: 'group', id: 5n }, { kind: 'group', id: 6n })).toBe(false)
  })

  it('instance contexts compare by BOTH id and component — same instance handle but a stale/different definition is a genuine change', () => {
    const a: EditContext = { kind: 'instance', id: 10n, component: 100n }
    const same: EditContext = { kind: 'instance', id: 10n, component: 100n }
    const differentComponent: EditContext = { kind: 'instance', id: 10n, component: 200n }
    const differentInstance: EditContext = { kind: 'instance', id: 20n, component: 100n }
    expect(editContextEq(a, same)).toBe(true)
    expect(editContextEq(a, differentComponent)).toBe(false)
    expect(editContextEq(a, differentInstance)).toBe(false)
  })
})

describe('toolHasArmedGesture', () => {
  function toolWithCapturingInput(capturing: boolean): Tool {
    return {
      onPointerMove: () => { /* no-op */ },
      onPointerDown: () => { /* no-op */ },
      onKey: () => { /* no-op */ },
      cancel: () => { /* no-op */ },
      name: 'Fake',
      capturingInput: () => capturing,
    }
  }

  function toolWithoutCapturingInput(): Tool {
    return {
      onPointerMove: () => { /* no-op */ },
      onPointerDown: () => { /* no-op */ },
      onKey: () => { /* no-op */ },
      cancel: () => { /* no-op */ },
      name: 'Fake',
    }
  }

  it('a tool reporting capturingInput() === true is armed', () => {
    expect(toolHasArmedGesture(toolWithCapturingInput(true))).toBe(true)
  })

  it('a tool reporting capturingInput() === false is not armed', () => {
    expect(toolHasArmedGesture(toolWithCapturingInput(false))).toBe(false)
  })

  it('a tool that does not implement capturingInput at all is never armed (e.g. Select, TapeMeasure)', () => {
    expect(toolHasArmedGesture(toolWithoutCapturingInput())).toBe(false)
  })

  // SliceTool: capturingInput() is unconditionally true (its VCB offset is
  // always live), which would make Escape never reach a context pop while
  // Slice is active if it were used as the armed signal. hasArmedGesture,
  // when present, wins over capturingInput for exactly this reason.
  function toolWithBoth(capturing: boolean, armed: boolean): Tool {
    return {
      onPointerMove: () => { /* no-op */ },
      onPointerDown: () => { /* no-op */ },
      onKey: () => { /* no-op */ },
      cancel: () => { /* no-op */ },
      name: 'Fake',
      capturingInput: () => capturing,
      hasArmedGesture: () => armed,
    }
  }

  it('hasArmedGesture, when implemented, wins over an unconditionally-true capturingInput (the SliceTool case)', () => {
    expect(toolHasArmedGesture(toolWithBoth(true, false))).toBe(false)
    expect(toolHasArmedGesture(toolWithBoth(true, true))).toBe(true)
  })
})
