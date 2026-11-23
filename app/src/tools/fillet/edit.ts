/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Edit stage: resizing, re-faceting, removing and switching an existing blend. */

import { afterLate, whenReady } from './previewer'
import type { Snap } from '../types'
import { sameRay, type Ray } from '../../viewport/math'
import { parseKernelErrorCode, kernelErrorMessage } from '../../kernelErrors'
import { formatLength, formatLengthLimit, typedReadout } from '../../settings/units'
import { applyAffine3x4Linear, normalizeV3 } from '../../viewport/geoHelpers'
import { arcSegments } from '../arcMath'
import { getFilletSettings, setFilletProfile, toggledProfile } from '../filletSettings'
import { MIN_FILLET_SIZE, clampBlendSegments, limitedSentence, type V3 } from '../filletMath'
import { lineParamNearRay } from '../filletGrips'
import { blendHoverAt, toWorldPoint, type Frame, type Hover } from '../filletPick'
import type { Pick, EditStage } from './types'
import { FACE_MASK_MAX_TRIANGLES, SLOW_PREVIEW_MS, SLOW_PREVIEW_HINT } from './constants'
import { offsetBy } from './helpers'
import { editProfileJson, editSize } from './spec'
import type { FilletContext } from './context'

export class EditStageOps {
  constructor(private readonly c: FilletContext) {}

  enterEdit(h: Extract<Hover, { kind: 'blend' }>, ray: Ray, held = false): void {
    let rows: Array<{ id: number; kind: string; size: number | null; intact: boolean }>
    try {
      rows = JSON.parse(this.c.wasmScene.blend_features_json(h.object))
    } catch {
      return
    }
    const row = rows.find((r) => r.id === h.blendId)
    if (row === undefined) return
    if (!row.intact) {
      this.c.hooks.onToast(kernelErrorMessage('BlendModified', ''))
      return
    }
    const n = this.c.wasmScene.face_normal(h.object, h.face)
    const world = toWorldPoint(h.frame, h.point)
    const normal: V3 = h.frame.pose === null ? [n[0], n[1], n[2]] : applyAffine3x4Linear(h.frame.pose, [n[0], n[1], n[2]])
    const axis = normalizeV3(normal) ?? [0, 0, 1]
    this.c.hover = { kind: 'none' }
    this.c.stage = {
      kind: 'edit',
      object: h.object,
      frame: h.frame,
      blendId: h.blendId,
      kindOfBlend: row.kind,
      size0: (row.size ?? 0.01) * h.frame.scale,
      typed: '',
      amount: null,
      anchorPoint: world,
      anchorNormal: normal,
      anchorRay: ray,
      armedScreen: this.c.screen,
      limit: null,
      handle: { base: world, axis },
      grab: held,
      pressEnter: held,
      grabScreen: held ? this.c.screen : null,
      grabRay: held ? ray : null,
    }
    this.c.preview.clearHighlight()
    this.c.measure(formatLength((row.size ?? 0) * h.frame.scale))
    this.drawEditPreview()
    this.refreshEditMarkers()
  }

  drawEditPreview(): void {
    const s = this.c.stage
    if (s.kind !== 'edit') return
    const round = this.c.preview.startRound()
    this.c.preview.clearPatch()
    const size = editSize(s)
    if (size === null) {
      this.c.preview.clearPreviewDrawing()
      this.refreshEditMarkers()
      return
    }
    const k = 1 / s.frame.scale
    const settings = getFilletSettings()
    const key = [settings.profile.kind, settings.profile.kind === 'shape' ? settings.profile.id : '', settings.segmentsOverride ?? ''].join('|')
    const limit = s.limit !== null && s.limit.key === key ? s.limit : null
    const atLimit = limit !== null && size * k > limit.max
    const json = editProfileJson(s, atLimit ? limit.max / k : size, k, size)
    if (json === null) {
      this.c.preview.clearPreviewDrawing()
      this.c.statusLine = 'Pick the profile under More shapes first.'
      this.refreshEditMarkers()
      return
    }
    this.c.statusLine = null
    this.c.measure(s.typed === '' ? formatLength(size) : typedReadout(s.typed))
    this.refreshEditMarkers()
    if (this.c.slowObjects.has(s.object)) {
      this.c.preview.releaseMask(s.object)
      this.c.statusLine = SLOW_PREVIEW_HINT
      return
    }
    const drawn = whenReady(
      this.c.previewer.request({ kind: 'edit', object: s.object, blendId: s.blendId, spec: json }),
      () => this.c.epoch === round,
      (r) => {
        if (r.kind === 'refused') {
          if (limit !== null && atLimit) {
            s.limit = null
            this.drawEditPreview()
            return
          }
          this.c.preview.clearPreviewDrawing()
          this.c.statusLine = this.c.preview.refusalText(r.message)
          return
        }
        if (!r.clamped && r.elapsedMs >= SLOW_PREVIEW_MS) this.c.slowObjects.add(s.object)
        const clamped = atLimit || r.clamped
        if (limit !== null && atLimit) {
          this.c.statusLine = limitedSentence('Radius', limit.max * s.frame.scale, limit.limitedBy, formatLengthLimit)
        } else if (clamped) {
          s.limit = { key, max: r.maxSize, limitedBy: r.limitedBy }
          this.c.statusLine = limitedSentence('Radius', r.maxSize * s.frame.scale, r.limitedBy, formatLengthLimit)
        }
        this.c.preview.drawResult(s.object, s.frame, r, clamped)
      },
    )
    // A late answer repaints outside any event; a direct one rides the event.
    afterLate(drawn, () => this.c.hooks.requestRender())
  }

