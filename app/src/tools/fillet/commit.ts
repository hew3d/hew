/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
/** Commit stage: committing, retyping and recommitting. */

import { i18n } from '@lingui/core'
import { BUSY_REFUSED } from '../../busyCard'
import { buildTreeIndexMap, nodeKey, nodeRefFromJs, resolveLabel } from '../../panels/treeModel'
import { parseKernelErrorCode } from '../../kernelErrors'
import { formatLengthLimit, typedReadout } from '../../settings/units'
import { retypeStaleMessage } from '../retypeWindow'
import { inHistoryBracket } from '../transformSelection'
import { runDraft, refreshAfterDraft } from '../draftCommit'
import { editCurveBuffer, isCurveInputKey, type CurveToken } from '../curveInput'
import { getFilletSettings, setFilletThreeWayCorners, type FilletProfile } from '../filletSettings'
import { MIN_FILLET_SIZE, clampCornerSegments } from '../filletMath'
import { cornerDefaultSegments } from '../filletCorner2d'
import { emptyGripState, gripStateIsActive, type GripState } from '../filletGrips'
import type { Amount, CornerArm, ArmTarget, Sized, Resolved, RetypeSpec } from './types'
import { tokenOf } from './helpers'
import { resolve, specJson, cornerClamp } from './spec'
import type { FilletContext } from './context'

export class CommitStage {
  constructor(private readonly c: FilletContext) {}

  commitArmed(amount: Amount): void {
    const s = this.c.stage
    if (s.kind !== 'armed') return
    const finishedBefore = this.c.finishedAt
    this.c.finishedAt = this.c.screen
    const commit = () => this.commitTargets(s.targets, s.profile, amount, s.grips)
    // An object too slow to preview takes seconds to commit: say so first.
    if (this.c.hooks.busy !== undefined && s.targets.some((t) => t.kind === 'solid' && this.c.slowObjects.has(t.object))) {
      const took = this.c.hooks.busy(s.profile.kind === 'chamfer' ? 'Chamfering…' : 'Rounding…', () => {
        // Should the stage have moved on while the card painted, commit
        // nothing.
        if (this.c.stage === s) commit()
      })
      // Another long change holds the card: nothing ran, the stage stays
      // armed with what was typed, and the toast says so.
      if (took === false) {
        this.c.finishedAt = finishedBefore
        this.c.hooks.onToast(i18n._(BUSY_REFUSED))
      }
      return
    }
    commit()
  }

  commitTargets(targets: ArmTarget[], profile: FilletProfile, amount: Amount, grips?: GripState): void {
    const resolved = resolve(targets[0], amount, profile)
    if ('error' in resolved) {
      this.c.hooks.onToast(resolved.error)
      return
    }
    if (resolved.headline !== null && !(resolved.headline > MIN_FILLET_SIZE)) {
      this.c.hooks.onToast('Move more before finishing, or type a size.')
      return
    }
    const gripState = grips ?? emptyGripState()
    const depthBefore = this.c.wasmScene.undo_depth()
    // The preview drew its facets at the asked size; a held commit keeps them.
    let facetsFor = resolved.sized
    let handleScale = 1
    if (amount.src === 'handle' && amount.held !== undefined) {
      handleScale = amount.held.handleScale
      const asked = resolve(targets[0], amount.held.asked, profile)
      if (!('error' in asked)) facetsFor = asked.sized
    }
    const ok = this.runCommit(targets, profile, resolved, gripState, facetsFor, handleScale)
    if (!ok) return
    this.c.lastLabel = resolved.label
    const token = tokenOf(resolved)
    if (token !== null) this.c.lastToken = token
    this.c.lastVaried = gripStateIsActive(gripState)
    this.clearAfterCommit()
    // Entries by undo depth: a bracketed multi-object commit moves the
    // generation by several but the depth by one.
    this.c.retype.arm(
      { targets, profile, token, grips: gripState, size: resolved.headline },
      Math.max(1, this.c.wasmScene.undo_depth() - depthBefore),
    )
  }

  /** Runs the kernel commit for every target as ONE undo step. True when
   *  accepted; a refusal is toasted and nothing changed. */
  runCommit(
    targets: ArmTarget[],
    profile: FilletProfile,
    resolved: Resolved,
    grips: GripState,
    facetsFor: Sized,
    handleScale: number,
  ): boolean {
    let refusing: ArmTarget | null = null
    const step = (): void => {
      for (const t of targets) {
        refusing = t
        this.commitOne(t, profile, resolved, grips, facetsFor, handleScale)
      }
    }
    try {
      if (targets.length > 1) inHistoryBracket(this.c.wasmScene, 'Fillet', step)
      else step()
    } catch (err) {
      const code = parseKernelErrorCode(err)
      let text = this.c.preview.refusalText(err)
      // Several objects commit as one step: one refusal leaves all of them
      // as they were, and the toast says which one stopped it.
      const who = refusing as ArmTarget | null
      if (targets.length > 1 && who !== null && who.kind === 'solid') {
        const name = this.objectLabel(who.object)
        text = `Nothing changed. ${name} can't take this: ${text}`
      }
      this.c.statusLine = null
      this.c.hooks.onToast(text, code ?? undefined)
      return false
    }
    const objects = targets.flatMap((t) => (t.kind === 'solid' ? [t.object] : []))
    if (objects.length > 0) this.c.hooks.onCommit(objects)
    if (targets.some((t) => t.kind === 'corner')) this.c.hooks.onSketchChange()
    return true
  }

