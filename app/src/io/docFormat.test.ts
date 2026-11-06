import { describe, expect, it } from 'vitest'
import {
  saveFormatForPath,
  SAVE_FORMAT_CHOICES,
  SKP_STREAM_FORMATS,
  extensionFor,
  formatDescription,
  isSkpFormat,
  nameForFormat,
  normalizeDocFormat,
  skpTarget,
  olderReleaseFor,
  olderReleaseFrom,
  takesCompat,
  stripKnownExtension,
} from './docFormat'

describe('stripKnownExtension', () => {
  it('removes one trailing known extension, case-insensitively', () => {
    expect(stripKnownExtension('Model.skp')).toBe('Model')
    expect(stripKnownExtension('Model.SKP')).toBe('Model')
    expect(stripKnownExtension('Chair.hew')).toBe('Chair')
    expect(stripKnownExtension('Kitchen.dae')).toBe('Kitchen')
    expect(stripKnownExtension('Part.stl')).toBe('Part')
    expect(stripKnownExtension('Scan.glb')).toBe('Scan')
    expect(stripKnownExtension('Scan.gltf')).toBe('Scan')
  })

  it('leaves other names alone and only strips the last extension', () => {
    expect(stripKnownExtension('Untitled')).toBe('Untitled')
    expect(stripKnownExtension('notes.txt')).toBe('notes.txt')
    expect(stripKnownExtension('Model.skp.hew')).toBe('Model.skp')
    expect(stripKnownExtension('v1.2 plan')).toBe('v1.2 plan')
  })
})

describe('nameForFormat', () => {
  it('swaps the extension instead of stacking (Model.skp -> Model.hew)', () => {
    expect(nameForFormat('Model.skp', 'hew')).toBe('Model.hew')
    expect(nameForFormat('Model.hew', 'skp2026')).toBe('Model.skp')
    expect(nameForFormat('Model', 'skp2017')).toBe('Model.skp')
    expect(nameForFormat('Model.skp', 'skp2026')).toBe('Model.skp')
  })
})

describe('format helpers', () => {
  it('knows which formats are SketchUp', () => {
    expect(isSkpFormat('hew')).toBe(false)
    expect(isSkpFormat('skp2026')).toBe(true)
    expect(isSkpFormat('skp2017')).toBe(true)
  })

  it('maps formats to extensions, targets and descriptions', () => {
    expect(extensionFor('hew')).toBe('.hew')
    expect(extensionFor('skp2017')).toBe('.skp')
    expect(skpTarget('skp2017')).toBe('skp2017')
    expect(skpTarget('skp2026')).toBe('skp2026')
    expect(skpTarget('skp2018')).toBe('skp2018')
    expect(skpTarget('skp2019')).toBe('skp2019')
    expect(skpTarget('skp2020')).toBe('skp2020')
    expect(skpTarget('skp8')).toBe('skp8')
    expect(skpTarget('skp2013')).toBe('skp2013')
    expect(skpTarget('hew')).toBe('skp2026')
    expect(formatDescription('hew')).toBe('Hew model')
    expect(formatDescription('skp2026')).toBe('SketchUp model')
    expect(formatDescription('skp2017')).toBe('SketchUp 2017 model')
    expect(formatDescription('skp2020')).toBe('SketchUp 2020 model')
    expect(formatDescription('skp8')).toBe('SketchUp 8 model')
  })

  it('normalizes unknown values to hew', () => {
    expect(normalizeDocFormat(undefined)).toBe('hew')
    expect(normalizeDocFormat('')).toBe('hew')
    expect(normalizeDocFormat('skp2026')).toBe('skp2026')
    expect(normalizeDocFormat('skp2017')).toBe('skp2017')
    expect(normalizeDocFormat('skp2018')).toBe('skp2018')
    expect(normalizeDocFormat('skp2019')).toBe('skp2019')
    expect(normalizeDocFormat('skp2020')).toBe('skp2020')
    // Every stream release, 3 to 8 and 2013 to 2020, is a format.
    for (const f of SKP_STREAM_FORMATS) expect(normalizeDocFormat(f)).toBe(f)
    expect(SKP_STREAM_FORMATS).toHaveLength(14)
    expect(normalizeDocFormat('skp8')).toBe('skp8')
    expect(normalizeDocFormat('skp3')).toBe('skp3')
    expect(normalizeDocFormat('skp2021')).toBe('hew')
    expect(normalizeDocFormat('skp9')).toBe('hew')
    expect(normalizeDocFormat('skp13')).toBe('hew')
  })

  it('offers the Save As choices in order', () => {
    // Hew, the 2026 format, then every stream release newest to oldest.
    expect(SAVE_FORMAT_CHOICES.map((c) => c.format)).toEqual(['hew', 'skp2026', ...SKP_STREAM_FORMATS])
    expect(SAVE_FORMAT_CHOICES.map((c) => c.format).slice(0, 4)).toEqual(['hew', 'skp2026', 'skp2020', 'skp2019'])
    expect(SAVE_FORMAT_CHOICES.at(-1)?.format).toBe('skp3')
    // The 2021-and-later container is named for what opens it, not a year.
    expect(SAVE_FORMAT_CHOICES.find((c) => c.format === 'skp2026')?.label).toBe('SketchUp 2021 and later (.skp)')
    // The pickers gather every stream release under one "older" group:
    // 2020 down to 2013, then 8 down to 3.
    expect(SAVE_FORMAT_CHOICES.filter((c) => c.group === 'older').map((c) => c.format)).toEqual([...SKP_STREAM_FORMATS])
    expect(SAVE_FORMAT_CHOICES.filter((c) => c.group === undefined).map((c) => c.format)).toEqual(['hew', 'skp2026'])
    const choice = (f: string) => SAVE_FORMAT_CHOICES.find((c) => c.format === f)
    expect(choice('skp2017')?.hint).toBe('Opens in SketchUp 2017 and later.')
    expect(choice('skp8')?.label).toBe('SketchUp 8 (.skp)')
    expect(choice('skp2013')?.label).toBe('SketchUp 2013 (.skp)')
  })
})

