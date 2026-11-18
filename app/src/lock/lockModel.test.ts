import { describe, expect, it } from 'vitest'
import { containerFreezeCause, copyAllowed, type LockScene } from './lockModel'
import { gateCopyOperands } from './lockGate'

/** A scene where object 1 is locked and sits in group 9, itself locked. */
function scene(over: Partial<Record<string, unknown>> = {}): LockScene {
  return {
    node_locked: (kind: number, id: bigint) => (kind === 0 && id === 1n) || (kind === 1 && id === 9n),
    node_frozen: (kind: number, id: bigint) =>
      (kind === 0 && id === 1n) || (kind === 1 && id === 9n) ? { kind: kind === 1 ? 'group' : 'object', id } : undefined,
    node_parent: (kind: number, id: bigint) => (kind === 0 && id === 1n ? 9n : undefined),
    owner_definition: () => undefined,
    instances_of: () => [],
    group_members: () => [],
    instance_def: () => undefined,
    definition_usage: () => 1,
    group_ids: () => [],
    instance_ids: () => [],
    object_ids: () => [],
    ...over,
  } as unknown as LockScene
}

describe('copyAllowed with a lock above', () => {
  it('refuses a locked box in a locked group and names the group', () => {
    const s = scene()
    expect(containerFreezeCause(s, { kind: 'object', id: 1n })).toEqual({ kind: 'group', id: 9n })
    expect(copyAllowed(s, { kind: 'object', id: 1n })).toBe(false)
    const toasts: Array<[string, string | undefined]> = []
    expect(gateCopyOperands(s, [{ kind: 'object', id: 1n }], (m, c) => toasts.push([m, c]))).toBeNull()
    expect(toasts).toEqual([['Locked. Unlock it to change it.', 'LockedHint:group:9']])
  })

  it('allows a locked box whose group is free', () => {
    const s = scene({
      node_locked: (kind: number, id: bigint) => kind === 0 && id === 1n,
      node_frozen: (kind: number, id: bigint) => (kind === 0 && id === 1n ? { kind: 'object', id } : undefined),
    })
    expect(copyAllowed(s, { kind: 'object', id: 1n })).toBe(true)
  })

  it('refuses a locked node at the top of a definition whose placement is frozen', () => {
    const s = scene({
      node_parent: () => undefined,
      owner_definition: () => 5n,
      instances_of: () => [7n],
      node_frozen: (kind: number, id: bigint) =>
        (kind === 0 && id === 1n) || (kind === 2 && id === 7n) ? { kind: kind === 2 ? 'instance' : 'object', id } : undefined,
    })
    expect(containerFreezeCause(s, { kind: 'object', id: 1n })).toEqual({ kind: 'instance', id: 7n })
    expect(copyAllowed(s, { kind: 'object', id: 1n })).toBe(false)
  })

  it('has no container cause for a world-level node', () => {
    const s = scene({ node_parent: () => undefined })
    expect(containerFreezeCause(s, { kind: 'object', id: 1n })).toBeNull()
    expect(copyAllowed(s, { kind: 'object', id: 1n })).toBe(true)
  })
})
