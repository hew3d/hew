/**
 * PartsSheet — Shop Mode's Parts sheet, the "Cutlist" (design_handoff_shop_
 * mode/README.md §1 "Sheet header" + §2 "Cutlist"): an iOS-style BOTTOM
 * sheet over the fullscreen viewport in BOTH orientations, merging the
 * Outliner + Tags intents into one mobile list built from
 * `partsSheetModel.ts`'s pure section/row builder (itself a read of the
 * SAME `treeModel.ts`/`tagModel.ts` logic the desktop DocumentTree/
 * TagsPanel use — those two panels are untouched).
 *
 * Landscape used to render a wholly different left-edge SIDE sheet (a
 * 44px tab that toggled open to a 340px panel) — Kurt's on-device playtest
 * called for removing that composition entirely: landscape now uses this
 * SAME bottom sheet as portrait, just width-capped and centered over the
 * bottom edge (the root style below) rather than full-bleed — the detent
 * state machine, drag math, and row content are otherwise identical between
 * the two orientations, not forked. `ShopApp.tsx`'s landscape layout keeps
 * its right rail (tools) as a separate, unrelated floating object — this
 * component no longer renders anything landscape-specific beyond its own
 * root's width/position.
 *
 * `detent` is a CONTROLLED prop, owned by `ShopApp.tsx` (`onDetentChange`
 * settles it) rather than this component's own `useState` — a local
 * `useState('peek')` would reset on a `ShopApp` remount (the kernel reload
 * path) instead of surviving it. This component still owns every bit of the
 * DRAG/SNAP math itself (`dragHeightPx`, `containerHeightPx`, the pointer
 * handlers below) — only the SETTLED detent value lives one level up.
 *
 * Three detents (`sheetDetents.ts`), driven by a touch drag anywhere in the
 * header (handle bar + title row — the design's "whole header remains the
 * drag surface"; the column-header/row list below it is NOT part of the
 * drag zone, its own vertical scroll gesture would fight a drag-anywhere
 * handle): peek, half, full — snapping to the nearest one on release,
 * tracking the pointer 1:1 while dragging. The header's own unit chip
 * `stopPropagation`s on `pointerDown` (matching `EyeToggle`'s existing
 * precedent below) so a tap on it is never also read as the start of a
 * (zero-distance, so harmless either way) drag. The header used to also
 * carry a "Pull up"/"Pull down" text link duplicating the handle's own
 * affordance — removed (Kurt's playtest call: the drag handle alone is
 * the affordance) without touching the drag surface itself.
 *
 * Column header (metric only — hidden in imperial, where each row is
 * stacked instead) + tag sections (color dot via `tagPalette.ts`, name +
 * count, tinted row background, "Hide all"/"Show all" master toggle) + part
 * rows (metric: three right-aligned mono columns; imperial: a stacked
 * name/dims line so nothing wraps or clips — design §2) round out the
 * content. Every dimension goes through `settings/units.formatLengthIn`
 * with the LIVE current format (subscribed below), never the app-wide
 * singleton `formatLength` this file used before Wave 2 — a `LengthFormat`
 * change (this sheet's own unit chip, the Settings picker, or the desktop
 * window) must re-render every row immediately.
 *
 * The sheet composes with long-press isolate (`ShopApp.tsx`'s `pushHidden`)
 * rather than fighting it — this component only reports taps/toggles
 * upward, it owns no hidden-state itself. Tap a row: `onSelectRow`
 * (highlight + zoom). Long-press a row: `onLongPressRow` (isolate).
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { Scene as WasmScene } from '../wasm/loader'
import {
  formatLengthIn,
  getLengthUnit,
  subscribe as subscribeLengthUnit,
  LENGTH_FORMAT_SHORT_LABEL,
  LENGTH_FORMAT_NAME,
  LENGTH_SYSTEM_OF,
  type LengthFormat,
} from '../settings/units'
import { nodeKey, type NodeRef } from '../panels/treeModel'
import { buildPartsSheetSections, totalPartCount, visiblePartCount, type PartsSheetRow, type PartsSheetSection } from './partsSheetModel'
import { clampDragHeightPx, detentHeightPx, nearestDetent, type SheetDetent } from './sheetDetents'
import { colorForTagPath, hexToRgba, UNFILED_TAG_COLOR } from './tagPalette'
import { ChevronDownIcon, EyeIcon, EyeOffIcon } from './icons'
import type { ShopOrientation } from './orientation'
import { LONG_PRESS_MS, LONG_PRESS_SLOP_PX, LANDSCAPE_RAIL_CLEARANCE_CSS } from './ShopApp'

/** Landscape's own cap on the (otherwise full-bleed) sheet width — wide
 *  enough for the metric column layout to keep breathing room, narrow
 *  enough to stay centered over the bottom edge rather than spanning a
 *  wide phone screen edge-to-edge the way portrait's sheet does. */
