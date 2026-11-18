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

/** Why a realm was started: a call to one of its commands, Hew starting
 *  (the person allowed it), or one of the events its manifest lists
 *  (`contributes.events`, with `on_event`). */
export type ActivationReason = 'command' | 'startup' | 'event'

/** Why a realm is being stopped. `idle`: nothing went either way for a
 *  while; its commands stay, and the next call starts it again. */
export type UnloadReason = 'disable' | 'reload' | 'window_closed' | 'idle'

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
      /** The window's slot for the document (`doc_<n>`): what the bus
       *  calls the document. */
      document: string
      /** The document's own persistent id (the `.hew` file's), or null
       *  while it has none (never saved, or a format with no place for
       *  one). What `hew.storage.document` is kept under. */
      document_id: string | null
      /** The host's count of document changes (`DocumentEpoch`) when it
       *  said hello: what the realm stamps its document storage with. */
      document_epoch: number
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
  | { t: 'storage_reply'; id: number; scope: StorageScope; ok: true; value?: string | null; keys?: string[] }
  | { t: 'storage_reply'; id: number; scope: StorageScope; ok: false; error: string }
  /** Keys another window (or a document change) changed. `document`
   *  scope: only for the document the realm has now. */
  | { t: 'storage_changed'; scope: StorageScope; keys: string[] }
  /** The realm's document changed: another one was opened or made, or
   *  the first save gave this one its id. Document storage reads and
   *  writes go to the new one from now on. `epoch` is the host's count
   *  of document changes now (`DocumentEpoch`); the first save of a
   *  document keeps it, since the data moves with the document. */
  | { t: 'document'; document_id: string | null; epoch: number }
  | { t: 'files'; id: number; entries: { path: string; bytes: ArrayBuffer }[]; missing: string[] }
  /** The host is stopping the realm. For `idle`, the realm has 2 s in
   *  which the host still takes storage writes and its `unloaded`;
   *  anything else it sends then is dropped. */
  | { t: 'unload'; reason: UnloadReason }
  | { t: 'ping'; n: number }
  /** The person granted the plugin more while it runs: its connection
   *  holds this grant (JSON text, as in `hello`) from now on. */
  | { t: 'grant'; grant: string }

export type StorageOp = 'get' | 'set' | 'delete' | 'list'

/** Whose storage: the plugin's own (`hew.storage`), or the plugin's for
 *  the document it has now (`hew.storage.document`). */
export type StorageScope = 'plugin' | 'document'

/**
 * The host's count of the document changes in its window: it goes up each
 * time document storage starts to mean another document (one opened or
 * made, or a save that moves to a new id), and stays when the first save
 * of a document gives it its id (its data moves with it). A non-negative
 * integer, at most `Number.MAX_SAFE_INTEGER`.
 */
export type DocumentEpoch = number

export type ConsoleLevel = 'log' | 'info' | 'warn' | 'error'

export type RealmToHost =
  | { t: 'hello_ack' }
  /** A JSON-RPC request. `forward` names the forward the request was made
   *  while serving, by the realm's number for it. */
  | { t: 'bus'; forward?: number; frame: string }
  /** `reply` is a `ForwardReplyJson` as JSON text. */
  | { t: 'forward_reply'; id: number; reply: string }
  | { t: 'forward_progress'; id: number; fraction: number | null; label?: string }
  /** `value` (for `set`) is the stored value as JSON text. `scope` absent
   *  means `plugin`. `epoch` is required for `document` scope and refused
   *  for `plugin` scope: the `DocumentEpoch` the realm had last heard when
   *  the plugin made the call. The host answers one that is not its
   *  current epoch `document_changed` and does nothing, so a call made
   *  for one document never lands in another. */
  | { t: 'storage'; id: number; op: StorageOp; scope?: StorageScope; key?: string; value?: string; epoch?: number }
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
  /** The longest the realm waits, after `unload`, for its unload
   *  listeners and the storage writes they made before it says
   *  `unloaded`; under the host's 2 s for an idle unload. */
  unloadWaitMs: 1_500,
} as const
