import { describe, it, expect, vi, beforeEach } from 'vitest'

const invoke = vi.fn()
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...args: unknown[]) => invoke(...args) }))

import { TauriRecoveryStore } from './tauriRecoveryStore'

describe('TauriRecoveryStore.claim', () => {
  beforeEach(() => invoke.mockReset())

  it.each([true, false])('passes on that the original path was withheld (%s)', async (withheld) => {
    invoke.mockImplementation(async (command: string) => {
      if (command === 'recovery_source_read') throw new Error('none stored')
      return { contents: [1, 2, 3], meta: null, modified_ms: 5, path_withheld: withheld }
    })
    const snapshot = await new TauriRecoveryStore().claim('main')
    expect(snapshot?.pathWithheld).toBe(withheld)
  })
})
