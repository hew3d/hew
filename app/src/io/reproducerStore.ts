/**
 * ReproducerStore — abstracts the platform's "write a reproducer bundle to
 * disk" storage (docs/DEVELOPMENT.md — the auto-reproducer dump).
 *
 * Mirrors the RecoveryStore seam (./recoveryStore.ts): WebReproducerStore is
 * statically imported (always needed for the web build), while under Tauri
 * the TauriReproducerStore is imported dynamically so its Tauri-only imports
 * never enter the web bundle.
 *
 * Unlike RecoveryStore (one fixed slot, overwritten), each dump gets its own
 * named file — `name` is expected to be unique per call (e.g. a timestamped
 * `reproducer-<ISO>.json`), so callers never collide or overwrite a prior
 * bundle.
 */

export interface ReproducerStore {
  /**
   * Persist `json` under `name`. Returns the absolute path on Tauri (where
   * the bundle lands in the app log dir), or null on web (where `write`
   * instead triggers a browser download — there is no addressable path).
   * Must never throw — callers are failure handlers and can't afford to.
   */
  write(name: string, json: string): Promise<string | null>
}

// WebReproducerStore is statically imported — it's always needed for the web build.
import { WebReproducerStore } from './webReproducerStore'

/**
 * True when running inside the Tauri desktop shell.
 * Re-derived here (rather than imported from fileHost.ts) to keep this module
 * self-contained — both checks read the same `window.__TAURI_INTERNALS__` flag.
 */
const isTauri: boolean =
  typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

/**
 * Return the appropriate ReproducerStore for the current platform.
 *
 * Under Tauri, TauriReproducerStore is imported dynamically (lazy) so its
 * Tauri-only imports never appear in the web bundle. makeReproducerStore() is
 * called once (in a module-level ref) so we return a synchronous shim that
 * defers to TauriReproducerStore once loaded.
 */
export function makeReproducerStore(): ReproducerStore {
  if (isTauri) {
    const storePromise = import('./tauriReproducerStore').then(
      (m) => new m.TauriReproducerStore(),
    )
    return {
      write: (name, json) => storePromise.then((s) => s.write(name, json)),
    }
  }
  return new WebReproducerStore()
}
