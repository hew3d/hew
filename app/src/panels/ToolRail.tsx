/**
 * ToolRail — the labeled left tool rail (`03_tool_rail.md`).
 *
 * A vertical rail grouped Draw/Modify/Inspect, every row showing icon + name
 * + keyboard shortcut; compacted (View ▸ Compact Tool Rail, or the chevron at
 * its top) it drops to icons only. Only tools with a `group` in
 * `tools/toolRegistry.ts` get a row here — the rest (Protractor/Slice/Edit
 * Vertex/camera tools) stay reachable via the Tools/Camera menus and the
 * command palette.
 *
 * Radio behavior: exactly one tool is active at a time, driven by the same
 * `activeTool`/`onSelectTool` pair MenuBar.tsx's Draw/Tools/Camera menus use.
 * Only the tool rows live inside the `radiogroup`; the compact toggle, the
 * palette field, and the Library row sit outside it, so assistive tech
 * counts exactly the tools as the group's options.
 */

import { useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { TOOL_ICON_SVG } from '../tools/toolIcons'
import { RAIL_GROUPS, TOOL_GROUP_LABELS, toolsInGroup, toolSpec, toolLabel, type ToolName } from '../tools/toolRegistry'
import { useKeymap } from '../keymap/useKeymap'
import { railShortcutLabel } from '../keymap/keymap'
import { InlineIcon } from './InlineIcon'
import libraryBooksSvg from '@material-symbols/svg-400/outlined/library_books.svg?raw'
import chevronLeftSvg from '@material-symbols/svg-400/outlined/chevron_left.svg?raw'
import chevronRightSvg from '@material-symbols/svg-400/outlined/chevron_right.svg?raw'
import searchSvg from '@material-symbols/svg-400/outlined/search.svg?raw'

/** Content width of the rail (the container adds its own padding and
 * hairline border): wide shows names and shortcut chips, narrow fits one
 * 16px icon inside a row's own horizontal padding. */
export const RAIL_WIDE_WIDTH = 172
export const RAIL_NARROW_WIDTH = 34

export interface ToolRailProps {
  activeTool: ToolName
  onSelectTool: (name: ToolName) => void
  /** When set, a resting command-palette search field is drawn at the top of
   * the rail (an icon button when narrow). Home of the field on every
   * platform: macOS forced it out of the menu bar — no in-window bar to host
   * it — and the other platforms follow for cross-platform consistency,
   * superseding `04_command_palette.md`'s menu-bar placement. Clicking it
   * opens the palette. */
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
   * field shrinks to a search icon, group headings become hairline
   * dividers. The rail stays visible and every tool stays one click away;
   * each row's `title` carries its name and shortcut as a tooltip. */
  narrow?: boolean
  /** When set, a chevron at the top of the rail toggles `narrow`. */
  onToggleNarrow?: () => void
}

/** Resting command-palette field for the top of the rail (all platforms —
 * see `onOpenPalette` above). */
function RailSearchField({ onOpen, kbd }: { onOpen: () => void; kbd: string }) {
  const { t } = useLingui()
  return (
    <button
      onClick={onOpen}
      aria-label={t`Search tools, actions, help`}
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
        <Trans>Search…</Trans>
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

/** The palette field's narrow-rail form: an icon-only row that opens the
 * same palette, its shortcut carried by the tooltip. */
function RailSearchButton({ onOpen, kbd }: { onOpen: () => void; kbd: string }) {
  const { t } = useLingui()
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      onClick={onOpen}
      aria-label={t`Search tools, actions, help`}
      title={t`Search tools, actions, help (${kbd})`}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        display: 'flex',
        alignItems: 'center',
        // One px less than a tool row's padding, so the hairline border
        // leaves the icon on the same column as the tool icons below.
        padding: '5px calc(var(--space-4) - 1px)',
        margin: '0 0 var(--space-3, 8px)',
        width: '100%',
        borderRadius: 'var(--radius-control)',
        border: '1px solid var(--border-hairline)',
        cursor: 'pointer',
        color: 'var(--text-faint)',
        background: hovered ? 'var(--surface-hover)' : 'var(--surface-input)',
      }}
    >
      <InlineIcon svg={searchSvg} />
    </button>
  )
}

function ToolIcon({ name, size = 16 }: { name: ToolName; size?: number }) {
  return <InlineIcon svg={TOOL_ICON_SVG[name]} size={size} />
}

