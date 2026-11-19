/**
 * Lock (OW-1) in the app, against the REAL compiled kernel (the
 * `makeUnique.wasm.test.ts` pattern): the reads, the copy a person sees, the
 * toast's buttons, and the selection split that leaves locked items behind.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import type { NodeRef } from '../panels/treeModel'
import { anyLocked, gateSelection, isLocked, lockBlockFor, lockedMessage, leftAloneMessage, leftBehindMessage, lockToastSteps, parseLockedHintCode, lockedHintCode, splitLocked, frozenCause, copyAllowed, containerFreezeCause } from './lockModel'
import { gateCopyOperands, leaveBehind, refuseIfLocked } from './lockGate'
import { lockedAmong, setNodesLocked, unlockAll, unlockedAmong } from './lockActions'
import { commitSelectionTransform } from '../tools/transformSelection'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  await init({ module_or_path: await readFile(resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')) })
})

function makeBox(scene: Scene, x0: number, x1: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const corners: Array<[number, number]> = [[x0, 0], [x1, 0], [x1, 1], [x0, 1]]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = corners[i]
    const [bx, by] = corners[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], 1)
}

const at = (x: number) => new Float64Array([1, 0, 0, x, 0, 1, 0, 0, 0, 0, 1, 0])
const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const inst = (id: bigint): NodeRef => ({ kind: 'instance', id })
const grp = (id: bigint): NodeRef => ({ kind: 'group', id })

/** One box made a component, placed `n` times in all (the first included). */
function copies(scene: Scene, n: number): bigint[] {
  const first = scene.make_component(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 4)]))
  const def = scene.instance_def(first)!
  const out = [first]
  for (let i = 1; i < n; i++) out.push(scene.place_instance(def, at(6 * i)))
  return out
}

describe('a locked standalone box', () => {
  it('says the plain line and offers only Unlock', () => {
    const scene = new Scene()
    const box = makeBox(scene, 0, 2)
    expect(lockBlockFor(scene, obj(box))).toBeNull()
    setNodesLocked(scene, [obj(box)], true)
    const block = lockBlockFor(scene, obj(box))
    expect(block).toMatchObject({ copies: 0, indirect: false })
    expect(lockedMessage(block!)).toBe('Locked')
    expect(lockToastSteps(scene, lockedHintCode(block!.cause!), [])).toEqual([{ action: 'unlock', node: obj(box) }])
  })

  it('undo goes back through the lock', () => {
    const scene = new Scene()
    const box = makeBox(scene, 0, 2)
    setNodesLocked(scene, [obj(box)], true)
    expect(isLocked(scene, obj(box))).toBe(true)
    scene.scene_undo()
    expect(isLocked(scene, obj(box))).toBe(false)
    scene.scene_redo()
    expect(isLocked(scene, obj(box))).toBe(true)
  })
})

describe('a locked copy of a shared shape (the lock belongs to the shape)', () => {
  it('locks every copy and says how many others share it, offering Unlock then Make Unique', () => {
    const scene = new Scene()
    const [a, b, c, d] = copies(scene, 4)
    setNodesLocked(scene, [inst(b)], true)
    for (const i of [a, b, c, d]) expect(isLocked(scene, inst(i))).toBe(true)
    const block = lockBlockFor(scene, inst(c))!
    expect(block.copies).toBe(3)
    expect(lockedMessage(block)).toBe('All 4 copies are locked')
    const steps = lockToastSteps(scene, lockedHintCode(block.cause!), [])
    expect(steps?.map((s) => s.action)).toEqual(['unlock', 'make-unique'])
  })

  it('Make Unique works on a locked copy, the new one stays locked, and unlocking it leaves the other three locked', () => {
    const scene = new Scene()
    const [a, b, c, d] = copies(scene, 4)
    setNodesLocked(scene, [inst(a)], true)
    scene.make_unique(b)
    expect(isLocked(scene, inst(b))).toBe(true)
    for (const i of [a, c, d]) expect(isLocked(scene, inst(i))).toBe(true)
    expect(lockBlockFor(scene, inst(a))!.copies).toBe(2)
    setNodesLocked(scene, [inst(b)], false)
    expect(isLocked(scene, inst(b))).toBe(false)
    for (const i of [a, c, d]) expect(isLocked(scene, inst(i))).toBe(true)
  })

  it('two copies read "Both copies are locked"', () => {
    const scene = new Scene()
    const [a] = copies(scene, 2)
    setNodesLocked(scene, [inst(a)], true)
    expect(lockedMessage(lockBlockFor(scene, inst(a))!)).toBe('Both copies are locked')
  })
})

