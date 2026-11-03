/**
 * The editing scope, pushed to the renderer and the kernel in one place.
 *
 * The renderer dims everything outside the lit sets; in Shaded and X-ray the
 * kernel's face and sketch picks skip everything outside the same sets
 * (`Scene.set_pick_scope`). Both are fed from this one function so the
 * dimming and the picking cannot drift apart.
 */
import { bumpPickScopeGeneration } from '../tools/pickPolicy'

/** The slice of the renderer the scope push needs. */
export interface ScopeRenderer {
  setActiveContext(lit: Set<bigint> | null, litInstances: Set<bigint> | null): void
}

/** The slice of the kernel scene the scope push needs. */
export interface ScopeKernel {
  set_pick_scope(objects: BigUint64Array, instances: BigUint64Array, frame_sketches: boolean): void
  clear_pick_scope(): void
}

/** Push the lit sets (`null` lit object set = no scope, the top level) to the
 *  renderer and the kernel, and start a new pick generation so memoized picks
 *  miss after a context change that arrives without a pointer move. */
export function pushActiveScope(
  renderer: ScopeRenderer | null | undefined,
  kernel: ScopeKernel,
  objectLit: Set<bigint> | null,
  instanceLit: Set<bigint> | null,
  /** True for a session's own scope (its free frame sketches are in it);
   *  false for an object or legacy-instance context. */
  frameSketches: boolean,
): void {
  renderer?.setActiveContext(objectLit, instanceLit)
  if (objectLit === null && instanceLit === null) {
    kernel.clear_pick_scope()
  } else {
    kernel.set_pick_scope(
      BigUint64Array.from(objectLit ?? []),
      BigUint64Array.from(instanceLit ?? []),
      frameSketches,
    )
  }
  bumpPickScopeGeneration()
}
