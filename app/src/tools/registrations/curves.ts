/**
 * Registrations for the curve tools: Revolve, Loft, Deform, Array along
 * Path, Shell, Project, Eraser, Fillet (which also answers to Chamfer).
 */

import { RevolveTool } from '../RevolveTool'
import { LoftTool } from '../LoftTool'
import { DeformTool } from '../DeformTool'
import { PathArrayTool } from '../PathArrayTool'
import { ShellTool } from '../ShellTool'
import { ProjectTool } from '../ProjectTool'
import { EraserTool } from '../EraserTool'
import { FilletTool } from '../FilletTool'
import { getDeformMode, subscribeDeformMode } from '../deformMode'
import { defineTool } from '../toolRegistration'

export const revolveTool = defineTool({
  name: 'Revolve',
  create(ctx) {
    const tool = new RevolveTool(
      ctx.scene,
      ctx.previewGroup,
      // A revolve births one new object and (for a drawn shape) consumes its
      // region; refresh the object plus the sketch line buffers, then select
      // the result, as Follow Me does.
      (objectId) => {
        ctx.refreshScene({ objectIds: [objectId] })
        ctx.refreshSketches()
        ctx.select({ kind: 'object', id: objectId })
      },
      ctx.toast,
      ctx.measurement,
      // A drawn shape selected first is the profile (Revolve opens at the axis).
      ctx.selection(),
    )
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
})

export const loftTool = defineTool({
  name: 'Loft',
  create(ctx) {
    const tool = new LoftTool(
      ctx.scene,
      ctx.previewGroup,
      // A loft births one object and consumes its shapes' regions.
      (objectId) => {
        ctx.refreshScene({ objectIds: [objectId] })
        ctx.refreshSketches()
        ctx.select({ kind: 'object', id: objectId })
      },
      ctx.toast,
      ctx.measurement,
    )
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
  // Alt-click adds a rail; read live from the real event at the press.
  onPointer(tool, ev, phase) {
    if (phase === 'down') tool.setAltModifier(ev.altKey)
  },
})

/** Unsubscribe handles for live Deform tools' mode subscriptions. */
const deformUnsubscribe = new WeakMap<DeformTool, () => void>()

export const deformTool = defineTool({
  name: 'Deform',
  create(ctx) {
    const tool = new DeformTool(
      ctx.scene,
      ctx.previewGroup,
      ctx.objectsGroup,
      ctx.selection(),
      // The deform reshapes one object in place.
      (objectId) => {
        ctx.refreshScene({ objectIds: [objectId] })
      },
      ctx.toast,
      ctx.measurement,
      getDeformMode(),
    )
    tool.setSelectionAcquirer(ctx.acquireTransformTargets)
    // A commit on an object too heavy to preview runs behind the busy card.
    tool.setBusyRunner(ctx.busy)
    ctx.scopeToEditContext(tool)
    // A Twist / Bend / Taper / Bend to Path menu pick while this tool runs
    // restarts it in that mode. Tools are rebuilt per activation, so a stale
    // one (no longer the active tool) unsubscribes itself.
    const unsubscribe = subscribeDeformMode((mode) => {
      if (ctx.isActiveTool(tool)) tool.setMode(mode)
      else unsubscribe()
    })
    deformUnsubscribe.set(tool, unsubscribe)
    return tool
  },
  dispose(tool) {
    deformUnsubscribe.get(tool)?.()
  },
})

export const pathArrayTool = defineTool({
  name: 'Array along Path',
  create(ctx) {
    const tool = new PathArrayTool(
      ctx.scene,
      ctx.previewGroup,
      ctx.objectsGroup,
      ctx.selection(),
      // The copies replace any copies a retype just retracted, so rebuild
      // fully. The selection stays on the sources: the next retype or path
      // pick copies the same thing.
      () => {
        ctx.refreshScene()
        ctx.refreshSketches()
      },
      ctx.toast,
      ctx.measurement,
      (id: bigint) => ctx.getInstanceGroup(id),
    )
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
})

export const shellTool = defineTool({
  name: 'Shell',
  create(ctx) {
    const tool = new ShellTool(
      ctx.scene,
      ctx.previewGroup,
      ctx.selection(),
      ctx.refreshTouched,
      ctx.toast,
      ctx.measurement,
    )
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
})

export const projectTool = defineTool({
  name: 'Project',
  create(ctx) {
    const tool = new ProjectTool(
      ctx.scene,
      ctx.previewGroup,
      ctx.selection(),
      ctx.refreshTouched,
      ctx.toast,
      ctx.measurement,
    )
    ctx.scopeToEditContext(tool)
    tool.setFaceEligibility(ctx.faceEligibility)
    return tool
  },
})

export const eraserTool = defineTool({
  name: 'Eraser',
  create(ctx) {
    return new EraserTool(
      ctx.scene,
      ctx.previewGroup,
      {
        resolveRef: (snap, ray) => ctx.resolveSelectable(snap, ray),
        runDelete: (nodes) => { ctx.deleteNodes(nodes) },
        onCreaseCommit: ctx.refreshTouched,
      },
      ctx.toast,
    )
  },
  // Ctrl/Cmd held means crease, read live off each event.
  onPointer(tool, ev) {
    tool.setCreaseModifier(ev.metaKey || ev.ctrlKey)
  },
})

/** An Alt that is part of a chord (Ctrl/Cmd down with it: precision
 *  snapping) or typed into a text field is not a tap, so Fillet never swaps
 *  Round and Chamfer on it. */
function altIsChorded(ev: KeyboardEvent): boolean {
  if (ev.ctrlKey || ev.metaKey) return true
  const t = ev.target as HTMLElement | null
  return t !== null && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable)
}

export const filletTool = defineTool({
  name: 'Fillet',
  create(ctx) {
    const tool = new FilletTool(ctx.scene, ctx.previewGroup, {
      // A blend changes the solid(s): targeted refresh.
      onCommit: (objectIds) => {
        ctx.refreshScene({ objectIds })
      },
      // A 2D corner changes sketch geometry (or a drawing on a face).
      onSketchChange: () => {
        ctx.refreshSketches()
        ctx.refreshGuides()
        ctx.documentChanged()
        ctx.scheduleRender()
      },
      onToast: ctx.toast,
      onMeasurement: ctx.measurement,
      // The preview moves on animation frames, outside any pointer event.
      requestRender: () => {
        ctx.reportToolHint()
        ctx.scheduleRender()
      },
      busy: ctx.busy,
      setFaceMask: (objectId, faceIds, maxTriangles) => ctx.faceMask.set(objectId, faceIds, maxTriangles),
      clearFaceMask: (objectId) => { ctx.faceMask.clear(objectId) },
    })
    ctx.scopeToEditContext(tool)
    return tool
  },
  // Arm the selected objects' sharp edges: the selection effect does not
  // re-run on a tool switch.
  onActivate(tool, ctx) {
    tool.activateDimension()
    tool.setSelection(ctx.selection())
  },
  onKeyDown(tool, ev) {
    if ((ev.key === 'Control' || ev.key === 'Meta') && ev.altKey) {
      // Ctrl/Cmd joined an Alt already down: that Alt is a chord now, in
      // whichever order the two are released.
      tool.markAltChorded()
    } else if (ev.key === 'Alt' && !ev.repeat) {
      // A tap of Alt swaps Round and Chamfer; Alt plus a click does not (the
      // tool tells them apart by whether a press came in between).
      tool.setAltHeld(true, altIsChorded(ev))
    }
    // Ctrl/Cmd held while hovering picks one edge without extending along
    // its smooth continuations.
    if (ev.key === 'Control' || ev.key === 'Meta') tool.setCtrlHeld(true)
  },
  onKeyUp(tool, ev, ctx) {
    if (ev.key === 'Alt') {
      tool.setAltHeld(false, altIsChorded(ev))
      ctx.reportToolHint()
      ctx.scheduleRender()
    }
    if (ev.key === 'Control' || ev.key === 'Meta') tool.setCtrlHeld(false)
  },
  // A blur swallows the Alt keyup: forget the hold without toggling.
  onWindowBlur(tool) {
    tool.resetAlt()
    tool.setCtrlHeld(false)
  },
  // Cmd/Ctrl+A over a solid arms every sharp edge of it.
  onSelectAllChord(tool) {
    return tool.armHoveredObject()
  },
})
