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

  it('takes the release version over package.json when one is stamped', () => {
    expect(resolveBuildStamp('1.4.0', undefined, '1.5.0-pre.2')).toEqual({
      version: '1.5.0-pre.2',
      beta: false,
    })
    expect(resolveBuildStamp('1.4.0', 'ab12', '1.5.0').version).toBe('1.5.0+beta.ab12')
  })

  it('ignores an empty release stamp and rejects a malformed one', () => {
    for (const v of [undefined, '', '  ']) {
      expect(resolveBuildStamp('1.4.0', undefined, v).version).toBe('1.4.0')
    }
    for (const v of ['v1.5.0', '1.5', '1.5.0+x', 'latest', '01.5.0', '1.05.0-pre.1']) {
      expect(() => resolveBuildStamp('1.4.0', undefined, v)).toThrow(/HEW_RELEASE_VERSION/)
    }
  })
})
