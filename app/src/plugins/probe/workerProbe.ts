/* oxlint-disable lingui/no-unlocalized-strings -- dev-only probe: protocol strings and scripts for the engine record, never user-facing */
/**
 * The probe Worker body (plugin system stage P0), started from an in-frame
 * `blob:` URL as a classic worker. After the `{ t: 'port' }` handshake it
 * serves the requests listed in the stage table on the transferred port.
 */
export const WORKER_PROBE_SOURCE = `
let port = null
const settle = (p, ms) => Promise.race([p, new Promise((r) => setTimeout(() => r({ timeout: true }), ms))])
// A rejected fetch can come from CORS or the network as well as from policy.
// securitypolicyviolation events say which directive actually fired, so each
// attempt records the violations seen while it ran.
let violations = []
self.addEventListener('securitypolicyviolation', (e) => { violations.push(e.effectiveDirective || e.violatedDirective) })
function describe(e) { return String(e && e.name) + ': ' + String(e && e.message) }

// One denial attempt. 'blocked': the call threw/rejected/errored before any
// response. 'timeout': no response in time (not a block). 'allowed': a response or an open event arrived. 'absent': the API
// is undefined. 'url' marks entries whose URL cannot even be parsed in a blob
// worker (a relative URL has no base there): those block for the wrong reason
// and must not count as policy evidence.
async function attempt(name, api, fn, relative, recordOnly, required) {
  if (api !== undefined && typeof globalThis[api] === 'undefined') return { name, outcome: 'absent', detail: api + ' undefined', required }
  violations = []
  try {
    const r = await settle(fn(), 1500)
    await new Promise((res) => setTimeout(res, 20))
    const csp = violations.length ? violations.join('+') : 'none'
    if (r && r.timeout) return { name, outcome: 'timeout', detail: 'timeout without response csp=' + csp, csp, required }
    return { name, outcome: 'allowed', detail: String(r && r.detail !== undefined ? r.detail : 'response'), csp, recordOnly, required }
  } catch (e) {
    await new Promise((res) => setTimeout(res, 20))
    const d = describe(e)
    const csp = violations.length ? violations.join('+') : 'none'
    const parse = /parse|invalid url|not a valid url|illegal/i.test(d)
    return { name, outcome: 'blocked', detail: (relative && parse ? 'url_unparseable ' : '') + d + ' csp=' + csp, csp, recordOnly, required }
  }
}
function socketLike(make) {
  return new Promise((resolve, reject) => {
    let s
    try { s = make() } catch (e) { reject(e); return }
    s.onopen = () => { try { s.close() } catch (e) {} resolve({ detail: 'open' }) }
    s.onerror = () => { try { s.close() } catch (e) {} reject(new Error('error event')) }
    if ('onclose' in s) s.onclose = () => reject(new Error('closed'))
  })
}
function idbOpen() {
  return new Promise((resolve, reject) => {
    const r = indexedDB.open('x')
    r.onsuccess = () => resolve({ detail: 'opened' })
    r.onerror = () => reject(r.error || new Error('idb error'))
  })
}
async function denials(hostOrigin) {
  const entries = [
    ['fetch("/")', 'fetch', () => fetch('/'), true],
    ['fetch(location.origin + "/favicon.svg")', 'fetch', () => fetch(location.origin + '/favicon.svg'), true],
    ['fetch("https://example.com/")', 'fetch', () => fetch('https://example.com/'), false],
    ['fetch(blob)', 'fetch', () => fetch(URL.createObjectURL(new Blob(['x']))), false],
    ['WebSocket("wss://example.com/")', 'WebSocket', () => socketLike(() => new WebSocket('wss://example.com/')), false],
    ['indexedDB.open("x")', 'indexedDB', () => idbOpen(), false],
    // Accepted: an ephemeral per-realm Cache API on WebKit; recorded, not required blocked.
    ['caches.open("x")', 'caches', () => caches.open('x'), false, true],
    // The two Cache.add entries the stage names must be blocked, not absent or stalled.
    ['caches.open("x").add("/favicon.svg")', 'caches', () => caches.open('x').then((c) => c.add('/favicon.svg')), true, false, true],
    ['caches.open("x").add("https://example.com/")', 'caches', () => caches.open('x').then((c) => c.add('https://example.com/')), false, false, true],
    ['EventSource("/x")', 'EventSource', () => socketLike(() => new EventSource('/x')), true],
  ]
  if (hostOrigin) {
    // Absolute twin of the relative Cache.add entry (a relative URL cannot parse in a blob worker).
    entries.push(['caches.open("x").add(hostOrigin + "/favicon.svg")', 'caches', () => caches.open('x').then((c) => c.add(hostOrigin + '/favicon.svg')), false])
    entries.push(['fetch(hostOrigin + "/")', 'fetch', () => fetch(hostOrigin + '/'), false])
    entries.push(['fetch(hostOrigin + "/favicon.svg")', 'fetch', () => fetch(hostOrigin + '/favicon.svg'), false])
    entries.push(['EventSource(hostOrigin + "/x")', 'EventSource', () => socketLike(() => new EventSource(hostOrigin + '/x')), false])
  }
  const results = []
  for (const [name, api, fn, rel, rec, req] of entries) results.push(await attempt(name, api, fn, rel, rec === true, req === true))
  return results
}
async function wasm() {
  try {
    await WebAssembly.instantiate(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))
    return { ok: true, error: null }
  } catch (e) { return { ok: false, error: describe(e) } }
}
async function serve(m) {
  switch (m.t) {
    case 'echo': port.postMessage({ t: 'echo', n: m.n }); break
    case 'burst':
      for (let i = 0; i < m.count; i++) port.postMessage({ t: 'burst_item', i })
      port.postMessage({ t: 'burst_done' })
      break
    case 'transfer':
      port.postMessage({ t: 'transfer', byteLength: m.buffer.byteLength, buffer: m.buffer, transfer: [m.buffer] }, [m.buffer])
      break
    case 'env':
      port.postMessage({
        t: 'env',
        isolated: self.crossOriginIsolated === true,
        sab: typeof SharedArrayBuffer,
        rtc: typeof RTCPeerConnection,
        ws: typeof WebSocket,
        idb: typeof indexedDB,
        caches: typeof caches,
        origin: self.origin,
      })
      break
    case 'denials': port.postMessage({ t: 'denials', results: await denials(m.hostOrigin) }); break
    case 'spin': for (;;) {}
    case 'wasm': port.postMessage({ t: 'wasm', ...(await wasm()) }); break
  }
}
self.onmessage = (ev) => {
  if (!ev.data || ev.data.t !== 'port' || !ev.ports[0]) return
  port = ev.ports[0]
  port.onmessage = (e) => { void serve(e.data) }
  port.start()
}
`
