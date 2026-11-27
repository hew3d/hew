/**
 * Where a plugin command forward is served. The live bridge and anything
 * else that receives a call keeps owning the call's compound, busy state
 * and completion; the router only answers what the plugin replied.
 *
 * The slot is empty unless something filled it, and the bridge reads it
 * only where the registry turns the coupling on, so with nothing here (and
 * in every build where that coupling is off) a forward is answered
 * `unavailable` exactly as before.
 */

import type { ForwardRequestJson } from './dispatchResult'

/** `api::ForwardReply` as JSON: what `Scene.api_complete_forward` takes. */
export type ForwardReplyJson =
  | { ok: unknown }
  | { refusal: { name: string; explanation: string; detail?: unknown } }
  | { failed: 'timeout' | 'crashed' | 'unavailable' | 'no_handler' | 'cancelled' }

export interface ForwardRouter {
  /**
   * Answers `fwd`. Never rejects: anything that goes wrong is a `failed`
   * reply. When `signal` aborts (the caller cancelled, or gave up waiting)
   * the caller has already been answered, and the router lets the plugin
   * know and drops whatever it says later.
   */
  serve(fwd: ForwardRequestJson, signal: AbortSignal): Promise<ForwardReplyJson>
}

let current: ForwardRouter | null = null

/** Fills (or, with `null`, empties) the slot. */
export function setForwardRouter(router: ForwardRouter | null): void {
  current = router
}

/** The router in the slot, or `null`. */
export function forwardRouter(): ForwardRouter | null {
  return current
}
