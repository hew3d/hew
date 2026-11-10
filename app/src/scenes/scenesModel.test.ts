import { describe, expect, it } from 'vitest'
import {
  PROP_ALL,
  PROP_CAMERA,
  PROP_SECTION,
  driftAny,
  hasProp,
  neighborScene,
  nextSceneName,
  parseDriftJson,
  parseScenesJson,
  parseViewStyleJson,
  parseSecondsInput,
  sceneIndex,
} from './scenesModel'

const CAM = { projection: 'perspective', fovDeg: 45, eye: [4, -6, 3], target: [0, 0, 0], up: [0, 0, 1] }

describe('parseScenesJson', () => {
  it('parses the wasm shape, keeping absent vs null section distinct', () => {
    const json = JSON.stringify([
      { sid: 42, name: 'Assembled', description: 'All', props: PROP_ALL, camera: CAM, display: { grid: true, axes: false, guides: true }, viewStyle: { background: [255, 255, 255], sky: null, ground: [200, 200, 200], axes: false }, section: null },
      { sid: 43, name: 'Cut', props: PROP_CAMERA | PROP_SECTION, camera: CAM, section: { origin: [0, 0, 1], normal: [0, 0, 1], active: true } },
      { sid: 44, name: 'Cam only', props: PROP_CAMERA, camera: CAM },
    ])
    const entries = parseScenesJson(json)
    expect(entries.map((e) => e.name)).toEqual(['Assembled', 'Cut', 'Cam only'])
    expect(entries[0].section).toBeNull()
    expect(entries[0].display).toEqual({ grid: true, axes: false, guides: true })
    expect(entries[0].viewStyle).toEqual({ background: [255, 255, 255], sky: null, ground: [200, 200, 200], axes: false })
    expect(entries[1].viewStyle).toBeUndefined()
    expect(entries[0].description).toBe('All')
    expect(entries[1].section).toEqual({ origin: [0, 0, 1], normal: [0, 0, 1], active: true })
    expect(entries[1].description).toBe('')
    expect('section' in entries[2]).toBe(false)
    expect(hasProp(entries[2], 'camera')).toBe(true)
    expect(hasProp(entries[2], 'section')).toBe(false)
  })

  it('drops malformed entries and tolerates garbage', () => {
    expect(parseScenesJson('nope')).toEqual([])
    expect(parseScenesJson('{}')).toEqual([])
    const entries = parseScenesJson(JSON.stringify([{ sid: 'x' }, { sid: 1, name: 'ok', props: 0, camera: { projection: 'iso' } }]))
    expect(entries).toHaveLength(1)
    expect(entries[0].camera).toBeUndefined()
  })
})

describe('parseDriftJson / driftAny', () => {
  it('reads the flags and stale count', () => {
    const d = parseDriftJson('{"camera":true,"hiddenNodes":false,"hiddenTags":false,"section":false,"display":false,"staleRefs":2}')
    expect(d).toEqual({ camera: true, hiddenNodes: false, hiddenTags: false, section: false, display: false, staleRefs: 2 })
    expect(driftAny(d)).toBe(true)
    expect(driftAny(parseDriftJson('{"staleRefs":0}'))).toBe(false)
    expect(driftAny(parseDriftJson('{"staleRefs":1}'))).toBe(true)
    expect(driftAny(null)).toBe(false)
    expect(parseDriftJson('[')).toBeNull()
  })
})

describe('ordering helpers', () => {
  const entries = parseScenesJson(
    JSON.stringify([
      { sid: 1, name: 'A', props: 0 },
      { sid: 2, name: 'B', props: 0 },
      { sid: 3, name: 'C', props: 0 },
    ]),
  )
  it('neighborScene wraps and handles no active', () => {
    expect(neighborScene(entries, 1, 1)?.sid).toBe(2)
    expect(neighborScene(entries, 3, 1)?.sid).toBe(1)
    expect(neighborScene(entries, 1, -1)?.sid).toBe(3)
    expect(neighborScene(entries, null, 1)?.sid).toBe(1)
    expect(neighborScene(entries, null, -1)?.sid).toBe(3)
    expect(neighborScene(entries, 99, 1)?.sid).toBe(1)
    expect(neighborScene([], null, 1)).toBeNull()
  })
  it('sceneIndex and nextSceneName', () => {
    expect(sceneIndex(entries, 2)).toBe(1)
    expect(sceneIndex(entries, 9)).toBe(-1)
    expect(nextSceneName(entries)).toBe('Scene 1')
    const named = parseScenesJson(JSON.stringify([{ sid: 1, name: 'Scene 1', props: 0 }, { sid: 2, name: 'Scene 3', props: 0 }]))
    expect(nextSceneName(named)).toBe('Scene 2')
  })
})

describe('scene timings', () => {
  it('parses transitionS / delayS, null for absent, null or invalid', () => {
    const [a, b, c] = parseScenesJson(
      JSON.stringify([
        { sid: 1, name: 'A', props: 0, transitionS: 1.5, delayS: 0 },
        { sid: 2, name: 'B', props: 0, transitionS: null },
        { sid: 3, name: 'C', props: 0, transitionS: -2, delayS: 'x' },
      ]),
    )
    expect([a.transitionS, a.delayS]).toEqual([1.5, 0])
    expect([b.transitionS, b.delayS]).toEqual([null, null])
    expect([c.transitionS, c.delayS]).toEqual([null, null])
  })

  it('parseSecondsInput: blank clears, a non-negative number sets, the rest is invalid', () => {
    expect(parseSecondsInput('')).toBeNull()
    expect(parseSecondsInput('  ')).toBeNull()
    expect(parseSecondsInput('2.5')).toBe(2.5)
    expect(parseSecondsInput('0')).toBe(0)
    expect(parseSecondsInput('-1')).toBeUndefined()
    expect(parseSecondsInput('abc')).toBeUndefined()
    expect(parseSecondsInput('Infinity')).toBeUndefined()
  })
})

describe('parseViewStyleJson', () => {
  const base = { background: [1, 2, 3], sky: null, ground: null, axes: true }

  it('reads the backdrop alone, as before', () => {
    expect(parseViewStyleJson(base)).toEqual(base)
  })

  it('keeps the section and guide settings a document sets, and drops malformed ones', () => {
    const style = parseViewStyleJson({
      ...base,
      sectionCutColor: [200, 0, 10],
      sectionCutWidth: 5,
      sectionFill: true,
      sectionFillColor: [1, 1, 1],
      activeSectionColor: [0, 200, 0],
      guidesColor: [0, 0, 255],
      guidesHidden: false,
    })
    expect(style).toMatchObject({
      sectionCutColor: [200, 0, 10],
      sectionCutWidth: 5,
      sectionFill: true,
      sectionFillColor: [1, 1, 1],
      activeSectionColor: [0, 200, 0],
      guidesColor: [0, 0, 255],
      guidesHidden: false,
    })
    const bad = parseViewStyleJson({ ...base, sectionCutColor: [300, 0, 0], sectionCutWidth: -1, guidesHidden: 'yes' })
    expect(bad).toEqual(base)
  })
})
