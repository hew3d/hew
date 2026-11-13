/**
 * Linked copies through the REAL compiled wasm module: the app-side helpers
 * (Unlink, Unlink all, Make component, rename, linked Paste, booleans on a
 * placement, labels) run against a real document, the way the other
 * `.wasm.test.ts` files exercise the kernel without mounting React.
 */

import { readFile } from 'node:fs/promises'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { beforeAll, beforeEach, describe, expect, it } from 'vitest'
import init, { Scene } from './wasm/pkg/wasm_api.js'
import {
  linkedPlacementOf,
  linkGates,
  promoteToComponent,
  promoteForSave,
  undoPromotionIfUnmoved,
  blockedWhileEditing,
  renameLinkedCopy,
  unlinkAllCopies,
  unlinkCopy,
} from './linkedCopies'
import { makeLabelFor, placementLabel, type NodeRef } from './panels/treeModel'
import { canPasteLinked, pasteLinked, planPaste, resolveLinkSource } from './clipboard/linkedPaste'
import { repointLinkSource, type ClipboardContent, type LinkSource } from './clipboard/modelClipboard'
import { booleanUnlinkingPlacements, linkedPlacementPickEligible } from './viewport/Viewport'
import { routeBoolean, subtractThroughPlacement } from './viewport/placementBoolean'
import { parseLinkedResult, mapSelection } from './viewport/linkedWraps'
import { unlinkedToJoin } from './linkedCopyCopy'

const __dirname = fileURLToPath(new URL('.', import.meta.url))

beforeAll(async () => {
  const wasmBytes = await readFile(resolve(__dirname, 'wasm/pkg/wasm_api_bg.wasm'))
  await init({ module_or_path: wasmBytes })
})

/** An axis-aligned box [x0,x1] x [y0,y1] standing `height` tall on the ground. */
function makeBox(scene: Scene, x0: number, x1: number, y0: number, y1: number, height: number, z = 0): bigint {
  const sketch = scene.begin_sketch_on_plane(0, 0, z, 0, 0, 1)
  const c: Array<[number, number]> = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
  for (let i = 0; i < 4; i++) {
    const [ax, ay] = c[i]
    const [bx, by] = c[(i + 1) % 4]
    scene.sketch_add_segment(sketch, ax, ay, z, bx, by, z)
  }
  const regions = Array.from(scene.sketch_regions(sketch))
  return scene.extrude_region(sketch, regions[0], height)
}

const shift = (dx: number): Float64Array => new Float64Array([1, 0, 0, dx, 0, 1, 0, 0, 0, 0, 1, 0])

/** A named Leg copied three times linked: four placements of one set. */
function fourLegs(scene: Scene): { leg: bigint; placements: bigint[] } {
  const leg = makeBox(scene, 0, 1, 0, 1, 3)
  scene.set_node_name(0, leg, 'Leg')
  const result = parseLinkedResult(
    scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([leg]), shift(2), 3),
  )
  const first = result.linked[0].instance
  return { leg, placements: [first, ...result.nodes.map((n) => n.id)] }
}

const triangles = (scene: Scene, object: bigint): number => scene.object_export_triangles(object, 8).length

const inst = (id: bigint): NodeRef => ({ kind: 'instance', id })
const undoLabels = (scene: Scene): string[] =>
  (JSON.parse(scene.history_entries_json()) as { undo: { label: string }[] }).undo.map((e) => e.label)

describe('a linked copy through Move or Paste is a placement of one set', () => {
  it('four copies share one definition and read as the board, with no component words', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    expect(placements).toHaveLength(4)
    const def = scene.instance_def(placements[0])!
    expect(Array.from(scene.instances_of(def)).sort()).toEqual([...placements].sort())
    expect(scene.definition_usage(def)).toBe(4)
    const labelFor = makeLabelFor(scene, new Map())
    expect(labelFor(inst(placements[2]))).toBe('Leg')
    expect(placementLabel(scene, placements[0], 0)).toBe('Leg')
  })

  it('mapSelection finds the placement that now stands where the wrapped source was', () => {
    const scene = new Scene()
    const leg = makeBox(scene, 0, 1, 0, 1, 3)
    const result = parseLinkedResult(
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([leg]), shift(2), 1),
    )
    expect(mapSelection([{ kind: 'object', id: leg }], result.linked)).toEqual([inst(result.linked[0].instance)])
  })
})

