/**
 * The one place the pane reaches the web host's state module (`hostState.ts`),
 * so the pane's own data code stays free of that module's exact names. Loaded
 * lazily by `paneData.ts`; preview-only like everything beside it.
 */

import type { HostStateSource } from './paneData'

/** The host's state module as the pane reads it. */
export async function hostStateSource(): Promise<HostStateSource> {
  const m = await import('./hostState')
  return {
    hostStates: () => m.hostStates(),
    subscribeHostStates: (fn) => m.subscribeHostStates(fn),
    consoleLines: (id) => m.consoleLines(id),
    subscribeConsole: (id, fn) => m.subscribeConsole(id, fn),
    activity: (id) => m.activity(id),
  }
}
