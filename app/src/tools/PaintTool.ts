/**
 * PaintTool — hover-highlight the picked face, click to paint it with the
 * current material. Also the eyedropper (Alt-sample) and Shift/Ctrl+Shift
 * "replace everywhere" gestures (paint-tool design §1–2).
 *
 * Gesture (single-click mode):
 *   1. Hover: pick_face() → highlight the face under the cursor
 *   2. Click: call scene.paint_face(object, face, currentMaterialId)
 *   3. Esc: cancel hover state
 *
 * Modifiers, read at pointerdown (the Viewport idiom Ctrl/Cmd already uses
 * for the whole-object fill below):
 *   - Alt held: eyedropper. Samples the clicked face's EFFECTIVE material
 *     (its own, else the object's base, else the Default swatch) and makes
 *     it current — the palette selection follows via `onSample`.
 *   - Shift: replace every face/object-default in the DOCUMENT whose
 *     material equals the clicked face's effective material with the
 *     current material (`scene.replace_material`, one atomic undo step).
 *   - Ctrl/Cmd+Shift: same replace, confined to the clicked object.
 *   - Ctrl/Cmd alone (no Shift): unchanged — fills the whole object's base
 *     material (`set_object_material`).
 *
 * The current material is set via `setCurrentMaterial(id)` from the
 * MaterialPalette panel. Sentinel `BigInt(0xFFFFFFFFFFFFFFFF)` = default/unpaint.
 *
 * Face picking reuses the existing pick_face() path (same as PushPullTool).
 *
 * Component placements (docs/agents/PLUGINS.md "Materials"): at the top of
 * the editing context, a click on geometry reached through an instance
 * paints THAT PLACEMENT (`set_instance_material`) — its unpainted faces
 * show the color, painted faces and painted bases keep theirs. Plain click
 * and ⌘/Ctrl-click do the same (an instance has one knob). Inside the
 * instance's own editing context the faces are the definition's, painted
 * one by one as above. Alt on a placement samples the material the clicked
 * member inherits (the nearest painted placement on its path, the same
 * nearest-wins rule the renderer draws with) when there is one, else the
 * face's own or base material. Shift replace is unchanged.
 */

import { t } from '@lingui/core/macro'
import type { Tool, Snap } from './types'
import type { Ray } from '../viewport/math'
import type { Scene as WasmScene } from '../wasm/loader'
import { parseKernelErrorCode, kernelErrorMessage } from '../kernelErrors'
import type { NodeRef } from '../panels/treeModel'

export type OnPaintCommit = (objectId: bigint) => void
export type OnToast = (message: string, code?: string) => void

/** Fired when Alt-click samples a face's effective material; the caller
 *  makes it current AND follows the palette selection (same effect as
 *  picking the swatch directly, minus the tool re-activation — Paint is
 *  already active). */
export type OnSampleMaterial = (id: bigint) => void

/** Fired after a committed `replace_material` — the caller re-tessellates.
 *  `scope` distinguishes a targeted single-object refresh from a
 *  document-wide one (an unknown, possibly large set of touched objects);
 *  `objectId` is the clicked object (meaningless, and ignored, for
 *  `'document'`). */
export type OnReplaceCommit = (scope: 'document' | 'object', objectId: bigint) => void

/** Fired after a placement is painted with the world instances whose
 *  drawing changed (the painted one, or every world placement drawing a
 *  nested one) — the caller refreshes them. */
export type OnInstancePaintCommit = (instanceIds: bigint[]) => void

/** Fired when the hovered placement changes (`null`: no placement under the
 *  pointer, or the tool went idle) — the caller outlines it. */
export type OnHoverInstance = (instanceId: bigint | null) => void

/** The first instance paint of a session explains itself once. */
let instancePaintExplained = false

/** Test hook: forget that the explanation was shown. */
export function resetInstancePaintExplained(): void {
  instancePaintExplained = false
}

/** `u64::MAX` as a BigInt — sentinel for "default / unpaint". */
export const MATERIAL_SENTINEL: bigint = BigInt('18446744073709551615')

export class PaintTool implements Tool {
  readonly name = 'Paint'

  /** Live status-bar guidance (see Tool.statusHint). `wholeObject`/
   *  `replaceScope` are set-then-consumed within one click, so the hint
   *  documents those modifiers as a static reminder rather than branching on
   *  a state a poll can never observe live; `eyedropper` DOES reflect the
   *  live Alt-held state (Viewport keeps it updated continuously, since the
   *  cursor swap needs the same live signal), so this hint IS state-aware
   *  for that one modifier. */
  statusHint(): string {
    if (this.eyedropper) {
      return t`Alt-click a face to sample its material.`
    }
    return t`Click a face to paint it with the current material — Shift-click replaces it everywhere, Ctrl/Cmd+Shift-click replaces within the object, Ctrl/Cmd-click fills the whole object, Alt = eyedropper.`
  }