describe('saveFormatForPath (the live API\'s hew.doc.save)', () => {
  it('follows the extension, keeping a SketchUp document\'s own container', () => {
    expect(saveFormatForPath('/m/a.hew', undefined, 'skp2017')).toBe('hew')
    expect(saveFormatForPath('/m/a.SKP', undefined, 'skp2017')).toBe('skp2017')
    expect(saveFormatForPath('/m/a.skp', undefined, 'hew')).toBe('skp2026')
    expect(saveFormatForPath('/m/a.skp', undefined, undefined)).toBe('skp2026')
  })

  it('takes a named format the extension agrees with, and refuses one it contradicts', () => {
    expect(saveFormatForPath('/m/a.skp', 'skp2017', 'hew')).toBe('skp2017')
    expect(saveFormatForPath('/m/a.skp', 'skp', 'skp2017')).toBe('skp2026')
    expect(saveFormatForPath('/m/a.skp', 'skp2019', 'hew')).toBe('skp2019')
    expect(saveFormatForPath('/m/a.skp', undefined, 'skp2020')).toBe('skp2020')
    expect(saveFormatForPath('/m/a.skp', 'skp8', 'hew')).toBe('skp8')
    expect(saveFormatForPath('/m/a.skp', undefined, 'skp3')).toBe('skp3')
    expect(saveFormatForPath('/m/a.hew', 'skp2013', undefined)).toBeNull()
    expect(saveFormatForPath('/m/a.skp', 'skp2021', undefined)).toBeNull()
    expect(saveFormatForPath('/m/a.hew', 'skp2018', undefined)).toBeNull()
    expect(saveFormatForPath('/m/a.hew', 'hew', 'skp2026')).toBe('hew')
    expect(saveFormatForPath('/m/a.skp', 'hew', undefined)).toBeNull()
    expect(saveFormatForPath('/m/a.hew', 'skp', undefined)).toBeNull()
    expect(saveFormatForPath('/m/a.hew', 'obj', undefined)).toBeNull()
  })
})

describe('the older-release group', () => {
  it('preselects the file\'s own release, else the newest', () => {
    expect(olderReleaseFor('skp8')).toBe('skp8')
    expect(olderReleaseFor('skp2013')).toBe('skp2013')
    expect(olderReleaseFor('skp2026')).toBe('skp2020')
    expect(olderReleaseFor('hew')).toBe('skp2020')
    expect(olderReleaseFor(undefined)).toBe('skp2020')
    expect(olderReleaseFrom('skp5')).toBe('skp5')
    expect(olderReleaseFrom('nonsense')).toBe('skp2020')
  })

  it('offers the avoid-newer-features choice for the 2021-and-later container only', () => {
    expect(takesCompat('skp2026')).toBe(true)
    expect(takesCompat('skp2017')).toBe(false)
    expect(takesCompat('hew')).toBe(false)
  })
})
