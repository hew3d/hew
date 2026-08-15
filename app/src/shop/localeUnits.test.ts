import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { regionFromLocale, seedLocaleLengthUnit } from './localeUnits'
import { getLengthUnit, setLengthUnit } from '../settings/units'

describe('regionFromLocale', () => {
  it('extracts the region subtag from a well-formed BCP-47 locale', () => {
    expect(regionFromLocale('en-US')).toBe('US')
    expect(regionFromLocale('en-GB')).toBe('GB')
    expect(regionFromLocale('my-MM')).toBe('MM')
  })

  it('returns null for a locale with no region subtag', () => {
    expect(regionFromLocale('en')).toBeNull()
  })

  it('returns null for empty/nullish input', () => {
    expect(regionFromLocale('')).toBeNull()
    expect(regionFromLocale(undefined)).toBeNull()
    expect(regionFromLocale(null)).toBeNull()
  })

  it('returns null rather than throwing on malformed input', () => {
    expect(regionFromLocale('not a locale!!')).toBeNull()
  })
})

describe('seedLocaleLengthUnit', () => {
  // `currentFormat` is a module-level singleton in units.ts, shared across
  // every `it()` in this file (module registries are per-FILE in vitest,
  // not per-test) — clearing localStorage alone leaves whatever a PRIOR
  // test's `setLengthUnit`/seed call left in memory. Reset it back to the
  // real default explicitly, then clear localStorage again so
  // `hasPersistedLengthUnit()` reads false for the next test same as a
  // fresh device would.
  beforeEach(() => {
    localStorage.clear()
    setLengthUnit('m')
    localStorage.clear()
  })
  afterEach(() => {
    localStorage.clear()
  })

  it('seeds Architectural for a US locale on a device with nothing persisted', () => {
    seedLocaleLengthUnit('en-US')
    expect(getLengthUnit()).toBe('arch')
    expect(localStorage.getItem('hew.settings.lengthUnit')).toBe('arch')
  })

  it('seeds Architectural for LR and MM locales too', () => {
    seedLocaleLengthUnit('en-LR')
    expect(getLengthUnit()).toBe('arch')
  })

  it('seeds Architectural for a Myanmar locale', () => {
    seedLocaleLengthUnit('my-MM')
    expect(getLengthUnit()).toBe('arch')
  })

  it('does not seed (leaves units.ts\'s own Meters default) for a metric locale', () => {
    seedLocaleLengthUnit('en-GB')
    expect(getLengthUnit()).toBe('m')
    expect(localStorage.getItem('hew.settings.lengthUnit')).toBeNull()
  })

  it('does not seed when the locale has no resolvable region', () => {
    seedLocaleLengthUnit('en')
    expect(getLengthUnit()).toBe('m')
    expect(localStorage.getItem('hew.settings.lengthUnit')).toBeNull()
  })

  it('never overwrites an already-persisted format, even an imperial locale', () => {
    setLengthUnit('cm') // an explicit prior choice (or an earlier seed)
    seedLocaleLengthUnit('en-US')
    expect(getLengthUnit()).toBe('cm')
  })

  it('is idempotent — a second call after a metric no-op still does not seed', () => {
    seedLocaleLengthUnit('en-GB')
    seedLocaleLengthUnit('en-GB')
    expect(getLengthUnit()).toBe('m')
    expect(localStorage.getItem('hew.settings.lengthUnit')).toBeNull()
  })
})