  private wasmScene: WasmScene
  private onCommit: OnPaintCommit
  private onToast: OnToast
  private onSample: OnSampleMaterial
  private onReplace: OnReplaceCommit
  private currentMaterialId: bigint = MATERIAL_SENTINEL
  /** When true, the next click fills the whole object (base material). */
  private wholeObject = false
  /** Live Alt-held state (Viewport keeps this updated continuously via
   *  keydown/keyup, not just at pointerdown, since the cursor swap needs it
   *  between clicks too — see the class doc). */
  private eyedropper = false
  /** Set fresh at each pointerdown (Shift → 'document', Ctrl/Cmd+Shift →
   *  'object', neither → null); consumed by that same click. */
  private replaceScope: 'document' | 'object' | null = null

  /** The face currently hovered over (for highlight), or null. */
  hoveredObject: bigint | null = null
  hoveredFace: bigint | null = null
  /** The placement a click would paint (an instance pick at the top of the
   *  editing context), or null. */
  hoveredInstance: bigint | null = null

  /** The snap last seen on hover (for cue rendering). */
  lastSnap: Snap | null = null

  private getSessionStack: () => NodeRef[]
  private onInstanceCommit: OnInstancePaintCommit
  private onHoverInstance: OnHoverInstance

  constructor(
    wasmScene: WasmScene,
    onCommit: OnPaintCommit,
    onToast: OnToast,
    onSample: OnSampleMaterial,
    onReplace: OnReplaceCommit,
    getSessionStack: () => NodeRef[] = () => [],
    onInstanceCommit: OnInstancePaintCommit = () => {},
    onHoverInstance: OnHoverInstance = () => {},
  ) {
    this.wasmScene = wasmScene
    this.onCommit = onCommit
    this.onToast = onToast
    this.onSample = onSample
    this.onReplace = onReplace
    this.getSessionStack = getSessionStack
    this.onInstanceCommit = onInstanceCommit
    this.onHoverInstance = onHoverInstance
  }

  /** The placement a pick paints, or undefined: the pick's instance, unless
   *  it is the frame being edited (or one enclosing it) — inside its own
   *  editing context a placement's faces are painted one by one. */
  private _paintableInstance(pick: { instance(): bigint | undefined }): bigint | undefined {
    const instance = pick.instance()
    if (instance === undefined) return undefined
    const inFrame = this.getSessionStack().some((f) => f.kind === 'instance' && f.id === instance)
    return inFrame ? undefined : instance
  }

  /** The material `member` inherits as drawn by `instance`: the nearest
   *  painted placement on its path (`instance_expanded_inherited`, parallel
   *  to `instance_expanded_members`; a member placed more than once reads
   *  its first placement, as a pick names the member, not the placement),
   *  or the sentinel. */
  private _inheritedAt(instance: bigint, member: bigint): bigint {
    const members = this.wasmScene.instance_expanded_members(instance)
    const inherited = this.wasmScene.instance_expanded_inherited(instance)
    for (let i = 0; i < members.length; i++) {
      if (members[i] === member) return inherited[i] ?? MATERIAL_SENTINEL
    }
    return MATERIAL_SENTINEL
  }

  private _setHoveredInstance(id: bigint | null): void {
    if (id === this.hoveredInstance) return
    this.hoveredInstance = id
    this.onHoverInstance(id)
  }

  /** Set the active material id. `MATERIAL_SENTINEL` = unpaint. */
  setCurrentMaterial(id: bigint): void {
    this.currentMaterialId = id
  }

  getCurrentMaterial(): bigint {
    return this.currentMaterialId
  }

  /**
   * When `b` is true the next click sets the **object base material** via
   * `set_object_material` instead of painting a single face. Auto-resets to
   * false after one click.
   */
  setWholeObject(b: boolean): void {
    this.wholeObject = b
  }

  /** Live Alt-held state — switches the cursor to the eyedropper and, at the
   *  next click, samples instead of paints. Unlike `wholeObject`, this is
   *  NOT one-shot: Viewport keeps it in sync with the real key state via
   *  keydown/keyup for as long as Paint stays active, so the cursor can
   *  react between clicks too. */
  setEyedropper(b: boolean): void {
    this.eyedropper = b
  }

  getEyedropper(): boolean {
    return this.eyedropper
  }

  /** Set the next click's replace scope (`null` = plain paint/whole-object,
   *  per `wholeObject`). Auto-resets after that click, same posture as
   *  `wholeObject`. */
  setReplaceScope(scope: 'document' | 'object' | null): void {
    this.replaceScope = scope
  }

  onPointerMove(snap: Snap | null, ray: Ray): void {
    this.lastSnap = snap

    // Ray-cast for the nearest face to drive the hover highlight.
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )

    if (pick !== undefined) {
      try {
        this.hoveredObject = pick.object()
        this.hoveredFace = pick.face()
        this._setHoveredInstance(this._paintableInstance(pick) ?? null)
      } finally {
        pick.free()
      }
    } else {
      this.hoveredObject = null
      this.hoveredFace = null
      this._setHoveredInstance(null)
    }
  }

  onPointerDown(snap: Snap | null, ray: Ray): void {
    // Re-pick at click position (move may have been slightly different).
    const pick = this.wasmScene.pick_face(
      ray.origin[0], ray.origin[1], ray.origin[2],
      ray.direction[0], ray.direction[1], ray.direction[2],
    )

    if (pick === undefined) return

    const wholeObject = this.wholeObject
    this.wholeObject = false
    const replaceScope = this.replaceScope
    this.replaceScope = null

    try {
      const objectHandle = pick.object()
      const faceHandle = pick.face()
      const instance = this._paintableInstance(pick)
      // Alt (eyedropper) takes priority over every other modifier — SketchUp
      // parity, and it keeps the gesture unambiguous (Alt+Shift held together
      // samples, it never also replaces).
      if (this.eyedropper) {
        const inherited =
          pick.instance() !== undefined ? this._inheritedAt(pick.instance() as bigint, objectHandle) : MATERIAL_SENTINEL
        if (inherited !== MATERIAL_SENTINEL) {
          this.currentMaterialId = inherited
          this.onSample(inherited)
        } else {
          this._sample(objectHandle, faceHandle)
        }
      } else if (replaceScope !== null) {
        this._commitReplace(objectHandle, faceHandle, replaceScope)
      } else if (instance !== undefined) {
        // Plain click and ⌘/Ctrl-click alike: a placement has one knob.
        this._commitInstance(instance)
      } else if (wholeObject) {
        this._commitObject(objectHandle)
      } else {
        this._commit(objectHandle, faceHandle)
      }
    } finally {
      pick.free()
    }
  }

  onKey(ev: KeyboardEvent): void {
    if (ev.key === 'Escape') {
      this.cancel()
    }
  }

  cancel(): void {
    this.hoveredObject = null
    this.hoveredFace = null
    this.lastSnap = null
    this._setHoveredInstance(null)
  }

  /** Paint one placement (its own material), explaining the first time per
   *  session that its faces are the component's to paint inside it. */
  private _commitInstance(instance: bigint): void {
    try {
      const touched = this.wasmScene.set_instance_material(instance, this.currentMaterialId)
      this.onInstanceCommit(touched.length > 0 ? [...touched] : [instance])
      if (!instancePaintExplained) {
        instancePaintExplained = true
        this.onToast(
          t`Painting a component colors this placement. Edit the component to paint its faces.`,
        )
      }
    } catch (err) {
      this._handleError(err)
    }
  }

  private _commit(objectHandle: bigint, faceHandle: bigint): void {
    try {
      this.wasmScene.paint_face(objectHandle, faceHandle, this.currentMaterialId)
      this.onCommit(objectHandle)
    } catch (err) {
      this._handleError(err)
    }
  }

  /** Set the base material on the whole object (⌘/Ctrl-click). */
  private _commitObject(objectHandle: bigint): void {
    try {
      this.wasmScene.set_object_material(objectHandle, this.currentMaterialId)
      this.onCommit(objectHandle)
    } catch (err) {
      this._handleError(err)
    }
  }

  /** Alt-click: sample the clicked face's effective material (its own,
   *  else the object's base, else the Default swatch) and make it current —
   *  the palette selection follows via `onSample`. A stale pick (the object
   *  vanished between the hover-pick and this click) is a silent no-op,
   *  same posture as a pick miss elsewhere in this tool. */
  private _sample(objectHandle: bigint, faceHandle: bigint): void {
    const info = this.wasmScene.face_material(objectHandle, faceHandle)
    if (info === undefined) return
    try {
      const effective = this._effectiveMaterial(info)
      this.currentMaterialId = effective
      this.onSample(effective)
    } finally {
      info.free()
    }
  }

  /** Shift/Ctrl+Shift-click: replace every assignment (in `scope`) equal to
   *  the clicked face's effective material with the current material, in
   *  one atomic kernel op. */
  private _commitReplace(objectHandle: bigint, faceHandle: bigint, scope: 'document' | 'object'): void {
    const info = this.wasmScene.face_material(objectHandle, faceHandle)
    if (info === undefined) return
    let from: bigint
    try {
      from = this._effectiveMaterial(info)
    } finally {
      info.free()
    }
    try {
      this.wasmScene.replace_material(scope === 'document', objectHandle, from, this.currentMaterialId)
      this.onReplace(scope, objectHandle)
    } catch (err) {
      this._handleError(err)
    }
  }

  /** A face's effective material: its own, else its object's base, else the
   *  Default swatch (both resolve to `MATERIAL_SENTINEL` already — the wasm
   *  boundary uses the identical sentinel convention on both knobs). */
  private _effectiveMaterial(info: { face(): bigint; object_default(): bigint }): bigint {
    const own = info.face()
    return own !== MATERIAL_SENTINEL ? own : info.object_default()
  }

  private _handleError(err: unknown): void {
    const code = parseKernelErrorCode(err)
    const rawMsg = err instanceof Error ? err.message : String(err)
    const message = kernelErrorMessage(code ?? 'Unknown', rawMsg)
    this.onToast(message, code ?? undefined)
  }
}
