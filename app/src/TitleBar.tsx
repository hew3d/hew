/**
 * Custom window title bar for the Linux desktop shell (M-fix).
 *
 * On Linux/WebKitGTK the KWin server-side titlebar does not repaint the window
 * title after the webview calls `setTitle`, so the shell runs borderless
 * (`set_decorations(false)` in main.rs) and we draw our own chrome here: a
 * draggable bar showing the document title (filename + dirty dot + "Hew") with
 * minimize / maximize-restore / close controls, plus edge + corner grips that
 * drive native window resizing (borderless Wayland windows lose edge resize).
 *
 * Rendered only when `isTauri && isLinux`; macOS/Windows keep native decorations.
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

const BAR_HEIGHT = 30
const GRIP = 6 // edge thickness / corner size, px

const barStyle: React.CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  height: BAR_HEIGHT,
  background: '#171717',
  borderBottom: '1px solid #2a2a2a',
  flexShrink: 0,
  userSelect: 'none',
  WebkitUserSelect: 'none',
}

const titleStyle: React.CSSProperties = {
  flex: 1,
  textAlign: 'center',
  fontSize: 12,
  color: '#cfcfcf',
  whiteSpace: 'nowrap',
  overflow: 'hidden',
  textOverflow: 'ellipsis',
  lineHeight: `${BAR_HEIGHT}px`,
  padding: '0 8px',
}

const btnStyle: React.CSSProperties = {
  width: 44,
  height: BAR_HEIGHT,
  display: 'flex',
  alignItems: 'center',
  justifyContent: 'center',
  border: 'none',
  background: 'transparent',
  color: '#cfcfcf',
  cursor: 'pointer',
  padding: 0,
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

export function TitleBar({ title }: { title: string }) {
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
        <div style={titleStyle} data-tauri-drag-region>
          {title}
        </div>
        <button style={btnStyle} title="Minimize" onClick={minimize} aria-label="Minimize">
          <svg width="11" height="11" viewBox="0 0 11 11"><rect x="1" y="5" width="9" height="1" fill="currentColor" /></svg>
        </button>
        <button style={btnStyle} title={maximized ? 'Restore' : 'Maximize'} onClick={toggleMax} aria-label="Maximize">
          {maximized ? (
            <svg width="11" height="11" viewBox="0 0 11 11" fill="none" stroke="currentColor">
              <rect x="1.5" y="3.5" width="6" height="6" /><path d=" 3.5V1.5h6v6h-2" />
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
          onMouseEnter={(e) => { e.currentTarget.style.background = '#c42b1c'; e.currentTarget.style.color = '#fff' }}
          onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#cfcfcf' }}
        >
          <svg width="11" height="11" viewBox="0 0 11 11" stroke="currentColor"><path d=" 1.5l8 8M9.5 1.5l-8 8" /></svg>
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