function RailWidthToggle({ narrow, onToggle }: { narrow: boolean; onToggle: () => void }) {
  const { t } = useLingui()
  const [hovered, setHovered] = useState(false)
  const label = narrow ? t`Expand tool rail` : t`Compact tool rail`
  return (
    <button
      type="button"
      aria-label={label}
      aria-expanded={!narrow}
      title={label}
      onClick={() => {
        // The click moves the chevron out from under the pointer (the rail
        // changes width and the chevron re-aligns), and no mouseleave
        // follows a move like that — clear the hover here or its wash sticks.
        setHovered(false)
        onToggle()
      }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
      style={{
        alignSelf: narrow ? 'center' : 'flex-end',
        display: 'flex',
        padding: '2px',
        borderRadius: 'var(--radius-control)',
        border: 'none',
        cursor: 'pointer',
        color: hovered ? 'var(--text-secondary)' : 'var(--text-faint)',
        background: hovered ? 'var(--surface-hover)' : 'transparent',
        marginBottom: 'var(--space-3, 8px)',
      }}
    >
      <InlineIcon svg={narrow ? chevronRightSvg : chevronLeftSvg} size={18} />
    </button>
  )
}

function GroupHeading({ label, narrow }: { label: string; narrow: boolean }) {
  // Narrow: a purely visual divider. The group names carry no meaning a
  // screen reader needs (each tool row is labeled on its own), and a
  // separator role inside the radiogroup would be counted among its options.
  if (narrow) {
    return (
      <div
        aria-hidden="true"
        data-rail-divider={label}
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
  const shortcut = railShortcutLabel(toolSpec(name).actionId)
  const label = toolLabel(name)

  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      aria-label={label}
      title={shortcut === '' ? label : `${label} (${shortcut})`}
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
        background: active ? 'var(--accent-tint-15)' : hovered ? 'var(--surface-hover)' : 'transparent',
        boxShadow: active ? 'inset 2px 0 0 var(--accent-base)' : 'none',
      }}
    >
      <ToolIcon name={name} />
      {!narrow && (
        <>
          <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{label}</span>
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
 * or the registry's `actionId` shortcut. */
function LibraryRow({ active, narrow, onSelect }: { active: boolean; narrow: boolean; onSelect: () => void }) {
  const { t } = useLingui()
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- command id
  const libraryKbd = railShortcutLabel('open-library')
  const [hovered, setHovered] = useState(false)
  return (
    <button
      type="button"
      aria-pressed={active}
      aria-label={t`Library`}
      title={libraryKbd === '' ? t`Library` : t`Library (${libraryKbd})`}
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
        background: active ? 'var(--accent-tint-15)' : hovered ? 'var(--surface-hover)' : 'transparent',
        boxShadow: active ? 'inset 2px 0 0 var(--accent-base)' : 'none',
      }}
    >
      <InlineIcon svg={libraryBooksSvg} />
      {!narrow && (
        <>
          <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
            <Trans>Library</Trans>
          </span>
          <KbdChip shortcut={libraryKbd} active={active} />
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
  // Subscribes the rail to locale changes so tool names and headings re-resolve.
  const { t, i18n } = useLingui()
  // …and to keymap changes, so every tool's shortcut chip stays true.
  useKeymap()
  // Fallback chip text when the host gives no palette shortcut label.
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- keyboard shortcut label, not copy
  const fallbackKbd = 'Ctrl K'
  return (
    <div
      role="complementary"
      aria-label={t`Tool rail`}
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
      {onOpenPalette !== undefined &&
        (narrow ? (
          <RailSearchButton onOpen={onOpenPalette} kbd={paletteKbd ?? fallbackKbd} />
        ) : (
          <RailSearchField onOpen={onOpenPalette} kbd={paletteKbd ?? fallbackKbd} />
        ))}
      <div role="radiogroup" aria-label={t`Tools`} style={{ display: 'flex', flexDirection: 'column', gap: '3px' }}>
        {RAIL_GROUPS.map((group) => (
          <div key={group}>
            <GroupHeading label={i18n._(TOOL_GROUP_LABELS[group])} narrow={narrow} />
            {toolsInGroup(group).map((tool) => (
              <ToolRow
                key={tool.name}
                name={tool.name}
                active={activeTool === tool.name}
                narrow={narrow}
                onSelect={() => onSelectTool(tool.name)}
              />
            ))}
          </div>
        ))}
      </div>
      {onOpenLibrary !== undefined && (
        <div>
          <GroupHeading label={t`Library`} narrow={narrow} />
          <LibraryRow active={libraryOpen} narrow={narrow} onSelect={onOpenLibrary} />
        </div>
      )}
    </div>
  )
}
