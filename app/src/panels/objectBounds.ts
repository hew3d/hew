/**
 * objectBounds — world axis-aligned bounding box for the current selection,
 * for the Object Info panel's Bounding Box readout ("does this part fit my
 * print bed?").
 *
 * Client-side, surface-free: reuses the exact accessors `SceneRenderer`
 * already calls to draw the scene (`object_mesh`, `instance_pose`,
 * `component_member_objects`), and `treeModel.collectLeafIds` to walk a
 * selection down to its leaf Objects/Instances — no new wasm-api method.
 *
 * A Group has no geometry or pose of its own; moving one bakes the transform
 * into every leaf Object beneath it (ARCHITECTURE.md §2.7), so a leaf
 * Object's mesh — whether top-level or nested in a Group — is already in
 * world space. A Component instance is the one place a persistent pose
 * exists: its definition owns geometry once, in definition-local space, and
 * each instance poses it only at render/tessellation time, so an instance's
 * member meshes must be transformed by its own affine before they're in
 * world space.
 */

import type { Scene as WasmScene } from '../wasm/loader'
import { collectLeafIds, nodeRefFromJs, type NodeRef } from './treeModel'

/** A 3D point or per-axis extent as a plain [x, y, z] tuple. */
export type Vec3 = readonly [number, number, number]

/** A world axis-aligned bounding box. */
export interface Bounds {
  min: Vec3
  max: Vec3
}

/**
 * The AABB of a mesh's position buffer — extends
 * `transformMath.meshBoundingBoxCenter`'s min/max scan to also return the
 * corners, and to optionally pose the mesh first.
 *
 * `positions` is a flat Float32Array of [x,y,z] triples (the same format
 * `MeshJs.positions()` returns). If `pose` is given — a row-major 3×4 affine
 * (the format `Scene.instance_pose` returns and `transform_object` accepts;
 * see `tools/transformMath.ts`'s `Affine`) — every vertex is transformed by
 * it before being folded into the box, so the result is the exact world AABB
 * of the posed mesh. This is deliberately NOT approximated by transforming
 * the 8 corners of the local box, which would only bound a rotated mesh
 * loosely — SketchUp's own bounding-box readout, which this feature matches,
 * is computed the same tight way.
 *
 * Returns null for an empty buffer.
 */
export function meshWorldBounds(positions: Float32Array, pose?: ArrayLike<number>): Bounds | null {
  if (positions.length < 3) return null
  let minX = Infinity, minY = Infinity, minZ = Infinity
  let maxX = -Infinity, maxY = -Infinity, maxZ = -Infinity
  for (let i = 0; i < positions.length; i += 3) {
    let x = positions[i]
    let y = positions[i + 1]
    let z = positions[i + 2]
    if (pose !== undefined) {
      const px = pose[0] * x + pose[1] * y + pose[2] * z + pose[3]
      const py = pose[4] * x + pose[5] * y + pose[6] * z + pose[7]
      const pz = pose[8] * x + pose[9] * y + pose[10] * z + pose[11]
      x = px
      y = py
      z = pz
    }
    if (x < minX) minX = x
    if (x > maxX) maxX = x
    if (y < minY) minY = y
    if (y > maxY) maxY = y
    if (z < minZ) minZ = z
    if (z > maxZ) maxZ = z
  }
  return { min: [minX, minY, minZ], max: [maxX, maxY, maxZ] }
}

/** Union two (possibly absent) bounds into the box that contains both. */
export function unionBounds(a: Bounds | null, b: Bounds | null): Bounds | null {
  if (a === null) return b
  if (b === null) return a
  return {
    min: [Math.min(a.min[0], b.min[0]), Math.min(a.min[1], b.min[1]), Math.min(a.min[2], b.min[2])],
    max: [Math.max(a.max[0], b.max[0]), Math.max(a.max[1], b.max[1]), Math.max(a.max[2], b.max[2])],
  }
}

/** Per-axis extents (max − min) of a bounding box. */
export function boundsExtents(bounds: Bounds): Vec3 {
  return [
    bounds.max[0] - bounds.min[0],
    bounds.max[1] - bounds.min[1],
    bounds.max[2] - bounds.min[2],
  ]
}

/**
 * World AABB of one Object's mesh — the (optionally posed) bounds of its
 * render mesh, or null if it contributes nothing.
 *
 * `object_mesh` is the ONE panel accessor that THROWS (a typed
 * `UnknownObject` JsError) on a stale/deleted object id, rather than
 * returning a quiet empty like `object_name`/`object_solid`/`node_tags`/
 * `instance_pose`/etc. all do. A selection can legitimately name a
 * just-removed object — undo bumps `docRev` without pruning the selection
 * — so this readout must NOT
 * crash the panel over it. Guard the call and treat a throw as "no
 * contribution" (return null), honoring the graceful-degradation contract
 * every other Object Info accessor already keeps.
 *
 * `.free()` discipline: the `MeshJs` is freed only on the success path,
 * where it was actually materialized — a throw leaves nothing to free.
 *
 * `pose`, if given, transforms the (definition-local) mesh into world space
 * before bounding — used for a Component instance's members; an Object's own
 * mesh is already baked world-space (module doc), so it's called without a
 * pose.
 */
function objectMeshBounds(
  scene: WasmScene,
  objectId: bigint,
  pose?: ArrayLike<number>,
): Bounds | null {
  let mesh
  try {
    mesh = scene.object_mesh(objectId)
  } catch {
    return null
  }
  try {
    return meshWorldBounds(mesh.positions(), pose)
  } finally {
    mesh.free()
  }
}

/**
 * World AABB of one Component instance: the union of every definition
 * member's (definition-local) mesh, posed by the instance's own affine — see
 * module doc. Null if the instance is stale (no pose), its definition is
 * stale, the definition has no members, or every member's mesh is stale.
 */
function instanceWorldBounds(scene: WasmScene, instanceId: bigint): Bounds | null {
  const pose = scene.instance_pose(instanceId)
  if (pose === undefined) return null
  const componentId = scene.instance_def(instanceId)
  if (componentId === undefined) return null

  let bounds: Bounds | null = null
  for (const memberId of scene.component_member_objects(componentId)) {
    bounds = unionBounds(bounds, objectMeshBounds(scene, memberId, pose))
  }
  return bounds
}

/**
 * World AABB of a selection — the union of every leaf Object's and Component
 * instance's bounds reachable from it. A Group contributes its members'
 * bounds, recursively (`treeModel.collectLeafIds` walks nested groups); a
 * Sketch (and its sub-entities) contributes nothing, since it has no mesh.
 * `selection` may hold any number of nodes (single selection or a
 * multi-selection).
 *
 * Returns null for an empty selection, or one that resolves to no mesh at
 * all (e.g. only Sketches) — the Object Info panel shows no Bounding Box row
 * in that case.
 */
export function worldBoundsForSelection(scene: WasmScene, selection: NodeRef[]): Bounds | null {
  const getGroupMembers = (groupId: bigint): NodeRef[] =>
    scene.group_members(groupId).map(nodeRefFromJs)

  let bounds: Bounds | null = null
  for (const node of selection) {
    const { objectIds, instanceIds } = collectLeafIds(node, getGroupMembers)
    for (const objectId of objectIds) {
      bounds = unionBounds(bounds, objectMeshBounds(scene, objectId))
    }
    for (const instanceId of instanceIds) {
      bounds = unionBounds(bounds, instanceWorldBounds(scene, instanceId))
    }
  }
  return bounds
}
