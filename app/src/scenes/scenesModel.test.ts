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
  sceneIndex,
} from './scenesModel'

const CAM = { projection: 'perspective', fovDeg: 45, eye: [4, -6, 3], target: [0, 0, 0], up: [0, 0, 1] }

describe('parseScenesJson', () => {
  it('parses the wasm shape, keeping absent vs null section distinct', () => {
    const json = JSON.stringify([
      { sid: 42, name: 'Assembled', description: 'All', props: PROP_ALL, camera: CAM, display: { grid: true, axes: false, guides: true }, section: null },
      { sid: 43, name: 'Cut', props: PROP_CAMERA | PROP_SECTION, camera: CAM, section: { origin: [0, 0, 1], normal: [0, 0, 1], active: true } },
      { sid: 44, name: 'Cam only', props: PROP_CAMERA, camera: CAM },
    ])
    const entries = parseScenesJson(json)
    expect(entries.map((e) => e.name)).toEqual(['Assembled', 'Cut', 'Cam only'])
    expect(entries[0].section).toBeNull()
    expect(entries[0].display).toEqual({ grid: true, axes: false, guides: true })
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
