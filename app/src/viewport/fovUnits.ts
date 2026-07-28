/**
 * fovUnits — millimetre <-> degree conversion and typed-entry parsing for
 * the Zoom tool's VCB (docs/design/camera-playtest2.md §1).
 *
 * SketchUp's Camera exposes both `fov` and `focal_length`; the two are
 * related through a notional 35mm-equivalent camera frame. This app stores
 * only the VERTICAL fov (`CameraRig.perspective.fov`, `three.js`'s own
 * convention), so the conversion is anchored to the frame's HALF-HEIGHT —
 * 18mm, half of the 36mm frame dimension. 18 is a convention (matching
 * SketchUp's own default lens closely enough to be recognizable to anyone
 * coming from it), not a physical law — if it ever needs to change, every
 * anchor in `fovUnits.test.ts` changes with it:
 *
 *   focalMm = HALF_FRAME_MM / tan(fovDeg/2)
 *   fovDeg  = 2 * atan(HALF_FRAME_MM / focalMm)
 *
 * Both directions are exact inverses of each other (mod floating-point),
 * so `focalMmToFovDeg(fovDegToFocalMm(x)) === x` within 1e-9.
 */

/** Half the 35mm-equivalent frame's relevant dimension — see the module
 * doc for the derivation. A convention, not a law; every call site that
 * needs it goes through the two functions below rather than inlining it. */
const HALF_FRAME_MM = 18

function degToRad(deg: number): number {
  return (deg * Math.PI) / 180
}

function radToDeg(rad: number): number {
  return (rad * 180) / Math.PI
}

/** Vertical fov (degrees) -> 35mm-equivalent focal length (millimetres). */
export function fovDegToFocalMm(fovDeg: number): number {
  return HALF_FRAME_MM / Math.tan(degToRad(fovDeg) / 2)
}

/** 35mm-equivalent focal length (millimetres) -> vertical fov (degrees). */
export function focalMmToFovDeg(focalMm: number): number {
  return 2 * radToDeg(Math.atan(HALF_FRAME_MM / focalMm))
}

export interface ParsedFovEntry {
  readonly fovDeg: number
}

// A leading run of digits with an optional single decimal point — the
// numeric part shared by every accepted spelling below.
const NUMBER = '[0-9]+(?:\\.[0-9]+)?|\\.[0-9]+'

// "45", "45.5", "45deg", "45 deg", "45°", "45 °" — degrees. The unit suffix
// is optional (a bare number IS degrees, matching the existing typed-entry
// behaviour this replaces).
const DEGREES_RE = new RegExp(`^(${NUMBER})\\s*(?:deg|°)?$`)

// "50mm", "50 mm" — focal length.
const MM_RE = new RegExp(`^(${NUMBER})\\s*mm$`)

/**
 * Parses a typed VCB buffer into a fov in degrees, accepting either
 * spelling (case-insensitive, tolerating internal whitespace before the
 * unit): `45` / `45.5` / `45deg` / `45 deg` / `45°` as degrees, `50mm` /
 * `50 mm` as a focal length converted via `focalMmToFovDeg`.
 *
 * Returns `null` for anything non-numeric, non-finite, or <= 0 — the
 * existing SketchUp-ish behaviour of silently discarding garbage input
 * rather than surfacing an error (`commitFovEntry`, Viewport.tsx).
 *
 * Deliberately does NOT clamp to [`MIN_FOV_DEG`, `MAX_FOV_DEG`] — that
 * stays `CameraRig.setFov`'s job (the single clamp point, so every caller
 * — typed entry, drag, wheel — is clamped identically). A typed `5mm`
 * therefore parses to a fov far past `MAX_FOV_DEG`; the caller applies it
 * through `rig.setFov` and re-reads the CLAMPED value for the readout.
 */
export function parseFovEntry(buffer: string): ParsedFovEntry | null {
  const trimmed = buffer.trim()
  if (trimmed.length === 0) return null
  const lower = trimmed.toLowerCase()

  const mm = MM_RE.exec(lower)
  if (mm !== null) {
    const value = Number(mm[1])
    return Number.isFinite(value) && value > 0 ? { fovDeg: focalMmToFovDeg(value) } : null
  }

  const deg = DEGREES_RE.exec(lower)
  if (deg !== null) {
    const value = Number(deg[1])
    return Number.isFinite(value) && value > 0 ? { fovDeg: value } : null
  }

  return null
}
