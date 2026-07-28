/**
 * Auto-explode-then-boolean, exercised through the REAL compiled wasm
 * module (playtest finding 4: the boolean flow refuses a component
 * instance operand outright — `BooleanOperandHasInstance`, since a
 * boolean consumes its operand and an instance's geometry is shared — so
 * the app transparently makes the instance unique, explodes it to plain
 * solids, and retries; see `Viewport.tsx`'s `runBoolean`/
 * `explodeInstanceOperand`). This proves the underlying KERNEL sequence
 * those two functions compose is sound: no new kernel surface, only
 * `make_unique` + `explode_instance` + (for a multi-solid definition)
 * `group_nodes`, then the boolean itself — and that undoing every step
 * restores the original instance exactly (rule 9). Mirrors
 * `text/placeText.wasm.test.ts`'s real-wasm-in-vitest pattern.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, describe, expect, it } from 'vitest'
import init, { Scene } from '../wasm/pkg/wasm_api.js'
import { runBooleanCore } from './Viewport'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmPath = resolve(__dirname, '../wasm/pkg/wasm_api_bg.wasm')
  const wasmBytes = await readFile(wasmPath)
  await init({ module_or_path: wasmBytes })
})

const UNION = 0

/** Draws an axis-aligned square [x0,x1] x [y0,y1] on the ground plane and
 *  extrudes it `height` up into a new watertight box Object. Returns the
 *  object handle. */
function makeBox(scene: Scene, x0: number, x1: number, y0: number, y1: number, height: number): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, 0, 0, 0, 1)
  const corners: Array<[number, number]> = [
    [x0, y0],
    [x1, y0],
    [x1, y1],
    [x0, y1],
  ]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = corners[i]
    const [bx, by] = corners[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, 0, bx, by, 0)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  expect(regions.length).toBe(1)
  return scene.extrude_region(sketch, regions[0], height)
}

