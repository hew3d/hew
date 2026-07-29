/**
 * annotationEdit — reads a live annotation's full current geometry and
 * re-issues it through `update_linear_dimension` / `update_radial_dimension`
 * / `update_leader_text` with an edited text field.
 *
 * The kernel's `update_annotation` always replaces the WHOLE annotation
 * value (there is no partial-field patch surface — see
 * `crates/kernel/src/document.rs`'s doc comment), so editing just the text
 * means: read every other field back from the live annotation, then commit
 * the full record with only the text changed. This is the seam
 * `AnnotationEditor`'s commit handler (double-click-to-edit, and a
 * dimension's `text_override` clear-to-restore) goes through — kept
 * import-light (no THREE/DOM) so it's unit-testable against a fake
 * `WasmScene` alone.
 */
import type { Scene as WasmScene } from '../wasm/loader'
import type { V3 } from './geoHelpers'

interface AnchorRef {
  nodeKind: number
  nodeId: bigint
  point: V3
}

export type AnnotationSnapshot =
  | {
      kind: 'linear'
      a: AnchorRef
      b: AnchorRef
      offset: V3
      plane: [number, number, number, number, number, number]
      textOverride: string | undefined
    }
  | {
      kind: 'radial'
      anchor: AnchorRef
      radialKind: 'radius' | 'diameter'
      curveCenter: V3
      curveRadius: number
      curvePlane: [number, number, number, number, number, number]
      leaderDir: V3
      textOverride: string | undefined
    }
  | {
      kind: 'leader'
      anchor: AnchorRef
      offset: V3
      text: string
    }

function readAnchor(wasmScene: WasmScene, id: bigint, which: number): AnchorRef | null {
  const nodeKind = wasmScene.annotation_anchor_node_kind(id, which)
  const point = wasmScene.annotation_anchor_point(id, which)
  if (nodeKind === undefined || point === undefined) return null
  const nodeId = nodeKind >= 0 ? wasmScene.annotation_anchor_node_id(id, which) ?? 0n : 0n
  return { nodeKind, nodeId, point: [point[0], point[1], point[2]] }
}

/** Read a live annotation's complete current geometry + text, or `null` if
 * `id` is stale/hidden or any read comes back incomplete (a torn read —
 * should not happen for a live id, but this is never allowed to throw). */
export function readAnnotation(wasmScene: WasmScene, id: bigint): AnnotationSnapshot | null {
  const kind = wasmScene.annotation_kind(id)
  if (kind === undefined) return null

  if (kind === 'linear') {
    const a = readAnchor(wasmScene, id, 0)
    const b = readAnchor(wasmScene, id, 1)
    const offset = wasmScene.annotation_offset(id)
    const plane = wasmScene.annotation_plane(id)
    if (a === null || b === null || offset === undefined || plane === undefined) return null
    return {
      kind: 'linear',
      a,
      b,
      offset: [offset[0], offset[1], offset[2]],
      plane: [plane[0], plane[1], plane[2], plane[3], plane[4], plane[5]],
      textOverride: wasmScene.annotation_text_override(id),
    }
  }

  if (kind === 'radial') {
    const anchor = readAnchor(wasmScene, id, 0)
    const radialKind = wasmScene.annotation_radial_kind(id)
    const curve = wasmScene.annotation_curve(id)
    const leaderDir = wasmScene.annotation_leader_dir(id)
    if (anchor === null || radialKind === undefined || curve === undefined || leaderDir === undefined) return null
    return {
      kind: 'radial',
      anchor,
      radialKind: radialKind as 'radius' | 'diameter',
      curveCenter: [curve[0], curve[1], curve[2]],
      curveRadius: curve[3],
      curvePlane: [curve[4], curve[5], curve[6], curve[7], curve[8], curve[9]],
      leaderDir: [leaderDir[0], leaderDir[1], leaderDir[2]],
      textOverride: wasmScene.annotation_text_override(id),
    }
  }

  // 'leader'
  const anchor = readAnchor(wasmScene, id, 0)
  const offset = wasmScene.annotation_offset(id)
  if (anchor === null || offset === undefined) return null
  return {
    kind: 'leader',
    anchor,
    offset: [offset[0], offset[1], offset[2]],
    text: wasmScene.annotation_text(id) ?? '',
  }
}

/** The text an editor should show for `snapshot` — a dimension's
 * `text_override` if set, else `''` (empty means "no override", NOT the
 * app-computed measurement — the editor's placeholder shows that instead,
 * mirroring SketchUp's `<>` convention where the field starts blank until
 * the user actually types an override). Leader text starts pre-filled with
 * its real content. */
export function initialEditorText(snapshot: AnnotationSnapshot): string {
  if (snapshot.kind === 'leader') return snapshot.text
  return snapshot.textOverride ?? ''
}

/**
 * Commit an edit to `id`, re-issuing its full current geometry with the
 * text field replaced: `newText` for a leader, or a linear/radial's
 * `text_override` (empty string clears the override, restoring the
 * app-computed measurement — the design doc's simplified SketchUp `<>`
 * semantics).
 */
export function commitAnnotationText(
  wasmScene: WasmScene,
  id: bigint,
  snapshot: AnnotationSnapshot,
  newText: string,
): void {
  if (snapshot.kind === 'linear') {
    const override = newText.trim() === '' ? undefined : newText
    wasmScene.update_linear_dimension(
      id,
      snapshot.a.nodeKind, snapshot.a.nodeId, new Float64Array(snapshot.a.point),
      snapshot.b.nodeKind, snapshot.b.nodeId, new Float64Array(snapshot.b.point),
      new Float64Array(snapshot.offset),
      new Float64Array(snapshot.plane),
      override,
    )
    return
  }
  if (snapshot.kind === 'radial') {
    const override = newText.trim() === '' ? undefined : newText
    wasmScene.update_radial_dimension(
      id,
      snapshot.anchor.nodeKind, snapshot.anchor.nodeId, new Float64Array(snapshot.anchor.point),
      snapshot.radialKind,
      new Float64Array(snapshot.curveCenter),
      snapshot.curveRadius,
      new Float64Array(snapshot.curvePlane),
      new Float64Array(snapshot.leaderDir),
      override,
    )
    return
  }
  wasmScene.update_leader_text(
    id,
    snapshot.anchor.nodeKind, snapshot.anchor.nodeId, new Float64Array(snapshot.anchor.point),
    new Float64Array(snapshot.offset),
    newText,
  )
}