describe('Unlink, Unlink all, Make component', () => {
  it('unlink_forks_only_the_selected_placement', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const [a, b, c, d] = placements
    const defBefore = scene.instance_def(b)!
    unlinkCopy(scene, b)
    expect(scene.instance_def(b)).not.toBe(defBefore)
    expect(scene.instance_def(a)).toBe(defBefore)
    expect(scene.instance_def(c)).toBe(defBefore)
    expect(scene.instance_def(d)).toBe(defBefore)
    expect(scene.definition_usage(defBefore)).toBe(3)
    // The forked copy keeps the board's name and is a set of one: a plain board.
    expect(placementLabel(scene, b, 0)).toBe('Leg')
    expect(scene.definition_usage(scene.instance_def(b)!)).toBe(1)
    expect(undoLabels(scene).at(-1)).toBe('Unlink')
  })

  it('unlink_all_is_one_undo_entry', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const depth = scene.undo_depth()
    expect(unlinkAllCopies(scene, placements[0])).toBe(4)
    expect(scene.undo_depth()).toBe(depth + 1)
    const defs = new Set(placements.map((p) => scene.instance_def(p)))
    expect(defs.size).toBe(4)
    scene.scene_undo()
    expect(new Set(placements.map((p) => scene.instance_def(p))).size).toBe(1)
  })

  it('make_component_promotes_asks_for_a_name_and_cancel_changes_nothing', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const [a, b] = placements
    // Cancel is "never calling promote": nothing changed, the set is still linked.
    expect(scene.is_linked(a)).toBe(true)
    promoteToComponent(scene, a, 'Table leg')
    expect(scene.is_linked(a)).toBe(false)
    // The other copies follow: they are placements of the same, now named, component.
    expect(scene.is_linked(b)).toBe(false)
    expect(scene.component_name(scene.instance_def(b)!)).toBe('Table leg')
    expect(placementLabel(scene, b, 0)).toBe('Table leg')
  })

  it('make_component_is_one_undo_entry', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const depth = scene.undo_depth()
    promoteToComponent(scene, placements[0], 'Leg')
    expect(scene.undo_depth()).toBe(depth + 1)
    expect(undoLabels(scene).at(-1)).toBe('Make Component')
    scene.scene_undo()
    expect(scene.is_linked(placements[0])).toBe(true)
  })

  it('gates: one linked placement, a set of one has nothing to unlink all', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    expect(linkGates(scene, [inst(placements[0])], false)).toEqual({ canUnlink: true, canUnlinkAll: true, canPromote: true })
    expect(linkGates(scene, [inst(placements[0])], true)).toEqual({ canUnlink: false, canUnlinkAll: false, canPromote: false })
    expect(linkGates(scene, [inst(placements[0]), inst(placements[1])], false).canUnlink).toBe(false)
    expect(linkGates(scene, [{ kind: 'object', id: 1n }], false).canUnlink).toBe(false)
    unlinkCopy(scene, placements[1])
    unlinkCopy(scene, placements[2])
    unlinkCopy(scene, placements[3])
    // The first is now a set of one: still a linked placement, but alone.
    expect(linkedPlacementOf(scene, [inst(placements[0])])?.copies).toBe(1)
    expect(linkGates(scene, [inst(placements[0])], false).canUnlinkAll).toBe(false)
  })

  it('renaming_a_row_edits_the_instance_name_when_set_else_all_copies', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const [a, b] = placements
    // No name on the copy: the shared name changes, every copy follows.
    expect(renameLinkedCopy(scene, a, 'Table leg')).toBe('all')
    expect(placementLabel(scene, b, 0)).toBe('Table leg')
    // A copy with its own name: only that copy changes.
    scene.set_node_name(2, a, 'Front left')
    expect(renameLinkedCopy(scene, a, 'Front right')).toBe('copy')
    expect(scene.instance_name(a)).toBe('Front right')
    expect(placementLabel(scene, b, 0)).toBe('Table leg')
  })
})

