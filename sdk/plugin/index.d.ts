/**
 * Types for a Hew plugin's background script. The runtime defines `hew` and
 * `HewRefusal` before the plugin's `main` runs; `main` is one classic script
 * (bundle to a plain script or an IIFE: `import` and `export` at the top
 * level are not allowed in it).
 */

export type ActivationReason = 'command' | 'startup'

export interface HandlerContext {
  /** Who called the command. */
  caller: { identity: string; plugin: string | null; app: string | null; forward: number | null }
  /** Aborts when the caller cancels, the call times out, or the plugin stops. */
  signal: AbortSignal
  /** Reports how far along the command is (0 to 1, or null when unknown). */
  progress(fraction: number | null, label?: string): void
  document: string
  activation: ActivationReason | null
}

export type CommandHandler = (params: any, ctx: HandlerContext) => unknown

export interface PluginInfo {
  id: string
  version: string
  grant: unknown
  api: 1
  document: string
  features: string[]
}

export interface Hew {
  /** Resolves once the host has introduced itself; `plugin`, `locale` and
   *  `theme` are null before. */
  readonly ready: Promise<void>
  readonly plugin: PluginInfo | null
  readonly locale: string | null
  readonly theme: 'light' | 'dark' | null
  /** The generated API client (`hew.api.query.scene({})`, ...). */
  readonly api: any
  /** One `hew.doc.transact` envelope; `expect` defaults to the generation of the last read. */
  transact(commands: { method: string; params?: object; as?: string }[], opts?: { label?: string; expect?: number; coalesce?: string }): Promise<unknown>
  /** Reads the scene, builds commands from it, transacts them; one retry when the document moved meanwhile. */
  commit(fn: (doc: any) => { method: string; params?: object }[] | Promise<{ method: string; params?: object }[]>, opts?: { label?: string }): Promise<unknown>
  readonly commands: {
    /** Answers the plugin's command `local` (declared in `contributes.commands`). */
    handle(local: string, handler: CommandHandler): void
  }
  readonly events: {
    on(name: string, fn: (params: any) => void): void
    off(name: string, fn: (params: any) => void): void
  }
  /** Plugin-wide storage: JSON values by string key, 5 MiB in all. */
  readonly storage: {
    get(key: string): Promise<unknown>
    set(key: string, value: unknown): Promise<void>
    delete(key: string): Promise<void>
    list(): Promise<string[]>
    onChange(fn: (keys: string[]) => void): () => void
  }
  /** The plugin's own packaged files. */
  readonly assets: {
    bytes(path: string): Promise<ArrayBuffer>
    url(path: string): Promise<string>
  }
  readonly lifecycle: { on(event: 'unload', fn: (reason: string) => void): void }
  supports(name: string): boolean
}

/** A refusal a handler answers with; `name` is snake_case. */
export declare class HewRefusal extends Error {
  readonly explanation: string
  readonly detail: unknown
  constructor(name: string, explanation: string, detail?: unknown)
}

export declare const hew: Hew
