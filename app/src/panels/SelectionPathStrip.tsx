/**
 * SelectionPathStrip — the strip at the top-center of the viewport that answers
 * "what is this thing, and where is it in the Outliner?"
 * (docs/design/selection-path.md, "The strip"). It also carries the open
 * session frames with the Outliner's `editing` chip, so it replaces the old
 * "Editing X" pill: one widget for both facts.
 *
 * Presentational only: `buildSelectionPath` decides the crumbs, the app
 * decides what a click does. A crumb click never opens a session or moves
 * the camera.
 *
 * Placement is adaptive (docs/design/selection-path.md): the strip sits in the
 * top-center band between the view HUD and the measurement box while its
 * content fits there (`data-row="band"`). When the content overflows that band
 * it moves to a second row just under the HUD chips (`data-row="second"`),
 * centered across the full viewport width less a 16 px gutter. A path longer
 * than the whole viewport still clips, with the selected crumb scrolled into
 * view. The decision is re-measured on strip/band resize and whenever the
 * crumbs change, in a layout effect so there is no visible jump.
 */
import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Trans, useLingui } from '@lingui/react/macro'
import { NodeIcon } from './DocumentTree'
import type { PathCrumb } from './selectionPath'
import type { NodeRef } from './treeModel'

interface Props {
  crumbs: PathCrumb[]
  /** Click a crumb that is a plain node: replace the selection with it. */
  onSelect: (node: NodeRef) => void
  /** Click `Model`: exit every session, or clear the selection with none open. */
  onModel: () => void
  /** Click an open frame: close the sessions below it (combined-path depth). */
  onFrame: (depth: number) => void
  /** Double-click a group/instance/object crumb: open it like its Outliner row. */
  onEnter: (node: NodeRef) => void
}

const CRUMB_MAX_PX = 180

/** Width the view die reserves at the viewport's top right (ViewDie.tsx:
 *  `right: 16px`, 76 px wide) plus a 12 px margin. The pips under it overhang
 *  by 3 px each side, inside the margin. */
const DIE_CLEAR_PX = 16 + 76 + 12
/** Left edge of the strip's band. The old top-left view chips are gone, so
 *  the band is symmetric about the viewport and clears the die on both
 *  sides to stay centered. */
export const STRIP_BAND_LEFT_PX = DIE_CLEAR_PX
/** Right edge of the strip's band: clears the view die. The measurement box
 *  never competes for this row: bottom-right by default, and under the die
 *  and its pips (below this row) when docked top-right. */
export const STRIP_BAND_RIGHT_PX = DIE_CLEAR_PX

/** Top of the second row, under the first row's 16 px inset and ~30 px height. */
const SECOND_ROW_TOP_PX = 54
const SECOND_ROW_GUTTER_PX = 16
/** The second row runs under the die's column, so its right edge still clears the die. */
const SECOND_ROW_RIGHT_PX = DIE_CLEAR_PX

