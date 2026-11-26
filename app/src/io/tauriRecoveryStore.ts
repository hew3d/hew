/**
 * TauriRecoveryStore — native desktop implementation of RecoveryStore.
 *
 * Backed by the shell's per-window snapshot store (recovery-<label>.hew +
 * recovery-<label>.json in the app config dir; the pre-multi-window
 * un-suffixed pair is exposed as the "legacy" slot). Five custom Tauri
 * commands: recovery_write (own slot), recovery_list (all slots, newest
 * first), recovery_claim (re-home a slot to this window and read it),
 * recovery_clear (own slot only), recovery_discard_all. See
 * shells/tauri/src-tauri/src/main.rs.
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build — see recoveryStore.ts's makeRecoveryStore().
 */

import { normalizeRecoveryMeta } from './recoveryStore'
import type { RecoveryListing, RecoveryMeta, RecoverySnapshot, RecoveryStore } from './recoveryStore'

interface RecoveryEntry {
  slot: string
  meta: string | null
  modified_ms: number
}

interface RecoveryPayload {
  contents: number[]
  meta: string | null
  modified_ms: number
  path_withheld: boolean
}

/**
 * Parse the RecoveryMeta sidecar JSON, synthesizing a stand-in from the
 * snapshot file's mtime when the sidecar is missing or corrupt — the
 * geometry is still recoverable even when its display name is not.
 */
function parseMeta(raw: string | null, modifiedMs: number): RecoveryMeta {
  if (raw !== null) {
    try {
      return normalizeRecoveryMeta(JSON.parse(raw))
    } catch {
      /* fall through to the synthesized meta */
    }
  }
  return { version: 1, savedAt: modifiedMs, name: 'Untitled', path: null, format: 'hew' }
}

export class TauriRecoveryStore implements RecoveryStore {
  async write(bytes: Uint8Array, meta: RecoveryMeta): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    // Raw-body IPC: a JSON byte array stalls the webview for seconds on a
    // large model, and this runs after every edit.
    await invoke('recovery_write', bytes as never, { headers: { meta: encodeURIComponent(JSON.stringify(meta)) } })
  }

  async writeSource(source: Uint8Array | null): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    if (source === null) {
      await invoke('recovery_source_clear')
      return
    }
    // Raw-body IPC: a source is tens to hundreds of MB, so no JSON array.
    await invoke('recovery_source_write', source as never)
  }

  async list(): Promise<RecoveryListing[]> {
    const { invoke } = await import('@tauri-apps/api/core')
    const entries = await invoke<RecoveryEntry[]>('recovery_list')
    return entries.map((e) => ({
      slot: e.slot,
      meta: parseMeta(e.meta, e.modified_ms),
    }))
  }

  async claim(slot: string): Promise<RecoverySnapshot | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    // Read the source companion BEFORE claiming: claim re-homes the file
    // pair, after which the claimed slot's companion no longer lives there.
    let source: Uint8Array | null = null
    try {
      source = new Uint8Array(await invoke<ArrayBuffer>('recovery_source_read', { slot }))
    } catch {
      source = null // none stored
    }
    const result = await invoke<RecoveryPayload | null>('recovery_claim', { slot })
    if (result == null) return null
    return {
      bytes: new Uint8Array(result.contents),
      meta: parseMeta(result.meta, result.modified_ms),
      source,
      pathWithheld: result.path_withheld,
    }
  }

  async clear(): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('recovery_clear')
  }

  async discardAll(): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('recovery_discard_all')
  }
}