describe('linked Paste', () => {
  const nonce = 'window-1'
  function source(scene: Scene, nodes: LinkSource['nodes']): LinkSource {
    return { documentId: scene.ensure_document_id(), nonce, generation: 0, nodes }
  }

  it('paste_into_the_same_document_wraps_a_loose_source_and_places_a_linked_copy', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const src = source(scene, [{ kind: 'object', id: board }])
    expect(canPasteLinked(scene, src, nonce, 0)).toBe(true)
    const depth = scene.undo_depth()
    // The ghost would anchor the copy 5 m along X from where the board sits.
    const anchor = Array.from(scene.extract_item_placement(new Uint8Array([0]), new BigUint64Array([board]))!)
    anchor[3] += 5
    const { result, sources } = pasteLinked(scene, src.nodes, null, Float64Array.from(anchor))
    // One undo entry for wrap + place together.
    expect(scene.undo_depth()).toBe(depth + 1)
    expect(result.rootKinds).toEqual([2])
    const pasted = result.rootIds[0]
    expect(scene.is_linked(pasted)).toBe(true)
    // The original is now the set's first placement; both share one definition.
    expect(sources).toHaveLength(1)
    expect(scene.instance_def(sources[0].id)).toBe(scene.instance_def(pasted))
    expect(scene.definition_usage(scene.instance_def(pasted)!)).toBe(2)
    // The paste landed 5 m along X from the copy.
    const pose = scene.instance_pose(pasted)!
    expect(pose[3]).toBeCloseTo(5)
    // One undo removes the paste AND restores the loose board.
    scene.scene_undo()
    expect(Array.from(scene.object_ids())).toContain(board)
    expect(Array.from(scene.instance_ids())).toHaveLength(0)
  })

  it('a second paste of the same copy stays linked once the clipboard points at the placement', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const first = pasteLinked(scene, [{ kind: 'object', id: board }], null, shift(5))
    // The loose board is no longer a live node: only the remapped source links.
    expect(canPasteLinked(scene, source(scene, [{ kind: 'object', id: board }]), nonce, 0)).toBe(false)
    const again = source(scene, first.sources)
    expect(canPasteLinked(scene, again, nonce, 0)).toBe(true)
    const second = pasteLinked(scene, again.nodes, null, shift(9))
    expect(scene.definition_usage(scene.instance_def(second.result.rootIds[0])!)).toBe(3)
  })

  it('paste_after_the_source_was_deleted_is_independent', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const src = source(scene, [{ kind: 'object', id: board }])
    scene.delete_node(0, board)
    expect(canPasteLinked(scene, src, nonce, 0)).toBe(false)
  })

  it('paste_into_another_document_is_independent', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const src = source(scene, [{ kind: 'object', id: board }])
    const other = new Scene()
    makeBox(other, 0, 2, 0, 1, 1)
    expect(canPasteLinked(other, src, nonce, 0)).toBe(false)
    // The same document in another window (a different nonce) is independent too.
    expect(canPasteLinked(scene, src, 'window-2', 0)).toBe(false)
    expect(canPasteLinked(scene, null, nonce, 0)).toBe(false)
  })

  it('an instance source places its own definition at the moved pose', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const src = source(scene, [{ kind: 'instance', id: placements[1] }])
    const { result } = pasteLinked(scene, src.nodes, null, shift(0.5))
    const pasted = result.rootIds[0]
    expect(scene.instance_def(pasted)).toBe(scene.instance_def(placements[1]))
    expect(scene.definition_usage(scene.instance_def(pasted)!)).toBe(5)
  })
})

