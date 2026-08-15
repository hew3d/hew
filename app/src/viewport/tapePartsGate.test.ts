import { describe, it, expect } from 'vitest'
import { isVisiblePartSnap, isClearFaceSnap, classifyTapeHold } from './tapePartsGate'
import type { Snap } from '../tools/types'

const snap = (o: Partial<Snap>): Snap => ({ x: 0, y: 0, z: 0, kind: 'on-edge', ...o })
const NONE = new Set<bigint>()

describe('isVisiblePartSnap — Shop Mode parts-only gate', () => {
  it('accepts a visible object snap (real provenance)', () => {
    expect(isVisiblePartSnap(snap({ kind: 'on-edge', object: 5n }), NONE, NONE)).toBe(true)
  })

  it('rejects a null (empty) tap', () => {
    expect(isVisiblePartSnap(null, NONE, NONE)).toBe(false)
  })

  it('rejects the ground/plane empty-space fallback', () => {
    expect(isVisiblePartSnap(snap({ kind: 'ground', object: undefined }), NONE, NONE)).toBe(false)
    expect(isVisiblePartSnap(snap({ kind: 'plane', object: undefined }), NONE, NONE)).toBe(false)
  })

  it('rejects topology-less inference snaps — on-axis / on-guide (the review gap)', () => {
    // These carry no object provenance and are NOT parts — a tap on a soft
    // axis or an inherited construction guide must cancel, not measure.
    expect(isVisiblePartSnap(snap({ kind: 'on-axis', object: undefined }), NONE, NONE)).toBe(false)
    expect(isVisiblePartSnap(snap({ kind: 'on-guide', object: undefined }), NONE, NONE)).toBe(false)
  })

  it('rejects a snap on a hidden object', () => {
    expect(isVisiblePartSnap(snap({ kind: 'on-edge', object: 5n }), new Set([5n]), NONE)).toBe(false)
  })

  it('rejects a snap on a hidden instance', () => {
    expect(isVisiblePartSnap(snap({ kind: 'on-edge', object: 5n, instance: 7n }), NONE, new Set([7n]))).toBe(false)
  })

  it('accepts a visible instance-member snap', () => {
    expect(isVisiblePartSnap(snap({ kind: 'on-edge', object: 5n, instance: 7n }), NONE, NONE)).toBe(true)
  })
})

describe('isClearFaceSnap — Shop Mode Tape isolate-vs-magnify face test', () => {
  it('accepts an elementKind:face pick on a visible part', () => {
    expect(isClearFaceSnap(snap({ kind: 'on-face', elementKind: 'face', object: 5n }), NONE, NONE)).toBe(true)
  })

  it('rejects an edge pick (near an edge → magnify, not isolate)', () => {
    expect(isClearFaceSnap(snap({ kind: 'on-edge', elementKind: 'edge', object: 5n }), NONE, NONE)).toBe(false)
  })

  it('rejects a face pick on a hidden part', () => {
    expect(isClearFaceSnap(snap({ kind: 'on-face', elementKind: 'face', object: 5n }), new Set([5n]), NONE)).toBe(false)
  })

  it('rejects a provenance-less face-less snap and null', () => {
    expect(isClearFaceSnap(snap({ kind: 'ground', elementKind: undefined, object: undefined }), NONE, NONE)).toBe(false)
    expect(isClearFaceSnap(null, NONE, NONE)).toBe(false)
  })
})

describe('classifyTapeHold — Shop Mode Tape hold dispatch', () => {
  it('a grabbed endpoint always moves — even index 0, even on a clear face', () => {
    expect(classifyTapeHold({ grabEndpoint: 0, midGesture: false, onClearFace: true })).toBe('grab')
    expect(classifyTapeHold({ grabEndpoint: 1, midGesture: true, onClearFace: false })).toBe('grab')
  })

  it('a clear-face hold while idle isolates', () => {
    expect(classifyTapeHold({ grabEndpoint: null, midGesture: false, onClearFace: true })).toBe('isolate')
  })

  it('a clear-face hold MID-measurement magnifies (places the next point, not isolate)', () => {
    expect(classifyTapeHold({ grabEndpoint: null, midGesture: true, onClearFace: true })).toBe('magnify')
  })

  it('an edge/endpoint/empty hold magnifies', () => {
    expect(classifyTapeHold({ grabEndpoint: null, midGesture: false, onClearFace: false })).toBe('magnify')
  })
})