const LANDSCAPE_SHEET_MAX_WIDTH_PX = 520

export interface PartsSheetProps {
  scene: WasmScene
  hiddenKeys: ReadonlySet<string>
  hiddenTagPaths: ReadonlySet<string>
  /** Selects only the root's width/position (module doc) — every row/
   *  section/detent below is shared, unforked, between orientations. */
  orientation: ShopOrientation
  /** Long-press isolate's current target, or `null` — unioned into every
   *  row's `hidden` flag (`partsSheetModel.ts`'s module doc) so the sheet
   *  never shows a stale "visible" eye for a part isolate is hiding. */
  isolatedNode: NodeRef | null
  /** The currently selected node (whatever tap-to-inspect or a previous row
   *  tap last selected), or `null` — drives a row's terracotta "selected"
   *  wash (design §2). Compared by `nodeKey`, not by reference. */
  selectedNode: NodeRef | null
  /** The fullscreen viewport container this sheet floats over — its height
   *  is what "half"/"full" are fractions OF (see `sheetDetents.ts`). */
  containerRef: React.RefObject<HTMLDivElement | null>
  /** The sheet's SETTLED detent — controlled, owned by `ShopApp.tsx` (see
   *  this file's module doc on why). */
  detent: SheetDetent
  /** Settles `detent` to a new value — fired on drag-release snap only.
   *  Never fired mid-drag: `dragHeightPx` below tracks an in-progress drag
   *  locally so every intermediate frame doesn't round-trip through the
   *  parent. */
  onDetentChange: (detent: SheetDetent) => void
  onToggleNode: (node: NodeRef) => void
  onToggleTagPath: (path: string[]) => void
  onSelectRow: (node: NodeRef) => void
  onLongPressRow: (node: NodeRef) => void
  /** The header's unit chip (design §1) opens the SAME `UnitPicker`
   *  `ShopApp.tsx` renders for the overflow menu's Settings row — this
   *  component only requests it, never renders the picker itself. */
  onOpenUnitPicker: () => void
  /** Reports this sheet's own CURRENT height (px) on every change — detent
   *  settle AND live drag alike, in BOTH orientations now (the old
   *  landscape side sheet reported a WIDTH instead — module doc, that
   *  concept is gone with it). `ShopApp.tsx`'s workbench dock/toast track
   *  wherever this sheet's top edge currently is. Optional so tests/other
   *  embeddings that don't care about toast placement can omit it. */
  onHeightChange?: (heightPx: number) => void
}

