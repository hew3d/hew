/**
 * The tagged answer of `Scene.api_dispatch` (crates/wasm-api): a reply
 * frame to send back, a plugin command to route to its plugin (completed
 * later with `Scene.api_complete_forward`), or nothing for a dropped
 * notification. docs/agents/HEW_API.md §10.2 has the forwarding contract.
 */

export type DispatchResult =
  | { kind: 'none' }
  | { kind: 'reply'; frame: string }
  | { kind: 'pending'; token: number; forward: ForwardRequestJson }

/** `api::ForwardRequest` as JSON. */
export interface ForwardRequestJson {
  token: number
  plugin: string
  local: string
  method: string
  request_id: number | string
  params: unknown
  mutating: boolean
  label: string | null
  timeout_ms: number
  concurrency: 'parallel' | 'serial'
  caller: { identity: string; plugin: string | null; app: string | null; forward: number | null }
}

/** Reads `Scene.api_dispatch`'s answer. Anything that is not one of the two
 * tagged shapes is a bug at the boundary: it is logged and treated as
 * `none`, so no frame is ever sent that the dispatcher did not produce. */
export function parseDispatch(raw: string | undefined): DispatchResult {
  if (raw === undefined) return { kind: 'none' }
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    parsed = undefined
  }
  if (typeof parsed === 'object' && parsed !== null) {
    const tagged = parsed as { reply?: unknown; pending?: unknown }
    if (typeof tagged.reply === 'string') return { kind: 'reply', frame: tagged.reply }
    const pending = tagged.pending as { token?: unknown; forward?: unknown } | undefined
    if (
      typeof pending === 'object' &&
      pending !== null &&
      typeof pending.token === 'number' &&
      typeof pending.forward === 'object' &&
      pending.forward !== null
    ) {
      return { kind: 'pending', token: pending.token, forward: pending.forward as ForwardRequestJson }
    }
  }
  console.error('Scene.api_dispatch answered an unreadable result', raw)
  return { kind: 'none' }
}
