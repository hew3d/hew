import { describe, expect, it } from 'vitest'
import { parsePreviewFlag } from './parseFlag'

describe('parsePreviewFlag', () => {
  it('reads 1 as on and 0 as off whatever the default', () => {
    expect(parsePreviewFlag('1', false)).toBe(true)
    expect(parsePreviewFlag('0', true)).toBe(false)
  })
  it('falls back to the default when unset or empty', () => {
    expect(parsePreviewFlag(undefined, true)).toBe(true)
    expect(parsePreviewFlag(undefined, false)).toBe(false)
    expect(parsePreviewFlag('', true)).toBe(true)
    expect(parsePreviewFlag('', false)).toBe(false)
  })
  it('rejects any other value with a clear message', () => {
    for (const bad of ['true', 'false', 'on', '2', ' 1', 'yes']) {
      expect(() => parsePreviewFlag(bad, true)).toThrow(/HEW_PREVIEW must be 1.*got/)
    }
  })
})
