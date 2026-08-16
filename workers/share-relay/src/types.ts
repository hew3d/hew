/**
 * Minimal local shims for the pieces of the Cloudflare Workers Durable
 * Object runtime this project touches — hand-rolled rather than depending
 * on `@cloudflare/workers-types` so the unit suite (`node --test`, see
 * package.json) runs with zero installs. `src/testSupport/fakeDurableObject.ts`
 * implements these interfaces over Node's built-in `node:sqlite`, standing
 * in for the real binding exactly the way the old R2 mock bucket did (see
 * git history) — `wrangler dev`/`deploy` type-checks against its own
 * bundled definitions regardless of what's declared here. Only the methods
 * this Worker actually calls are declared; extend as needed rather than
 * reaching for the full official types package.
 */

/** A cursor over a `SqlStorage.exec` result set. The real runtime's cursor
 *  is also directly iterable and has `one()`/`raw()`/`columnNames`; only
 *  `toArray()` is used here. */
export interface SqlStorageCursor<T> {
  toArray(): T[]
}

/** `ShareDrop`'s only storage dependency: synchronous SQL over the DO's
 *  private SQLite database. Column values (bound in, or read back) are one
 *  of `ArrayBuffer | string | number | null` — the real runtime's
 *  `SqlStorageValue` union — which is why `shareDrop.ts` converts chunk
 *  bytes to/from `ArrayBuffer` at the storage boundary rather than binding
 *  `Uint8Array`s directly. */
export interface SqlStorage {
  exec<T = Record<string, unknown>>(query: string, ...bindings: unknown[]): SqlStorageCursor<T>
}

/** The subset of `DurableObjectStorage` `shareDrop.ts` calls. `setAlarm`/
 *  `getAlarm`/`deleteAlarm`/`deleteAll` are genuinely asynchronous host
 *  calls in the real runtime (unlike `sql.exec`), but the runtime's input/
 *  output gates still serialize them against other requests to the same DO
 *  id — see `shareDrop.ts`'s class doc for why that's what makes
 *  `consume()` one-shot. */
export interface DurableObjectStorage {
  readonly sql: SqlStorage
  setAlarm(scheduledTime: number): void | Promise<void>
  getAlarm(): Promise<number | null>
  deleteAlarm(): void | Promise<void>
  deleteAll(): Promise<void>
}

/** Trimmed down from the real `DurableObjectState` to the one property
 *  `ShareDrop`'s constructor reads. */
export interface DurableObjectState {
  readonly storage: DurableObjectStorage
}

/** Opaque DO identity — `handlers.ts` never inspects this beyond passing it
 *  straight from `idFromName` to `get`. */
export interface DurableObjectId {
  toString(): string
}

/** The RPC surface `ShareDrop` (`src/shareDrop.ts`) exposes — hand-written
 *  here rather than inferred from the class so this shim layer has zero
 *  import dependency on `shareDrop.ts` and stays a leaf, same as the rest
 *  of this file. Real Durable Object RPC (compatibility date ≥
 *  2024-04-03, comfortably true for this Worker's pinned date) exposes any
 *  public method on a DO class as an async call on its stub — this
 *  interface is exactly `ShareDrop`'s six public methods (`dropStore.ts`'s
 *  batched write/read protocol — no single call ever carries a whole drop). */
export interface ShareDropStub {
  store(name: string, chunkCount: number, totalBytes: number, chunks: Uint8Array[]): Promise<void>
  append(chunks: Uint8Array[]): Promise<void>
  consume(): Promise<{ name: string; totalBytes: number; chunkCount: number } | null>
  take(from: number, count: number): Promise<Uint8Array[]>
  destroy(): Promise<void>
  peek(): Promise<{ exists: boolean }>
}

/** The subset of `DurableObjectNamespace<T>` this Worker calls: mint a
 *  deterministic id from the drop token, then get the RPC stub for it. */
export interface DurableObjectNamespace<T> {
  idFromName(name: string): DurableObjectId
  get(id: DurableObjectId): T
}

/** The Worker's env bindings — the one Durable Object namespace
 *  (`wrangler.toml`'s `SHARE_DROP` binding, `new_sqlite_classes`
 *  migration). Replaces the old `BUCKET: R2Bucket` binding entirely — this
 *  Worker no longer touches R2 at all. */
export interface DropEnv {
  SHARE_DROP: DurableObjectNamespace<ShareDropStub>
  /** Optional comma-separated extra CORS origins, merged onto the base
   *  allowlist by `handlers.ts`'s `resolveAllowedOrigins`. Set via a
   *  Cloudflare Worker var (dashboard or `wrangler.toml [vars]`) to let a
   *  self-hosted HTTPS test origin (a homelab `https://hew.granroth.xyz`)
   *  read relay responses. Unset in production. */
  EXTRA_ALLOWED_ORIGINS?: string
  /** Optional upload key (docs/design/self-hosting-relay.md §4). When set,
   *  `PUT /drop` requires `Authorization: Bearer <key>` and the identity
   *  route reports `auth: "bearer"`. A Worker SECRET (`wrangler secret put
   *  HEW_RELAY_UPLOAD_KEY`), never a plaintext var, and unset in production
   *  — the public relay stays open by design. */
  HEW_RELAY_UPLOAD_KEY?: string
}