describe('boolean auto-explode through the real wasm module', () => {
  it('a single-solid component instance operand: refused directly, then make_unique + explode + boolean succeeds, and undoing all three restores the instance exactly', () => {
    const scene = new Scene()

    const glyphBox = makeBox(scene, -1, 1, -1, 1, 1)
    // make_component returns the INSTANCE handle directly (the definition
    // is reachable via instance_def).
    const instance = scene.make_component(new Uint8Array([0]), new BigUint64Array([glyphBox]))
    const originalDef = scene.instance_def(instance)
    expect(originalDef).toBeDefined()

    const otherBox = makeBox(scene, -0.5, 0.5, -0.5, 0.5, 1)

    // Direct attempt: refused typed, nothing touched.
    expect(() => scene.boolean_nodes(UNION, 2, instance, 0, otherBox)).toThrow(
      /BooleanOperandHasInstance/,
    )
    expect(Array.from(scene.instance_ids())).toContain(instance)

    // Auto-explode sequence: make_unique, then explode (one solid ->
    // one plain object), then the boolean against the exploded solid.
    const uniqueDef = scene.make_unique(instance)
    expect(uniqueDef).not.toBe(originalDef)
    const exploded = Array.from(scene.explode_instance(instance))
    expect(exploded.length).toBe(1)
    expect(Array.from(scene.instance_ids())).not.toContain(instance)

    const result = scene.boolean_nodes(UNION, 0, exploded[0], 0, otherBox)
    expect(result.kind).toBe('object')
    expect(scene.object_watertight(result.id)).toBe(true)

    // Undo all three steps (boolean, explode, make_unique) and confirm the
    // ORIGINAL instance comes back exactly — same handle, same shared
    // definition it pointed to before make_unique, not the private one.
    expect(scene.can_scene_undo()).toBe(true)
    scene.scene_undo() // reverses boolean_nodes
    scene.scene_undo() // reverses explode_instance
    scene.scene_undo() // reverses make_unique
    expect(Array.from(scene.instance_ids())).toContain(instance)
    expect(scene.instance_def(instance)).toBe(originalDef)
    expect(Array.from(scene.object_ids())).toContain(otherBox)
  })

  it('a multi-solid exploded set (the 3D Text shape: several disjoint solids under one instance) groups the exploded pieces and booleans the composite — matching the existing multi-select semantics, which already require grouping before a boolean of more than two solids', () => {
    const scene = new Scene()

    const glyphA = makeBox(scene, -3, -1, -1, 1, 1)
    const glyphB = makeBox(scene, 1, 3, -1, 1, 1)
    const instance = scene.make_component(
      new Uint8Array([0, 0]),
      new BigUint64Array([glyphA, glyphB]),
    )
    const originalDef = scene.instance_def(instance)

    // Bridges and overlaps both exploded pieces, so unioning the group
    // against it yields one connected solid.
    const bridge = makeBox(scene, -4, 4, -1, 1, 1)

    expect(() => scene.boolean_nodes(UNION, 2, instance, 0, bridge)).toThrow(
      /BooleanOperandHasInstance/,
    )

    scene.make_unique(instance)
    const exploded = Array.from(scene.explode_instance(instance))
    expect(exploded.length).toBe(2)

    // >1 exploded solid: existing multi-select boolean semantics
    // (`canBoolean`/`boolean_nodes`) require exactly two top-level
    // operands, so combining more than one piece against the other
    // operand means grouping them first — the same thing a user would
    // have to do by hand for any other >2-solid boolean. group_nodes
    // internally unions a group operand's leaves before applying `op`
    // between the two composites (Document::compose_operand_union).
    const groupId = scene.group_nodes(
      new Uint8Array([0, 0]),
      new BigUint64Array(exploded),
    )
    const result = scene.boolean_nodes(UNION, 1, groupId, 0, bridge)
    expect(result.kind).toBe('object')
    expect(scene.object_watertight(result.id)).toBe(true)

    // Four undo steps this time (boolean, group, explode, make_unique) —
    // reported honestly rather than forced into one step (no sanctioned
    // Compound covers this sequence; DocAction::Compound is scoped to
    // place_text alone). Undoing all four restores the original instance.
    scene.scene_undo() // reverses boolean_nodes
    scene.scene_undo() // reverses group_nodes
    scene.scene_undo() // reverses explode_instance
    scene.scene_undo() // reverses make_unique
    expect(Array.from(scene.instance_ids())).toContain(instance)
    expect(scene.instance_def(instance)).toBe(originalDef)
    expect(Array.from(scene.object_ids())).toContain(bridge)
  })

  it('a RETRIED boolean that still fails after auto-explode leaves the exploded piece committed, not the pre-explode instance — undoing both committed steps restores it exactly (finding 2: the failure-after-mutation case)', async () => {
    const scene = new Scene()

    const glyphBox = makeBox(scene, -1, 1, -1, 1, 1)
    const instance = scene.make_component(new Uint8Array([0]), new BigUint64Array([glyphBox]))
    const originalDef = scene.instance_def(instance)

    // A real second operand that passes the UI's `canBoolean` gate
    // (treeModel.ts checks kind + top-level-ness only, never solidity) but
    // fails the kernel's own typed check: import the reviewer-mandated
    // cavity-gate STL fixture (crates/stl-import/tests/fixtures) — a
    // closed outer shell with a genuinely OPEN shell nested inside,
    // imported as two separate Objects (`import_stl` never fabricates a
    // fix for a leaky import). One of the two is honestly non-watertight.
    const stlPath = resolve(
      __dirname,
      '../../../crates/stl-import/tests/fixtures/open_nested_in_solid.stl',
    )
    const stlBytes = await readFile(stlPath)
    scene.import_stl(stlBytes, 0.001, 'shells')
    const imported = Array.from(scene.object_ids())
    expect(imported.length).toBe(2)
    const leaky = imported.find((id) => !scene.object_watertight(id))
    expect(leaky).toBeDefined()

    // Direct attempt: refused typed on the instance operand, nothing
    // committed yet.
    expect(() => scene.boolean_nodes(UNION, 2, instance, 0, leaky!)).toThrow(
      /BooleanOperandHasInstance/,
    )

    // Auto-explode sequence: this part commits for real.
    scene.make_unique(instance)
    const exploded = Array.from(scene.explode_instance(instance))
    expect(exploded.length).toBe(1)
    expect(Array.from(scene.instance_ids())).not.toContain(instance)

    // The retry: the exploded solid is fine, but the leaky import as the
    // OTHER operand refuses typed — AFTER the explode already committed.
    // This is finding 2's exact failure-after-mutation shape: `runBoolean`
    // must not treat this like a clean no-op refusal.
    expect(() => scene.boolean_nodes(UNION, 0, exploded[0], 0, leaky!)).toThrow(
      /BooleanOperandNotSolid/,
    )

    // The document now genuinely holds the exploded piece, not the
    // pre-explode instance — never silently reverted on the failed retry,
    // exactly what the app's "undo N times to restore it" recovery toast
    // promises (finding 2: the count is exact, not a vague "undo").
    expect(Array.from(scene.instance_ids())).not.toContain(instance)
    expect(Array.from(scene.object_ids())).toContain(exploded[0])

    // Exactly two committed steps (explode_instance, make_unique) — no
    // partial/wedged state from the failed retry, since a refused
    // `boolean_nodes` call validates entirely on clones before touching
    // the document (DEVELOPMENT.md rule 9). Undoing both restores the
    // ORIGINAL instance exactly: same handle, same shared definition.
    expect(scene.can_scene_undo()).toBe(true)
    scene.scene_undo() // reverses explode_instance
    scene.scene_undo() // reverses make_unique
    expect(Array.from(scene.instance_ids())).toContain(instance)
    expect(scene.instance_def(instance)).toBe(originalDef)
    // The (untouched) import survives throughout, watertight states
    // unchanged by the failed retry.
    expect(Array.from(scene.object_ids()).sort()).toEqual([...imported].sort())
    expect(scene.object_watertight(leaky!)).toBe(false)
  })
})

