/* oxlint-disable lingui/no-unlocalized-strings -- CSS values and tool IDs are identifiers, not copy */
/**
 * toolPanels — the floating panels a tool shows over the viewport while it
 * is the active tool. App.tsx mounts every entry whose `tool` is active, so a
 * tool's panels are declared here and nowhere in App's layout.
 */

import type { ReactNode } from 'react'
import { toolAllowed } from '../preview'
import { ToolOptionsCard } from '../viewport/ToolOptionsCard'
import { FilletOptions, type ProfileOptionSection } from '../viewport/FilletOptions'
import type { ToolName } from './toolIcons'

/** What App.tsx hands every panel: app-level data a panel renders. */
export interface ToolPanelProps {
  /** The Fillet profile picker's sections (Library and built-in profiles). */
  profileSections: ProfileOptionSection[]
}

export interface ToolPanel {
  tool: ToolName
  render(props: ToolPanelProps): ReactNode
}

const ALL_TOOL_PANELS: readonly ToolPanel[] = [
  {
    tool: 'Fillet',
    render: ({ profileSections }) => (
      <ToolOptionsCard collapsible={false} tool="Fillet" info="Round or chamfer an edge, or give it another shape.">
        <FilletOptions sections={profileSections} />
      </ToolOptionsCard>
    ),
  },
]

/** The panels this build mounts: a tool the Preview wall withholds has none. */
export const TOOL_PANELS: readonly ToolPanel[] = ALL_TOOL_PANELS.filter((p) => toolAllowed(p.tool))

/** The panels to mount while `activeTool` runs. */
export function panelsForTool(activeTool: string): readonly ToolPanel[] {
  return TOOL_PANELS.filter((p) => p.tool === activeTool)
}
