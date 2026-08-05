/**
 * Solid gating — the product point of the STL/3MF export dialogs: warn
 * before writing a file containing any non-watertight object. Query-only;
 * never repairs.
 *
 * Split out of the retired `stlExport.ts` (the exporters themselves moved
 * to `crates/mesh-export`; this piece has no Rust equivalent because it is
 * pure UI policy — a pre-export confirmation, not part of any file format)
 * so `App.tsx`'s warning dialog keeps a home once that file is gone.
 */

/**
 * The minimal slice of the wasm `Scene` surface the gating query needs —
 * structural, so tests can pass a plain mock.
 */
export interface SolidQueryScene {
  object_ids(): BigUint64Array
  instance_ids(): BigUint64Array
  instance_def(instance: bigint): bigint | undefined
  component_member_objects(component: bigint): BigUint64Array
  object_solid(id: bigint): boolean
  object_name(object: bigint): string | undefined
}

/** One non-watertight object that would be included in the export. */
export interface NonSolidObject {
  id: bigint
  name: string
}

/**
 * Every object an STL/3MF export would include that is NOT a watertight
 * solid: the top-level objects plus each placed instance's definition
 * members (the same set `crates/mesh-export`'s writers walk), deduplicated.
 * The export itself always includes these objects regardless of this
 * check's result — this only powers the warning dialog naming them first.
 */
export function collectNonSolidObjects(scene: SolidQueryScene): NonSolidObject[] {
  const seen = new Set<bigint>()
  const out: NonSolidObject[] = []

  const check = (id: bigint) => {
    if (seen.has(id)) return
    seen.add(id)
    if (scene.object_solid(id)) return
    out.push({ id, name: scene.object_name(id) ?? `Object ${id}` })
  }

  for (const id of scene.object_ids()) check(id)
  for (const instanceId of scene.instance_ids()) {
    const def = scene.instance_def(instanceId)
    if (def === undefined) continue
    for (const memberId of scene.component_member_objects(def)) check(memberId)
  }

  return out
}
