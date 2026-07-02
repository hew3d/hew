/**
 * ToolRail — the labeled left tool rail (`03_tool_rail.md`).
 *
 * Replaces the old horizontal icon-only toolbar: a fixed 172px vertical rail,
 * grouped Draw/Modify/Inspect, every row showing icon + name + keyboard
 * shortcut. Only tools with a `group` in `tools/toolRegistry.ts` get a row
 * here — the rest (Protractor/Slice/Edit Vertex/camera tools) stay reachable
 * via the Tools/Camera menus (and, once lands, the command palette).
 *
 * Radio behavior: exactly one tool is active at a time, driven by the same
 * `activeTool`/`onSelectTool` pair MenuBar.tsx's Draw/Tools/Camera menus use.
 */

import { useState } from 'react'
import { TOOL_ICON_SVG } from '../tools/toolIcons'
import { RAIL_GROUPS, toolsInGroup, shortcutFor, type ToolName } from '../tools/toolRegistry'
import { isMac } from '../platform'

export interface ToolRailProps {
  activeTool: ToolName
  onSelectTool: (name: ToolName) => void
  /** When set, a resting command-palette search field is drawn at the top of
   * the rail. Used on macOS, where there is no in-window menu bar to host the
   * field (the Windows/Linux/Web build keeps it in `MenuBar.tsx` per
   * `04_command_palette.md`). Clicking it opens the palette. */
  onOpenPalette?: () => void
  /** Shortcut label shown in the field's kbd chip (e.g. '⌘/' on macOS). */
  paletteKbd?: string
}

/** Resting command-palette field for the top of the rail (macOS — see
 * `onOpenPalette` above). Mirrors `MenuBar.tsx`'s menu-bar field, widened to
 * the rail's full width. */
function RailSearchField({ onOpen, kbd }: { onOpen: () => void; kbd: string }) {
  return (
    <button
      onClick={onOpen}
      aria-label="Search tools, actions, help"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--space-3, 8px)',
        width: '100%',
        margin: '0 0 var(--space-3, 8px)',
        padding: '6px var(--space-4, 9px)',
        background: 'var(--surface-input, #14161a)',
        border: '1px solid var(--border-hairline, #3a3a3a)',
        borderRadius: '9px',
        cursor: 'pointer',
        fontFamily: 'var(--font-family-ui)',
      }}
    >
      <span aria-hidden="true" style={{ color: 'var(--text-faint, #888)', fontSize: '13px' }}>⌕</span>
      <span style={{ flex: 1, textAlign: 'left', fontSize: '12.5px', color: 'var(--text-faint, #888)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
        Search…
      </span>
      <span
        style={{
          fontFamily: 'var(--font-family-mono)',
          fontSize: 'var(--font-size-kbd, 10px)',
          fontWeight: 600,
          color: 'var(--kbd-text, #9aa3b0)',
          background: 'var(--kbd-bg, rgba(255,255,255,0.07))',
          border: '1px solid var(--kbd-border, rgba(255,255,255,0.08))',
          borderRadius: 'var(--radius-kbd, 4px)',
          padding: '1.5px 5px',
          flexShrink: 0,
        }}
      >
        {kbd}
      </span>
    </button>
  )
}

/** Inline Material Symbols icon (moved here from App.tsx in). The
 * source SVGs carry no `fill` attribute, so `fill="currentColor"` is spliced
 * onto the root `<svg>` tag here — letting the row's `color` style (active
 * vs. idle) drive icon color without a stylesheet. */
function ToolIcon({ name, size = 16 }: { name: ToolName; size?: number }) {
  const svg = TOOL_ICON_SVG[name]
    .replace(/\swidth="[^"]*"/, '')
    .replace(/\sheight="[^"]*"/, '')
    .replace('<svg ', `<svg fill="currentColor" width="${size}" height="${size}" `)
  return (
    <span
      aria-hidden="true"
      style={{ width: `${size}px`, height: `${size}px`, display: 'block', overflow: 'hidden', flexShrink: 0 }}
      dangerouslySetInnerHTML={{ __html: svg }}
    />
  )
}

function KbdChip({ shortcut, active }: { shortcut: string; active: boolean }) {
  if (shortcut === '') return null
  return (
    <span
      style={{
        fontFamily: 'var(--font-family-mono)',
        fontSize: 'var(--font-size-kbd)',
        fontWeight: 600,
        lineHeight: 1,
        padding: '1.5px 5px',
        borderRadius: 'var(--radius-kbd)',
        whiteSpace: 'nowrap',
        color: active ? 'var(--kbd-active-text)' : 'var(--kbd-text)',
        background: active ? 'var(--kbd-active-bg)' : 'var(--kbd-bg)',
        border: `1px solid ${active ? 'transparent' : 'var(--kbd-border)'}`,
      }}
    >
      {shortcut}
    </span>
  )
}

function ToolRow({
  name,
  active,
  onSelect,
}: {
  name: ToolName
  active: boolean
  onSelect: () => void
}) {
  const [hovered, setHovered] = useState(false)
  const shortcut = shortcutFor(name, isMac)

  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={name}
      title={shortcut === '' ? name : `${name} (${shortcut})`}
      onClick={onSelect}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 'var(--space-4)',
        padding: '6px var(--space-4)',
        borderRadius: 'var(--radius-control)',
        border: 'none',
        width: '100%',
        textAlign: 'left',
        cursor: 'pointer',
        fontFamily: 'var(--font-family-ui)',
        fontSize: 'var(--font-size-tool-row)',
        fontWeight: active ? 600 : 400,
        color: active ? 'var(--accent-text-on-tint)' : 'var(--text-secondary)',
        background: active ? 'var(--accent-tint-15)' : hovered ? 'rgba(255,255,255,0.04)' : 'transparent',
        boxShadow: active ? 'inset 2px 0 0 var(--accent-base)' : 'none',
      }}
    >
      <ToolIcon name={name} />
      <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</span>
      <KbdChip shortcut={shortcut} active={active} />
    </button>
  )
}

export function ToolRail({ activeTool, onSelectTool, onOpenPalette, paletteKbd }: ToolRailProps) {
  return (
    <div
      role="radiogroup"
      aria-label="Tools"
      style={{
        width: '172px',
        flexShrink: 0,
        background: 'var(--surface-panel)',
        borderRight: '1px solid var(--border-hairline)',
        display: 'flex',
        flexDirection: 'column',
        gap: '3px',
        padding: 'var(--space-5) var(--space-4)',
        overflowY: 'auto',
      }}
    >
      {onOpenPalette !== undefined && (
        <RailSearchField onOpen={onOpenPalette} kbd={paletteKbd ?? 'Ctrl K'} />
      )}
      {RAIL_GROUPS.map((group) => (
        <div key={group}>
          <div
            style={{
              fontFamily: 'var(--font-family-mono)',
              fontSize: 'var(--font-size-section-header)',
              fontWeight: 600,
              textTransform: 'uppercase',
              letterSpacing: '0.12em',
              color: 'var(--text-section)',
              padding: '12px var(--space-4) 6px',
            }}
          >
            {group}
          </div>
          {toolsInGroup(group).map((t) => (
            <ToolRow
              key={t.name}
              name={t.name}
              active={activeTool === t.name}
              onSelect={() => onSelectTool(t.name)}
            />
          ))}
        </div>
      ))}
    </div>
  )
}
