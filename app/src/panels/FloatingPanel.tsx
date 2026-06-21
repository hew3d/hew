/**
 * FloatingPanel — reusable chrome wrapper for the four overlay panels
 * (Model Info, Materials, Tags, Object Info).
 *
 * Owns:
 *   - Absolute positioning over the viewport.
 *   - Drag-to-move via pointer events on the title bar.
 *   - Resize via a bottom-right grip (pointer events + setPointerCapture).
 *   - Bring-to-front (z-index bump) on mousedown anywhere in the panel.
 *   - Position + size persistence to localStorage, keyed per panel.
 *   - Clamping so the panel (position AND size) stays within the bounds of
 *     its offset parent — the viewport container — rather than the whole
 *     window, so a panel can never be dragged/resized over the status bar
 *     or outside the viewport area. The header must always stay grabbable.
 *
 * The wrapper renders the title bar (title text + × close button); the
 * child content is just the panel body — no double header.
 */

import { useCallback, useEffect, useRef, useState } from 'react'

export interface FloatingPanelProps {
  /** Stable identity for this panel — used as the localStorage key suffix. */
  panelId: string
  /** Title shown in the drag handle / header bar. */
  title: string
  /** Fallback position used when nothing is persisted yet. */
  defaultPosition: { x: number; y: number }
  /** Default (and persisted-fallback) width in pixels. */
  width: number
  /** Called when the user clicks the × close button. */
  onClose: () => void
  /** Base z-index for this panel before any bring-to-front bump. */
  zIndex: number
  /** Called on mousedown/pointerdown anywhere in the panel — bring to front. */
  onFocus: () => void
  children: React.ReactNode
}

const STORAGE_PREFIX = 'hew.panel.'
const HEADER_HEIGHT = 28
/** Minimum visible sliver of the header that must remain within the container. */
const MIN_VISIBLE = 24
/** Minimum panel dimensions when resizing. */
const MIN_WIDTH = 160
const MIN_HEIGHT = 80
/** Default (unresized) height cap, as a fraction of the container height. */
const DEFAULT_HEIGHT_FRACTION = 0.5
/** Magnetic snap distance, in px, for container-edge and panel-to-panel docking. */
const SNAP_THRESHOLD = 10

interface Size {
  w: number
  /** null = unset (auto height, capped at DEFAULT_HEIGHT_FRACTION of container). */
  h: number | null
}

interface Rect {
  x: number
  y: number
  w: number
  /** Effective height in px (auto-height panels report their measured height). */
  h: number
}

/**
 * Module-level registry of every mounted FloatingPanel's current rect, keyed
 * by panelId. Used purely for panel-to-panel magnetic snapping while dragging
 * — each panel publishes its rect on mount/move/resize/unmount, and a panel
 * being dragged reads the others (excluding itself) as snap targets. This is
 * an in-memory, render-cycle-scoped cache, not persisted state.
 */
const panelRectRegistry = new Map<string, Rect>()

/** Snap `value` to `target` if within SNAP_THRESHOLD, else return `value` unchanged. */
function snapTo(value: number, target: number): number {
  return Math.abs(value - target) <= SNAP_THRESHOLD ? target : value
}

/** Resolve the offset parent (viewport container) the panel is positioned within. */
function getContainer(root: HTMLDivElement | null): HTMLElement | null {
  if (root === null) return null
  return root.offsetParent as HTMLElement | null
}

function clampPosition(
  x: number,
  y: number,
  width: number,
  containerW: number,
  containerH: number,
): { x: number; y: number } {
  // Keep at least MIN_VISIBLE px of the header within the container on all sides.
  const minX = MIN_VISIBLE - width
  const maxX = containerW - MIN_VISIBLE
  const minY = 0
  const maxY = containerH - MIN_VISIBLE
  return {
    x: Math.min(Math.max(x, minX), Math.max(minX, maxX)),
    y: Math.min(Math.max(y, minY), Math.max(minY, maxY)),
  }
}

/** Clamp width/height so the panel fits within the container (from its current position). */
function clampSize(
  w: number,
  h: number | null,
  x: number,
  y: number,
  containerW: number,
  containerH: number,
): Size {
  const maxW = Math.max(MIN_WIDTH, containerW - x)
  const clampedW = Math.min(Math.max(w, MIN_WIDTH), maxW)
  if (h === null) return { w: clampedW, h: null }
  const maxH = Math.max(MIN_HEIGHT, containerH - y)
  const clampedH = Math.min(Math.max(h, MIN_HEIGHT), maxH)
  return { w: clampedW, h: clampedH }
}

