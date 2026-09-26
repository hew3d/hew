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
import libraryBooksSvg from '@material-symbols/svg-400/outlined/library_books.svg?raw'
import chevronLeftSvg from '@material-symbols/svg-400/outlined/chevron_left.svg?raw'
import chevronRightSvg from '@material-symbols/svg-400/outlined/chevron_right.svg?raw'

export const RAIL_WIDE_WIDTH = 172
export const RAIL_NARROW_WIDTH = 34

export interface ToolRailProps {
  activeTool: ToolName
  onSelectTool: (name: ToolName) => void
  /** When set, a resting command-palette search field is drawn at the top of
   * the rail. Home of the field on every platform since  (macOS forced it
   * out of the menu bar — no in-window bar to host it — and the other
   * platforms follow for cross-platform consistency, superseding
   * `04_command_palette.md`'s menu-bar placement). Clicking it opens the
   * palette. */
  onOpenPalette?: () => void
  /** Shortcut label shown in the field's kbd chip (e.g. '⌘/' on macOS
   * desktop, 'Ctrl K' on Windows/Linux/Web). */
  paletteKbd?: string
  /** When set, a LIBRARY section is drawn at the bottom of the rail with one
   * "Library" row — every platform with a working backend
   * (`libraryStore().available()`: desktop, and browsers with
   * origin-private storage); omitted entirely elsewhere rather than shown
   * disabled. */
  onOpenLibrary?: () => void
  /** Whether the Library dialog is currently open — drives the row's
   * `aria-pressed`/highlight, same posture as a tool's active state even
   * though this isn't a tool (no ToolName of its own). */
  libraryOpen?: boolean
  /** Icons-only mode: rows lose their name and shortcut chip, the palette
   * field is absent, group headings become hairline dividers. The rail
   * stays visible and every tool stays one click away; the row's `title`
   * carries the name as a tooltip. */
  narrow?: boolean
  onToggleNarrow?: () => void
}

/** Resting command-palette field for the top of the rail (all platforms —
 * see `onOpenPalette` above). */
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
export function InlineIcon({ svg, size = 16 }: { svg: string; size?: number }) {
  const sized = svg
    .replace(/\swidth="[^"]*"/, '')
    .replace(/\sheight="[^"]*"/, '')
    .replace('<svg ', `<svg fill="currentColor" width="${size}" height="${size}" `)
  return (
    <span
      aria-hidden="true"
      style={{ width: `${size}px`, height: `${size}px`, display: 'block', overflow: 'hidden', flexShrink: 0 }}
      dangerouslySetInnerHTML={{ __html: sized }}
    />
  )
}

function ToolIcon({ name, size = 16 }: { name: ToolName; size?: number }) {
  return <InlineIcon svg={TOOL_ICON_SVG[name]} size={size} />
}

function RailWidthToggle({ narrow, onToggle }: { narrow: boolean; onToggle: () => void }) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-label={narrow ? 'Expand tool rail' : 'Collapse tool rail'}
      title={narrow ? 'Expand tool rail' : 'Collapse tool rail'}
      onClick={onToggle}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        alignSelf: narrow ? 'center' : 'flex-end',
        display: 'flex',
        padding: '2px',
        borderRadius: 'var(--radius-control)',
        border: 'none',
        cursor: 'pointer',
        color: 'var(--text-faint, #888)',
        background: hovered ? 'rgba(255,255,255,0.04)' : 'transparent',
        marginBottom: narrow ? 0 : 'var(--space-3, 8px)',
      }}
    >
      <InlineIcon svg={narrow ? chevronRightSvg : chevronLeftSvg} size={18} />
    </button>
  )
}

function GroupHeading({ label, narrow }: { label: string; narrow: boolean }) {
  if (narrow) {
    return (
      <div
        role="separator"
        aria-label={label}
        style={{ borderTop: '1px solid var(--border-hairline)', margin: '8px var(--space-4)' }}
      />
    )
  }
  return (
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
      {label}
    </div>
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
  narrow,
  onSelect,
}: {
  name: ToolName
  active: boolean
  narrow: boolean
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
      {!narrow && (
        <>
          <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{name}</span>
          <KbdChip shortcut={shortcut} active={active} />
        </>
      )}
    </button>
  )
}

/** The rail's one non-tool row (LIBRARY section): same layout/styling as
 * `ToolRow`, but keyed on `active`/`onSelect` directly rather than a
 * `ToolName` — the Library dialog is a viewport-level modal, not a
 * registry-driven tool, so it can't reuse `ToolRow`'s `TOOL_ICON_SVG` lookup
 * or `shortcutFor`. */
function LibraryRow({ active, narrow, onSelect }: { active: boolean; narrow: boolean; onSelect: () => void }) {
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-pressed={active}
      aria-label="Library"
      title="Library (⇧L)"
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
      <InlineIcon svg={libraryBooksSvg} />
      {!narrow && (
        <>
          <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>Library</span>
          <KbdChip shortcut="⇧L" active={active} />
        </>
      )}
    </button>
  )
}

export function ToolRail({
  activeTool,
  onSelectTool,
  onOpenPalette,
  paletteKbd,
  onOpenLibrary,
  libraryOpen = false,
  narrow = false,
  onToggleNarrow,
}: ToolRailProps) {
  return (
    <div
      role="radiogroup"
      aria-label="Tools"
      style={{
        width: `${narrow ? RAIL_NARROW_WIDTH : RAIL_WIDE_WIDTH}px`,
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
      {onToggleNarrow !== undefined && <RailWidthToggle narrow={narrow} onToggle={onToggleNarrow} />}
      {onOpenPalette !== undefined && !narrow && (
        <RailSearchField onOpen={onOpenPalette} kbd={paletteKbd ?? 'Ctrl K'} />
      )}
      {RAIL_GROUPS.map((group) => (
        <div key={group}>
          <GroupHeading label={group} narrow={narrow} />
          {toolsInGroup(group).map((t) => (
            <ToolRow
              key={t.name}
              name={t.name}
              active={activeTool === t.name}
              narrow={narrow}
              onSelect={() => onSelectTool(t.name)}
            />
          ))}
        </div>
      ))}
      {onOpenLibrary !== undefined && (
        <div>
          <GroupHeading label="Library" narrow={narrow} />
          <LibraryRow active={libraryOpen} narrow={narrow} onSelect={onOpenLibrary} />
        </div>
      )}
    </div>
  )
}
