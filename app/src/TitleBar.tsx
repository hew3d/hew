/**
 * Custom window title bar for the Linux and Windows desktop shells
 * (M-fix Linux-only; widened to Windows in).
 *
 * On Linux/WebKitGTK the KWin server-side titlebar does not repaint the window
 * title after the webview calls `setTitle`, so that shell runs borderless
 * (`set_decorations(false)` in main.rs) and draws its own chrome here.
 * Windows joined this treatment in for the Studio design's in-window
 * chrome (`02_app_shell.md`'s Windows/Linux title bar spec): a draggable bar
 * showing a placeholder app glyph, the document name, and the "Edited/Saved
 * <relative time>" save-state indicator (replaces a Save button as the
 * primary save-state cue — no button here or elsewhere in Hew), with
 * minimize / maximize-restore / close caption buttons, plus edge + corner
 * grips that drive native window resizing (borderless Wayland windows lose
 * edge resize).
 *
 * Rendered only when `isTauri && isLinux`. Windows reverted to native
 * decorations (WebView2 repaints the native caption correctly, unlike
 * WebKitGTK), so it — like macOS — keeps the OS-drawn title bar and window
 * controls; Hew has no way to inject a custom save-state indicator into that
 * native chrome (see the docked tray's Object Info, for a possible future home).
 */
import { useEffect, useRef, useState } from 'react'

type WindowApi = typeof import('@tauri-apps/api/window')

/** Mirrors Tauri's (non-exported) `ResizeDirection` string union. */
type ResizeDir =
  | 'North'
  | 'South'
  | 'East'
  | 'West'
  | 'NorthEast'
  | 'NorthWest'
  | 'SouthEast'
  | 'SouthWest'

// `02_app_shell.md`'s Windows/Linux title bar spec: 34px height (vs macOS's
// native 46px, which Hew doesn't draw).
const BAR_HEIGHT = 34
const GRIP = 6 // edge thickness / corner size, px

const barStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  height: BAR_HEIGHT,
  background: 'var(--surface-titlebar-bottom, #1b1f26)',
  borderBottom: '1px solid var(--border-hairline, #2a2a2a)',
  flexShrink: 0,
  userSelect: 'none',
  WebkitUserSelect: 'none',
  gap: 'var(--space-3, 8px)',
  padding: '0 0 0 var(--space-6, 13px)',
}

/** App glyph — the Hew mark (open isometric-cube wireframe, "Terracotta" from
 * the Hew Brand Sheet v1). Kept as brand Terracotta (#C45D3C) rather than the
 * UI `--accent-base` on purpose: the mark is brand chrome, not a themed accent.
 * Stroke is thickened vs. the source SVG (4.6→7 in the 100-unit viewBox) so the
 * wireframe stays legible at ~15px. */
const glyphStyle: React.CSSProperties = {
  flexShrink: 0,
  display: 'block',
}

const nameStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)',
  fontSize: 'var(--font-size-titlebar-filename, 13px)',
  fontWeight: 600,
  color: 'var(--text-primary, #cdd4de)',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
}

const saveStateStyle: React.CSSProperties = {
  fontFamily: 'var(--font-family-ui)',
  fontSize: 'var(--font-size-titlebar-meta, 11px)',
  color: 'var(--text-section, #5b6472)',
  whiteSpace: 'nowrap',
  flexShrink: 0,
}

const dragFillStyle: React.CSSProperties = {
  flex: 1,
  height: '100%',
  minWidth: 0,
}

const btnStyle: React.CSSProperties = {
  width: 44,
  height: BAR_HEIGHT,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  border: 'none',
  background: 'transparent',
  color: 'var(--text-tertiary, #9aa3b0)',
  cursor: 'pointer',
  padding: 0,
  flexShrink: 0,
}

/** One resize grip's CSS box + the native resize direction it triggers. */
const GRIPS: { dir: ResizeDir; style: React.CSSProperties }[] = [
  { dir: 'North', style: { top: 0, left: GRIP, right: GRIP, height: GRIP, cursor: 'ns-resize' } },
  { dir: 'South', style: { bottom: 0, left: GRIP, right: GRIP, height: GRIP, cursor: 'ns-resize' } },
  { dir: 'West', style: { top: GRIP, bottom: GRIP, left: 0, width: GRIP, cursor: 'ew-resize' } },
  { dir: 'East', style: { top: GRIP, bottom: GRIP, right: 0, width: GRIP, cursor: 'ew-resize' } },
  { dir: 'NorthWest', style: { top: 0, left: 0, width: GRIP, height: GRIP, cursor: 'nwse-resize' } },
  { dir: 'NorthEast', style: { top: 0, right: 0, width: GRIP, height: GRIP, cursor: 'nesw-resize' } },
  { dir: 'SouthWest', style: { bottom: 0, left: 0, width: GRIP, height: GRIP, cursor: 'nesw-resize' } },
  { dir: 'SouthEast', style: { bottom: 0, right: 0, width: GRIP, height: GRIP, cursor: 'nwse-resize' } },
]

