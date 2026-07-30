import { describe, it, expect } from 'vitest'
import { eyeHeightCapturesKey, eyeHeightHandleKey } from './cameraEyeHeightVcb'

describe('eyeHeightCapturesKey', () => {
  it('always captures digits and the decimal point, even with an empty buffer', () => {
    expect(eyeHeightCapturesKey('', '1')).toBe(true)
    expect(eyeHeightCapturesKey('', '.')).toBe(true)
    expect(eyeHeightCapturesKey('', '5')).toBe(true)
  })

  it('does NOT capture a bare letter shortcut while the buffer is empty (M for Move, etc.)', () => {
    expect(eyeHeightCapturesKey('', 'm')).toBe(false)
    expect(eyeHeightCapturesKey('', 'M')).toBe(false)
  })

  it('does NOT capture Space while the buffer is empty (Spc switches to Select)', () => {
    expect(eyeHeightCapturesKey('', ' ')).toBe(false)
  })

  it('does NOT capture Enter/Backspace while the buffer is empty (nothing to commit/delete)', () => {
    expect(eyeHeightCapturesKey('', 'Enter')).toBe(false)
    expect(eyeHeightCapturesKey('', 'Backspace')).toBe(false)
  })

  it('captures a unit-suffix letter, Enter, and Backspace once the buffer has started', () => {
    expect(eyeHeightCapturesKey('1', 'm')).toBe(true)
    expect(eyeHeightCapturesKey('1', 'Enter')).toBe(true)
    expect(eyeHeightCapturesKey('1', 'Backspace')).toBe(true)
  })
})

describe('eyeHeightHandleKey', () => {
  it('builds up a typed buffer digit by digit', () => {
    let r = eyeHeightHandleKey('', '1', 'm')
    expect(r.typed).toBe('1')
    r = eyeHeightHandleKey(r.typed, '.', 'm')
    expect(r.typed).toBe('1.')
    r = eyeHeightHandleKey(r.typed, '8', 'm')
    expect(r.typed).toBe('1.8')
    expect(r.committed).toBeNull()
  })

  it('Enter commits a valid positive value and clears the buffer', () => {
    const r = eyeHeightHandleKey('1.8', 'Enter', 'm')
    expect(r.committed).toBeCloseTo(1.8, 9)
    expect(r.typed).toBe('')
    expect(r.readout).toBe('')
  })

  it('Enter on an empty/invalid buffer commits nothing and clears it', () => {
    const empty = eyeHeightHandleKey('', 'Enter', 'm')
    expect(empty.committed).toBeNull()

    const garbage = eyeHeightHandleKey('abc', 'Enter', 'm')
    expect(garbage.committed).toBeNull()
  })

  it('Enter on a non-positive value commits nothing (eye height must be > 0)', () => {
    const zero = eyeHeightHandleKey('0', 'Enter', 'm')
    expect(zero.committed).toBeNull()
  })

  it('an unrecognized key is a no-op', () => {
    const r = eyeHeightHandleKey('1.5', 'q', 'm')
    expect(r.typed).toBe('1.5')
    expect(r.committed).toBeNull()
  })
})