  enterOnEdit(s: EditStage): void {
    if (s.typed !== '') {
      const t = this.c.size.parseTyped(s.typed)
      if (t === null) return
      if (t.kind === 'segments') {
        this.c.size.applySegmentsOverride(t.n)
        s.typed = ''
        this.refacetEdited(s)
        return
      }
      if (t.kind !== 'length') {
        this.c.hooks.onToast('Type one length.')
        return
      }
    }
    this.commitEdit()
  }

  /** A facet count typed over a blend being edited re-facets it at its
   *  current size (`8s` on a band: eight facets, nothing else changed). The
   *  note stays up through the commit so the status bar says what happened. */
  refacetEdited(s: EditStage): void {
    const json = editProfileJson(s, s.size0, 1)
    if (json === null) {
      this.drawEditPreview()
      return
    }
    try {
      const report =
        s.frame.instance === null
          ? this.c.wasmScene.resize_blend(s.object, s.blendId, json)
          : this.c.wasmScene.resize_blend_in_component(s.frame.instance, s.object, s.blendId, json)
      report.free()
    } catch (err) {
      this.c.hooks.onToast(this.c.preview.refusalText(err), parseKernelErrorCode(err) ?? undefined)
      return
    }
    const { note, noteRay } = this.c
    const object = s.object
    this.c.commit.clearAfterCommit()
    this.c.hooks.onCommit([object])
    this.c.note = note
    this.c.noteRay = noteRay
  }

  commitEdit(): void {
    const s = this.c.stage
    if (s.kind !== 'edit') return
    this.c.finishedAt = this.c.screen
    const size = editSize(s)
    if (size === null || (s.typed === '' && Math.abs(size - s.size0) < 1e-9)) {
      // Nothing changed: the edit closes and the blend stays as it was.
      this.c.commit.clearAfterCommit()
      return
    }
    if (!(size > MIN_FILLET_SIZE)) {
      this.c.hooks.onToast('Type a size larger than zero.')
      return
    }
    const json = editProfileJson(s, size, 1 / s.frame.scale)
    if (json === null) {
      this.c.hooks.onToast('Pick the profile under More shapes first.')
      return
    }
    try {
      const report =
        s.frame.instance === null
          ? this.c.wasmScene.resize_blend(s.object, s.blendId, json)
          : this.c.wasmScene.resize_blend_in_component(s.frame.instance, s.object, s.blendId, json)
      report.free()
    } catch (err) {
      this.c.hooks.onToast(this.c.preview.refusalText(err), parseKernelErrorCode(err) ?? undefined)
      return
    }
    const object = s.object
    this.c.commit.clearAfterCommit()
    this.c.hooks.onCommit([object])
  }

  removeEdited(): void {
    const s = this.c.stage
    if (s.kind !== 'edit') return
    this.removeBlend(s.object, s.frame, s.blendId)
  }

  removeHovered(h: Extract<Hover, { kind: 'blend' }>): void {
    this.removeBlend(h.object, h.frame, h.blendId)
  }

  removeBlend(object: bigint, frame: Frame, blendId: number): void {
    try {
      if (frame.instance === null) this.c.wasmScene.remove_blend(object, blendId)
      else this.c.wasmScene.remove_blend_in_component(frame.instance, object, blendId)
    } catch (err) {
      this.c.hooks.onToast(this.c.preview.refusalText(err), parseKernelErrorCode(err) ?? undefined)
      return
    }
    this.c.commit.clearAfterCommit()
    this.c.hooks.onCommit([object])
  }

  toggleKind(): void {
    const s = this.c.stage
    if (s.kind === 'edit') {
      this.switchEditedKind(s)
      return
    }
    const next = toggledProfile(s.kind === 'armed' ? s.profile : getFilletSettings().profile)
    setFilletProfile(next)
    if (s.kind === 'armed') {
      s.profile = next
      this.c.preview.drawArmedPreview(true)
      this.c.hooks.requestRender()
    }
  }