export function PartsSheet({
  scene,
  hiddenKeys,
  hiddenTagPaths,
  orientation,
  isolatedNode,
  selectedNode,
  containerRef,
  detent,
  onDetentChange,
  onToggleNode,
  onToggleTagPath,
  onSelectRow,
  onLongPressRow,
  onOpenUnitPicker,
  onHeightChange,
}: PartsSheetProps) {
  const isLandscape = orientation === 'landscape'

  const sections = useMemo(
    () => buildPartsSheetSections(scene, hiddenKeys, hiddenTagPaths, isolatedNode),
    [scene, hiddenKeys, hiddenTagPaths, isolatedNode],
  )
  const totalCount = useMemo(() => totalPartCount(sections), [sections])
  const visibleCount = useMemo(() => visiblePartCount(sections), [sections])
  const shownLabel = `${visibleCount} of ${totalCount} shown`

  // Live length format (design §1's unit chip must reflect — and every
  // row's dims must re-render on — ANY change: this sheet's own chip, the
  // Settings picker, or another window under Tauri, `settings/units.ts`'s
  // module doc on cross-window sync).
  const [unit, setUnit] = useState<LengthFormat>(getLengthUnit)
  useEffect(() => subscribeLengthUnit(setUnit), [])
  const metric = LENGTH_SYSTEM_OF[unit] === 'metric'

  // ---------------------------------------------------------------- container height tracking
  // `sheetDetents.ts`'s half/full fractions need the viewport container's
  // REAL height in px, reactively (a phone rotation mid-session must
  // reflow the detents) — a ResizeObserver on the same containerRef
  // ShopApp already threads through for InspectCard positioning.
  // window.innerHeight is a reasonable pre-measurement default (avoids a
  // 0-height flash on first paint); ResizeObserver is absent in some test
  // runners, guarded rather than assumed.
  const [containerHeightPx, setContainerHeightPx] = useState(
    () => containerRef.current?.clientHeight ?? (typeof window === 'undefined' ? 800 : window.innerHeight),
  )
  useEffect(() => {
    const el = containerRef.current
    if (el === null || typeof ResizeObserver === 'undefined') return
    const ro = new ResizeObserver((entries) => {
      const h = entries[0]?.contentRect.height
      if (h !== undefined) setContainerHeightPx(h)
    })
    ro.observe(el)
    return () => ro.disconnect()
  }, [containerRef])

  // ---------------------------------------------------------------- detent + drag
  // `detent` itself is the controlled prop above (module doc) — everything
  // below is the drag-tracking math that eventually calls `onDetentChange`
  // to settle it.
  // Non-null only while a handle drag is in progress — tracks the pointer
  // 1:1; once it settles (pointerup), `detent`'s own CSS-transitioned
  // height takes back over.
  const [dragHeightPx, setDragHeightPx] = useState<number | null>(null)
  const dragStartRef = useRef<{ startY: number; startHeightPx: number } | null>(null)

  const handlePointerDown = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    // Guarded the same way the (now-removed) side sheet tab's own pointerDown
    // always was: jsdom (this component's unit tests) has no Pointer Capture
    // API at all, so an unconditional call here would throw before a
    // synthetic drag ever got the chance to move the sheet.
    if (typeof ev.currentTarget.setPointerCapture === 'function') {
      ev.currentTarget.setPointerCapture(ev.pointerId)
    }
    const startHeightPx = detentHeightPx(detent, containerHeightPx)
    dragStartRef.current = { startY: ev.clientY, startHeightPx }
    setDragHeightPx(startHeightPx)
  }, [detent, containerHeightPx])

  const handlePointerMove = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    const start = dragStartRef.current
    if (start === null) return
    // Dragging UP (clientY decreases) grows the sheet — same sign
    // convention as an iOS sheet's grab handle.
    const delta = start.startY - ev.clientY
    setDragHeightPx(clampDragHeightPx(start.startHeightPx + delta, containerHeightPx))
  }, [containerHeightPx])

  const endDrag = useCallback(() => {
    const start = dragStartRef.current
    if (start === null) return
    dragStartRef.current = null
    // Reads `dragHeightPx` directly rather than through `setDragHeightPx`'s
    // own functional-updater form (the pre-lift version of this function
    // settled the OLD local `detent` state that way) — `onDetentChange` now
    // crosses into the PARENT's state, and calling a different component's
    // setter from inside a `setState` updater is exactly the "Cannot update
    // a component while rendering a different component" pattern React's
    // dev mode flags (caught during this fix's own E2E run). `endDrag` only
    // ever fires from THIS render's own pointerup/pointercancel handlers, so
    // the closed-over `dragHeightPx` is already current — no updater needed.
    const finalHeightPx = dragHeightPx ?? detentHeightPx(detent, containerHeightPx)
    onDetentChange(nearestDetent(finalHeightPx, containerHeightPx))
    setDragHeightPx(null)
  }, [dragHeightPx, detent, containerHeightPx, onDetentChange])

  const heightPx = dragHeightPx ?? detentHeightPx(detent, containerHeightPx)

  useEffect(() => {
    onHeightChange?.(heightPx)
  }, [heightPx, onHeightChange])

  return (
    <div
      data-testid="parts-sheet"
      style={
        isLandscape
          ? {
              // Self-positioned (unlike portrait's normal-flow root below,
              // which nests inside ShopApp's fused dock+sheet wrapper) —
              // landscape has no dock for this to fuse with (module doc:
              // the rail is a wholly separate object), so this floats on
              // its own, centered and width-capped over the bottom edge.
              position: 'absolute', left: '50%', bottom: 0, transform: 'translateX(-50%)', zIndex: 25,
              // Finding 7 (adversarial review): a flat `- 32px` only ever
              // budgeted a fixed 16px per side — under a narrow-enough
              // landscape viewport (~656px and below) the centered sheet's
              // right edge reached past that into the right rail
              // (`LANDSCAPE_RAIL_CLEARANCE_CSS` — ShopApp.tsx's own
              // export). Subtracting that SAME clearance from both sides
              // keeps the sheet centered while guaranteeing its right edge
              // never crosses into the rail's own reserved space.
              width: `min(${LANDSCAPE_SHEET_MAX_WIDTH_PX}px, calc(100% - (2 * ${LANDSCAPE_RAIL_CLEARANCE_CSS})))`,
              height: `${heightPx}px`,
              display: 'flex',
              flexDirection: 'column',
              background: 'var(--surface-sheet)',
              borderRadius: '18px 18px 0 0',
              boxShadow: '0 -10px 34px -12px rgba(27,26,23,.55)',
              transition: dragHeightPx === null ? 'height 320ms cubic-bezier(.32,.72,.28,1)' : 'none',
              overflow: 'hidden',
            }
          : {
              // A normal-flow flex child of ShopApp's dock+sheet wrapper
              // (that wrapper owns the `position:absolute; bottom:0` anchor
              // and the dock row above this) — NOT self-positioned, so the
              // charcoal dock and this sheet body read as one continuous
              // object with the dock riding above whatever height this
              // settles at (design_handoff_shop_mode/README.md §1
              // "Workbench dock"). No border/radius of its own — the fused
              // object's only rounded corners are the dock's top corners.
              flex: '0 0 auto',
              width: '100%',
              height: `${heightPx}px`,
              display: 'flex',
              flexDirection: 'column',
              background: 'var(--surface-sheet)',
              // Sheet snap timing (Motion summary: "height 320ms
              // cubic-bezier(.32,.72,.28,1)").
              transition: dragHeightPx === null ? 'height 320ms cubic-bezier(.32,.72,.28,1)' : 'none',
              overflow: 'hidden',
            }
      }
    >
      {/* Header (design §1 "Sheet header"): the drag surface — handle bar +
          the "Cutlist" title row (pill / unit chip). The column-header and
          row list below are siblings OUTSIDE this pointer-handling div,
          matching the prototype's own DOM split (its drag zone likewise
          stops before the column-header row) — the row list needs its own
          vertical scroll gesture, which a drag-anywhere handle would fight.
          touchAction 'none' stops the browser's own scroll/refresh gestures
          from competing with the pointer drag on a real touchscreen. The
          unit chip stops propagation on its own pointerDown (EyeToggle's
          existing precedent) so a tap on it is never also read as the
          start of a drag. */}
      <div
        data-testid="parts-sheet-handle"
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        style={{
          flex: '0 0 auto',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          padding: '10px 16px 6px',
          cursor: 'grab',
          touchAction: 'none',
          userSelect: 'none',
        }}
      >
        <div style={{ width: '40px', height: '5px', borderRadius: '3px', background: 'var(--shop-hairline)', marginBottom: '8px' }} />
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', width: '100%', gap: '8px' }}>
          <span style={{ fontFamily: 'var(--font-family-ui)', fontSize: '16px', fontWeight: 600, lineHeight: 1.2, color: 'var(--shop-text)', flexShrink: 0 }}>
            Cutlist
          </span>
          <div style={{ display: 'flex', alignItems: 'center', gap: '8px', minWidth: 0 }}>
            <span style={{
              fontFamily: 'var(--font-family-ui)', fontSize: '11.5px', fontWeight: 600, whiteSpace: 'nowrap',
              // --shop-accent-fill, not the mark/wash --shop-accent itself
              // (tokens.css's own doc comment on it) — this is cream TEXT
              // on the fill, which needs the ≥4.5:1 darker terracotta.
              color: 'var(--shop-on-accent)', background: 'var(--shop-accent-fill)', borderRadius: '9px', padding: '7px 11px',
            }}>
              {shownLabel}
            </span>
            <UnitChip unit={unit} onOpenUnitPicker={onOpenUnitPicker} />
          </div>
        </div>
      </div>

      {/* Column header (design §2): metric only — an imperial format's rows
          are stacked instead (each carries its own L/W/H letters inline),
          so a fixed-column header above them would describe nothing. */}
      {metric && (
        <div style={{
          display: 'flex', alignItems: 'center', height: '22px', padding: '0 16px', flex: '0 0 auto',
          fontFamily: 'var(--font-family-mono)', fontSize: '10px', fontWeight: 600, letterSpacing: '0.08em',
          color: 'var(--shop-text-faint)',
        }}>
          <span style={{ flex: 1 }}>PART</span>
          <span style={{ width: '64px', textAlign: 'right', color: 'var(--shop-axis-l)' }}>L</span>
          <span style={{ width: '64px', textAlign: 'right', color: 'var(--shop-axis-w)' }}>W</span>
          <span style={{ width: '64px', textAlign: 'right', color: 'var(--shop-axis-h)' }}>H</span>
          <span aria-hidden="true" style={{ width: '48px', flexShrink: 0 }} />
        </div>
      )}

      {/* README §1: "Home indicator always sits on sheet surface:
          padding-bottom env(safe-area-inset-bottom) + 10px" — on the
          scrollable content, not the sheet root, so the LAST row clears
          the home indicator strip without adding dead space below a short
          list (a `full`-height empty document's scroll area would
          otherwise just show a bigger gap for nothing). The centered
          landscape sheet never touches a horizontal edge, so it needs no
          horizontal inset here — only the vertical (bottom) one applies
          regardless of orientation. */}
      <div style={{ flex: 1, minHeight: 0, overflowY: 'auto', paddingBottom: 'calc(env(safe-area-inset-bottom) + 10px)' }}>
        {sections.map((section) => (
          <SectionBlock
            key={section.key}
            section={section}
            unit={unit}
            selectedNode={selectedNode}
            onToggleNode={onToggleNode}
            onToggleTagPath={onToggleTagPath}
            onSelectRow={onSelectRow}
            onLongPressRow={onLongPressRow}
          />
        ))}
      </div>
    </div>
  )
}

