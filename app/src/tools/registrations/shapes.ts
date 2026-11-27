/* oxlint-disable lingui/no-unlocalized-strings -- tool names are identifiers */
/**
 * Registrations for the drawing tools: Line, Arc, Rectangle, Circle,
 * Polygon. They share one wiring: a committed drawing rebuilds the sketch
 * and guide buffers, a face region that became a solid refreshes that one
 * object, and detached mode badges the cursor with a `+`.
 */

import { RectangleTool } from '../RectangleTool'
import { CircleTool } from '../CircleTool'
import { PolygonTool } from '../PolygonTool'
import { ArcTool } from '../ArcTool'
import { LineTool } from '../LineTool'
import { defineTool, type ToolContext } from '../toolRegistration'

/** A committed drawing: new sketch geometry, no solid refresh. */
function sketchCommitted(ctx: ToolContext): () => void {
  return () => {
    ctx.refreshSketches()
    ctx.refreshGuides()
    ctx.documentChanged()
    ctx.scheduleRender()
  }
}

/** A drawing that closed a face region into a solid: refresh that object. */
function solidFormed(ctx: ToolContext): (objectId: bigint) => void {
  return (objectId) => {
    ctx.refreshScene({ objectIds: [objectId] })
  }
}

/** Scope a drawing tool to the editing context. Plain objects are directly
 *  drawable (context-path-aware eligibility). */
function scopeDrawing<T extends { setFaceEligibility(f: ToolContext['faceEligibility']): void }>(
  ctx: ToolContext,
  tool: T,
): T {
  ctx.scopeToEditContext(tool as never)
  tool.setFaceEligibility(ctx.faceEligibility)
  return tool
}

export const rectangleTool = defineTool({
  name: 'Rectangle',
  create(ctx) {
    return scopeDrawing(
      ctx,
      new RectangleTool(
        ctx.scene,
        ctx.previewGroup,
        sketchCommitted(ctx),
        ctx.toast,
        solidFormed(ctx),
        ctx.measurement,
        // Detached mode: a `+` on the cursor. A new tool starts off.
        (on: boolean) => { ctx.setToolCursor('Rectangle', on) },
      ),
    )
  },
})

export const circleTool = defineTool({
  name: 'Circle',
  create(ctx) {
    return scopeDrawing(
      ctx,
      new CircleTool(
        ctx.scene,
        ctx.previewGroup,
        sketchCommitted(ctx),
        ctx.toast,
        solidFormed(ctx),
        ctx.measurement,
        (on: boolean) => { ctx.setToolCursor('Circle', on) },
      ),
    )
  },
})

export const polygonTool = defineTool({
  name: 'Polygon',
  create(ctx) {
    const tool = new PolygonTool(
      ctx.scene,
      ctx.previewGroup,
      sketchCommitted(ctx),
      ctx.toast,
      solidFormed(ctx),
      ctx.measurement,
      // Side count persists across tool re-selection for the session, the
      // same way Paint's current material does.
      (sides) => { ctx.polygonSides.set(sides) },
      (on: boolean) => { ctx.setToolCursor('Polygon', on) },
    )
    tool.setSideCount(ctx.polygonSides.get())
    return scopeDrawing(ctx, tool)
  },
})

export const arcTool = defineTool({
  name: 'Arc',
  create(ctx) {
    return scopeDrawing(
      ctx,
      new ArcTool(
        ctx.scene,
        ctx.previewGroup,
        sketchCommitted(ctx),
        ctx.toast,
        solidFormed(ctx),
        ctx.measurement,
        (on: boolean) => { ctx.setToolCursor('Arc', on) },
      ),
    )
  },
})

export const lineTool = defineTool({
  name: 'Line',
  create(ctx) {
    return scopeDrawing(
      ctx,
      new LineTool(
        ctx.scene,
        ctx.previewGroup,
        sketchCommitted(ctx),
        ctx.toast,
        solidFormed(ctx),
        ctx.measurement,
        (on: boolean) => { ctx.setToolCursor('Line', on) },
      ),
    )
  },
})
