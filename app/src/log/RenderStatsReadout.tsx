/**
 * RenderStatsReadout — compact render-statistics line for the LogPanel header.
 *
 * Shows the most recent frame's three.js `renderer.info` counters with draw
 * calls first and most prominent (the number that quantifies the instancing
 * win: dozens, not thousands, on instance-heavy models), plus the CPU-side
 * duration of the last renderer.render() call and — only while renders are
 * streaming (orbit, tool drag) — a smoothed frames-per-second.
 *
 * The viewport renders ON DEMAND, so "no new frames" is the normal resting
 * state, not an error: when no render has landed recently the fps slot shows
 * "idle" and the other numbers still describe the frame currently on screen.
 * GPU work is asynchronous, so the timing is labeled "cpu ms", not frame time.
 *
 * Mounting this component is what turns stats collection on (it is the
 * subscriber that makes renderStats.isRenderStatsActive() true); unmounting
 * it returns the render loop to a single boolean check per frame.
 */

import { useEffect, useRef, useState } from 'react'
import { getSnapshot, subscribe, type RenderStatsSnapshot } from '../viewport/renderStats'

// Snapshots arrive at most every NOTIFY_INTERVAL_MS (250ms) while renders
// stream, so anything older than this means the stream has stopped and the
// fps figure would be stale.
const STALE_AFTER_MS = 600

const TITLE =
  'three.js renderer.info for the most recent frame. ' +
  '"cpu ms" is the CPU-side renderer.render() duration (GPU work is async). ' +
  'fps is smoothed over the current burst of renders; "idle" means the ' +
  'on-demand render loop is at rest.'

const containerStyle: React.CSSProperties = {
  fontFamily: 'monospace',
  fontSize: '11px',
  whiteSpace: 'nowrap',
  cursor: 'default',
}

const drawCallsStyle: React.CSSProperties = {
  color: 'var(--accent-base)',
  fontWeight: 'bold',
}

const mutedStyle: React.CSSProperties = {
  color: 'var(--text-muted)',
}

const idleStyle: React.CSSProperties = {
  color: 'var(--text-faint)',
  fontStyle: 'italic',
}

/** Compact count: 1234 → "1234", 56789 → "56.8k", 1234567 → "1.23M". */
function fmtCount(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2)}M`
  if (n >= 10_000) return `${(n / 1_000).toFixed(1)}k`
  return String(n)
}

export function RenderStatsReadout() {
  const [snap, setSnap] = useState<RenderStatsSnapshot | null>(() => getSnapshot())
  const [stale, setStale] = useState(true)
  const staleTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    const unsub = subscribe((s) => {
      setSnap(s)
      setStale(false)
      // Each snapshot re-arms the idle timer; when snapshots stop arriving
      // the fps slot flips to "idle" rather than freezing a dead number.
      if (staleTimer.current !== null) clearTimeout(staleTimer.current)
      staleTimer.current = setTimeout(() => setStale(true), STALE_AFTER_MS)
    })
    return () => {
      unsub()
      if (staleTimer.current !== null) clearTimeout(staleTimer.current)
    }
  }, [])

  if (snap === null) {
    return (
      <span style={{ ...containerStyle, ...mutedStyle }} title={TITLE}>
        draw — · no frames yet
      </span>
    )
  }

  const streaming = !stale && snap.fps !== null
  return (
    <span style={containerStyle} title={TITLE}>
      <span style={drawCallsStyle}>draw {snap.drawCalls}</span>
      <span style={mutedStyle}>
        {' · tri '}
        {fmtCount(snap.triangles)}
        {' · geo '}
        {snap.geometries}
        {' · tex '}
        {snap.textures}
        {' · '}
        {snap.cpuMs.toFixed(1)} cpu ms
        {' · '}
      </span>
      {streaming ? (
        <span style={mutedStyle}>{Math.round(snap.fps as number)} fps</span>
      ) : (
        <span style={idleStyle}>idle</span>
      )}
    </span>
  )
}
