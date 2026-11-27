import { describe, expect, it } from 'vitest'
import { historyTrimNote, takeHistoryTrim } from './historyTrim'

describe('historyTrim', () => {
  it('says how many steps are kept, in plain words', () => {
    expect(historyTrimNote(12)).toBe('Undo history trimmed to save memory: 12 steps kept')
    expect(historyTrimNote(1)).toBe('Undo history trimmed to save memory: 1 step kept')
  })

  it('collects a trim once and frees it', () => {
    let freed = 0
    let pending: { kept(): number; free(): void } | undefined = { kept: () => 3, free: () => { freed += 1 } }
    const scene = {
      take_history_trim: () => {
        const t = pending
        pending = undefined
        return t
      },
    }
    expect(takeHistoryTrim(scene)).toBe(3)
    expect(freed).toBe(1)
    expect(takeHistoryTrim(scene)).toBeNull()
  })
})
