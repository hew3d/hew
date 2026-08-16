/**
 * relayClient — the desktop's typed wrappers over the four Rust "Open on
 * Phone" relay commands (shells/tauri/src-tauri/src/relay_client.rs;
 * docs/design/self-hosting-relay.md §3). None of these takes a URL: the
 * Rust side reads the configured server (Settings ▸ Advanced ▸ Server,
 * `settings/server.ts`) and derives `<origin>/relay/…` itself, so nothing in
 * the webview can ever aim a request anywhere else — that is the whole point
 * of moving the requests out of `tauri-plugin-http`, whose scope would have
 * had to admit every self-hosted origin (and whose bundled TLS roots could
 * not trust a homelab CA).
 *
 * Tauri-only: `PhoneShareDialog.tsx` is the consumer, and it is only ever
 * rendered under the desktop shell. `@tauri-apps/api/core` is imported
 * dynamically so nothing here lands in the web bundle's entry chunk. A
 * failed command rejects with the Rust `RelayError` (`{ kind, message,
 * status? }`), surfaced here as a `RelayError` instance so callers can
 * `instanceof` and switch on `kind` for the specific wording the dialog
 * shows.
 */

/** Mirrors `RelayErrorKind` in relay_client.rs (serde camelCase). */
export type RelayErrorKind =
  | 'invalidOrigin'
  | 'io'
  | 'unreachable'
  | 'tls'
  | 'unauthorized'
  | 'full'
  | 'tooLarge'
  | 'notARelay'
  | 'status'

export class RelayError extends Error {
  readonly kind: RelayErrorKind
  readonly status: number | undefined

  constructor(kind: RelayErrorKind, message: string, status?: number) {
    super(message)
    this.name = 'RelayError'
    this.kind = kind
    this.status = status
  }
}

const KINDS: ReadonlySet<string> = new Set<RelayErrorKind>([
  'invalidOrigin',
  'io',
  'unreachable',
  'tls',
  'unauthorized',
  'full',
  'tooLarge',
  'notARelay',
  'status',
])

/** Turns whatever `invoke` rejected with into a `RelayError`. A typed Rust
 *  error arrives as a plain object; anything else (an IPC-level failure, a
 *  string) is reported as `unreachable` — the command never ran to a
 *  conclusion, which from the user's seat is the same thing. */
export function toRelayError(err: unknown): RelayError {
  if (err instanceof RelayError) return err
  if (typeof err === 'object' && err !== null && 'kind' in err) {
    const kind = (err as { kind: unknown }).kind
    const message = (err as { message?: unknown }).message
    const status = (err as { status?: unknown }).status
    if (typeof kind === 'string' && KINDS.has(kind)) {
      return new RelayError(
        kind as RelayErrorKind,
        typeof message === 'string' ? message : kind,
        typeof status === 'number' ? status : undefined,
      )
    }
  }
  return new RelayError('unreachable', err instanceof Error ? err.message : String(err))
}

/** Mirrors `RelayIdentity` in relay_client.rs. */
export interface RelayIdentity {
  origin: string
  service: string
  contract: number
  maxBytes: number
  ttlMs: number
  auth: 'none' | 'bearer' | string
  /** Only present when the server wants a key AND this desktop has one
   *  configured: whether the probe was let past the key check. */
  keyAccepted?: boolean
}

async function invokeRelay<T>(command: string, args?: unknown): Promise<T> {
  const { invoke } = await import('@tauri-apps/api/core')
  try {
    return await invoke<T>(command, args as never)
  } catch (err) {
    throw toRelayError(err)
  }
}

/** *Test connection* / the identity route: what the configured server says
 *  it is. Rejects with a `RelayError` naming the specific problem. */
export function relayIdentity(): Promise<RelayIdentity> {
  return invokeRelay<RelayIdentity>('relay_identity')
}

/** Uploads ciphertext to the configured relay's `/drop`; resolves with the
 *  drop token. `bytes` ride the invoke as a RAW body (Tauri's byte-array
 *  argument form), never a JSON array. */
export function relayPut(bytes: Uint8Array): Promise<{ token: string }> {
  return invokeRelay<{ token: string }>('relay_put', bytes)
}

/** Non-consuming existence check (`HEAD /drop/<token>`). Rejects on any
 *  answer other than 200/404 — the poll treats that as a missed tick. */
export function relayPeek(token: string): Promise<'present' | 'gone'> {
  return invokeRelay<'present' | 'gone'>('relay_peek', { token })
}

/** Best-effort invalidation (`DELETE /drop/<token>`). */
export function relayDelete(token: string): Promise<void> {
  return invokeRelay<void>('relay_delete', { token })
}
