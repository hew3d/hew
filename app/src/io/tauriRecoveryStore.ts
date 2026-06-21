/**
 * TauriRecoveryStore — native desktop implementation of RecoveryStore.
 *
 * Calls three custom Tauri commands (recovery_write/read/clear) which persist
 * two files in the app config dir: recovery.hew (geometry bytes) and
 * recovery.json (the meta string verbatim). See shells/tauri/src-tauri/src/main.rs.
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build — see recoveryStore.ts's makeRecoveryStore().
 */

import type { RecoveryMeta, RecoverySnapshot, RecoveryStore } from './recoveryStore'

interface RecoveryPayload {
  contents: number[]
  meta: string
}

export class TauriRecoveryStore implements RecoveryStore {
  async write(bytes: Uint8Array, meta: RecoveryMeta): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('recovery_write', {
      contents: Array.from(bytes),
      meta: JSON.stringify(meta),
    })
  }

  async read(): Promise<RecoverySnapshot | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    const result = await invoke<RecoveryPayload | null>('recovery_read')
    if (result == null) return null
    let meta: RecoveryMeta
    try {
      meta = JSON.parse(result.meta) as RecoveryMeta
    } catch {
      // Malformed meta — treat as no recoverable snapshot.
      return null
    }
    return { bytes: new Uint8Array(result.contents), meta }
  }

  async clear(): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('recovery_clear')
  }
}