  /** The Outliner's label for an object (same resolver and positional index). */
  objectLabel(object: bigint): string {
    const scene = this.c.wasmScene
    let idx = 0
    try {
      const map = buildTreeIndexMap(
        scene.top_level_nodes().map(nodeRefFromJs),
        (groupId) => scene.group_members(groupId).map(nodeRefFromJs),
      )
      idx = map.get(nodeKey({ kind: 'object', id: object })) ?? 0
    } catch {
      idx = 0
    }
    return resolveLabel(scene.object_name(object), undefined, 'object', idx)
  }

  commitOne(
    t: ArmTarget,
    profile: FilletProfile,
    resolved: Resolved,
    grips: GripState,
    facetsFor: Sized,
    handleScale: number,
  ): void {
    if (t.kind === 'corner') {
      this.commitCorner(t, resolved.sized)
      return
    }
    const json = specJson(t, resolved.sized, profile, 1, grips, facetsFor, handleScale)
    const report =
      t.frame.instance === null
        ? this.c.wasmScene.blend_edges(t.object, json)
        : this.c.wasmScene.blend_edges_in_component(t.frame.instance, t.object, json)
    report.free()
  }

  commitCorner(t: CornerArm, sized: Sized): void {
    const settings = getFilletSettings()
    // The preview held at the corner's limit: the commit makes that size.
    const held = cornerClamp(t, sized)
    let clampNote: string | null = null
    if (held !== null) {
      sized = held.sized
      // The size was cut down to fit: say so, as the preview did, but only
      // once the kernel has taken the commit (a refusal gets its own toast).
      clampNote = `${sized.kind === 'chamfer' ? 'Cut' : 'Rounded'} as far as this corner allows: ${formatLengthLimit(held.limit)}.`
    }
    let json: string
    if (sized.kind === 'round') {
      json = JSON.stringify({
        kind: 'round',
        radius: sized.radius,
        segments: clampCornerSegments(settings.segmentsOverride ?? cornerDefaultSegments(sized.radius, t.theta)),
      })
    } else if (sized.kind === 'chamfer') {
      json = JSON.stringify({ kind: 'chamfer', setback_a: sized.a, setback_b: sized.b })
    } else {
      throw new Error('BadSpec: a drawn corner takes a round or a chamfer')
    }
    const c = t.corner
    if (c.commit.kind === 'sketch') {
      this.c.wasmScene.sketch_blend_corner(c.commit.sketch, c.commit.vertex, json)
      if (clampNote !== null) this.c.hooks.onToast(clampNote)
      return
    }
    const { report } = runDraft(this.c.wasmScene, { kind: 'ctx', ctx: c.ctx }, 'Round corner', (d) => {
      this.c.wasmScene.draft_blend_corner(d, c.point[0], c.point[1], c.point[2], json)
    })
    refreshAfterDraft(report, {
      onObject: (o) => this.c.hooks.onCommit([o]),
      onSketch: () => this.c.hooks.onSketchChange(),
    })
    if (clampNote !== null) this.c.hooks.onToast(clampNote)
  }

  clearAfterCommit(): void {
    this.c.unsubSettings?.()
    this.c.unsubSettings = null
    this.c.stage = { kind: 'idle' }
    this.c.preview.clearPreviewDrawing()
    this.c.gripsView.clear()
    this.c.labels.clear()
    this.c.statusLine = null
    setFilletThreeWayCorners(0)
    this.c.measure('')
    this.c.hover = { kind: 'none' }
    this.c.preview.clearHighlight()
    this.c.pick.syncCornerProfile()
  }

  retypeKey(key: string): void {
    if (key === 'Enter') {
      const token = this.c.size.parseTyped(this.c.retypeTyped)
      this.c.retypeTyped = ''
      this.c.measure('')
      if (token === null) return
      if (token.kind === 'segments') {
        this.c.size.applySegmentsOverride(token.n)
        return
      }
      this.retypeSize(token)
      return
    }
    if (!isCurveInputKey(key)) return
    this.c.retypeTyped = editCurveBuffer(this.c.retypeTyped, key)
    this.c.measure(this.c.retypeTyped === '' ? '' : typedReadout(this.c.retypeTyped))
  }

  retypeSize(token: CurveToken): void {
    const outcome = this.c.retype.apply(
      (hot) => this.recommit(hot, token),
      (hot) => this.recommit(hot, hot.token),
      (hot) => ({ ...hot, token }),
    )
    if (outcome === 'stale') this.c.hooks.onToast(retypeStaleMessage('fillet'))
  }

  recommit(hot: RetypeSpec, token: CurveToken | null): boolean {
    if (token === null) return false
    const resolved = resolve(hot.targets[0], { src: 'typed', token }, hot.profile)
    if ('error' in resolved) {
      this.c.hooks.onToast(resolved.error)
      return false
    }
    const ok = this.runCommit(hot.targets, hot.profile, resolved, hot.grips, resolved.sized, 1)
    if (ok) {
      this.c.lastToken = tokenOf(resolved) ?? this.c.lastToken
      this.c.lastVaried = gripStateIsActive(hot.grips)
    }
    return ok
  }
}