/** The header's unit chip (design §1).
 *
 *  Hit target: the design's 7×11px-padded chip alone is well under the
 *  44px `--hit-min` floor, but simply padding IT out would visibly grow
 *  the chip's own background/border pill — so the extra padding lands on
 *  this OUTER (invisible: no background/border of its own) button instead,
 *  cancelled by an equal-magnitude negative margin so the header row's
 *  layout doesn't inflate to match; the chip's real look (background,
 *  border, radius, its own 7×11 padding) moves to the inner span, sized
 *  exactly as before. Same technique as the section "Hide all"/"Show all"
 *  button below. */
function UnitChip({ unit, onOpenUnitPicker }: { unit: LengthFormat; onOpenUnitPicker: () => void }) {
  return (
    <button
      type="button"
      aria-label={`Units: ${LENGTH_FORMAT_NAME[unit]}`}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={onOpenUnitPicker}
      style={{
        display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
        background: 'none', border: 'none', cursor: 'pointer',
        padding: 'calc((var(--hit-min) - 30px) / 2) 0',
        margin: 'calc((30px - var(--hit-min)) / 2) 0',
      }}
    >
      <span style={{
        display: 'flex', alignItems: 'center', gap: '5px', whiteSpace: 'nowrap',
        fontFamily: 'var(--font-family-mono)', fontSize: '12px', fontWeight: 600, color: 'var(--shop-text)',
        background: 'var(--shop-eyebg)', border: '1px solid var(--shop-hairline)', borderRadius: '9px',
        padding: '7px 11px',
      }}>
        {LENGTH_FORMAT_SHORT_LABEL[unit]}
        <ChevronDownIcon size={11} />
      </span>
    </button>
  )
}

