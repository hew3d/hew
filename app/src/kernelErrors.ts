/**
 * Plain-language error messages for every operation Hew refuses.
 *
 * The wasm boundary throws stringly `"CODE: message"` errors
 * (docs/DEVELOPMENT.md B3), where CODE is the innermost typed kernel error's
 * variant name. This module owns the CODE → user-facing copy table: what
 * happened in plain words, then a suggested next step — never a raw
 * technical error (ROADMAP: plain-language error messages).
 *
 * Copy ground rules:
 *  - First sentence: what was refused, in the user's vocabulary. Hew's
 *    audience knows "watertight solid" (it's the product's core promise);
 *    they don't know "manifold", "coplanar", or "topology".
 *  - Second sentence: the most likely next step, phrased as an action.
 *  - Refusals are the kernel keeping its no-silent-repair promise (rule 4);
 *    the copy says what to change, never apologizes for refusing.
 *
 * The table is exhaustively checked against the kernel's error inventory by
 * `kernelErrors.test.ts` — adding a kernel error variant without copy here
 * fails that test, not a user.
 */

/** Stale-handle copy: the model changed underneath a tool's cached pick. */
const stale = (what: string): string =>
  `That ${what} is no longer there — the model changed since it was picked. Click it again.`

const checkSolid = 'Check its solid status in the Object Info panel.'

/**
 * Human copy for every kernel error code that can cross the wasm boundary,
 * keyed by the `"CODE: message"` prefix.
 */
