/**
 * TauriLogFileStore — native desktop implementation of LogFileStore.
 *
 * Calls two custom Tauri commands (log_append/log_rotate) which write a
 * rolling diagnostic.log file in the app log dir, with one backup
 * (diagnostic.1.log) kept on rotation. See
 * shells/tauri/src-tauri/src/main.rs.
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build — see logFileStore.ts's makeLogFileStore().
 */

import type { LogFileStore } from './logFileStore'

export class TauriLogFileStore implements LogFileStore {
  async append(ndjson: string): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('log_append', { lines: ndjson })
  }

  async rotateIfNeeded(): Promise<void> {
    const { invoke } = await import('@tauri-apps/api/core')
    await invoke('log_rotate')
  }

  async path(): Promise<string | null> {
    const { appLogDir } = await import('@tauri-apps/api/path')
    try {
      const dir = await appLogDir()
      return `${dir}/diagnostic.log`
    } catch {
      return null
    }
  }
}