function SectionBlock({
  section,
  unit,
  selectedNode,
  onToggleNode,
  onToggleTagPath,
  onSelectRow,
  onLongPressRow,
}: {
  section: PartsSheetSection
  unit: LengthFormat
  selectedNode: NodeRef | null
  onToggleNode: (node: NodeRef) => void
  onToggleTagPath: (path: string[]) => void
  onSelectRow: (node: NodeRef) => void
  onLongPressRow: (node: NodeRef) => void
}) {
  // Catch-all "Unfiled"/"Parts" section: the design's reserved fixed color,
  // never hashed (tagPalette.ts's module doc) — every real tag section gets
  // a stable hash-derived hue instead.
  const color = section.path === null ? UNFILED_TAG_COLOR : colorForTagPath(section.path)
  return (
    <div>
      <div
        style={{
          position: 'sticky',
          top: 0,
          display: 'flex',
          alignItems: 'center',
          gap: '8px',
          padding: '8px 16px',
          background: hexToRgba(color, 0.08),
          fontFamily: 'var(--font-family-ui)',
        }}
      >
        <span aria-hidden="true" style={{ width: '9px', height: '9px', borderRadius: '3px', background: color, flexShrink: 0 }} />
        <span style={{
          flex: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          fontSize: '13px', fontWeight: 600, color: 'var(--shop-text)',
        }}>
          {section.label} · {section.rows.length}
        </span>
        {/* No master toggle for the untagged catch-all — nothing backs one
            (partsSheetModel.ts's module doc). Hit target: same outer-
            padding/negative-margin/inner-span technique as `UnitChip`
            above — the visible pill (background/radius/its own padding)
            stays on the inner span so growing the hit box doesn't grow
            the pill itself. */}
        {section.path !== null && (
          <button
            type="button"
            onPointerDown={(e) => e.stopPropagation()}
            onClick={(e) => { e.stopPropagation(); onToggleTagPath(section.path as string[]) }}
            style={{
              flexShrink: 0, background: 'none', border: 'none', cursor: 'pointer',
              display: 'flex', alignItems: 'center',
              padding: 'calc((var(--hit-min) - 25px) / 2) 0',
              margin: 'calc((25px - var(--hit-min)) / 2) 0',
            }}
          >
            <span style={{
              fontFamily: 'var(--font-family-ui)', fontWeight: 500, fontSize: '11px',
              color: 'var(--shop-text-muted)', background: 'var(--shop-eyebg)',
              borderRadius: '8px', padding: '6px 10px',
            }}>
              {section.hidden ? 'Show all' : 'Hide all'}
            </span>
          </button>
        )}
      </div>
      {section.rows.map((row) => (
        <PartRow
          key={`${row.node.kind}:${row.node.id}`}
          row={row}
          unit={unit}
          selected={selectedNode !== null && nodeKey(row.node) === nodeKey(selectedNode)}
          onToggleNode={onToggleNode}
          onSelectRow={onSelectRow}
          onLongPressRow={onLongPressRow}
        />
      ))}
    </div>
  )
}