/**
 * Apply magnetic docking to a candidate (already container-clamped) position:
 * snap to container edges, then snap to sibling panel edges (with
 * perpendicular alignment so stacked panels line up flush), independently on
 * each axis. The result is re-clamped so snapping can never push the panel
 * outside the container.
 */
function applySnapping(
  x: number,
  y: number,
  w: number,
  h: number,
  containerW: number,
  containerH: number,
  selfId: string,
): { x: number; y: number } {
  let snappedX = x
  let snappedY = y

  // --- Container-edge docking ---
  snappedX = snapTo(snappedX, 0)
  snappedX = snapTo(snappedX, containerW - w)
  snappedY = snapTo(snappedY, 0)
  snappedY = snapTo(snappedY, containerH - h)

  // --- Panel-to-panel docking against siblings ---
  for (const [siblingId, sib] of panelRectRegistry) {
    if (siblingId === selfId) continue

    // Horizontal (x-axis) edge snapping: left<->right, right<->left.
    if (Math.abs(x - sib.x) <= SNAP_THRESHOLD) {
      snappedX = sib.x // left flush with sibling's left
    } else if (Math.abs(x - (sib.x + sib.w)) <= SNAP_THRESHOLD) {
      snappedX = sib.x + sib.w // left flush against sibling's right
    } else if (Math.abs(x + w - sib.x) <= SNAP_THRESHOLD) {
      snappedX = sib.x - w // right flush against sibling's left
    } else if (Math.abs(x + w - (sib.x + sib.w)) <= SNAP_THRESHOLD) {
      snappedX = sib.x + sib.w - w // right flush with sibling's right
    }

    // Vertical (y-axis) edge snapping: top<->bottom, bottom<->top.
    if (Math.abs(y - sib.y) <= SNAP_THRESHOLD) {
      snappedY = sib.y // top flush with sibling's top
    } else if (Math.abs(y - (sib.y + sib.h)) <= SNAP_THRESHOLD) {
      snappedY = sib.y + sib.h // top flush against sibling's bottom (stack below)
    } else if (Math.abs(y + h - sib.y) <= SNAP_THRESHOLD) {
      snappedY = sib.y - h // bottom flush against sibling's top (stack above)
    } else if (Math.abs(y + h - (sib.y + sib.h)) <= SNAP_THRESHOLD) {
      snappedY = sib.y + sib.h - h // bottom flush with sibling's bottom
    }

    // Perpendicular alignment: when vertically docked against this sibling
    // (top or bottom snap engaged), also align left/right edges if close —
    // this is what makes a stacked panel click into a clean left-aligned
    // column rather than just touching edges at an arbitrary x offset.
    const verticallyDocked = snappedY === sib.y + sib.h || snappedY === sib.y - h
    if (verticallyDocked) {
      if (Math.abs(x - sib.x) <= SNAP_THRESHOLD) snappedX = sib.x
      else if (Math.abs(x + w - (sib.x + sib.w)) <= SNAP_THRESHOLD) snappedX = sib.x + sib.w - w
    }
    // Symmetric case: horizontally docked against this sibling (left/right
    // edges flush) — align tops if close.
    const horizontallyDocked = snappedX === sib.x + sib.w || snappedX === sib.x - w
    if (horizontallyDocked) {
      if (Math.abs(y - sib.y) <= SNAP_THRESHOLD) snappedY = sib.y
      else if (Math.abs(y + h - (sib.y + sib.h)) <= SNAP_THRESHOLD) snappedY = sib.y + sib.h - h
    }
  }

  // Re-clamp: snapping must never push the panel outside the container.
  return clampPosition(snappedX, snappedY, w, containerW, containerH)
}

function loadPosition(panelId: string, fallback: { x: number; y: number }): { x: number; y: number } {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + panelId + '.pos')
    if (raw === null) return fallback
    const parsed = JSON.parse(raw)
    if (typeof parsed?.x === 'number' && typeof parsed?.y === 'number') {
      return { x: parsed.x, y: parsed.y }
    }
  } catch {
    /* ignore malformed storage — fall back to default */
  }
  return fallback
}

