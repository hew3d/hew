/**
 * The messages a plugin realm and the Hew host exchange over the realm's
 * `MessagePort`. Shared by the SDK runtime (inside the Worker), the boot
 * script (inside the sandboxed frame) and the host (the app, which imports
 * this file for types only and checks every message with its own
 * validator).
 *
 * Every field whose size the sender chooses is a string or an
 * `ArrayBuffer`, never a nested object: the receiver measures a message by
 * adding up lengths, and never has to walk or stringify something a
 * plugin built. Values that are JSON (command params, results, stored
 * values, the grant) cross as JSON text.
 *
 * There are two channels:
 *
 *  - the control port, between the host and the boot script in the frame
 *    (`HostToBoot`, `BootToHost`), used once to start the Worker and then
 *    only for the Worker's uncaught errors;
 *  - the realm port, between the host and the SDK runtime in the Worker
 *    (`HostToRealm`, `RealmToHost`), which carries everything else.
 */

/** Why a realm was started. */
export type ActivationReason = 'command' | 'startup'

/** Why a realm is being stopped. */
export type UnloadReason = 'disable' | 'reload' | 'window_closed'

/** A forward reply as `Scene.api_complete_forward` takes it. */
export type ForwardReplyJson =
  | { ok: unknown }
  | { refusal: { name: string; explanation: string; detail?: unknown } }
  | { failed: 'timeout' | 'crashed' | 'unavailable' | 'no_handler' | 'cancelled' }

// ------------------------------------------------------------ control port

/** Host to boot script, once, after the handshake: the plugin's `main` as
 *  bytes (transferred) and, in the transfer list, the realm port for the
 *  Worker. */
export interface BootStart {
  t: 'boot'
  code: ArrayBuffer
}

export type HostToBoot = BootStart

/** Boot script to host. */
export type BootToHost =
  /** The Worker was constructed and handed its port. */
  | { t: 'booted' }
  /** The Worker failed to start, or threw an error nothing caught. */
  | { t: 'error'; message: string }

// -------------------------------------------------------------- realm port

export type HostToRealm =
  | {
      t: 'hello'
      plugin: string
      version: string
      realm: 'background'
      document: string
      locale: string
      theme: 'light' | 'dark'
      /** The connection's grant, as JSON text. */
      grant: string
      api: 1
      activation: ActivationReason
      features: string[]
    }
  /** A reply frame to a request the realm sent, or an event frame. */
  | { t: 'bus'; frame: string }
  | {
      t: 'forward'
      /** The realm's own number for this forward (never a kernel token). */
      id: number
      local: string
      /** The command's params, as JSON text. */
      params: string
      /** Who called, as JSON text (`ForwardCaller`). */
      caller: string
      timeout_ms: number
    }
  | { t: 'forward_cancel'; id: number }
  | { t: 'storage_reply'; id: number; ok: true; value?: string | null; keys?: string[] }
  | { t: 'storage_reply'; id: number; ok: false; error: string }
  | { t: 'storage_changed'; scope: 'plugin'; keys: string[] }
  | { t: 'files'; id: number; entries: { path: string; bytes: ArrayBuffer }[]; missing: string[] }
  | { t: 'unload'; reason: UnloadReason }
  | { t: 'ping'; n: number }

export type StorageOp = 'get' | 'set' | 'delete' | 'list'

export type ConsoleLevel = 'log' | 'info' | 'warn' | 'error'

export type RealmToHost =
  | { t: 'hello_ack' }
  /** A JSON-RPC request. `forward` names the forward the request was made
   *  while serving, by the realm's number for it. */
  | { t: 'bus'; forward?: number; frame: string }
  /** `reply` is a `ForwardReplyJson` as JSON text. */
  | { t: 'forward_reply'; id: number; reply: string }
  | { t: 'forward_progress'; id: number; fraction: number | null; label?: string }
  /** `value` (for `set`) is the stored value as JSON text. */
  | { t: 'storage'; id: number; op: StorageOp; key?: string; value?: string }
  | { t: 'files'; id: number; paths: string[] }
  /** Each argument already turned into text and cut by the runtime. */
  | { t: 'console'; level: ConsoleLevel; args: string[]; at: number }
  | { t: 'pong'; n: number }
  | { t: 'unloaded' }
  | { t: 'error'; message: string; stack?: string }

/** The `t` values a realm may send. */
export const REALM_TO_HOST_TYPES = [
  'hello_ack',
  'bus',
  'forward_reply',
  'forward_progress',
  'storage',
  'files',
  'console',
  'pong',
  'unloaded',
  'error',
] as const

/**
 * What the runtime cuts before sending, so a well-behaved plugin never trips
 * the host's limits. The host checks its own (larger or equal) limits on
 * every message regardless.
 */
export const RUNTIME_LIMITS = {
  /** Characters kept of one console argument. */
  consoleArgChars: 2_000,
  /** Console arguments kept per call. */
  consoleArgs: 16,
  /** Console calls sent per second; the rest are counted and reported once. */
  consolePerSecond: 50,
  /** Characters kept of an error message and of its stack. */
  errorMessageChars: 2_000,
  errorStackChars: 8_000,
  /** Characters kept of a progress label. */
  progressLabelChars: 200,
  /** The longest frame or reply the runtime sends, in characters. */
  frameChars: 1_000_000,
  /** A storage key, in UTF-8 bytes; longer keys are refused
   *  `storage_key_invalid` before anything is sent. */
  storageKeyBytes: 256,
  /** A stored value's JSON text, in characters; longer values are refused
   *  `storage_value_too_large`. With a key of at most 256 characters a set
   *  stays inside the host's 1 MiB (1,048,576 characters) per message. */
  storageValueChars: 1_000_000,
  /** Storage requests and file requests open at once; more wait in the
   *  runtime. */
  storageOpen: 32,
  filesOpen: 8,
  /** The longest package path asked for. */
  pathChars: 1_024,
  /** Characters kept of a refusal's explanation. */
  refusalExplanationChars: 2_000,
} as const
