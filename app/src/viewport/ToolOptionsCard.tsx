/* oxlint-disable lingui/no-unlocalized-strings -- True Curves strings are not yet wrapped for translation: this file is English-only until they are */
import { createContext, useContext, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import './ToolOptionsCard.css'
import { isComposingKey } from '../input/keyboard'

export const ToolOptionsExpandedContext = createContext(true)

// A card unmounts when its tool ends; only its collapsed state survives.
const collapsedTools = new Map<string, boolean>()

const iconStyle: CSSProperties = {
  border: 'none', background: 'transparent', color: 'var(--text-secondary)',
  font: 'inherit', cursor: 'pointer', padding: '2px 5px',
}

/** Native button activation/focus keys belong to the card, not the viewport.
 * Other CAD shortcuts, including undo/redo and Alt, continue to bubble. */
function containButtonKey(ev: ReactKeyboardEvent<HTMLDivElement>): void {
  if (ev.target instanceof Element && ev.target.closest('button') && ['Enter', ' ', 'Spacebar', 'Tab'].includes(ev.key)) ev.stopPropagation()
}

/** Text-only help shared by the title and the option rows. */
export function ToolOptionsInfo({ label, children }: { label: string; children: ReactNode }) {
  const [open, setOpen] = useState(false)
  const expanded = useContext(ToolOptionsExpandedContext)
  useEffect(() => { if (!expanded) setOpen(false) }, [expanded])
  const root = useRef<HTMLSpanElement>(null)
  const id = useId()
  useEffect(() => {
    if (!open) return
    const outside = (ev: PointerEvent): void => {
      if (!root.current?.contains(ev.target as Node)) setOpen(false)
    }
    const escape = (ev: KeyboardEvent): void => {
      if (ev.key !== 'Escape' || isComposingKey(ev)) return
      setOpen(false)
      // A field owns its own Escape (including restoring its draft).
      const target = ev.target
      if (target instanceof Element && target.closest('input, textarea, [contenteditable="true"]')) return
      ev.preventDefault()
      ev.stopPropagation()
    }
    window.addEventListener('pointerdown', outside, true)
    window.addEventListener('keydown', escape, true)
    return () => {
      window.removeEventListener('pointerdown', outside, true)
      window.removeEventListener('keydown', escape, true)
    }
  }, [open])
  return (
    <span ref={root} style={{ position: 'relative', display: 'inline-flex' }}>
      <button type="button" aria-label={`About ${label}`} aria-expanded={open}
        aria-controls={open ? id : undefined} onClick={() => setOpen(!open)} style={iconStyle}>ⓘ</button>
      {open && <span id={id} role="note" style={{
        position: 'absolute', bottom: 'calc(100% + 6px)', right: 0, width: 208,
        padding: 10, background: 'var(--surface-overlay)', border: '1px solid var(--border-strong)',
        borderRadius: 'var(--radius-panel-item)', boxShadow: 'var(--shadow-dock)',
        color: 'var(--text-primary)', fontSize: 12, fontWeight: 400, lineHeight: 1.5,
        letterSpacing: 'normal', textTransform: 'none', zIndex: 40,
      }}>{children}</span>}
    </span>
  )
}

export interface ToolOptionsCardProps {
  tool: string
  info: string
  children: ReactNode
}

/** Shared viewport chrome; it has no knowledge of a tool's settings. */
export function ToolOptionsCard({ tool, info, children }: ToolOptionsCardProps) {
  const [collapsed, setCollapsed] = useState(() => collapsedTools.get(tool) ?? false)
  const root = useRef<HTMLDivElement>(null)
  const rowsId = useId()
  useLayoutEffect(() => {
    // Panels live below a display:contents wrapper. The positioned ancestor
    // is the viewport container shared with the dock, not the DOM parent.
    const viewport = root.current?.offsetParent
    if (!(viewport instanceof HTMLElement)) return
    const previous = viewport.style.getPropertyValue('--tool-card-inset')
    viewport.style.setProperty('--tool-card-inset', '272px')
    return () => {
      if (previous) viewport.style.setProperty('--tool-card-inset', previous)
      else viewport.style.removeProperty('--tool-card-inset')
    }
  }, [])
  return (
    <div ref={root} role="region" aria-label={`${tool} options`} data-testid="tool-options-card" className="tool-options-card"
      onKeyDown={containButtonKey} onKeyUp={containButtonKey}
      onPointerDown={(ev) => {
        // A pointer click on a choice must not steal typed-size input.
        if (!(ev.target as HTMLElement).closest('input, textarea, [contenteditable="true"]')) {
          ev.preventDefault()
          const focused = document.activeElement
          if (focused instanceof HTMLElement && root.current?.contains(focused)) focused.blur()
        }
      }}
      style={{
        position: 'absolute', left: 16, bottom: 16, width: 240, zIndex: 20,
        boxSizing: 'border-box', background: 'var(--surface-overlay)',
        border: '1px solid var(--border-strong)', borderRadius: 'var(--radius-panel-item)',
        boxShadow: 'var(--shadow-dock)', fontFamily: 'var(--font-family-ui)', color: 'var(--text-primary)',
      }}>
      <div style={{ display: 'flex', flexShrink: 0, alignItems: 'center', gap: 4, padding: '10px 12px' }}>
        <span style={{
          flex: 1, fontFamily: 'var(--font-family-mono)', fontSize: 'var(--font-size-section-header)',
          fontWeight: 600, letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--text-section)',
        }}>{tool}</span>
        <ToolOptionsInfo label={tool}>{info}</ToolOptionsInfo>
        <button type="button" aria-label={`${collapsed ? 'Expand' : 'Collapse'} ${tool} options`}
          aria-expanded={!collapsed} aria-controls={rowsId} style={iconStyle}
          onClick={() => {
            collapsedTools.set(tool, !collapsed)
            setCollapsed(!collapsed)
          }}>{collapsed ? '▸' : '▾'}</button>
      </div>
      <div className="tool-options-content" data-collapsed={collapsed} aria-hidden={collapsed} inert={collapsed}>
        <div className="tool-options-scroll">
          <ToolOptionsExpandedContext.Provider value={!collapsed}>
            <div id={rowsId} style={{ padding: '0 12px 12px' }}>{children}</div>
          </ToolOptionsExpandedContext.Provider>
        </div>
      </div>
    </div>
  )
}