describe('a shape also used inside a locked group (the indirect case)', () => {
  function house() {
    const scene = new Scene()
    const [window, outside] = copies(scene, 2)
    const group = scene.group_nodes(new Uint8Array([2]), new BigUint64Array([window]))
    setNodesLocked(scene, [grp(group)], true)
    return { scene, window, outside, group }
  }

  it('the outside copy is free to move but editing its shape is refused: Make Unique first, then Unlock the group', () => {
    const { scene, outside, group } = house()
    expect(lockBlockFor(scene, inst(outside))).toBeNull()
    const block = lockBlockFor(scene, inst(outside), { shape: true })!
    expect(block).toMatchObject({ indirect: true })
    expect(lockedMessage(block)).toBe('Also used in a locked group')
    const steps = lockToastSteps(scene, lockedHintCode(inst(outside), true), [])
    expect(steps).toEqual([
      { action: 'make-unique', node: inst(outside) },
      { action: 'unlock', node: grp(group) },
    ])
  })

  it("the kernel's own refusal, with no node named, takes its buttons from the selection", () => {
    const { scene, outside, group } = house()
    const steps = lockToastSteps(scene, 'LockedNodeViaSharedShape', [inst(outside)])
    expect(steps?.map((s) => [s.action, s.node.id])).toEqual([['make-unique', outside], ['unlock', group]])
  })

  it('a member of the locked group is frozen with the group as its cause', () => {
    const { scene, window, group } = house()
    expect(frozenCause(scene, inst(window))).toEqual(grp(group))
    expect(lockBlockFor(scene, inst(window))).toMatchObject({ cause: grp(group), copies: 0 })
  })
})

describe('locked items are left behind', () => {
  function mixed() {
    const scene = new Scene()
    const locked = makeBox(scene, 0, 2)
    const free = makeBox(scene, 4, 6)
    setNodesLocked(scene, [obj(locked)], true)
    return { scene, locked, free }
  }

  it('splits a selection into what runs and what is left alone', () => {
    const { scene, locked, free } = mixed()
    const split = splitLocked(scene, [obj(locked), obj(free)])
    expect(split.go).toEqual([obj(free)])
    expect(split.left).toEqual([obj(locked)])
  })

  it('runs on the rest and says so once; the kernel move then works', () => {
    const { scene, locked, free } = mixed()
    const toasts: string[] = []
    const go = leaveBehind(scene, [obj(locked), obj(free)], (m) => toasts.push(m))
    expect(go).toEqual([obj(free)])
    expect(toasts).toEqual(['1 locked item was left alone.'])
    // Select All then Move: the whole selection would be refused (the kernel stays strict) ...
    expect(() => commitSelectionTransform(scene, [obj(locked), obj(free)], at(1))).toThrow(/LockedNode/)
    // ... what the app sends is the rest.
    expect(() => commitSelectionTransform(scene, go!, at(1))).not.toThrow()
  })

  it('says the plural', () => {
    expect(leftAloneMessage(2)).toBe('2 locked items were left alone.')
  })

  it('does not call an unlocked group that merely holds something locked a locked item', () => {
    const scene = new Scene()
    const a = makeBox(scene, 0, 2)
    const b = makeBox(scene, 4, 6)
    const free = makeBox(scene, 8, 10)
    const lockedLone = makeBox(scene, 12, 14)
    const g = scene.group_nodes(new Uint8Array([0, 0]), new BigUint64Array([a, b]))
    setNodesLocked(scene, [obj(a), obj(lockedLone)], true)
    const toasts: string[] = []
    const go = leaveBehind(scene, [grp(g), obj(free)], (m) => toasts.push(m), { descendants: true })
    expect(go).toEqual([obj(free)])
    expect(toasts).toEqual(['1 group that holds something locked was left alone.'])
    // Both kinds at once say both, each truthfully.
    expect(leftBehindMessage(scene, [grp(g), obj(lockedLone)])).toBe(
      '1 locked item was left alone. 1 group that holds something locked was left alone.',
    )
    expect(leftBehindMessage(scene, [obj(lockedLone)])).toBe('1 locked item was left alone.')
  })

  it('does nothing when everything is locked, and says the Locked line with its target', () => {
    const { scene, locked } = mixed()
    const toasts: Array<[string, string | undefined]> = []
    expect(leaveBehind(scene, [obj(locked)], (m, c) => toasts.push([m, c]))).toBeNull()
    expect(toasts).toEqual([['Locked', lockedHintCode(obj(locked))]])
    expect(parseLockedHintCode(toasts[0][1])).toEqual({ node: obj(locked), indirect: false })
  })

  it('an unlocked group holding a locked member counts as left behind for Move, Delete and Group', () => {
    const scene = new Scene()
    const a = makeBox(scene, 0, 2)
    const b = makeBox(scene, 4, 6)
    const g = scene.group_nodes(new Uint8Array([0, 0]), new BigUint64Array([a, b]))
    setNodesLocked(scene, [obj(a)], true)
    expect(isLocked(scene, grp(g))).toBe(false)
    expect(splitLocked(scene, [grp(g)], { descendants: true }).left).toEqual([grp(g)])
    expect(splitLocked(scene, [grp(g)]).left).toEqual([])
    // The kernel agrees: it refuses to move the group and names the member.
    expect(() => scene.transform_selection(new Uint8Array([1]), new BigUint64Array([g]), new BigUint64Array(0), at(1))).toThrow(/LockedNode/)
    const outcome = gateSelection(scene, [grp(g)], { descendants: true })
    expect(outcome.kind).toBe('refused')
  })

  it('needs-every-operand commands are refused whole', () => {
    const { scene, locked, free } = mixed()
    const toasts: string[] = []
    expect(refuseIfLocked(scene, [obj(locked), obj(free)], (m) => toasts.push(m))).toBe(true)
    expect(toasts).toEqual(['Locked'])
    expect(refuseIfLocked(scene, [obj(free)], (m) => toasts.push(m))).toBe(false)
  })
})

