import { useEffect, useId, useRef, useState } from 'react'

/** Sections share available space until resized; chosen heights survive collapse.
 * The tray scrolls when their combined minimum/chosen heights no longer fit. */
const MIN_HEIGHT = 100
// A generous ceiling keeps resize controls in a finite, accessible range.
const MAX_HEIGHT = 10_000
const KEYBOARD_STEP = 20
export interface TraySectionProps {
  title: string
  collapsed: boolean
  onToggle: () => void
  children: React.ReactNode
  /** Extra control rendered at the header's trailing edge, alongside the
   *  title/chevron toggle button rather than inside it (a nested `<button>`
   *  would be invalid HTML and would also fire `onToggle` on every click) —
   *  Scenes' header ⊕ Add Scene icon button (SPEC.md §1) is the first
   *  consumer. Omitted for every other section, so their header markup and
   *  behavior stay unchanged. */
  headerRight?: React.ReactNode
}

export function TraySection({ title, collapsed, onToggle, children, headerRight }: TraySectionProps) {
  const sectionRef = useRef<HTMLDivElement>(null)
  const stopDrag = useRef<(() => void) | null>(null)
  const [height, setHeight] = useState<number | null>(null)
  const bodyId = useId()
  const [automaticHeight, setAutomaticHeight] = useState(MIN_HEIGHT)

  useEffect(() => {
    const section = sectionRef.current
    if (collapsed || height !== null || !section) return
    const measure = () => setAutomaticHeight(Math.max(MIN_HEIGHT, section.getBoundingClientRect().height))
    measure()
    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(measure)
    observer.observe(section)
    return () => observer.disconnect()
  }, [collapsed, height])

  useEffect(() => () => stopDrag.current?.(), [collapsed])

  const resize = (next: number) => setHeight(Math.max(MIN_HEIGHT, Math.min(MAX_HEIGHT, next)))

  return (
    <div
      ref={sectionRef}
      style={{
        display: 'flex',
        flexDirection: 'column',
        borderBottom: '1px solid var(--border-hairline)',
        boxSizing: 'border-box',
        minHeight: collapsed ? 0 : MIN_HEIGHT,
        height: collapsed || height === null ? undefined : height,
        flex: collapsed || height !== null ? '0 0 auto' : '1 1 0px',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', flexShrink: 0 }}>
        <button
          type="button"
          onClick={onToggle}
          aria-expanded={!collapsed}
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            flex: '1 1 auto',
            minWidth: 0,
            padding: '11px 13px',
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            fontFamily: 'var(--font-family-mono)',
            fontSize: 'var(--font-size-panel-header, 11px)',
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--text-secondary)',
          }}
        >
          <span>{title}</span>
          <span aria-hidden="true" style={{ color: 'var(--text-section)' }}>{collapsed ? '▸' : '▾'}</span>
        </button>
        {headerRight !== undefined && (
          <span style={{ flexShrink: 0, paddingRight: '11px' }}>{headerRight}</span>
        )}
      </div>
      {!collapsed && (
        // `data-tray-section-body` lets App find this body to settle focus
        // inside it before a menu or shortcut collapses the section — see
        // App.tsx's blurFocusWithin.
        <div id={bodyId} data-tray-section-body={title} style={{ flex: '1 1 0', minHeight: 0, overflowY: 'auto', padding: '0 13px 13px' }}>
          {children}
        </div>
      )}
      {!collapsed && (
        <div
          role="separator"
          tabIndex={0}
          aria-label={title}
          aria-orientation="horizontal"
          aria-controls={bodyId}
          aria-valuemin={MIN_HEIGHT}
          aria-valuemax={MAX_HEIGHT}
          aria-valuenow={height ?? automaticHeight}
          onKeyDown={(event) => {
            if (event.key !== 'ArrowUp' && event.key !== 'ArrowDown') return
            event.preventDefault()
            event.stopPropagation()
            const current = height ?? sectionRef.current!.getBoundingClientRect().height
            resize(current + (event.key === 'ArrowDown' ? KEYBOARD_STEP : -KEYBOARD_STEP))
          }}
          onPointerDown={(event) => {
            if (event.button !== 0) return
            event.preventDefault()
            stopDrag.current?.()
            const handle = event.currentTarget
            const pointerId = event.pointerId
            const startY = event.clientY
            const startHeight = sectionRef.current!.getBoundingClientRect().height
            const onMove = (move: PointerEvent) => {
              if (move.pointerId === pointerId) resize(startHeight + move.clientY - startY)
            }
            const endDrag = (end: PointerEvent) => {
              if (end.pointerId === pointerId) cleanup()
            }
            const cleanup = () => {
              window.removeEventListener('pointermove', onMove)
              window.removeEventListener('pointerup', endDrag)
              window.removeEventListener('pointercancel', endDrag)
              handle.removeEventListener('lostpointercapture', endDrag)
              window.removeEventListener('blur', cleanup)
              stopDrag.current = null
              try { handle.releasePointerCapture(pointerId) } catch { /* capture already lost */ }
            }
            stopDrag.current = cleanup
            window.addEventListener('pointermove', onMove)
            window.addEventListener('pointerup', endDrag)
            window.addEventListener('pointercancel', endDrag)
            handle.addEventListener('lostpointercapture', endDrag)
            window.addEventListener('blur', cleanup)
            try { handle.setPointerCapture(pointerId) } catch { /* window listeners also cover capture failure */ }
          }}
          style={{
            height: 5,
            flexShrink: 0,
            cursor: 'row-resize',
            touchAction: 'none',
            userSelect: 'none',
            background: 'var(--border-hairline)',
          }}
        />
      )}
    </div>
  )
}
