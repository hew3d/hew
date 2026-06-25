import { describe, it, expect } from 'vitest'
import { buildSessionRecording, extractInput } from './sessionRecording'
import type { InputEvent } from './inputRecorder'

const HIGH_LEVEL =
  '{"version":2,"calls":[{"method":"begin_ground_sketch"}],"golden_hash":5678}'

const sampleInput: InputEvent[] = [
  {
    kind: 'pointerdown',
    seq: 0,
    t: 0,
    gesture: 1,
    x: 10,
    y: 20,
    button: 0,
    buttons: 1,
    mods: { shift: false, alt: false, ctrl: false, meta: false },
  },
  {
    kind: 'camera',
    seq: 1,
    t: 3,
    gesture: 1,
    position: [8, 6, 8],
    target: [0, 0, 0],
    up: [0, 0, 1],
    fovDeg: 45,
  },
]

describe('buildSessionRecording', () => {
  it('returns the high-level JSON unchanged when there is no input', () => {
    expect(buildSessionRecording(HIGH_LEVEL, [])).toBe(HIGH_LEVEL)
  })

  it('splices input as a top-level sibling array, valid JSON, round-trips', () => {
    const merged = buildSessionRecording(HIGH_LEVEL, sampleInput)
    const obj = JSON.parse(merged)
    expect(obj.version).toBe(2)
    expect(obj.calls).toEqual([{ method: 'begin_ground_sketch' }])
    expect(obj.golden_hash).toBe(5678)
    expect(obj.input).toEqual(sampleInput)
    expect(extractInput(merged)).toEqual(sampleInput)
  })

  it('preserves a u64 golden_hash beyond MAX_SAFE_INTEGER verbatim (no reparse)', () => {
    // 12345678901234567890 is > Number.MAX_SAFE_INTEGER; a parse/stringify round
    // trip would corrupt it. String surgery must leave the digits untouched.
    const big =
      '{"version":2,"calls":[],"golden_hash":12345678901234567890}'
    const merged = buildSessionRecording(big, sampleInput)
    expect(merged).toContain('"golden_hash":12345678901234567890')
    expect(merged).toContain('"input":')
  })

  it('handles trailing whitespace before the closing brace', () => {
    const merged = buildSessionRecording('{"version":2,"calls":[],"golden_hash":1}\n', sampleInput)
    const obj = JSON.parse(merged)
    expect(obj.input).toHaveLength(2)
  })

  it('throws on a non-object high-level payload', () => {
    expect(() => buildSessionRecording('[1,2,3]', sampleInput)).toThrow(
      /not a JSON object/,
    )
  })
})

describe('extractInput', () => {
  it('returns [] when no input array is present', () => {
    expect(extractInput(HIGH_LEVEL)).toEqual([])
  })
})