describe('booleans on a placement', () => {
  it('subtract_with_a_placement_host_goes_through_the_definition (every copy shows the cut)', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const def = scene.instance_def(placements[0])!
    const part = Array.from(scene.component_member_objects(def))[0]
    const before = triangles(scene, part)
    // A mortise: a box poking into the first leg.
    const cutter = makeBox(scene, 0.25, 0.75, 0.25, 0.75, 1, 2.5)
    const depth = scene.undo_depth()
    subtractThroughPlacement(scene, placements[0], cutter, 'all')
    expect(scene.undo_depth()).toBe(depth + 1)
    // All four are still placements of the one set, and the set's part changed.
    const after = Array.from(scene.component_member_objects(def))[0]
    expect(scene.definition_usage(def)).toBe(4)
    expect(triangles(scene, after)).toBeGreaterThan(before)
    // The cutter was consumed.
    expect(Array.from(scene.object_ids())).not.toContain(cutter)
  })

  it('the_result_keeps_the_hosts_name', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const cutter = makeBox(scene, 0.25, 0.75, 0.25, 0.75, 1, 2.5)
    subtractThroughPlacement(scene, placements[0], cutter, 'all')
    expect(placementLabel(scene, placements[1], 0)).toBe('Leg')
    const def = scene.instance_def(placements[1])!
    expect(scene.object_name(Array.from(scene.component_member_objects(def))[0])).toBe('Leg')
  })

  it('only_this_one_unlinks_then_cuts_in_one_undo_entry', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const [a, b] = placements
    const defBefore = scene.instance_def(a)!
    const originalPart = Array.from(scene.component_member_objects(defBefore))[0]
    const original = triangles(scene, originalPart)
    const cutter = makeBox(scene, 0.25, 0.75, 0.25, 0.75, 1, 2.5)
    const depth = scene.undo_depth()
    subtractThroughPlacement(scene, a, cutter, 'one')
    expect(scene.undo_depth()).toBe(depth + 1)
    expect(undoLabels(scene).at(-1)).toBe('Subtract (unlinked)')
    // `a` has its own, cut, part; the other three keep the uncut set.
    const mine = Array.from(scene.component_member_objects(scene.instance_def(a)!))[0]
    expect(scene.instance_def(a)).not.toBe(defBefore)
    expect(triangles(scene, mine)).toBeGreaterThan(original)
    expect(scene.instance_def(b)).toBe(defBefore)
    expect(scene.definition_usage(defBefore)).toBe(3)
    expect(triangles(scene, Array.from(scene.component_member_objects(defBefore))[0])).toBe(original)
    scene.scene_undo()
    expect(scene.definition_usage(defBefore)).toBe(4)
  })

  it('routes: only a LINKED copy takes the placement routes; an ordinary component keeps its old boolean', () => {
    const o: NodeRef = { kind: 'object', id: 1n }
    const g: NodeRef = { kind: 'group', id: 2n }
    const i = inst(3n)
    const linked = (id: bigint): boolean => id === 3n || id === 4n
    expect(routeBoolean(1, i, o, linked)).toBe('through-placement')
    expect(routeBoolean(1, i, g, linked)).toBe('refuse-cutter')
    expect(routeBoolean(1, i, inst(4n), linked)).toBe('refuse-cutter')
    expect(routeBoolean(0, i, o, linked)).toBe('unlink-and-join')
    expect(routeBoolean(2, o, i, linked)).toBe('unlink-and-join')
    expect(routeBoolean(1, o, i, linked)).toBe('unlink-and-join')
    expect(routeBoolean(1, o, g, linked)).toBe('plain')
    // An ordinary component (id 9 is not linked), cut, joined, or cutting.
    const ordinary = inst(9n)
    expect(routeBoolean(1, ordinary, o, linked)).toBe('plain')
    expect(routeBoolean(1, ordinary, g, linked)).toBe('plain')
    expect(routeBoolean(0, ordinary, o, linked)).toBe('plain')
    expect(routeBoolean(2, o, ordinary, linked)).toBe('plain')
    expect(routeBoolean(1, o, ordinary, linked)).toBe('plain')
  })

  it('a two-instance ordinary component is not a linked copy, so no scope strip and no both-copies cut', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 2, 1)
    const first = scene.make_component(new Uint8Array([0]), new BigUint64Array([board]))
    const def = scene.instance_def(first)!
    const second = scene.place_instance(def, shift(10))
    expect(scene.definition_usage(def)).toBe(2)
    expect(scene.is_linked(first)).toBe(false)
    expect(routeBoolean(1, inst(first), { kind: 'object', id: 1n }, (id) => scene.is_linked(id))).toBe('plain')
    // The old path (explode, then cut) still works and leaves the other instance alone.
    const cutter = makeBox(scene, 0.5, 1.5, 0.5, 1.5, 1, 0.5)
    const out = booleanUnlinkingPlacements(scene, 1, inst(first), { kind: 'object', id: cutter })
    expect(out.kind).toBe('ok')
    expect(scene.is_linked(second)).toBe(false)
    expect(scene.definition_usage(def)).toBe(1)
  })

  it('union_with_a_placement_unlinks_both_and_announces_it', () => {
    const scene = new Scene()
    // Two placements of two different sets, overlapping.
    const a = parseLinkedResult(
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 2, 0, 2, 1)]), shift(10), 1),
    )
    const b = parseLinkedResult(
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 1, 3, 1, 3, 1, 0)]), shift(10), 1),
    )
    const hostA = a.linked[0].instance
    const hostB = b.linked[0].instance
    const depth = scene.undo_depth()
    const out = booleanUnlinkingPlacements(scene, 0, inst(hostA), inst(hostB))
    expect(out.kind).toBe('ok')
    if (out.kind !== 'ok') return
    expect(out.unlinked).toBe(2)
    expect(unlinkedToJoin(out.unlinked)).toBe('Unlinked 2 copies to join them')
    expect(unlinkedToJoin(1)).toBe('Unlinked 1 copy to join it')
    // One undo entry, labeled "(unlinked)".
    expect(scene.undo_depth()).toBe(depth + 1)
    expect(undoLabels(scene).at(-1)).toBe('Union (unlinked)')
    scene.scene_undo()
    expect(scene.is_linked(hostA)).toBe(true)
    expect(scene.is_linked(hostB)).toBe(true)
  })

  it('intersect_with_a_placement_unlinks_and_labels_the_undo_unlinked', () => {
    const scene = new Scene()
    const a = parseLinkedResult(
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 2, 0, 2, 1)]), shift(10), 1),
    )
    const loose = makeBox(scene, 1, 3, 1, 3, 1)
    const out = booleanUnlinkingPlacements(scene, 2, inst(a.linked[0].instance), { kind: 'object', id: loose })
    expect(out.kind).toBe('ok')
    expect(undoLabels(scene).at(-1)).toBe('Intersect (unlinked)')
    if (out.kind === 'ok') expect(out.unlinked).toBe(1)
  })

  it('subtract_with_a_placement_cutter_unlinks_it_and_announces_it', () => {
    const scene = new Scene()
    const plate = makeBox(scene, 0, 4, 0, 4, 1)
    const cutterSet = parseLinkedResult(
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 1, 2, 1, 2, 2, 0.5)]), shift(10), 1),
    )
    const out = booleanUnlinkingPlacements(scene, 1, { kind: 'object', id: plate }, inst(cutterSet.linked[0].instance))
    expect(out.kind).toBe('ok')
    if (out.kind === 'ok') expect(out.unlinked).toBe(1)
    expect(undoLabels(scene).at(-1)).toBe('Subtract (unlinked)')
  })

  it('a refused join leaves the document exactly as it was', () => {
    const scene = new Scene()
    const a = parseLinkedResult(
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([makeBox(scene, 0, 2, 0, 2, 1)]), shift(10), 1),
    )
    // A cutter that never reaches the plate removes nothing: the kernel refuses.
    const far = makeBox(scene, 50, 51, 50, 51, 1)
    const depth = scene.undo_depth()
    const out = booleanUnlinkingPlacements(scene, 1, { kind: 'object', id: far }, inst(a.linked[0].instance))
    expect(out.kind).toBe('refused')
    expect(scene.undo_depth()).toBe(depth)
    expect(scene.is_linked(a.linked[0].instance)).toBe(true)
  })
})

