/* oxlint-disable lingui/no-unlocalized-strings -- dev-only probe: protocol strings and scripts for the engine record, never user-facing */
/**
 * Scripts the probe runs inside a surface realm (plugin system stage P0).
 * Each is delivered as a `blob:` script through the `surface` message and
 * ends by calling `window.hewProbeReport({...})` (defined by the boot script).
 */

export const SURFACE_ENV = `(() => {
  const name = (f) => { try { f(); return 'ok' } catch (e) { return String(e && e.name) } }
  hewProbeReport({
    rtc: typeof RTCPeerConnection,
    webgl2: !!document.createElement('canvas').getContext('webgl2'),
    isolated: crossOriginIsolated === true,
    localStorage: name(() => localStorage.length),
    cookie: name(() => document.cookie),
    origin: location.origin,
    prefetchLink: (() => {
      const l = document.createElement('link')
      l.rel = 'dns-prefetch'
      l.href = 'https://example.invalid'
      document.head.appendChild(l)
      return l.relList.supports('dns-prefetch')
    })(),
  })
})()`

export const SURFACE_STYLES = `(() => {
  document.head.innerHTML = '<style>#a{width:17px}</style>'
  document.body.innerHTML = '<div id="a"></div><div id="b" style="width:23px"></div>'
  const w = (id) => getComputedStyle(document.getElementById(id)).width
  const link = document.createElement('link')
  link.rel = 'stylesheet'
  link.href = URL.createObjectURL(new Blob(['#c{width:29px}'], { type: 'text/css' }))
  const c = document.createElement('div')
  c.id = 'c'
  document.body.appendChild(c)
  let done = false
  const finish = (blobSheet) => {
    if (done) return
    done = true
    hewProbeReport({ styleTag: w('a'), styleAttr: w('b'), blobSheet })
  }
  link.addEventListener('load', () => finish(w('c')))
  link.addEventListener('error', () => finish(w('c')))
  setTimeout(() => finish('timeout'), 2000)
  document.head.appendChild(link)
})()`

// Besides the stage's self-origin image, a data: image is loaded as a control:
// an 'error' for the self image only means the policy blocked it if the
// control loads (a 404 would error too).
export const SURFACE_IMG = `(() => {
  const report = {}
  const load = (key, src) => new Promise((resolve) => {
    const img = document.createElement('img')
    const t = setTimeout(() => { report[key] = 'timeout'; resolve() }, 2000)
    img.addEventListener('load', () => { clearTimeout(t); report[key] = 'load'; resolve() })
    img.addEventListener('error', () => { clearTimeout(t); report[key] = 'error'; resolve() })
    img.src = src
    document.body.appendChild(img)
  })
  const px = 'data:image/gif;base64,R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7'
  load('selfImg', '/favicon.svg').then(() => load('dataImg', px)).then(() => hewProbeReport(report))
})()`

export const SURFACE_SPIN = `hewProbeReport({ spinning: true }); for (;;) {}`

export const SURFACE_NAVIGATE_REMOTE = `hewProbeReport({ ran: true }); location.href = 'https://example.com/probe-escape'`

export const SURFACE_NAVIGATE_SELF = `hewProbeReport({ ran: true }); location.href = '/?probe-escape=1'`

export const SURFACE_CLICK = `(() => {
  document.body.innerHTML = '<button id="go" style="width:200px;height:80px">go</button>'
  document.getElementById('go').addEventListener('click', () => hewProbeReport({ clicked: true }))
})()`

export const SURFACE_HIT = `(() => {
  document.body.innerHTML = '<div id="hit" style="position:fixed;inset:0"></div>'
  document.getElementById('hit').addEventListener('pointerdown', (e) => hewProbeReport({ hit: [e.clientX, e.clientY] }))
})()`

// What a realm frame can see of the desktop shell's command bridge. The shell
// injects its bridge into its own pages; a sandboxed frame must find none of
// it. Reported: the bridge's globals, the engine's raw message channels
// (WebKit's `webkit.messageHandlers`, WebView2's `chrome.webview`) and what
// posting to one does, a read of the parent's bridge, and whether the bridge's
// two request URLs answer.
export const SURFACE_IPC = `(() => {
  const kind = (f) => { try { return typeof f() } catch (e) { return 'threw ' + String(e && e.name) } }
  const violations = []
  document.addEventListener('securitypolicyviolation', (e) => violations.push(e.effectiveDirective || e.violatedDirective))
  const post = (name, f) => {
    try {
      const target = f()
      if (target === undefined || target === null) return 'absent'
      // Two invoke-shaped messages with nothing a realm could not write: one
      // harmless, one that would retitle the main window if the shell acted
      // on it. The host compares the title before and after.
      target.postMessage(JSON.stringify({ cmd: 'plugin:app|version', callback: 0, error: 0, payload: {} }))
      for (const key of [undefined, '', 0]) {
        target.postMessage(JSON.stringify({
          cmd: 'plugin:window|set_title', callback: 0, error: 0,
          payload: { label: 'main', value: 'hew-probe-reached-the-shell' },
          options: {}, __TAURI_INVOKE_KEY__: key,
        }))
      }
      return 'posted'
    } catch (e) { return 'threw ' + String(e && e.name) }
  }
  const tryFetch = (url) => {
    const before = violations.length
    return Promise.race([
      fetch(url, { method: 'POST', body: '{}' }).then((r) => 'allowed ' + r.status, (e) => 'blocked ' + String(e && e.name)),
      new Promise((res) => setTimeout(() => res('timeout'), 1500)),
    ]).then((outcome) => new Promise((res) => setTimeout(() => res(outcome + ' csp=' + (violations.slice(before).join('+') || 'none')), 20)))
  }
  const report = {
    globals: {
      __TAURI_INTERNALS__: kind(() => window.__TAURI_INTERNALS__),
      __TAURI__: kind(() => window.__TAURI__),
      __TAURI_INVOKE_KEY__: kind(() => window.__TAURI_INVOKE_KEY__),
      ipc: kind(() => window.ipc),
      webkit: kind(() => window.webkit),
      messageHandlers: kind(() => window.webkit.messageHandlers),
      chromeWebview: kind(() => window.chrome.webview),
    },
    parentBridge: kind(() => window.parent.__TAURI_INTERNALS__),
    topBridge: kind(() => window.top.__TAURI_INTERNALS__),
    posts: {
      'window.ipc': post('ipc', () => window.ipc),
      'webkit.messageHandlers.ipc': post('webkit', () => window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.ipc),
      'chrome.webview': post('webview2', () => window.chrome && window.chrome.webview),
    },
    fetches: {},
  }
  tryFetch('ipc://localhost/plugin%3Aapp%7Cversion')
    .then((r) => { report.fetches.ipc = r; return tryFetch('http://ipc.localhost/plugin%3Aapp%7Cversion') })
    .then((r) => { report.fetches.http = r; hewProbeReport(report) })
})()`
