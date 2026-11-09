import { describe, expect, it } from 'vitest'
import { formatAngle, formatMapValue, parseGeoReference } from './geoReference'

describe('parseGeoReference', () => {
  it('is null for no geo-reference and for junk', () => {
    expect(parseGeoReference('null')).toBeNull()
    expect(parseGeoReference('not json')).toBeNull()
    expect(parseGeoReference('3')).toBeNull()
  })

  it('reads every field and keeps an unrecorded angle and unit factor null', () => {
    const g = parseGeoReference(
      JSON.stringify({
        name: 'Oregon North',
        easting: 2500000.5,
        northing: 250000,
        height: 12,
        xAxisAngle: null,
        scale: 0.9999,
        geodeticDatum: 'NAD83',
        mapProjection: 'LCC',
        mapUnit: 'foot',
        mapUnitMetres: null,
      }),
    )
    expect(g).toMatchObject({
      name: 'Oregon North',
      easting: 2500000.5,
      xAxisAngle: null,
      mapUnitMetres: null,
      geodeticDatum: 'NAD83',
      verticalDatum: '',
    })
  })
})

describe('formatting', () => {
  it('prints a map value without grouping and with its unit', () => {
    expect(formatMapValue(2500000.5, 'foot')).toBe('2500000.5 foot')
    expect(formatMapValue(1.23456, '')).toBe('1.235')
  })

  it('prints an angle in degrees', () => {
    expect(formatAngle(Math.PI / 2)).toBe('90°')
  })
})