const DESCRIPTIONS: Record<string, string> = {
  // ------------------------------------------------------- stale handles
  UnknownObject: stale('object'),
  UnknownFace: stale('face'),
  UnknownEdge: stale('edge'),
  UnknownVertex: stale('point'),
  UnknownSketch: stale('sketch'),
  UnknownRegion: stale('profile'),
  UnknownIsland: stale('shape'),
  UnknownGroup: stale('group'),
  UnknownComponent: stale('component'),
  UnknownInstance: stale('component instance'),
  UnknownGuide: stale('guide'),
  UnknownMaterial: 'That material is no longer in the palette. Pick another swatch.',
  DegenerateFace: 'That face has no usable area. Pick a different face.',

  // ---------------------------------------------------------- drawing
  PointOffPlane:
    "That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.",
  DegenerateSegment:
    "The line's two ends are the same point. Click a second, different point.",
  WouldRetopologize:
    'Moving this point would make lines cross or merge. Move it somewhere clear, or delete and redraw the lines instead.',
  MalformedRegion:
    "This profile's outline couldn't be traced. Redraw the shape; if it keeps failing, use Report Bug.",
  DegenerateCurve:
    'That circle or arc is too small to keep. Drag a larger radius, or type an exact one.',
  RestoreConflicts:
    'Undo needs to put the original outline back, but newer drawing is in its way. Erase the overlapping lines and undo again.',
  OffsetTooSmall:
    'That offset distance is too small to make a new boundary. Drag further, or type an exact distance.',
  OffsetCollapsed:
    "The shape can't absorb that offset — its boundary would collapse, cross itself, or spike out of a sharp corner. Try a smaller distance, or soften the sharpest corner.",
  SketchGestureAlreadyOpen:
    'The drawing tools got out of step. Press Escape and try again.',
  SketchGestureNotOpen:
    'The drawing tools got out of step. Press Escape and try again.',
  DegenerateGuide:
    'The guide needs a definite direction. Drag a little further before dropping it.',

  // ------------------------------------------------- extrude / push-pull
  DistanceTooSmall:
    'That distance is too small to build anything. Drag further, or type an exact length.',
  DegenerateGeometry:
    "This profile can't be extruded into a valid solid. Simplify the shape and try again.",
  ObjectNotSolid: `This object isn't a watertight solid, so it can't be pushed or pulled. ${checkSolid}`,
  WouldVanish:
    'Pushing that far would remove the whole object. Push a shorter distance, or delete the object instead.',
  NonManifoldResult:
    "The walls this would create run into the object's other geometry. Try a different distance, or reshape the surrounding faces first.",
  NotASubFace:
    'Push/Pull here needs a shape drawn on the face. Draw a closed outline on the face first.',
  RadiusVanishes:
    'Pushing that far would shrink the curved wall to nothing. Push a shorter distance.',
  WallNeighborNonPlanar:
    'Offsetting this curved wall would bend a neighboring face out of flat, so it was refused. Adjust or simplify the touching faces first.',

  // -------------------------------------------------------- follow me
  EmptyPath: 'Follow Me needs a path. Select or click the line, curve, or face edge to follow.',
  UnknownPathEdge: stale('path line'),
  PathBranches:
    'The path forks — Follow Me needs one continuous run of lines. Select a single chain with no branches.',
  PathDisconnected:
    'The path is in separate pieces. Select one connected run of lines.',
  PathSegmentTooShort:
    'Part of the path is too short to follow. Redraw the path without tiny segments.',
  ProfileNotPerpendicular:
    'The profile must sit square across the path where the sweep starts. Draw the profile on a surface perpendicular to the path.',
  PathDetachedFromProfile:
    "The path doesn't start on the profile's surface. Start the path at the profile — or, on a loop, cross the profile partway along a straight run, not at a corner.",
  PathReverses:
    'The path doubles back on itself — or turns nearly all the way around — so there is no clean corner to turn. Remove the reversal, or soften the corner.',
  PathTooTight:
    'The path turns tighter than the profile is wide, so the sweep would fold into itself. Use a smaller profile or gentler turns.',
  SweepSelfIntersects:
    'Following this path would make the shape run into itself. Shorten the path or use a smaller profile.',
  SweepDegenerate:
    "This profile can't be swept along that path into a valid solid. Simplify the profile or the path and try again.",

  // ------------------------------------------- drawing on faces / merging
  PathTooShort: 'The cut needs at least two points. Click a start and an end.',
  EndpointNotOnBoundary:
    "A splitting line must start and end on the face's edges. Snap both ends to the face boundary.",
  PointNotOnFace:
    'Part of the line leaves the face. Keep every point on the face being split.',
  PathNotSimple:
    "The line crosses itself or touches the face's edge partway along. Draw a simple path from edge to edge.",
  FacesNotCoplanar:
    "These two faces aren't in the same plane, so they can't be merged. Pick an edge whose two faces lie flat in one plane.",
  BoundaryEdge:
    "This edge has a face on only one side — there's nothing to merge it with.",
  SameFaceOnBothSides:
    'The same face is on both sides of this edge, so dissolving it would puncture the surface.',
  SharedChainDisconnected:
    "These faces touch along more than one separate edge run, which merge can't dissolve yet. Merge along one shared run at a time.",
  SharedChainCoversBoundary:
    "One face's entire boundary lies on the other, so this isn't an edge merge. Select the inner face itself and remove it instead.",
  CurveClaimOffLoop:
    "The drawn outline and its circle disagree, so the imprint was refused. Redraw the circle; if it keeps failing, use Report Bug.",
  LoopNotStrictlyInside:
    'The shape must sit fully inside the face, clear of its edges. Draw it a little smaller or further from the boundary.',
  LoopSelfIntersects:
    "The shape's outline crosses itself. Draw a simple, non-crossing outline.",
  NotAnInnerFace:
    'Only a shape drawn fully inside a face can be removed this way. Select the imprinted inner face itself.',
  WouldCorrupt:
    'That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.',
  BadLoop: 'The outline needs at least three points.',

  // -------------------------------------------------------- booleans
  OperandNotSolid: `Combining needs watertight solids on both sides. ${checkSolid.replace('its', "each object's")}`,
  EmptyResult:
    "The result would be empty — the objects don't overlap that way. Check that the solids actually intersect.",
  SingularTransform:
    "One object's placement is scaled down to nothing, so the operation can't run.",
  DegenerateContact:
    'The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.',

  // ----------------------------------------------------------- slice
  NotSolid: `Only a watertight solid can be sliced. ${checkSolid}`,
  PlaneMissesSolid:
    "The slicing plane doesn't pass through the object. Position the cut so it goes through the solid.",
  Degenerate:
    "The cut lines up exactly with an existing face or edge, so it wouldn't create two pieces. Move the cut slightly.",

  // ------------------------------------------------------- transforms
  Singular:
    'That transform would scale the object down to nothing, so it was refused.',
  DegenerateAxis:
    'The rotation axis needs two distinct points. Pick a second point further from the first.',
  Reflection:
    "This would turn the object inside out (a mirror), which can't be baked into a solid. Mirror a component instance instead.",

  // ------------------------------------------- groups & components
  EmptyGroup: 'Select at least one object to group.',
  EmptySelection:
    'The selection has nothing visible to transform — everything in it is hidden or empty. Unhide its contents, or select something visible.',
  EmptyComponent: 'Select at least one object to turn into a component.',
  NestedComponentUnsupported:
    "A component can't contain another component yet. Explode the inner instance first, then try again.",
  CannotExplodeReflected:
    "A mirrored instance can't be exploded — baking the mirror would turn the solid inside out. Use Make Unique instead.",
  DuplicateMember:
    'The same object is in the selection twice. Reselect and try again.',
  MixedParents:
    'Only siblings can be grouped — everything selected must be top-level, or all inside the same group. Move them to one level first.',
  GroupedOperand:
    "This operation can't target an object inside a group. Ungroup it, or leave the group context, first.",
  BooleanOperandHasInstance:
    "Combining can't consume a component instance — its geometry is shared with every copy. Explode the instance (or Make Unique, then Explode) first.",
  BooleanOperandNotSolid:
    'Something in that selection is not a watertight solid, so it can\'t be combined. Check each object\'s solid badge in Object Info and fix or remove the leaky one.',
  BooleanOperandEmpty:
    'That selection has no solids to combine. Pick a solid object or a group of solids.',

  // ---------------------------------------------------------- history
  NothingToUndo: 'Nothing to undo.',
  NothingToRedo: 'Nothing to redo.',
  InverseFailed:
    "This step couldn't be undone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.",
  InverseDiverged:
    'Undo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.',

  // ------------------------------------------------------ file loading
  NotAContainer: "This file isn't a Hew document. Pick a .hew file saved by Hew.",
  UnsupportedVersion:
    'This file needs a newer version of Hew than this one. Update Hew and try again.',
  MalformedManifest: "This file is damaged and can't be opened.",
  DanglingReference: "This file is damaged and can't be opened.",
  MissingAsset: "This file is missing some of its data and can't be opened.",
  Geometry: "This file's geometry data is damaged and can't be opened.",

  // ----------------------------------------------------- math helpers
  DegenerateVector:
    'That direction is too short to work with. Pick points further apart.',
  DegeneratePlane:
    "Those points don't define a plane. Pick three points that aren't in a line.",
}

