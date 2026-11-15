/**
 * The viewport's tool registration table: ONE entry per tool the Viewport
 * constructs. The Viewport iterates this list to build tools and to route
 * the modifier-key and pointer hooks; nothing else enumerates its tools.
 *
 * Tools NOT in the table are still built by `Viewport.tsx` itself because
 * each is tied to Viewport-internal machinery a `ToolContext` would have to
 * mirror wholesale (listed in `registrations.test.ts` as
 * `NOT_REGISTERED`): the transform tools and the drag-to-move handoff, Paint,
 * Position Texture, the measuring and annotating tools, and the camera tools.
 */

import { toolAllowed } from '../../preview'
import type { ToolName } from '../toolIcons'
import type { ToolRegistration } from '../toolRegistration'
import { arcTool, circleTool, lineTool, polygonTool, rectangleTool } from './shapes'
import { followMeTool, offsetTool, pushPullTool } from './modify'
import {
  deformTool,
  eraserTool,
  filletTool,
  loftTool,
  pathArrayTool,
  projectTool,
  revolveTool,
  shellTool,
} from './curves'

/** Table order is the order tools are listed in; it carries no behavior. */
const ALL_TOOL_REGISTRATIONS: readonly ToolRegistration[] = [
  // ---- Draw ----
  lineTool,
  rectangleTool,
  circleTool,
  polygonTool,
  arcTool,
  // ---- Modify ----
  pushPullTool,
  followMeTool,
  offsetTool,
  filletTool,
  loftTool,
  revolveTool,
  deformTool,
  pathArrayTool,
  shellTool,
  projectTool,
  eraserTool,
]

/** The registrations this build offers. A tool the Preview wall withholds is
 *  left out here, so the Viewport can never construct or activate it. */
export const TOOL_REGISTRATIONS: readonly ToolRegistration[] = ALL_TOOL_REGISTRATIONS.filter((r) => toolAllowed(r.name))

const BY_NAME: ReadonlyMap<ToolName, ToolRegistration> = new Map(TOOL_REGISTRATIONS.map((r) => [r.name, r]))

/** The registration that builds `name`, or undefined for a tool the
 *  Viewport still constructs itself. */
export function toolRegistration(name: string): ToolRegistration | undefined {
  return BY_NAME.get(name as ToolName)
}