export function SelectionPathStrip({ crumbs, onSelect, onModel, onFrame, onEnter }: Props) {
  const { t } = useLingui()
  const boxRef = useRef<HTMLDivElement>(null)
  const rootRef = useRef<HTMLDivElement>(null)
  const [row, setRow] = useState<'band' | 'second'>('band')
  const rowRef = useRef(row)
  rowRef.current = row
  const selectedRef = useRef<HTMLElement | null>(null)
  const [hoverKey, setHoverKey] = useState<string | null>(null)

  const selectedKey = crumbs.find((c) => c.state === 'selected')?.key ?? null
  // Keep the selected crumb visible in a clipped strip when the selection
  // changes. Scrolls the strip's own box, never an ancestor.
  useEffect(() => {
    const box = boxRef.current
    const el = selectedRef.current
    if (box === null || el === null) return
    const left = el.offsetLeft
    const right = left + el.offsetWidth
    if (left < box.scrollLeft) box.scrollLeft = Math.max(0, left - 8)
    else if (right > box.scrollLeft + box.clientWidth) box.scrollLeft = right - box.clientWidth + 8
  }, [selectedKey, crumbs.length, row])

  const empty = crumbs.length === 0
  const measure = () => {
    const box = boxRef.current
    const root = rootRef.current
    if (box === null || root === null) return
    let next: 'band' | 'second'
    if (rowRef.current === 'band') {
      // Band-constrained: overflow means the content is wider than the band.
      next = box.scrollWidth > box.clientWidth ? 'second' : 'band'
    } else {
      // On the second row the strip is no longer band-constrained, so compare
      // its natural width with the band's width instead.
      const band = root.clientWidth + (SECOND_ROW_GUTTER_PX + SECOND_ROW_RIGHT_PX) - (STRIP_BAND_LEFT_PX + STRIP_BAND_RIGHT_PX)
      next = box.scrollWidth <= band ? 'band' : 'second'
    }
    if (next !== rowRef.current) {
      rowRef.current = next
      setRow(next)
    }
  }
  useLayoutEffect(measure, [crumbs]) // eslint-disable-line react-hooks/exhaustive-deps
  useLayoutEffect(() => {
    if (empty || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver(() => measure())
    if (boxRef.current !== null) ro.observe(boxRef.current)
    if (rootRef.current !== null) ro.observe(rootRef.current)
    return () => ro.disconnect()
  }, [empty]) // eslint-disable-line react-hooks/exhaustive-deps

  if (empty) return null

  const activate = (c: PathCrumb) => {
    if (c.kind === 'model') onModel()
    else if (c.frameDepth !== null) onFrame(c.frameDepth)
    else if (c.node !== null) onSelect(c.node)
  }

  return (
    <div
      ref={rootRef}
      data-testid="selection-path-band"
      data-row={row}
      style={{
        position: 'absolute',
        top: row === 'band' ? '16px' : `${SECOND_ROW_TOP_PX}px`,
        left: row === 'band' ? `${STRIP_BAND_LEFT_PX}px` : `${SECOND_ROW_GUTTER_PX}px`,
        right: row === 'band' ? `${STRIP_BAND_RIGHT_PX}px` : `${SECOND_ROW_RIGHT_PX}px`,
        display: 'flex',
        justifyContent: 'center',
        pointerEvents: 'none',
        zIndex: 20,
      }}
    >
    <div
      ref={boxRef}
      data-testid="selection-path"
      data-row={row}
      style={{
        pointerEvents: 'none',
        maxWidth: '100%',
        boxSizing: 'border-box',
        overflow: 'hidden',
        display: 'flex',
        alignItems: 'center',
        gap: '3px',
        padding: '4px 10px 4px 8px',
        background: 'var(--surface-overlay)',
        border: '1px solid var(--border-hairline)',
        borderRadius: '7px',
        fontFamily: 'var(--font-family-ui)',
        fontSize: '11.5px',
        color: 'var(--text-secondary)',
        whiteSpace: 'nowrap',
      }}
    >
      {crumbs.map((c, i) => {
        if (c.kind === 'divider') {
          return (
            <span
              key={c.key}
              data-testid="selection-path-divider"
              aria-hidden="true"
              style={{ color: 'var(--border-strong)', margin: '0 5px', flexShrink: 0 }}
            >
              │
            </span>
          )
        }
        const member = c.state === 'member'
        const selected = c.state === 'selected'
        const hovered = hoverKey === c.key && !member
        let color = 'var(--accent-base)'
        if (selected) color = 'var(--text-primary)'
        else if (member) color = 'var(--text-faint)'
        else if (c.state === 'below') color = 'var(--text-secondary)'
        if (hovered) color = 'var(--text-primary)'
        const style: React.CSSProperties = {
          display: 'inline-flex',
          alignItems: 'center',
          gap: '5px',
          padding: '2px 6px',
          borderRadius: '4px',
          flexShrink: 0,
          border: 'none',
          background: selected ? 'var(--accent-tint-18)' : 'none',
          color,
          font: 'inherit',
          fontWeight: selected ? 600 : 'normal',
          fontStyle: member ? 'italic' : 'normal',
          cursor: member ? 'default' : 'pointer',
          pointerEvents: 'auto',
          ...(hovered
            ? {
                ...(selected ? {} : { background: 'var(--surface-hover)' }),
                textDecoration: 'underline',
                textDecorationColor: 'var(--accent-base)',
                textUnderlineOffset: '2px',
              }
            : {}),
        }
        const body = (
          <>
            {c.kind !== 'model' && <NodeIcon kind={member ? 'object' : (c.kind as NodeRef['kind'])} />}
            <span
              style={{
                maxWidth: `${CRUMB_MAX_PX}px`,
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {c.label}
            </span>
            {member && (
              <span style={{ color: 'var(--text-faint)', fontStyle: 'normal', fontSize: '10.5px', marginLeft: '2px' }}>
                <Trans>part of the component</Trans>
              </span>
            )}
            {c.frameDepth !== null && (
              <span style={{ fontSize: '10px', color: 'var(--accent-text-on-tint)', marginLeft: '3px', fontWeight: 'normal' }}>
                <Trans>editing</Trans>
              </span>
            )}
          </>
        )
        const crumbLabel = c.label
        const common = {
          'data-testid': 'selection-path-crumb',
          'data-kind': c.kind,
          'data-state': c.state,
          'data-editing': c.frameDepth !== null ? 'true' : undefined,
          title: member ? t`${crumbLabel} (part of the component)` : c.label,
        } as const
        return (
          <span key={c.key} style={{ display: 'inline-flex', alignItems: 'center', gap: '3px', flexShrink: 0 }}>
            {i > 0 && crumbs[i - 1].kind !== 'divider' && <span aria-hidden="true" style={{ color: 'var(--text-faint)', margin: '0 1px' }}>›</span>}
            {member ? (
              <span
                {...common}
                style={style}
                onDoubleClick={() => { if (c.enterNode !== null) onEnter(c.enterNode) }}
              >
                {body}
              </span>
            ) : (
              <button
                type="button"
                {...common}
                ref={selected ? (el) => { selectedRef.current = el } : undefined}
                style={style}
                onMouseEnter={() => setHoverKey(c.key)}
                onMouseLeave={() => setHoverKey((k) => (k === c.key ? null : k))}
                onClick={() => activate(c)}
                onDoubleClick={() => {
                  if (c.enterNode !== null && c.frameDepth === null) onEnter(c.enterNode)
                }}
              >
                {body}
              </button>
            )}
          </span>
        )
      })}
    </div>
    </div>
  )
}