describe('runBooleanCore — the mutated-failed accounting itself (delta-review finding 1)', () => {
  // These exercise `Viewport.tsx`'s actual decision function (not just the
  // kernel sequence it composes, which the suite above already proves sound)
  // against a real compiled `Scene`, per `pickResolution.test.ts`'s pattern
  // of importing pure/near-pure helpers straight out of Viewport.tsx without
  // mounting the (WebGL-backed) component.
  const UNION = 0

  it('a MIRRORED instance operand (legal via import/load — place_instance accepts a reflected affine): make_unique commits, then explode_instance refuses CannotExplodeReflected — reported as mutated-failed with exactly one committed step, not a bare no-op', () => {
    const scene = new Scene()

    const glyphBox = makeBox(scene, -1, 1, -1, 1, 1)
    const instance = scene.make_component(new Uint8Array([0]), new BigUint64Array([glyphBox]))
    const def = scene.instance_def(instance)

    // A second instance of the SAME definition, placed with a
    // determinant-negative (mirrored) pose — row-major 3x4, X axis flipped.
    const mirrored = scene.place_instance(
      def!,
      new Float64Array([-1, 0, 0, 6, 0, 1, 0, 0, 0, 0, 1, 0]),
    )
    const otherBox = makeBox(scene, 8, 10, -1, 1, 1)

    const outcome = runBooleanCore(
      scene,
      UNION,
      { kind: 'instance', id: mirrored },
      { kind: 'object', id: otherBox },
    )

    expect(outcome.kind).toBe('mutated-failed')
    if (outcome.kind !== 'mutated-failed') throw new Error('unreachable')
    // Exactly one step: make_unique committed, explode_instance refused
    // before touching the document (DEVELOPMENT.md rule 9) — this is the
    // ENTIRE point of finding 1: that single committed step must never be
    // reported as a no-op.
    expect(outcome.committedSteps).toBe(1)
    expect(outcome.retryFailed).toBe(false)
    expect(outcome.code).toBe('CannotExplodeReflected')
    // The instance is still what's rendered in its place (now on a private,
    // no-longer-shared definition) — exactly what a "docRev" refresh must
    // pick back up, and exactly what `onSelectMany` gets pushed.
    expect(outcome.settledNodes).toEqual([{ kind: 'instance', id: mirrored }])

    // The document genuinely changed — the caller's docRev-equivalent
    // "treat this as a committed mutation" signal is `kind === 'mutated-failed'`
    // together with `committedSteps > 0`, both true here.
    expect(scene.instance_def(mirrored)).not.toBe(def)
    expect(Array.from(scene.instance_ids())).toContain(mirrored)

    // Undoing exactly the reported step count restores the original shared
    // definition on the exact same instance handle (earlier undo history
    // from the setup above — make_component, place_instance — still sits
    // below these, so `can_scene_undo` staying true afterward is expected).
    expect(scene.can_scene_undo()).toBe(true)
    for (let i = 0; i < outcome.committedSteps; i++) scene.scene_undo() // reverses make_unique
    expect(Array.from(scene.instance_ids())).toContain(mirrored)
    expect(scene.instance_def(mirrored)).toBe(def)
  })

  it('both operands are instances: the first fully explodes (committed for real), then the second (mirrored) fails — mutated-failed accumulates BOTH operands\' committed steps, keeps the first operand\'s exploded piece in settledNodes, and undoing every reported step restores both original instances exactly', () => {
    const scene = new Scene()

    const boxA = makeBox(scene, -3, -1, -1, 1, 1)
    const instanceA = scene.make_component(new Uint8Array([0]), new BigUint64Array([boxA]))
    const defA = scene.instance_def(instanceA)

    const boxB = makeBox(scene, 1, 3, -1, 1, 1)
    const instanceB = scene.make_component(new Uint8Array([0]), new BigUint64Array([boxB]))
    const defB = scene.instance_def(instanceB)
    // A mirrored second instance of B's definition — this is the operand
    // that will fail auto-explode, AFTER instanceA has already fully
    // exploded for real (delta-review finding 1c: the worst case).
    const mirroredB = scene.place_instance(
      defB!,
      new Float64Array([-1, 0, 0, 10, 0, 1, 0, 0, 0, 0, 1, 0]),
    )

    const outcome = runBooleanCore(
      scene,
      UNION,
      { kind: 'instance', id: instanceA },
      { kind: 'instance', id: mirroredB },
    )

    expect(outcome.kind).toBe('mutated-failed')
    if (outcome.kind !== 'mutated-failed') throw new Error('unreachable')
    // instanceA: make_unique + explode_instance (2) fully committed and
    // succeeded; mirroredB: make_unique alone (1) committed before
    // explode_instance refused. Total = 3, not "nothing happened".
    expect(outcome.committedSteps).toBe(3)
    expect(outcome.retryFailed).toBe(false)
    expect(outcome.otherOperandExploded).toBe(true)
    expect(outcome.code).toBe('CannotExplodeReflected')
    // Both settled nodes are present: instanceA's exploded plain object AND
    // mirroredB's own (now-unique) instance ref — nothing here is silently
    // dropped, so the caller can select the full, honest post-failure state.
    expect(outcome.settledNodes).toHaveLength(2)
    expect(outcome.settledNodes).toContainEqual({ kind: 'instance', id: mirroredB })
    expect(outcome.settledNodes.some((n) => n.kind === 'object')).toBe(true)

    // The document genuinely holds instanceA's exploded piece, not
    // instanceA itself, while mirroredB survives as an instance (now
    // uniquely defined) — never silently reverted.
    expect(Array.from(scene.instance_ids())).not.toContain(instanceA)
    expect(Array.from(scene.instance_ids())).toContain(mirroredB)
    expect(scene.instance_def(mirroredB)).not.toBe(defB)

    // Undoing every one of the reported committed steps restores BOTH
    // original instances exactly, on their original shared definitions
    // (earlier setup history still sits below these on the undo stack).
    for (let i = 0; i < outcome.committedSteps; i++) scene.scene_undo()
    expect(Array.from(scene.instance_ids())).toContain(instanceA)
    expect(scene.instance_def(instanceA)).toBe(defA)
    expect(Array.from(scene.instance_ids())).toContain(mirroredB)
    expect(scene.instance_def(mirroredB)).toBe(defB)
  })
})
