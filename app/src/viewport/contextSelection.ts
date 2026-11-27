/**
 * What Select All, Invert Selection and a marquee can reach inside an open
 * editing context, other than a group (whose direct members the Viewport
 * lists itself). It is exactly what a click can select there:
 *
 * - inside a component instance: the definition's member objects, the
 *   definition's own sketch shapes, and the shapes drawn on the members'
 *   faces, all posed through the instance;
 * - inside a single solid: the shapes drawn on that solid's faces (the solid
 *   itself is not click-selectable there), which may be nothing.
 */
import { nodeKey, type NodeRef } from '../panels/treeModel'
import {
  ctxOfSketchUnits,
  readPlaneUnits,
  draftRef,
  type PlaneUnitsScene,
} from '../tools/planeUnits'
import type { Scene as WasmScene } from '../wasm/loader'
import { objectDrawingIslands } from './drawnSelection'

type ContextScene = PlaneUnitsScene & {
  instance_def(instance: bigint): bigint | undefined
  instance_pose(instance: bigint): ArrayLike<number> | undefined
  component_member_objects(component: bigint): ArrayLike<bigint>
  component_member_sketches(component: bigint): ArrayLike<bigint>
  object_drawing_planes?(object: bigint): ArrayLike<number>
}

export interface ContextRefs {
  /** Selectable objects (instance context only). */
  objects: NodeRef[]
  /** Drawn shapes, one island each. */
  drawn: NodeRef[]
}

/** The refs clickable inside `ctx`'s deepest frame when that is an instance
 *  or an object; `null` for the top level and for a group. `isHidden` says
 *  whether a member object (seen through `instance`, or `undefined` for a
 *  world object) is hidden. */
export function contextSelectableRefs(
  scene: ContextScene,
  ctx: readonly NodeRef[],
  isHidden: (object: bigint, instance: bigint | undefined) => boolean,
): ContextRefs | null {
  if (ctx.length === 0) return null
  const deepest = ctx[ctx.length - 1]
  const seen = new Set<string>()
  const drawn: NodeRef[] = []
  if (deepest.kind === 'object') {
    if (!isHidden(deepest.id, undefined)) objectDrawingIslands(scene, [deepest.id], null, null, seen, drawn)
    return { objects: [], drawn }
  }
  if (deepest.kind !== 'instance') return null
  const def = scene.instance_def(deepest.id)
  const pose = scene.instance_pose(deepest.id)
  if (def === undefined || pose === undefined) return { objects: [], drawn }
  const members = Array.from(scene.component_member_objects(def)).filter((o) => !isHidden(o, deepest.id))
  const objects: NodeRef[] = members.map((id) => ({ kind: 'object', id }))
  objectDrawingIslands(scene, members, deepest.id, pose, seen, drawn)
  // The definition's own sketches, listed in the instance's frame.
  const mine = new Set(Array.from(scene.component_member_sketches(def)))
  const doneCtx = new Set<string>()
  for (const sid of mine) {
    const c = ctxOfSketchUnits(scene as unknown as WasmScene, sid, deepest.id)
    if (c === null || doneCtx.has(c)) continue
    doneCtx.add(c)
    const pu = readPlaneUnits(scene, c)
    if (pu === null) continue
    for (const unit of pu.units) {
      if (!unit.edges.some((e) => e.store.kind === 'free' && mine.has(e.store.sketch))) continue
      const ref = draftRef('draft-island', c, unit.key)
      const k = nodeKey(ref)
      if (seen.has(k)) continue
      seen.add(k)
      drawn.push(ref)
    }
  }
  return { objects, drawn }
}

/**
 * The drawn shapes a marquee may pick. An instance or single-solid context
 * scopes them to what lives inside it (`contextSelectableRefs`); everywhere
 * else — the top level and an open GROUP session, where drawing is
 * world-global by design — it is the world's drawn shapes (`worldDrawn`,
 * read lazily).
 */
export function marqueeDrawnRefs(
  scene: ContextScene,
  ctx: readonly NodeRef[],
  isHidden: (object: bigint, instance: bigint | undefined) => boolean,
  worldDrawn: () => NodeRef[],
): NodeRef[] {
  const scoped = contextSelectableRefs(scene, ctx, isHidden)
  return scoped !== null ? scoped.drawn : worldDrawn()
}