describe('copying locked things', () => {
  it('a locked standalone box may be copied: the gate lets it through and the kernel copies it', () => {
    const scene = new Scene()
    const a = makeBox(scene, 0, 2)
    setNodesLocked(scene, [obj(a)], true)
    const toasts: string[] = []
    expect(gateCopyOperands(scene, [obj(a)], (m) => toasts.push(m))).toEqual([obj(a)])
    expect(toasts).toEqual([])
    const copies = scene.duplicate_selection_array(new Uint8Array([0]), new BigUint64Array([a]), at(3), 1, false)
    expect(copies).toHaveLength(1)
    // The original stays put and locked; an independent copy is free.
    expect(isLocked(scene, obj(a))).toBe(true)
  })

  it('a member of a locked group is refused for copying, naming the group', () => {
    const scene = new Scene()
    const a = makeBox(scene, 0, 2)
    const b = makeBox(scene, 4, 6)
    const g = scene.group_nodes(new Uint8Array([0, 0]), new BigUint64Array([a, b]))
    setNodesLocked(scene, [grp(g)], true)
    const toasts: Array<[string, string | undefined]> = []
    expect(gateCopyOperands(scene, [obj(a)], (m, c) => toasts.push([m, c]))).toBeNull()
    expect(toasts).toEqual([['Locked', lockedHintCode(grp(g))]])
  })
})

describe('a locked box inside a locked group', () => {
  it('is refused for copying up front, naming the group, and the document does not change', () => {
    const scene = new Scene()
    const a = makeBox(scene, 0, 2)
    const b = makeBox(scene, 4, 6)
    const g = scene.group_nodes(new Uint8Array([0, 0]), new BigUint64Array([a, b]))
    setNodesLocked(scene, [obj(a)], true)
    setNodesLocked(scene, [grp(g)], true)
    // The kernel names the box itself, since it is locked too.
    expect(frozenCause(scene, obj(a))).toEqual(obj(a))
    expect(isLocked(scene, obj(a))).toBe(true)
    expect(copyAllowed(scene, obj(a))).toBe(false)
    expect(containerFreezeCause(scene, obj(a))).toEqual(grp(g))
    const before = scene.object_ids().length
    const toasts: Array<[string, string | undefined]> = []
    expect(gateCopyOperands(scene, [obj(a)], (m, c) => toasts.push([m, c]))).toBeNull()
    expect(toasts).toEqual([['Locked', lockedHintCode(grp(g))]])
    expect(scene.object_ids().length).toBe(before)
    // The group is lifted: the box's own lock alone still lets it copy.
    setNodesLocked(scene, [grp(g)], false)
    expect(copyAllowed(scene, obj(a))).toBe(true)
  })
})

describe('lock and unlock calls', () => {
  it('Lock sends only what is not locked; Unlock All finds everything', () => {
    const scene = new Scene()
    const a = makeBox(scene, 0, 2)
    const b = makeBox(scene, 4, 6)
    setNodesLocked(scene, [obj(a)], true)
    expect(unlockedAmong(scene, [obj(a), obj(b)])).toEqual([obj(b)])
    expect(lockedAmong(scene, [obj(a), obj(b)])).toEqual([obj(a)])
    expect(anyLocked(scene)).toBe(true)
    expect(unlockAll(scene)).toBe(true)
    expect(anyLocked(scene)).toBe(false)
    expect(unlockAll(scene)).toBe(false)
  })
})
