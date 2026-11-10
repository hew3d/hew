import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import type { Scene } from '../wasm/loader'
import { GeoReferenceInfo } from './GeoReferenceInfo'

function sceneWith(json: string): Scene {
  return { geo_reference_json: () => json } as unknown as Scene
}

function renderInfo(json: string) {
  return render(<GeoReferenceInfo scene={sceneWith(json)} docRev={0} />)
}

describe('GeoReferenceInfo', () => {
  it('renders nothing for a model with no geo-reference', () => {
    renderInfo('null')
    expect(screen.queryByTestId('geo-reference-info')).toBeNull()
  })

  it('lists the system, datum, projection and origin', () => {
    renderInfo(
      JSON.stringify({
        name: 'Oregon North',
        geodeticDatum: 'NAD83',
        mapProjection: 'LCC',
        mapUnit: 'foot',
        easting: 2500000.5,
        northing: 250000,
        height: 12,
        scale: 0.9999,
        xAxisAngle: Math.PI / 2,
      }),
    )
    const box = screen.getByTestId('geo-reference-info')
    expect(box.textContent).toContain('Oregon North')
    expect(box.textContent).toContain('NAD83')
    expect(box.textContent).toContain('LCC')
    expect(box.textContent).toContain('2500000.5 foot')
    expect(box.textContent).toContain('90°')
    expect(box.textContent).not.toContain('Vertical datum')
  })
})