export interface TitleBarProps {
  /** Bare document name (no dirty mark, no " — Hew" suffix) — `documentSession.ts`'s `documentName()`. */
  name: string
  /** "Edited <relative time>" / "Saved <relative time>" / "" — `documentSession.ts`'s `saveStateLabel()`. */
  saveState: string
}

export function TitleBar({ name, saveState }: TitleBarProps) {
  const apiRef = useRef<WindowApi | null>(null)
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    let unlisten: (() => void) | undefined
    let cancelled = false
    import('@tauri-apps/api/window')
      .then(async (api) => {
        if (cancelled) return
        apiRef.current = api
        const win = api.getCurrentWindow()
        setMaximized(await win.isMaximized().catch(() => false))
        unlisten = await win.onResized(() => {
          win.isMaximized().then(setMaximized).catch(() => { /* ignore */ })
        })
      })
      .catch(() => { /* not in Tauri */ })
    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  const win = () => apiRef.current?.getCurrentWindow()
  const minimize = () => win()?.minimize().catch(() => { /* ignore */ })
  const toggleMax = () => win()?.toggleMaximize().catch(() => { /* ignore */ })
  const close = () => win()?.close().catch(() => { /* ignore */ })

  const startResize = (dir: ResizeDir) => (e: React.PointerEvent) => {
    if (e.button !== 0) return
    const api = apiRef.current
    if (api === null) return
    e.preventDefault()
    // Tauri's `startResizeDragging` takes a string-union direction (our
    // ResizeDir mirrors it); the grip names match the expected values.
    api.getCurrentWindow().startResizeDragging(dir).catch(() => { /* ignore */ })
  }

  return (
    <>
      <div style={barStyle} data-tauri-drag-region>
        <svg
          aria-hidden="true"
          width="15"
          height="15"
          viewBox="-50 -50 100 100"
          style={glyphStyle}
        >
          <g fill="none" stroke="#C45D3C" strokeWidth="7" strokeLinejoin="round" strokeLinecap="round">
            <polygon points="0,-34 29.44,-17 29.44,17 0,34 -29.44,17 -29.44,-17" />
            <line x1="0" y1="0" x2="0" y2="-34" />
            <line x1="0" y1="0" x2="-29.44" y2="-17" />
            <line x1="0" y1="0" x2="29.44" y2="-17" />
          </g>
        </svg>
        <span style={nameStyle}>{name}</span>
        {saveState !== '' && <span style={saveStateStyle}>{saveState}</span>}
        <div style={dragFillStyle} data-tauri-drag-region />
        <button style={btnStyle} title="Minimize" onClick={minimize} aria-label="Minimize">
          <svg width="11" height="11" viewBox="0 0 11 11"><rect x="1" y="5" width="9" height="1" fill="currentColor" /></svg>
        </button>
        <button style={btnStyle} title={maximized ? 'Restore' : 'Maximize'} onClick={toggleMax} aria-label="Maximize">
          {maximized ? (
            <svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor">
              <rect x="1.5" y="3.5" width="6" height="6" /><path d="M3.5 3.5V1.5h6v6h-2" />
            </svg>
          ) : (
            <svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor">
              <rect x="1.5" y="1.5" width="8" height="8" />
            </svg>
          )}
        </button>
        <button
          style={btnStyle}
          title="Close"
          onClick={close}
          aria-label="Close"
          onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--chrome-win-close-hover, #e53b41)'; e.currentTarget.style.color = '#fff' }}
          onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = 'var(--text-tertiary, #9aa3b0)' }}
        >
          <svg width="11" height="11" viewBox="0 0 11 11" stroke="currentColor"><path d="M1.5 1.5l8 8M9.5 1.5l-8 8" /></svg>
        </button>
      </div>
      {/* Resize grips — hidden while maximized (no edges to drag). */}
      {!maximized &&
        GRIPS.map((g) => (
          <div
            key={g.dir}
            onPointerDown={startResize(g.dir)}
            style={{ position: 'fixed', zIndex: 9999, ...g.style }}
          />
        ))}
    </>
  )
}
