/**
 * ShareDrop — the Durable Object that stores one drop's ciphertext, one
 * instance per token (`env.SHARE_DROP.idFromName(token)` in `handlers.ts`),
 * backed by that instance's private SQLite database. This is what replaces R2
 * (see README.md's "Storage" section): with no R2 there is no billing surface
 * at all — every SQLite-backed-DO free-tier limit fails CLOSED (the operation
 * errors, nothing bills).
 *
 * This class MUST `extend DurableObject`: Durable Object RPC — the
 * `stub.store(...)` / `stub.consume()` / `stub.destroy()` calls `handlers.ts`
 * makes on the stub returned by `SHARE_DROP.get(id)` — is only exposed for a
 * class that extends the `DurableObject` base from `cloudflare:workers`. A
 * plain class gets no RPC surface, so those calls throw at runtime (Cloudflare
 * error 1101) even though the class "has" the methods. That is exactly the bug
 * an earlier revision shipped: the methods lived on a plain class, `node --test`
 * (which can't import `cloudflare:workers`, and drove the class directly)
 * passed, and the deployed Worker 500'd on the first PUT. The real logic now
 * lives in `DropStore` (`dropStore.ts`, framework-free and unit-tested); this
 * file is the thin runtime wrapper, validated against real `workerd` via
 * `wrangler dev` because bare `node --test` structurally cannot exercise DO RPC.
 */

import { DurableObject } from 'cloudflare:workers'

import { DropStore } from './dropStore.ts'
import type { DropEnv, DurableObjectStorage } from './types.ts'

export { TTL_MS } from './dropStore.ts'

export class ShareDrop extends DurableObject<DropEnv> {
  private readonly store_: DropStore

  constructor(ctx: DurableObjectState, env: DropEnv) {
    super(ctx, env)
    // The real runtime's `ctx.storage` satisfies this project's
    // `DurableObjectStorage` shim structurally (it has `sql` + the alarm/
    // deleteAll host calls DropStore uses); the cast bridges the shim and the
    // `cloudflare:workers` type without pulling the full types package in.
    this.store_ = new DropStore(ctx.storage as unknown as DurableObjectStorage)
  }

  /** RPC — `handlers.ts` `handlePutDrop`. */
  store(name: string, chunks: Uint8Array[]): Promise<void> {
    return this.store_.store(name, chunks)
  }

  /** RPC — `handlers.ts` `handleGetDrop` (one-shot). */
  consume(): Promise<{ name: string; bytes: Uint8Array } | null> {
    return this.store_.consume()
  }

  /** RPC — `handlers.ts` `handlePeek` (non-consuming existence check). */
  peek(): Promise<{ exists: boolean }> {
    return this.store_.peek()
  }

  /** RPC — `handlers.ts` `handleDeleteDrop` (dialog close). */
  destroy(): Promise<void> {
    return this.store_.destroy()
  }

  /** DO alarm handler — fires at `uploadedAt + TTL_MS` (armed in `store`) and
   *  wipes an unconsumed drop. Idempotent, so firing against an already-
   *  consumed or never-populated drop is harmless. */
  async alarm(): Promise<void> {
    await this.store_.destroy()
  }
}
