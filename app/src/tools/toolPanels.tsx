/* oxlint-disable lingui/no-unlocalized-strings -- CSS values and tool IDs are identifiers, not copy */
/**
 * toolPanels — the floating panels a tool shows over the viewport while it
 * is the active tool. App.tsx mounts every entry whose `tool` is active, so a
 * tool's panels are declared here and nowhere in App's layout.
 */

import type { ReactNode } from 'react'
import { CornersChip } from '../viewport/CornersChip'
import { ProfileChip, type ProfileChipSection } from '../viewport/ProfileChip'
import type { ToolName } from './toolIcons'

/** What App.tsx hands every panel: app-level data a panel renders. */
export interface ToolPanelProps {
  /** The Fillet profile picker's sections (Library and built-in profiles). */
  profileSections: ProfileChipSection[]
}

export interface ToolPanel {
  tool: ToolName
  render(props: ToolPanelProps): ReactNode
}

export const TOOL_PANELS: readonly ToolPanel[] = [
  {
    // The Fillet tool's chips sit to the left of the measurement box.
    tool: 'Fillet',
    render: ({ profileSections }) => (
      <div
        style={{
          position: 'absolute',
          top: '16px',
          right: '236px',
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          zIndex: 20,
        }}
      >
        <CornersChip />
        <ProfileChip sections={profileSections} />
      </div>
    ),
  },
]

/** The panels to mount while `activeTool` runs. */
export function panelsForTool(activeTool: string): readonly ToolPanel[] {
  return TOOL_PANELS.filter((p) => p.tool === activeTool)
}
