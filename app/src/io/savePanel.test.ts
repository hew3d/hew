import { describe, expect, it } from 'vitest'
import { scalePresetsFor } from '../print/scale'
import {
  DEFAULT_STL_SEGMENTS,
  EXPORT_FORMATS,
  STL_RESOLUTIONS,
  defaultSvgScaleIndex,
  exportChoiceFrom,
  exportPanelFormats,
  panelIdFor,
  saveCompatFrom,
  saveFormatFrom,
  savePanelFormats,
} from './savePanel'
import { SKP_STREAM_FORMATS } from './docFormat'

function byIdCompat(formats: ReturnType<typeof exportPanelFormats>) {
  return formats.find((f) => f.id === 'skp')?.options
}

describe('the native save panel\'s formats', () => {
  it('Save As lists Hew, the 2021-and-later container and one older-versions entry', () => {
    const formats = savePanelFormats()
    expect(formats.map((f) => [f.id, f.extension])).toEqual([
      ['hew', 'hew'],
      ['skp2026', 'skp'],
      ['skp-older', 'skp'],
    ])
    expect(formats[1].label).toBe('SketchUp 2021 and later (.skp)')
    // The compat checkbox belongs to the 2021-and-later container alone.
    expect(formats[0].options).toEqual([])
    expect(formats[1].options).toEqual([
      { kind: 'check', id: 'compat', label: 'Avoid features older SketchUp versions cannot open', value: false },
    ])
    const older = formats[2].options?.[0]
    expect(older?.kind === 'select' && older.choices.map((c) => c.value)).toEqual([...SKP_STREAM_FORMATS])
    expect(older?.kind === 'select' && older.choices.find((c) => c.value === 'skp8')?.label).toBe('SketchUp 8')
    // Preselected to the file's own release, and the compat box to its state.
    expect(older).toMatchObject({ id: 'release', value: 'skp2020' })
    expect(savePanelFormats('skp2013')[2].options?.[0]).toMatchObject({ value: 'skp2013' })
    expect(savePanelFormats('skp2026', true)[1].options?.[0]).toMatchObject({ id: 'compat', value: true })
    expect(panelIdFor('skp2013')).toBe('skp-older')
    expect(panelIdFor('skp2026')).toBe('skp2026')
    expect(panelIdFor('hew')).toBe('hew')
  })

  it('reads the format Save As picked, the release of the older entry included', () => {
    expect(saveFormatFrom({ path: '/a.skp', format: 'skp-older', options: { release: 'skp8' } })).toBe('skp8')
    expect(saveFormatFrom({ path: '/a.skp', format: 'skp-older', options: { release: 'skp2013' } })).toBe('skp2013')
    expect(saveFormatFrom({ path: '/a.skp', format: 'skp-older', options: {} })).toBe('skp2020')
    expect(saveFormatFrom({ path: '/a.skp', format: 'skp2026', options: {} })).toBe('skp2026')
    expect(saveFormatFrom({ path: '/a.hew', format: 'anything', options: {} })).toBe('hew')
    expect(saveCompatFrom({ path: '/a.skp', format: 'skp2026', options: { compat: true } })).toBe(true)
    expect(saveCompatFrom({ path: '/a.skp', format: 'skp2026', options: { compat: false } })).toBe(false)
    // A stale compat value from another entry never applies to it.
    expect(saveCompatFrom({ path: '/a.skp', format: 'skp-older', options: { compat: true } })).toBe(false)
  })

  it('Export lists every format the in-app dialog does, STL and SVG with their options', () => {
    const formats = exportPanelFormats('mm')
    expect(formats.map((f) => f.id)).toEqual([
      ...EXPORT_FORMATS.filter((f) => f.group === undefined).map((f) => f.format),
      'skp-older',
    ])
    expect(formats.map((f) => f.id)).toEqual(['glb', 'stl', '3mf', 'usdz', 'svg', 'skp', 'skp-older'])
    expect(byIdCompat(formats)).toEqual([{ kind: 'check', id: 'compat', label: 'Avoid features older SketchUp versions cannot open', value: false }])
    const byId = Object.fromEntries(formats.map((f) => [f.id, f]))
    expect(byId.glb.options).toEqual([])
    const stl = byId.stl.options ?? []
    expect(stl).toHaveLength(1)
    expect(stl[0]).toMatchObject({ kind: 'select', id: 'segments', value: String(DEFAULT_STL_SEGMENTS) })
    expect(stl[0].kind === 'select' && stl[0].choices.map((c) => c.value)).toEqual(
      STL_RESOLUTIONS.map((r) => String(r.value)),
    )
    expect((byId.svg.options ?? []).map((o) => [o.kind, o.id])).toEqual([
      ['select', 'view'],
      ['select', 'scale'],
      ['check', 'hidden'],
      ['check', 'dimensions'],
    ])
  })
})

describe('an export from the panel\'s answer', () => {
  it('carries the STL resolution picked', () => {
    const choice = exportChoiceFrom({ path: '/m.stl', format: 'stl', options: { segments: '96' } }, 'mm')
    expect(choice).toEqual({ format: 'stl', skpCompat: false, stlSegmentsPerTurn: 96 })
  })

  it('carries the older release and the compat box picked', () => {
    expect(exportChoiceFrom({ path: '/m.skp', format: 'skp-older', options: { release: 'skp2017' } }, 'mm')).toMatchObject({
      format: 'skp2017',
      skpCompat: false,
    })
    expect(exportChoiceFrom({ path: '/m.skp', format: 'skp', options: { compat: true } }, 'mm')).toMatchObject({
      format: 'skp',
      skpCompat: true,
    })
    expect(exportChoiceFrom({ path: '/m.skp', format: 'skp', options: {} }, 'mm')?.skpCompat).toBe(false)
    // A release is reached through the older entry, not by its own id.
    expect(exportChoiceFrom({ path: '/m.skp', format: 'skp2017', options: {} }, 'mm')).toBeNull()
  })

  it('carries the SVG view, scale and switches picked', () => {
    const presets = scalePresetsFor('mm')
    const choice = exportChoiceFrom(
      {
        path: '/m.svg',
        format: 'svg',
        options: { view: 'top', scale: '2', hidden: true, dimensions: false },
      },
      'mm',
    )
    expect(choice?.svg).toEqual({
      view: 'top',
      scale: presets[2],
      hiddenDashed: true,
      includeDimensions: false,
    })
  })

  it('falls back to the defaults for values it does not know, and refuses an unknown format', () => {
    const presets = scalePresetsFor('arch')
    const choice = exportChoiceFrom(
      { path: '/m.svg', format: 'svg', options: { view: 'sideways', scale: '999', segments: 'many' } },
      'arch',
    )
    expect(choice?.stlSegmentsPerTurn).toBe(DEFAULT_STL_SEGMENTS)
    expect(choice?.svg?.view).toBe('current')
    expect(choice?.svg?.scale).toEqual(presets[defaultSvgScaleIndex(presets)])
    expect(choice?.svg?.includeDimensions).toBe(true)
    expect(exportChoiceFrom({ path: '/m.obj', format: 'obj', options: {} }, 'mm')).toBeNull()
  })
})
