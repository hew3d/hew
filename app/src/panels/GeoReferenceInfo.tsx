/**
 * GeoReferenceInfo — a read-only summary of the document's coordinate
 * reference system (kernel `geo.rs`), shown at the head of the Model Info
 * section. Renders nothing for a model with no geo-reference, so a plain
 * document looks as it always did.
 */

import { useMemo } from 'react'
import { useLingui } from '@lingui/react/macro'
import type { Scene } from '../wasm/loader'
import { formatAngle, formatMapValue, parseGeoReference } from '../geo/geoReference'

export interface GeoReferenceInfoProps {
  scene: Scene | null
  /** Bumped on every document change — the re-read trigger. */
  docRev: number
}

export function GeoReferenceInfo({ scene, docRev }: GeoReferenceInfoProps) {
  const { t } = useLingui()
  const geo = useMemo(
    () => (scene ? parseGeoReference(scene.geo_reference_json()) : null),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- docRev is the invalidation signal
    [scene, docRev],
  )
  if (!geo) return null

  const rows: Array<[string, string]> = []
  const add = (label: string, value: string) => {
    if (value) rows.push([label, value])
  }
  add(t`Coordinate system`, geo.name)
  add(t`Datum`, geo.geodeticDatum)
  add(t`Vertical datum`, geo.verticalDatum)
  add(t`Projection`, geo.mapProjection)
  add(t`Zone`, geo.mapZone)
  add(t`Easting`, formatMapValue(geo.easting, geo.mapUnit))
  add(t`Northing`, formatMapValue(geo.northing, geo.mapUnit))
  add(t`Height`, formatMapValue(geo.height, geo.mapUnit))
  add(t`Scale`, String(geo.scale))
  if (geo.xAxisAngle !== null) add(t`X axis`, formatAngle(geo.xAxisAngle))

  return (
    <dl
      aria-label={t`Geo-location`}
      data-testid="geo-reference-info"
      style={{
        display: 'grid',
        gridTemplateColumns: 'auto 1fr',
        gap: '2px 10px',
        margin: '0 0 8px',
        padding: '0 0 8px',
        borderBottom: '1px solid var(--border-subtle, rgba(128,128,128,0.25))',
        fontSize: '12px',
        lineHeight: 1.3,
      }}
    >
      {rows.map(([label, value]) => (
        <div key={label} style={{ display: 'contents' }}>
          <dt style={{ color: 'var(--text-tertiary)' }}>{label}</dt>
          <dd style={{ margin: 0, color: 'var(--text-primary)', overflowWrap: 'anywhere' }}>{value}</dd>
        </div>
      ))}
    </dl>
  )
}
