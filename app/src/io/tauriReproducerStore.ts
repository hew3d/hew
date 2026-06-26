/**
 * TauriReproducerStore — native desktop implementation of ReproducerStore.
 *
 * Calls the `reproducer_write` Tauri command, which writes `contents` to
 * `<app_log_dir>/reproducers/<name>` (creating dirs as needed) and returns
 * the absolute path. See shells/tauri/src-tauri/src/main.rs.
 *
 * All Tauri imports are DYNAMIC so this module is never bundled into the web
 * build — see reproducerStore.ts's makeReproducerStore().
 */

import type { ReproducerStore } from './reproducerStore'

export class TauriReproducerStore implements ReproducerStore {
  async write(name: string, json: string): Promise<string | null> {
    const { invoke } = await import('@tauri-apps/api/core')
    return await invoke<string>('reproducer_write', { name, contents: json })
  }
}