/**
 * Parse a `"CODE: message"` thrown error string, returning the code prefix.
 * Returns null if the format doesn't match.
 */
export function parseKernelErrorCode(err: unknown): string | null {
  const msg = err instanceof Error ? err.message : String(err)
  const match = /^([A-Za-z][A-Za-z0-9_]*):\s/.exec(msg)
  return match ? match[1] : null
}

/**
 * Plain-language copy (what happened + suggested next step) for a kernel
 * error code. An unmapped code — a new kernel error the table hasn't caught
 * up with, kept from ever being a user's problem by the exhaustiveness
 * test — still gets a plain sentence, with the code kept for Report Bug.
 */
export function kernelErrorMessage(code: string, rawMsg: string): string {
  return (
    DESCRIPTIONS[code] ??
    `Hew couldn't complete that (${code}: ${rawMsg}). If this keeps happening, use Report Bug.`
  )
}

/**
 * Importer wrapper prefixes: `import_dae`/`import_gltf`/`import_skp` (and,
 * historically, `Scene::load`) tag errors with the FORMAT, not a typed
 * variant, and their payload text is already written for people — e.g. the
 * `.skp` version message carries exact "Save As 2017" guidance. Unwrap the
 * tag and show the payload as-is; running it through the table's fallback
 * would bury real guidance inside boilerplate.
 */
const WRAPPER_CODES = new Set(['DAE', 'glTF', 'SKP', 'LOAD'])

/**
 * One-step convenience for `catch` sites holding a raw thrown value: map a
 * `"CODE: message"` kernel error to its plain-language copy, and pass any
 * other error text (host file errors, etc. — usually already human-readable)
 * through unchanged.
 */
export function friendlyErrorText(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err)
  const code = parseKernelErrorCode(err)
  if (code === null) return raw
  const rest = raw.slice(code.length + 2)
  return WRAPPER_CODES.has(code) ? rest : kernelErrorMessage(code, rest)
}

/** Every kernel error code with copy — exported for the exhaustiveness test. */
export function describedErrorCodes(): string[] {
  return Object.keys(DESCRIPTIONS)
}

/**
 * Codes logged at ERROR level (a real refusal of the user's geometry) rather
 * than warn — the single source App.tsx's toast/log path reads, kept here
 * beside the copy table so a new code gets its severity where it gets its
 * message, instead of drifting in a second hand-maintained array.
 */
const ERROR_LEVEL_CODES: ReadonlySet<string> = new Set([
  'WouldVanish', 'NonManifoldResult', 'ObjectNotSolid', 'DegenerateGeometry',
  'OperandNotSolid', 'DegenerateContact', 'EmptyResult', 'SingularTransform',
  'BooleanOperandHasInstance', 'BooleanOperandNotSolid', 'BooleanOperandEmpty',
])

/** Whether `code` should be logged/toasted as an error (vs a warning). */
export function isErrorLevelCode(code: string): boolean {
  return ERROR_LEVEL_CODES.has(code)
}
