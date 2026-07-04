/**
 * ContextualDock — the bottom-center contextual verb bar (
 * `05_contextual_dock.md`, rescoped — see `dockLogic.ts`'s doc comment for
 * why Face/Edge/Component became Object/Group/Instance/Empty/Multi).
 *
 * Reuses the exact same `menuActionRef.current(id)` dispatch the command
 * palette and every menu item already go through — this component
 * owns no execution logic, only layout + the context->verbs derivation in
 * `dockLogic.ts`.
 *
 * Still deferred from the spec (M24 scope decision, not silently dropped):
 * the trailing "more actions" overflow affordance (would open the right-click
 * context menu, which isn't a reusable component yet). The two
 * deferrals M24 landed: context swaps cross-fade ( — the container is
 * keyed on the context, so a swap remounts it and replays the `.hew-dock`
 * animation, ~140ms, spec's "quick cross-fade... rather than a hard cut"),
 * and the dock fades out while the camera is being pointer-dragged ( —
 * `hidden` prop, fed by Viewport.tsx's onCameraDragChange, riding the same
 * `.hew-dock` opacity transition). Reduced motion drops both (index.css).
 */

import { useState } from 'react'
import { TOOL_ICON_SVG, type ToolName } from '../tools/toolIcons'
import inkEraserSvg from '@material-symbols/svg-400/outlined/ink_eraser.svg?raw'
import editSvg from '@material-symbols/svg-400/outlined/edit.svg?raw'
import groupOffSvg from '@material-symbols/svg-400/outlined/group_off.svg?raw'
import contentCopySvg from '@material-symbols/svg-400/outlined/content_copy.svg?raw'
import callSplitSvg from '@material-symbols/svg-400/outlined/call_split.svg?raw'
import { deriveDockContext, dockVerbsFor, dockChipLabel, type DockContext, type DockVerb } from './dockLogic'
import type { NodeRef } from './treeModel'

const NON_TOOL_ICON_SVG: Record<string, string> = {
  'edit-delete': inkEraserSvg,
  'enter-context': editSvg,
  'ungroup': groupOffSvg,
  'make-unique': contentCopySvg,
  'explode-instance': callSplitSvg,
}

const CHIP_COLOR: Record<DockContext, string> = {
  empty: 'var(--text-section)',
  object: 'var(--axis-blue)',
  group: 'var(--axis-green)',
  instance: 'var(--axis-green)',
  multi: 'var(--text-section)',
}

function VerbIcon({ verb }: { verb: DockVerb }) {
  const raw = TOOL_ICON_SVG[verb.label as ToolName] ?? NON_TOOL_ICON_SVG[verb.id]
  if (raw === undefined) return null
  const svg = raw
    .replace(/\swidth="[^"]*"/, '')
    .replace(/\sheight="[^"]*"/, '')
    .replace('<svg ', '<svg fill="currentColor" width="21" height="21" ')
  return (
    <span
      aria-hidden="true"
      style={{ width: '21px', height: '21px', display: 'block', overflow: 'hidden' }}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

/**
 * `selected` is the ONLY thing that drives the accent styling — it's true
 * iff this verb is the actual active tool (`activeToolId` matched, from
 * `ContextualDock`'s prop). Being first in the verbs array (formerly styled
 * as "primary") no longer renders any different from the rest: a dock
 * showing Rectangle/Line/Circle/Arc must never look like Rectangle is
 * selected just because it's first — only Arc lights up while Arc is the
 * live tool.
 */
function DockItem({ verb, selected, onRun }: { verb: DockVerb; selected: boolean; onRun: (id: string) => void }) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      onClick={() => onRun(verb.id)}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      title={verb.label}
      style={{
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        gap: '4px',
        padding: '8px var(--space-6, 13px)',
        minWidth: '60px',
        border: selected ? '1px solid var(--accent-border)' : '1px solid transparent',
        borderRadius: 'var(--radius-panel-item, 11px)',
        background: selected ? 'var(--accent-tint-18)' : hovered ? 'rgba(255,255,255,0.04)' : 'transparent',
        color: selected ? 'var(--accent-text-strong)' : 'var(--text-secondary)',
        cursor: 'pointer',
        fontFamily: 'var(--font-family-ui)',
      }}
    >
      <VerbIcon verb={verb} />
      <span
        style={{
          fontSize: 'var(--font-size-dock-label, 11px)',
          fontWeight: selected ? 600 : 400,
          whiteSpace: 'nowrap',
        }}
      >
        {verb.label}
      </span>
    </button>
  )
}

export interface ContextualDockProps {
  selectedIds: NodeRef[]
  selectedGuide: bigint | null
  onRun: (id: string) => void
  /** Fade the dock out (opacity 0 + click-through) while the camera is being
   * dragged (spec: "Hidden during pure camera navigation... reappears
   * on release"). Kept mounted so the fade can transition both ways. */
  hidden?: boolean
  /** The dock-verb id of the ACTUAL active tool (e.g. `'tool-arc'`), or
   * undefined/no-match if the active tool has no dock verb. Drives the one
   * accent-highlighted item  — replaces the old
   * "first verb always looks selected" behavior. */
  activeToolId?: string
}

export function ContextualDock({ selectedIds, selectedGuide, onRun, hidden = false, activeToolId }: ContextualDockProps) {
  const context = deriveDockContext(selectedIds, selectedGuide)
  if (context === null) return null

  const verbs = dockVerbsFor(context)

  return (
    <div
      key={context}
      className="hew-dock"
      data-hidden={hidden || undefined}
      style={{
        opacity: hidden ? 0 : 1,
        pointerEvents: hidden ? 'none' : undefined,
        position: 'absolute',
        bottom: '18px',
        left: '50%',
        transform: 'translateX(-50%)',
        display: 'flex',
        alignItems: 'stretch',
        gap: '3px',
        padding: '8px 10px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-strong)',
        borderRadius: 'var(--radius-dock, 15px)',
        boxShadow: 'var(--shadow-dock)',
        backdropFilter: 'blur(12px)',
      }}
    >
      {/* Context chip */}
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: '4px',
          padding: '0 var(--space-5, 11px) 0 var(--space-3, 8px)',
          borderRight: '1px solid var(--border-hairline)',
          marginRight: '3px',
        }}
      >
        <span
          aria-hidden="true"
          style={{ width: '9px', height: '9px', borderRadius: '2px', background: CHIP_COLOR[context] }}
        />
        <span
          style={{
            fontFamily: 'var(--font-family-mono)',
            fontSize: 'var(--font-size-dock-chip, 9.5px)',
            fontWeight: 600,
            color: 'var(--text-section)',
            whiteSpace: 'nowrap',
          }}
        >
          {dockChipLabel(context)}
        </span>
      </div>

      {verbs.map((verb) => (
        <DockItem key={verb.id} verb={verb} selected={verb.id === activeToolId} onRun={onRun} />
      ))}
    </div>
  )
}
