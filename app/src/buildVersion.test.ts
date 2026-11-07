import { describe, expect, it } from 'vitest'
import { resolveBuildStamp } from './buildVersion'

describe('resolveBuildStamp', () => {
  it('keeps the plain version when the sha is absent, empty or whitespace', () => {
    for (const sha of [undefined, '', '   ', '\t\n']) {
      expect(resolveBuildStamp('1.3.1', sha)).toEqual({ version: '1.3.1', beta: false })
    }
  })

  it('appends +beta.<sha> when the sha is present', () => {
    expect(resolveBuildStamp('1.3.1', '18eb47ad')).toEqual({
      version: '1.3.1+beta.18eb47ad',
      beta: true,
    })
  })

  it('trims the sha', () => {
    expect(resolveBuildStamp('1.3.1', '  deadbeef\n').version).toBe('1.3.1+beta.deadbeef')
  })

  it('accepts hyphens and mixed case', () => {
    expect(resolveBuildStamp('1.3.1', 'Ab-9').version).toBe('1.3.1+beta.Ab-9')
  })

  it('throws on characters outside [0-9A-Za-z-], dots included', () => {
    for (const sha of ['dead.beef', 'dead beef', 'dead+beef', 'dead_beef', 'dead/beef', 'été']) {
      expect(() => resolveBuildStamp('1.3.1', sha)).toThrow(/HEW_BETA_SHA/)
    }
  })
})
