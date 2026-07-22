/**
 * Plain-language error copy tests.
 *
 * The load-bearing one is the exhaustiveness check: `KERNEL_ERROR_CODES`
 * below is the hand-maintained inventory of every innermost typed-error
 * variant the wasm boundary can emit as a `"CODE: message"` prefix (see
 * `doc_err`/`api_err`/`stale` in crates/wasm-api/src/lib.rs). A kernel error
 * added without copy fails here instead of reaching a user as a raw code.
 */

import { describe, it, expect } from 'vitest'
import {
  parseKernelErrorCode,
  kernelErrorMessage,
  friendlyErrorText,
  describedErrorCodes,
  isErrorLevelCode,
} from './kernelErrors'

/**
 * Every error code the wasm boundary can emit, by source enum. Update this
 * list — and the copy table — together with the kernel.
 */
const KERNEL_ERROR_CODES = [
  // DocumentError (flat variants)
  'UnknownSketch', 'UnknownObject', 'UnknownFace',
  'UnknownMaterial', 'UnknownGroup', 'UnknownComponent', 'UnknownInstance',
  'UnknownGuide', 'SketchGestureAlreadyOpen', 'SketchGestureNotOpen',
  'DegenerateGuide', 'EmptyGroup', 'EmptySelection', 'EmptyComponent',
  'NestedComponentUnsupported', 'CannotExplodeReflected', 'DuplicateMember',
  'MixedParents', 'GroupedOperand', 'BooleanOperandHasInstance',
  'BooleanOperandNotSolid', 'BooleanOperandEmpty', 'NothingToUndo',
  'NothingToRedo', 'InverseFailed', 'InverseDiverged',
  // SketchError
  'PointOffPlane', 'DegenerateSegment', 'UnknownEdge', 'UnknownVertex',
  'UnknownRegion', 'WouldRetopologize', 'UnknownIsland', 'MalformedRegion',
  'DegenerateCurve', 'RestoreConflicts', 'OffsetTooSmall', 'OffsetCollapsed',
  'UnknownCurve', 'CurveNotAnalytic', 'CurveNotRefacetable',
  'SegmentsBelowFloor', 'SegmentsAboveCap',
  // ExtrudeError
  'DistanceTooSmall', 'DegenerateGeometry',
  // FollowMeError
  'EmptyPath', 'UnknownPathEdge', 'PathBranches', 'PathDisconnected',
  'PathSegmentTooShort', 'ProfileNotPerpendicular', 'PathDetachedFromProfile',
  'PathReverses', 'PathTooTight', 'SweepSelfIntersects', 'SweepDegenerate',
  // PushPullError
  'ObjectNotSolid', 'WouldVanish', 'NonManifoldResult', 'NotASubFace',
  'RadiusVanishes', 'WallNeighborNonPlanar',
  // StickyError
  'PathTooShort', 'EndpointNotOnBoundary', 'PointNotOnFace', 'PathNotSimple',
  'FacesNotCoplanar', 'BoundaryEdge', 'SameFaceOnBothSides',
  'SharedChainDisconnected', 'SharedChainCoversBoundary',
  'LoopNotStrictlyInside', 'LoopSelfIntersects',
  'NotAnInnerFace', 'WouldCorrupt', 'CurveClaimOffLoop',
  // BooleanError
  'OperandNotSolid', 'EmptyResult', 'SingularTransform', 'DegenerateContact',
  // SliceError
  'NotSolid', 'PlaneMissesSolid', 'Degenerate',
  // TransformError
  'Singular', 'DegenerateAxis', 'Reflection',
  // MathError
  'DegenerateVector', 'DegeneratePlane',
  // LoadError (open path)
  'NotAContainer', 'UnsupportedVersion', 'MalformedManifest',
  'DanglingReference', 'MissingAsset', 'Geometry',
  // wasm-api boundary-minted codes (stale()/inline)
  'DegenerateFace', 'BadLoop',
]

describe('parseKernelErrorCode', () => {
  it('parses a CODE: message format', () => {
    const code = parseKernelErrorCode(new Error('WouldVanish: face would be removed'))
    expect(code).toBe('WouldVanish')
  })

  it('parses multi-word codes', () => {
    expect(parseKernelErrorCode(new Error('NonManifoldResult: edge shared by 3+ faces'))).toBe(
      'NonManifoldResult',
    )
  })

  it('returns null when format does not match', () => {
    expect(parseKernelErrorCode(new Error('something went wrong'))).toBeNull()
    expect(parseKernelErrorCode(new Error(''))).toBeNull()
    expect(parseKernelErrorCode('plain string')).toBeNull()
  })

  it('handles non-Error objects', () => {
    expect(parseKernelErrorCode('WouldVanish: bad things')).toBe('WouldVanish')
  })
})

