/**
 * sectionManager — session/app-layer state for the non-destructive Section
 * Plane (DESIGN §2). A `SectionPlane` is a plain data
 * record, exactly like the current editing context or camera state: created,
 * mutated, and dropped entirely in the app, never serialized to `.hew` and
 * never undo-wired (v0.3.0 is session-only — DESIGN §8). Exactly one section
 * exists at a time; placing a new one replaces the previous (DESIGN §1).
 *
 * This module is pure data/logic — no three.js, no DOM, fully testable in
 * Node/vitest. `SceneRenderer` (rendering) and `SectionPlaneTool` (gesture)
 * both consume `SectionPlane` records but own no section state themselves.
 */

/** A single non-destructive clipping plane. `origin` is any point ON the
 * plane (world meters); `normal` is unit and points toward the near/outer
 * side the clip REMOVES (the face's outward normal — the side you clicked),
 * so geometry on the `+normal` side is cut away and the `-normal` interior
 * side is kept while `active` (DESIGN §2/§3; the clip plane the renderer
 * builds negates this normal — see `SceneRenderer.setSectionPlane`).
 * `active` toggles whether the plane currently clips — SketchUp's "Active
 * Cut" — independent of whether the section exists at all (a `null`
 * SectionManager.current means no section is placed). */
export interface SectionPlane {
  origin: [number, number, number]
  normal: [number, number, number]
  active: boolean
}

/** Normalize a 3-vector; degenerate (near-zero) input falls back to +Z
 * (the ground-plane default DESIGN §1 specifies for an empty-ground click). */
function normalize3(v: readonly [number, number, number]): [number, number, number] {
  const len = Math.hypot(v[0], v[1], v[2])
  if (len < 1e-9) return [0, 0, 1]
  return [v[0] / len, v[1] / len, v[2] / len]
}

/** Build a freshly PLACED section: active by construction (DESIGN §1 — "On
 * placement the section becomes the active section"). */
export function createSectionPlane(
  origin: readonly [number, number, number],
  normal: readonly [number, number, number],
): SectionPlane {
  return { origin: [...origin], normal: normalize3(normal), active: true }
}

/** A copy of `plane` with `origin` moved `distance` meters along its own
 * normal (signed — negative sweeps backward). Pure translation; `normal`
 * and `active` are unchanged. */
export function offsetSectionPlane(plane: SectionPlane, distance: number): SectionPlane {
  const [nx, ny, nz] = plane.normal
  return {
    ...plane,
    origin: [
      plane.origin[0] + nx * distance,
      plane.origin[1] + ny * distance,
      plane.origin[2] + nz * distance,
    ],
  }
}

/** A copy of `plane` with `active` flipped (SketchUp's "Active Cut" toggle —
 * DESIGN §1). `origin`/`normal` are unchanged. */
export function toggleSectionPlaneActive(plane: SectionPlane): SectionPlane {
  return { ...plane, active: !plane.active }
}

/**
 * Session-only manager: holds at most one `SectionPlane`. Not undo-wired —
 * view state, like the camera and overlay toggles (DESIGN §2). Owned by the
 * Viewport for the lifetime of one mounted viewport; a fresh document does
 * NOT need to reset it explicitly (there is nothing document-derived in a
 * `SectionPlane` — an object handle is never stored), though the Viewport
 * clears it on unmount along with every other session overlay.
 */
export class SectionManager {
  private plane: SectionPlane | null = null

  /** The current section, or null if none is placed. */
  get current(): SectionPlane | null {
    return this.plane
  }

  /** Place a new section (or replace the existing one — DESIGN §1: "placing
   * a new one replaces the previous"). Returns the new plane. */
  place(origin: readonly [number, number, number], normal: readonly [number, number, number]): SectionPlane {
    this.plane = createSectionPlane(origin, normal)
    return this.plane
  }

  /** Set the current plane to `plane` outright and return it — used to
   * commit a drag-offset gesture whose intermediate frames were only
   * previewed, not written here (see `SectionPlaneTool`). Unconditional:
   * it establishes the section whether or not one already existed. */
  setPlane(plane: SectionPlane): SectionPlane {
    this.plane = plane
    return this.plane
  }

  /** Offset the current plane by `distance` meters along its normal.
   * Returns the new plane, or null if no section exists. */
  offset(distance: number): SectionPlane | null {
    if (this.plane === null) return null
    this.plane = offsetSectionPlane(this.plane, distance)
    return this.plane
  }

  /** Toggle the current plane's active flag. Returns the new plane, or null
   * if no section exists. */
  toggleActive(): SectionPlane | null {
    if (this.plane === null) return null
    this.plane = toggleSectionPlaneActive(this.plane)
    return this.plane
  }

  /** Remove the section entirely — "the model returns to whole" (DESIGN §1). */
  delete(): void {
    this.plane = null
  }
}