describe('refusals are never silent', () => {
  it('copy, boolean through a placement, and unlink refuse while a component is open for editing', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const board = makeBox(scene, 20, 21, 20, 21, 1)
    scene.open_explode_session(placements[0])
    expect(() =>
      scene.duplicate_selection_array_linked(new Uint8Array([0]), new BigUint64Array([board]), shift(1), 1),
    ).toThrow(/ExplodeSessionScope/)
    expect(() => unlinkCopy(scene, placements[1])).toThrow(/ExplodeSessionScope/)
  })
})

describe('linked paste: the source moved, undo, and the clipboard', () => {
  const nonce = 'window-1'
  const srcOf = (scene: Scene, nodes: LinkSource['nodes']): LinkSource => ({
    documentId: scene.ensure_document_id(),
    nonce,
    generation: 0,
    nodes,
  })

  it('paste_linked_lands_where_the_ghost_shows_when_the_source_moved_after_copy', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const kinds = new Uint8Array([0])
    const ids = new BigUint64Array([board])
    // Copy time: where the independent paste would anchor.
    const copyAffine = Array.from(scene.extract_item_placement(kinds, ids)!)
    const src = srcOf(scene, [{ kind: 'object', id: board }])
    // The source moves 7 m along X after Copy.
    scene.transform_selection(kinds, ids, new BigUint64Array(), shift(7))
    // The ghost pastes 5 m from the copy-time anchor.
    const paste = copyAffine.slice()
    paste[3] += 5
    const { result } = pasteLinked(scene, src.nodes, copyAffine, Float64Array.from(paste))
    const pasted = result.rootIds[0]
    const def = scene.instance_def(pasted)!
    const part = Array.from(scene.component_member_objects(def))[0]
    const b = scene.object_bounds(part, scene.instance_pose(pasted)!)
    // The board was 0..2 wide; the independent paste would put it at 5..7.
    expect(b[0]).toBeCloseTo(5, 5)
    expect(b[3]).toBeCloseTo(7, 5)
  })

  it('in place with a moved source, the copy returns to where the copy was taken', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const kinds = new Uint8Array([0])
    const ids = new BigUint64Array([board])
    const copyAffine = Array.from(scene.extract_item_placement(kinds, ids)!)
    scene.transform_selection(kinds, ids, new BigUint64Array(), shift(7))
    const { result } = pasteLinked(scene, [{ kind: 'object', id: board }], copyAffine, Float64Array.from(copyAffine))
    const pasted = result.rootIds[0]
    const part = Array.from(scene.component_member_objects(scene.instance_def(pasted)!))[0]
    expect(scene.object_bounds(part, scene.instance_pose(pasted)!)[0]).toBeCloseTo(0, 5)
  })

  it('undo_of_a_linked_paste_keeps_the_next_paste_linked', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const content = { linkSource: srcOf(scene, [{ kind: 'object', id: board }]) } as unknown as ClipboardContent
    const first = pasteLinked(scene, content.linkSource!.nodes, null, shift(5))
    repointLinkSource(content, first.sources)
    // Live: the wrapper stands in for the board.
    expect(resolveLinkSource(scene, content.linkSource, nonce, 0)).toEqual(first.sources)
    scene.scene_undo() // the paste and its wrap go away; the loose board is back
    const nodes = resolveLinkSource(scene, content.linkSource, nonce, 0)
    expect(nodes).toEqual([{ kind: 'object', id: board }])
    const again = pasteLinked(scene, nodes!, null, shift(9))
    expect(scene.is_linked(again.result.rootIds[0])).toBe(true)
  })

  it('a copy never links across a reload of the same document (generation)', () => {
    const scene = new Scene()
    const board = makeBox(scene, 0, 2, 0, 1, 1)
    const src = srcOf(scene, [{ kind: 'object', id: board }])
    expect(canPasteLinked(scene, src, nonce, 0)).toBe(true)
    expect(canPasteLinked(scene, src, nonce, 1)).toBe(false)
  })

  it('planPaste: independent and a preference-off paste carry no origin; explicit linked refuses when it cannot link', () => {
    const nodes: LinkSource['nodes'] = [{ kind: 'object', id: 1n }]
    expect(planPaste('independent', true, nodes, nonce)).toEqual({ kind: 'plain', origin: undefined })
    expect(planPaste('default', false, nodes, nonce)).toEqual({ kind: 'plain', origin: undefined })
    expect(planPaste('default', true, nodes, nonce)).toEqual({ kind: 'linked', nodes })
    expect(planPaste('default', true, null, nonce)).toEqual({ kind: 'plain', origin: nonce })
    expect(planPaste('linked', false, nodes, nonce)).toEqual({ kind: 'linked', nodes })
    expect(planPaste('linked', true, null, nonce)).toEqual({ kind: 'refuse' })
  })
})

