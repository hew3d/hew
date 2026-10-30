import { useEffect, useRef } from 'react'
import { runProbe, type ProbeResults } from './probe'

declare global {
  interface Window {
    __hew_probe?: { done: boolean; results?: ProbeResults }
    /** Re-runs rows on the mounted probe page (the offline row needs this). */
    __hew_probe_run?: (rows: string[]) => Promise<ProbeResults>
  }
}

/**
 * Dev-only render root for the realm probe (plugin system stage P0). Selected
 * by `#plugin-probe?rows=a,b&interactive=1` in main.tsx, which only honors it
 * in dev or VITE_HEW_TEST builds.
 */
export function ProbeRoot() {
  const started = useRef(false)
  const mountRef = useRef<HTMLDivElement>(null)
  const viewportRef = useRef<HTMLDivElement>(null)
  const outRef = useRef<HTMLPreElement>(null)

  useEffect(() => {
    // StrictMode runs effects twice in dev; the probe must run once.
    if (started.current) return
    started.current = true
    const query = new URLSearchParams(window.location.hash.split('?')[1] ?? '')
    const rowsParam = query.get('rows')
    const rows = rowsParam === null ? undefined : rowsParam.split(',').filter(Boolean)
    const interactive = query.get('interactive') === '1'
    const offline = query.get('offline') === '1'
    const run = (r?: string[]): Promise<ProbeResults> =>
      runProbe(mountRef.current!, { rows: r, viewport: viewportRef.current!, interactive, offline })
    window.__hew_probe_run = async (r) => {
      const results = await run(r)
      outRef.current!.textContent = JSON.stringify(results, null, 2)
      return results
    }
    window.__hew_probe = { done: false }
    void run(rows).then((results) => {
      window.__hew_probe = { done: true, results }
      outRef.current!.textContent = JSON.stringify(results, null, 2)
    })
  }, [])

  return (
    <div style={{ padding: 8, fontFamily: 'monospace', color: '#ddd', background: '#222', minHeight: '100vh' }}>
      <div id="probe-mount" ref={mountRef} />
      <div id="probe-viewport" ref={viewportRef} style={{ position: 'relative', width: 600, height: 400, background: '#334', margin: '8px 0' }} />
      <pre id="probe-results" ref={outRef} />
    </div>
  )
}
