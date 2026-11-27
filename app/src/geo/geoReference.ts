/**
 * The document's geo-reference (kernel `geo.rs`): the coordinate reference
 * system a model is placed in. Read-only in the app; `Scene.
 * geo_reference_json()` is the source, `hew.geo.set` the way to change it.
 */

export interface GeoReference {
  name: string
  description: string
  /** The model origin, in the system's own map unit. */
  easting: number
  northing: number
  height: number
  /** Radians counter-clockwise from map east; null when the file records none. */
  xAxisAngle: number | null
  scale: number
  geodeticDatum: string
  verticalDatum: string
  mapProjection: string
  mapZone: string
  /** The unit's name as the file holds it (`METRE`, `foot`, ...). */
  mapUnit: string
  mapUnitMetres: number | null
  tglId: string
  calibrationFileName: string
}

/** Parses `Scene.geo_reference_json()`; null for "no geo-reference" or JSON that is not one. */
export function parseGeoReference(json: string): GeoReference | null {
  let v: unknown
  try {
    v = JSON.parse(json)
  } catch {
    return null
  }
  if (typeof v !== 'object' || v === null) return null
  const o = v as Record<string, unknown>
  const str = (k: string) => (typeof o[k] === 'string' ? (o[k] as string) : '')
  const num = (k: string) => (typeof o[k] === 'number' ? (o[k] as number) : 0)
  const opt = (k: string) => (typeof o[k] === 'number' ? (o[k] as number) : null)
  return {
    name: str('name'),
    description: str('description'),
    easting: num('easting'),
    northing: num('northing'),
    height: num('height'),
    xAxisAngle: opt('xAxisAngle'),
    scale: num('scale'),
    geodeticDatum: str('geodeticDatum'),
    verticalDatum: str('verticalDatum'),
    mapProjection: str('mapProjection'),
    mapZone: str('mapZone'),
    mapUnit: str('mapUnit'),
    mapUnitMetres: opt('mapUnitMetres'),
    tglId: str('tglId'),
    calibrationFileName: str('calibrationFileName'),
  }
}

/** A map position or height to the precision a survey grid needs, with the unit's name when it has one. */
export function formatMapValue(value: number, unit: string): string {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a locale tag, not copy
  const text = value.toLocaleString('en-US', { useGrouping: false, maximumFractionDigits: 3 })
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a number, a space and the file's own unit name
  return unit ? `${text} ${unit}` : text
}

/** The x axis angle in degrees, to two places. */
export function formatAngle(radians: number): string {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a locale tag and the degree sign, not copy
  return `${((radians * 180) / Math.PI).toLocaleString('en-US', { useGrouping: false, maximumFractionDigits: 2 })}°`
}