describe('kernelErrorMessage — coverage', () => {
  it('has copy for every kernel error code the boundary can emit', () => {
    const described = new Set(describedErrorCodes())
    const missing = KERNEL_ERROR_CODES.filter((c) => !described.has(c))
    expect(missing).toEqual([])
  })

  it('has no orphaned copy for codes the kernel no longer emits', () => {
    // The app itself emits a few refusals through the same "CODE: copy"
    // toast convention without a kernel enum behind them — enumerate them
    // here so the guard still catches genuinely stale kernel copy.
    const APP_ERROR_CODES = ['InvalidSelection']
    const known = new Set([...KERNEL_ERROR_CODES, ...APP_ERROR_CODES])
    const orphans = describedErrorCodes().filter((c) => !known.has(c))
    expect(orphans).toEqual([])
  })

  it('every description is plain language: complete sentences, no raw code, no jargon', () => {
    for (const code of describedErrorCodes()) {
      const msg = kernelErrorMessage(code, 'raw detail')
      expect(msg, code).toMatch(/\.$/)
      expect(msg, code).not.toContain(code)
      expect(msg.toLowerCase(), code).not.toMatch(/\bmanifold\b/)
      expect(msg.toLowerCase(), code).not.toMatch(/\bcoplanar\b/)
      expect(msg.toLowerCase(), code).not.toMatch(/\btopology\b/)
      expect(msg, code).not.toContain('raw detail')
    }
  })

  it('refusals that need action carry a suggested next step (second sentence)', () => {
    // Spot the pattern on representative refusals across the op families.
    for (const code of [
      'WouldVanish', 'NonManifoldResult', 'RadiusVanishes', 'DistanceTooSmall',
      'EndpointNotOnBoundary', 'DegenerateContact', 'PlaneMissesSolid',
      'MixedParents', 'CannotExplodeReflected', 'WouldRetopologize',
      'RestoreConflicts', 'WallNeighborNonPlanar',
    ]) {
      const sentences = kernelErrorMessage(code, '').match(/[.!?](\s|$)/g) ?? []
      expect(sentences.length, code).toBeGreaterThanOrEqual(2)
    }
  })

  it('still degrades safely for a code the table has never seen', () => {
    const msg = kernelErrorMessage('SomeUnknownCode', 'raw detail')
    expect(msg).toContain('SomeUnknownCode')
    expect(msg).toContain('raw detail')
    expect(msg).toContain('Report Bug')
  })
})

describe('friendlyErrorText', () => {
  it('maps a kernel CODE: message error to its plain-language copy', () => {
    const text = friendlyErrorText(new Error('WouldVanish: face 42 would be removed'))
    expect(text).toBe(kernelErrorMessage('WouldVanish', 'face 42 would be removed'))
    expect(text).toContain('Push a shorter distance')
  })

  it('passes non-kernel errors through unchanged (host errors are already human text)', () => {
    expect(friendlyErrorText(new Error('permission denied: C:\\models\\a.hew'))).toBe(
      'permission denied: C:\\models\\a.hew',
    )
    expect(friendlyErrorText('disk full')).toBe('disk full')
  })

  it('unwraps importer format tags and shows their human payload as-is', () => {
    // The .skp importer's own message carries exact guidance — it must reach
    // the toast intact, not buried in the unknown-code fallback.
    const skp =
      'SKP: unsupported SketchUp version {26.2.0}: open it in SketchUp and ' +
      'File ▸ Save As ▸ SketchUp Version 2017, then import that'
    expect(friendlyErrorText(new Error(skp))).toBe(skp.slice('SKP: '.length))
    expect(friendlyErrorText(new Error('DAE: missing <library_geometries>'))).toBe(
      'missing <library_geometries>',
    )
    expect(friendlyErrorText(new Error('glTF: buffer 0 out of range'))).toBe(
      'buffer 0 out of range',
    )
  })

  it('carries copy for the app-side InvalidSelection refusal (structuralSelection boundary)', () => {
    const text = kernelErrorMessage('InvalidSelection', '')
    expect(text).toContain('group or component')
    expect(text).toContain('Sketches')
  })

  it('maps load failures through their typed variant codes', () => {
    // Scene::load emits LoadError variant codes ("NotAContainer: …"), the
    // same boundary convention as every other typed error.
    const text = friendlyErrorText(new Error('NotAContainer: bad magic'))
    expect(text).toBe(kernelErrorMessage('NotAContainer', 'bad magic'))
    expect(text).toContain('Hew document')
  })
})

describe('isErrorLevelCode', () => {
  it('classifies every boolean-operand refusal as an error, like its siblings', () => {
    // The three group-boolean refusals must not render one level softer than
    // OperandNotSolid (adversarial review, minor).
    for (const code of [
      'OperandNotSolid', 'DegenerateContact', 'EmptyResult',
      'BooleanOperandHasInstance', 'BooleanOperandNotSolid', 'BooleanOperandEmpty',
    ]) {
      expect(isErrorLevelCode(code), code).toBe(true)
    }
  })

  it('leaves ordinary refusals at warning level', () => {
    expect(isErrorLevelCode('DistanceTooSmall')).toBe(false)
    expect(isErrorLevelCode('GroupedOperand')).toBe(false)
  })

  it('only classifies codes that actually have copy', () => {
    const described = new Set(describedErrorCodes())
    for (const code of [
      'WouldVanish', 'NonManifoldResult', 'ObjectNotSolid', 'DegenerateGeometry',
      'OperandNotSolid', 'DegenerateContact', 'EmptyResult', 'SingularTransform',
      'BooleanOperandHasInstance', 'BooleanOperandNotSolid', 'BooleanOperandEmpty',
    ]) {
      expect(described.has(code), `${code} has copy`).toBe(true)
    }
  })
})