function savePosition(panelId: string, pos: { x: number; y: number }): void {
  try {
    localStorage.setItem(STORAGE_PREFIX + panelId + '.pos', JSON.stringify(pos))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
}

function loadSize(panelId: string, fallbackWidth: number): Size {
  try {
    const raw = localStorage.getItem(STORAGE_PREFIX + panelId + '.size')
    if (raw === null) return { w: fallbackWidth, h: null }
    const parsed = JSON.parse(raw)
    if (typeof parsed?.w === 'number') {
      const h = typeof parsed?.h === 'number' ? parsed.h : null
      return { w: parsed.w, h }
    }
  } catch {
    /* ignore malformed storage — fall back to default */
  }
  return { w: fallbackWidth, h: null }
}

function saveSize(panelId: string, size: Size): void {
  try {
    localStorage.setItem(STORAGE_PREFIX + panelId + '.size', JSON.stringify(size))
  } catch {
    /* ignore quota / privacy-mode errors */
  }
}

export function FloatingPanel({
  panelId,
  title,
  defaultPosition,
  width,
  onClose,
  zIndex,
  onFocus,
  children,
}: FloatingPanelProps) {
  const [pos, setPos] = useState(() => loadPosition(panelId, defaultPosition))
  const [size, setSize] = useState(() => loadSize(panelId, width))

  const dragRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null)
  const resizeRef = useRef<{ startX: number; startY: number; origW: number; origH: number } | null>(null)
  const rootRef = useRef<HTMLDivElement>(null)

  // Mirror the latest pos/size into refs every render so callbacks with
  // stable identities (e.g. reclamp, below) can always read the current
  // values instead of whatever was captured when the closure was created.
  const posRef = useRef(pos)
  posRef.current = pos
  const sizeRef = useRef(size)
  sizeRef.current = size

  // Re-clamp position + size against the container's current dimensions.
  // Reads pos/size via refs (always current) rather than closing over the
  // state values directly, so this can keep a stable identity (empty deps)
  // without going stale after a manual move/resize.
  const reclamp = useCallback(() => {
    const container = getContainer(rootRef.current)
    if (container === null) return
    const cw = container.clientWidth
    const ch = container.clientHeight
    setPos((p) => clampPosition(p.x, p.y, sizeRef.current.w, cw, ch))
    setSize((s) => {
      const next = clampSize(s.w, s.h, posRef.current.x, posRef.current.y, cw, ch)
      return next.w === s.w && next.h === s.h ? s : next
    })
  }, [])

  // Re-clamp once on mount (the container's real size is only known post-mount)
  // and again on every window resize so a panel dragged/sized near an edge
  // doesn't strand fully outside the container after it shrinks.
  useEffect(() => {
    reclamp()
    window.addEventListener('resize', reclamp)
    return () => window.removeEventListener('resize', reclamp)
  }, [reclamp])

  // Publish this panel's current rect to the module-level registry on every
  // mount/move/resize so siblings can snap against it, and remove it on
  // unmount so a closed panel can't act as a stale snap target. The
  // "effective" height mirrors the heightStyle logic below: when size.h is
  // unset (auto), measure the rendered height instead.
  useEffect(() => {
    const effectiveH = size.h ?? rootRef.current?.getBoundingClientRect().height ?? MIN_HEIGHT
    panelRectRegistry.set(panelId, { x: pos.x, y: pos.y, w: size.w, h: effectiveH })
    return () => {
      panelRectRegistry.delete(panelId)
    }
  }, [panelId, pos.x, pos.y, size.w, size.h])

  const handleHeaderPointerDown = useCallback((ev: React.PointerEvent) => {
    if (ev.button !== 0) return
    onFocus()
    dragRef.current = { startX: ev.clientX, startY: ev.clientY, origX: pos.x, origY: pos.y }
    ev.currentTarget.setPointerCapture(ev.pointerId)
  }, [onFocus, pos.x, pos.y])

  const handleHeaderPointerMove = useCallback((ev: React.PointerEvent) => {
    const drag = dragRef.current
    if (drag === null) return
    const container = getContainer(rootRef.current)
    if (container === null) return
    const dx = ev.clientX - drag.startX
    const dy = ev.clientY - drag.startY
    const cw = container.clientWidth
    const ch = container.clientHeight
    const clamped = clampPosition(drag.origX + dx, drag.origY + dy, size.w, cw, ch)
    const effectiveH = size.h ?? rootRef.current?.getBoundingClientRect().height ?? MIN_HEIGHT
    const snapped = applySnapping(clamped.x, clamped.y, size.w, effectiveH, cw, ch, panelId)
    setPos(snapped)
  }, [size.w, size.h, panelId])

  const handleHeaderPointerUp = useCallback((ev: React.PointerEvent) => {
    if (dragRef.current === null) return
    dragRef.current = null
    setPos((p) => {
      savePosition(panelId, p)
      return p
    })
    ev.currentTarget.releasePointerCapture(ev.pointerId)
  }, [panelId])

  const handleResizePointerDown = useCallback((ev: React.PointerEvent) => {
    if (ev.button !== 0) return
    ev.stopPropagation()
    onFocus()
    // Resolve the *effective* current height (auto → measured) so dragging
    // starts from where the panel visually is, not from the unset sentinel.
    const measuredH = rootRef.current?.getBoundingClientRect().height ?? MIN_HEIGHT
    resizeRef.current = {
      startX: ev.clientX,
      startY: ev.clientY,
      origW: size.w,
      origH: size.h ?? measuredH,
    }
    ev.currentTarget.setPointerCapture(ev.pointerId)
  }, [onFocus, size.w, size.h])

  const handleResizePointerMove = useCallback((ev: React.PointerEvent) => {
    const resize = resizeRef.current
    if (resize === null) return
    const container = getContainer(rootRef.current)
    if (container === null) return
    const dx = ev.clientX - resize.startX
    const dy = ev.clientY - resize.startY
    const next = clampSize(
      resize.origW + dx,
      resize.origH + dy,
      pos.x,
      pos.y,
      container.clientWidth,
      container.clientHeight,
    )
    setSize(next)
  }, [pos.x, pos.y])

  const handleResizePointerUp = useCallback((ev: React.PointerEvent) => {
    if (resizeRef.current === null) return
    resizeRef.current = null
    setSize((s) => {
      saveSize(panelId, s)
      return s
    })
    ev.currentTarget.releasePointerCapture(ev.pointerId)
  }, [panelId])

  // Default (never manually resized) height is capped at half the container
  // height — content beyond that scrolls via the body's overflowY:auto.
  // Once resized, size.h is an explicit pixel value (already clamped to fit).
  const heightStyle: React.CSSProperties =
    size.h === null
      ? { maxHeight: `${DEFAULT_HEIGHT_FRACTION * 100}%` }
      : { height: `${size.h}px` }

  return (
    <div
      ref={rootRef}
      onPointerDownCapture={onFocus}
      style={{
        position: 'absolute',
        left: `${pos.x}px`,
        top: `${pos.y}px`,
        width: `${size.w}px`,
        background: '#2a2a2a',
        color: '#ddd',
        borderRadius: '4px',
        boxShadow: '0 4px 16px rgba(0,0,0,0.5)',
        display: 'flex',
        flexDirection: 'column',
        zIndex,
        ...heightStyle,
      }}
    >
      {/* Title bar — drag handle */}
      <div
        onPointerDown={handleHeaderPointerDown}
        onPointerMove={handleHeaderPointerMove}
        onPointerUp={handleHeaderPointerUp}
        style={{
          height: `${HEADER_HEIGHT}px`,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '0 8px',
          background: '#333',
          borderRadius: '4px 4px 0 0',
          cursor: 'move',
          touchAction: 'none',
        }}
      >
        <span style={{ fontWeight: 'bold', fontSize: '12px', color: '#eee', userSelect: 'none' }}>
          {title}
        </span>
        <button
          onClick={onClose}
          onPointerDown={(e) => e.stopPropagation()}
          title="Close panel"
          style={{
            background: 'none',
            border: 'none',
            color: '#888',
            cursor: 'pointer',
            fontSize: '14px',
            lineHeight: 1,
            padding: '0 2px',
          }}
        >
          ×
        </button>
      </div>

      {/* Body — supplied by the caller; scrolls independently of the header */}
      <div style={{ overflowY: 'auto', minHeight: 0, padding: '8px' }}>
        {children}
      </div>

      {/* Resize grip — bottom-right corner */}
      <div
        onPointerDown={handleResizePointerDown}
        onPointerMove={handleResizePointerMove}
        onPointerUp={handleResizePointerUp}
        title="Resize panel"
        style={{
          position: 'absolute',
          right: 0,
          bottom: 0,
          width: '14px',
          height: '14px',
          cursor: 'nwse-resize',
          touchAction: 'none',
          background:
            'linear-gradient(135deg, transparent 0%, transparent 45%, #777 45%, #777 55%, transparent 55%, transparent 100%)',
        }}
      />
    </div>
  )
}