function PartRow({
  row,
  unit,
  selected,
  onToggleNode,
  onSelectRow,
  onLongPressRow,
}: {
  row: PartsSheetRow
  unit: LengthFormat
  selected: boolean
  onToggleNode: (node: NodeRef) => void
  onSelectRow: (node: NodeRef) => void
  onLongPressRow: (node: NodeRef) => void
}) {
  // Tap-vs-long-press-vs-scroll, the SAME shape as ShopApp.tsx's viewport
  // wrapper handlers (shared timing constants) — a row sits inside a
  // scrollable list, so a drag-to-scroll starting on one must cancel the
  // long-press timer rather than firing isolate mid-scroll.
  const pressStartRef = useRef<{ x: number; y: number } | null>(null)
  const longPressTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const firedLongPressRef = useRef(false)

  const clearTimer = useCallback(() => {
    if (longPressTimerRef.current !== null) {
      clearTimeout(longPressTimerRef.current)
      longPressTimerRef.current = null
    }
  }, [])

  // Disarm on unmount: rows unmount wholesale when the document swaps (or
  // the sheet's sections rebuild), and an armed timer surviving that would
  // fire `onLongPressRow` with a node from the torn-down list — against
  // whatever document is loaded by then.
  useEffect(() => clearTimer, [clearTimer])

  const handlePointerDown = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    pressStartRef.current = { x: ev.clientX, y: ev.clientY }
    firedLongPressRef.current = false
    clearTimer()
    longPressTimerRef.current = setTimeout(() => {
      longPressTimerRef.current = null
      firedLongPressRef.current = true
      onLongPressRow(row.node)
    }, LONG_PRESS_MS)
  }, [clearTimer, onLongPressRow, row.node])

  const handlePointerMove = useCallback((ev: React.PointerEvent<HTMLDivElement>) => {
    const start = pressStartRef.current
    if (start === null) return
    const dx = ev.clientX - start.x
    const dy = ev.clientY - start.y
    if (dx * dx + dy * dy > LONG_PRESS_SLOP_PX * LONG_PRESS_SLOP_PX) clearTimer()
  }, [clearTimer])

  const handlePointerUp = useCallback(() => {
    clearTimer()
    pressStartRef.current = null
    if (!firedLongPressRef.current) onSelectRow(row.node)
  }, [clearTimer, onSelectRow, row.node])

  // Imperial: a stacked name/dims line (design §2) so nothing wraps or
  // clips. Metric: three right-aligned mono columns matching the column
  // header above. Both share the row's outer div/pointer-handling; only
  // the NAME+DIMS content differs.
  const imperial = LENGTH_SYSTEM_OF[unit] === 'imperial'
  const nameColor = selected ? 'var(--shop-accent-text)' : 'var(--shop-text)'
  const nameWeight = selected ? 700 : 500

  return (
    <div
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
      onPointerCancel={() => { clearTimer(); pressStartRef.current = null }}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: '10px',
        minHeight: '54px',
        // Nested indent +14px per depth level (design §2), on top of the
        // row's own 16px base inset.
        padding: `0 16px 0 ${16 + row.depth * 14}px`,
        borderBottom: '1px solid var(--shop-hairline-2)',
        fontFamily: 'var(--font-family-ui)',
        cursor: 'pointer',
        userSelect: 'none',
        touchAction: 'pan-y',
        opacity: row.hidden ? 0.42 : 1,
        background: selected ? 'var(--shop-row-selected-wash)' : 'transparent',
      }}
    >
      {imperial ? (
        <span style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: '3px', padding: '8px 0' }}>
          <span style={{
            fontSize: '15px', fontWeight: nameWeight, color: nameColor,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {row.label}
          </span>
          {row.extentsM !== null && (
            <span style={{ fontFamily: 'var(--font-family-mono)', fontSize: '12px', color: 'var(--shop-text-dim)', whiteSpace: 'nowrap' }}>
              <span style={{ color: 'var(--shop-axis-l)' }}>L</span> {formatLengthIn(row.extentsM[0], unit)}
              {' · '}
              <span style={{ color: 'var(--shop-axis-w)' }}>W</span> {formatLengthIn(row.extentsM[1], unit)}
              {' · '}
              <span style={{ color: 'var(--shop-axis-h)' }}>H</span> {formatLengthIn(row.extentsM[2], unit)}
            </span>
          )}
        </span>
      ) : (
        <>
          <span style={{
            flex: 1, minWidth: 0, fontSize: '15px', fontWeight: nameWeight, color: nameColor,
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {row.label}
          </span>
          <span style={dimColumnStyle}>{row.extentsM !== null ? formatLengthIn(row.extentsM[0], unit) : ''}</span>
          <span style={dimColumnStyle}>{row.extentsM !== null ? formatLengthIn(row.extentsM[1], unit) : ''}</span>
          <span style={dimColumnStyle}>{row.extentsM !== null ? formatLengthIn(row.extentsM[2], unit) : ''}</span>
        </>
      )}
      <EyeToggle
        hidden={row.hidden}
        onToggle={() => onToggleNode(row.node)}
        label={row.hidden ? `Show ${row.label}` : `Hide ${row.label}`}
      />
    </div>
  )
}