describe('guards and saving', () => {
  it('blockedWhileEditing toasts while a component is open and lets everything else through', () => {
    const toast = (() => {
      const calls: string[] = []
      const fn = (m: string): void => void calls.push(m)
      return Object.assign(fn, { calls })
    })()
    expect(blockedWhileEditing(false, toast)).toBe(false)
    expect(toast.calls).toHaveLength(0)
    expect(blockedWhileEditing(true, toast)).toBe(true)
    expect(toast.calls).toHaveLength(1)
  })

  it('promoteForSave keeps the promotion on success and takes it back when the save fails', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const depth = scene.undo_depth()
    expect(() => promoteForSave(scene, placements[0], 'Table leg', () => { throw new Error('extract refused') })).toThrow('extract refused')
    expect(scene.is_linked(placements[0])).toBe(true)
    expect(scene.undo_depth()).toBe(depth)

    const ok = promoteForSave(scene, placements[0], 'Table leg', () => 'bytes')
    expect(ok.value).toBe('bytes')
    expect(scene.is_linked(placements[0])).toBe(false)
    // A later failure (the file write) undoes it only while nothing else happened.
    expect(undoPromotionIfUnmoved(scene, ok.promotedAt)).toBe(true)
    expect(scene.is_linked(placements[0])).toBe(true)
    const again = promoteForSave(scene, placements[0], 'Table leg', () => 1)
    scene.set_node_name(2, placements[1], 'Elsewhere')
    expect(undoPromotionIfUnmoved(scene, again.promotedAt)).toBe(false)
    expect(scene.is_linked(placements[0])).toBe(false)
  })
})