  /** After a kind switch the pointer is still over the band: the edit
   *  re-opens on the result (one frame later, after the scene refresh). */
  reenterEditAt(point: V3, normal: V3): void {
    // A ray from just outside the band, down its normal: the rebuilt band
    // is under it whether or not the pointer still is (it may sit on the
    // handle, a size away).
    const ray: Ray = {
      origin: [point[0] + normal[0] * 0.05, point[1] + normal[1] * 0.05, point[2] + normal[2] * 0.05],
      direction: [-normal[0], -normal[1], -normal[2]],
    }
    this.c.schedule(() => {
      // Cancelled before the frame (a tool switch, Esc): nothing to re-open.
      if (this.c.stage.kind !== 'idle' || this.c.root.parent !== this.c.previewGroup) return
      const h = blendHoverAt(this.c.wasmScene, ray)
      if (h === null) return
      this.enterEdit(h, ray, false)
      this.c.hooks.requestRender()
    })
  }

  /** Alt on a blend being edited: resize to the other kind at the same size. */
  switchEditedKind(s: EditStage): void {
    const size = editSize(s) ?? s.size0
    const toChamfer = s.kindOfBlend !== 'chamfer'
    // `size` is world meters; the kernel takes the instance's own.
    const local = size / s.frame.scale
    const json = toChamfer
      ? JSON.stringify({ kind: 'chamfer', setback_a: local, setback_b: local })
      : JSON.stringify({
          kind: 'round',
          radius: local,
          segments: clampBlendSegments(arcSegments(size, Math.PI / 2)),
        })
    try {
      const report =
        s.frame.instance === null
          ? this.c.wasmScene.resize_blend(s.object, s.blendId, json)
          : this.c.wasmScene.resize_blend_in_component(s.frame.instance, s.object, s.blendId, json)
      report.free()
    } catch (err) {
      this.c.hooks.onToast(this.c.preview.refusalText(err), parseKernelErrorCode(err) ?? undefined)
      return
    }
    const object = s.object
    const { anchorPoint, anchorNormal } = s
    this.c.commit.clearAfterCommit()
    this.c.hooks.onCommit([object])
    this.reenterEditAt(anchorPoint, anchorNormal)
  }

  pointerDownEdit(s: EditStage, snap: Snap | null, ray: Ray): void {
    if (s.grab) {
      this.releaseEditHandle(s, ray)
      return
    }
    if (this.editHandleUnder(s, ray)) {
      s.grab = true
      s.pressEnter = false
      s.grabScreen = this.c.screen
      s.grabRay = ray
      return
    }
    // Elsewhere finishes the edit; an edge, or a band on another object,
    // gets the click.
    const object = s.object
    this.commitEdit()
    if (this.c.stage.kind === 'idle' && this.c.pick.clickWantsNext(ray, new Set([object]))) this.c.dispatchPointerDown(snap, ray)
  }

  editHandleUnder(s: EditStage, ray: Ray): boolean {
    const m = this.c.gripsView.pick(ray.origin, ray.direction)
    void s
    return m !== null && m.id.kind === 'radius'
  }

  releaseEditHandle(s: EditStage, ray: Ray): void {
    const moved = s.grabScreen === null ? !sameRay(s.grabRay, ray) : this.c.movedEnough(s.grabScreen)
    const pressEnter = s.pressEnter
    s.pressEnter = false
    s.grab = false
    if (moved) {
      this.c.pendingMove = null
      if (s.typed === '') this.editSizeFromHandle(s, ray)
      this.drawEditPreview()
      this.refreshEditMarkers()
      return
    }
    if (!pressEnter) this.commitEdit()
  }

  editSizeFromHandle(s: EditStage, ray: Ray): void {
    const along = lineParamNearRay(s.handle.base, s.handle.axis, ray.origin, ray.direction)
    if (along === null) return
    s.amount = Math.max(MIN_FILLET_SIZE, along)
  }

  /** The edit stage's gizmo: the handle at the band, at the current size. */
  refreshEditMarkers(): void {
    const s = this.c.stage
    if (s.kind !== 'edit') {
      this.c.gripsView.clear()
      this.c.labels.clear()
      return
    }
    const size = editSize(s) ?? s.size0
    const pos = offsetBy(s.handle.base, s.handle.axis, size)
    this.c.gripsView.setMarkers([{ id: { kind: 'radius' }, pos, from: s.handle.base, active: true }])
    this.c.labels.set([
      { key: 'radius', pos, dir: s.handle.axis, text: s.typed === '' ? formatLength(size) : typedReadout(s.typed), active: true },
    ])
  }
}
