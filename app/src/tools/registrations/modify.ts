/* oxlint-disable lingui/no-unlocalized-strings -- tool names are identifiers */
/**
 * Registrations for the everyday shaping tools: Push/Pull, Follow Me,
 * Offset. Each scopes itself to the editing context and to the shared
 * context-path-aware face eligibility.
 */

import { PushPullTool } from '../PushPullTool'
import { FollowMeTool } from '../FollowMeTool'
import { OffsetTool } from '../OffsetTool'
import { defineTool } from '../toolRegistration'

export const pushPullTool = defineTool({
  name: 'Push/Pull',
  create(ctx) {
    const tool = new PushPullTool(
      ctx.scene,
      ctx.previewGroup,
      // Targeted refresh: the committed object (a world object or a def
      // member, whose every placement refreshTouched rebuilds). A
      // through-cut's extra result objects or consumed source are caught by
      // refreshTouched's id diff.
      (objectId) => {
        ctx.refreshScene({ objectIds: [objectId] })
        ctx.refreshSketches()
        ctx.refreshGuides()
      },
      ctx.toast,
      ctx.measurement,
      // Durable extrude-as-new toggle: a `+` on the Push/Pull cursor.
      (on: boolean) => { ctx.setToolCursor('Push/Pull', on) },
    )
    // Plain objects are directly push/pullable; group/instance members only
    // from inside their container's editing context, except a linked copy's
    // face.
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.pushPullEligibility)
    return tool
  },
})

export const followMeTool = defineTool({
  name: 'Follow Me',
  create(ctx) {
    const tool = new FollowMeTool(
      ctx.scene,
      ctx.previewGroup,
      // A sweep births one new object and consumes its profile sketch's
      // outline; refresh the object plus all sketch line buffers, then select
      // the result so the highlight lands on the new solid.
      (objectId) => {
        ctx.refreshScene({ objectIds: [objectId] })
        ctx.refreshSketches()
        ctx.select({ kind: 'object', id: objectId })
      },
      ctx.toast,
      ctx.measurement,
      // The path may be preselected (SketchUp's primary Follow Me idiom).
      ctx.selection(),
    )
    // Face path/profile use the same face-eligibility system as every other
    // face tool. The editing-context channel also carries Follow-Me-specific
    // behavior: an instance context routes through the `_in_instance`
    // family; a group context births the sweep inside the group.
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
  // Cmd/Ctrl-click on the profile commits the MERGED sweep: a live read of
  // the real event at the press and again at the release, so a
  // press-drag-release commit sees the modifier held at commit time.
  onPointer(tool, ev, phase) {
    if (phase !== 'move') tool.setMergeModifier(ev.metaKey || ev.ctrlKey)
  },
})

export const offsetTool = defineTool({
  name: 'Offset',
  create(ctx) {
    const tool = new OffsetTool(
      ctx.scene,
      ctx.previewGroup,
      // Region offset: new sketch geometry, rebuild sketch lines and fills.
      () => {
        ctx.refreshSketches()
        ctx.refreshGuides()
        ctx.documentChanged()
        ctx.scheduleRender()
      },
      ctx.toast,
      // Face offset: an imprint on one object, targeted refresh.
      (objectId) => {
        ctx.refreshScene({ objectIds: [objectId] })
      },
      ctx.measurement,
    )
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
})