describe('direct placement tools only act on a copy a click would select', () => {
  it('linkedPlacementPickEligible: a loose copy yes; in a group, in a session, in a context, or an ordinary component no', () => {
    const scene = new Scene()
    const { placements } = fourLegs(scene)
    const part = Array.from(scene.component_member_objects(scene.instance_def(placements[0])!))[0]
    expect(linkedPlacementPickEligible(scene, [], false, part, placements[0])).toBe(true)
    expect(linkedPlacementPickEligible(scene, [], true, part, placements[0])).toBe(false)
    expect(linkedPlacementPickEligible(scene, [{ kind: 'object', id: 1n }], false, part, placements[0])).toBe(false)
    expect(linkedPlacementPickEligible(scene, [], false, part, undefined)).toBe(false)
    // Inside a closed group the click selects the group, so the copy is not directly editable.
    scene.group_nodes(new Uint8Array([2]), new BigUint64Array([placements[1]]))
    expect(linkedPlacementPickEligible(scene, [], false, part, placements[1])).toBe(false)
    // An ordinary component is never eligible.
    const b = makeBox(scene, 20, 21, 20, 21, 1)
    const inst1 = scene.make_component(new Uint8Array([0]), new BigUint64Array([b]))
    const member = Array.from(scene.component_member_objects(scene.instance_def(inst1)!))[0]
    expect(linkedPlacementPickEligible(scene, [], false, member, inst1)).toBe(false)
  })
})