const dimColumnStyle: React.CSSProperties = {
  width: '64px', textAlign: 'right', flexShrink: 0, whiteSpace: 'nowrap',
  fontFamily: 'var(--font-family-mono)', fontSize: '12px', fontWeight: 500, color: 'var(--shop-text-dim)',
}

/** Eye toggle shared by a row's own hide/show — a plain 48×48 hit-area
 *  button (design §2), `stopPropagation`d on pointerdown (not just click)
 *  so it never also arms the owning row's long-press timer (the row's own
 *  `onPointerDown` never runs — matches DocumentTree.tsx's `Row` eye
 *  button's `e.stopPropagation()`, extended to pointer events here since
 *  this row's OWN gesture handling is pointer-based, not click-based). */
function EyeToggle({
  hidden,
  onToggle,
  label,
}: {
  hidden: boolean
  onToggle: () => void
  label: string
}) {
  return (
    <button
      type="button"
      aria-label={label}
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation()
        onToggle()
      }}
      style={{
        flexShrink: 0,
        width: '48px',
        height: '48px',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'flex-end',
        background: 'none',
        border: 'none',
        color: hidden ? 'var(--shop-text-faint)' : 'var(--shop-text-muted)',
        cursor: 'pointer',
        padding: 0,
      }}
    >
      {hidden ? <EyeOffIcon size={20} /> : <EyeIcon size={20} />}
    </button>
  )
}
