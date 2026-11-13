/* oxlint-disable lingui/no-unlocalized-strings -- dev-only probe: protocol strings and scripts for the engine record, never user-facing */
/**
 * The probe realm's boot script (plugin system stage P0). The text between
 * the template backticks is the exact byte sequence placed between the
 * <script> tags of the srcdoc, and the exact bytes the realm policy pins by
 * SHA-256. It carries no nonce and no per-instance data, which keeps the hash
 * constant; the nonce travels in a <meta name="hew-nonce"> element beside it.
 */
export const BOOT_PROBE_SOURCE = `(() => {
  'use strict';
  const nonce = document.querySelector('meta[name="hew-nonce"]').content;
  const mode = document.querySelector('meta[name="hew-realm"]').content; // "background" | "surface"
  let port = null;
  const blobUrls = [];
  function blobUrl(bytes, type) {
    const u = URL.createObjectURL(new Blob([bytes], { type }));
    blobUrls.push(u);
    return u;
  }
  function onPort(p) {
    port = p;
    port.onmessage = (ev) => handle(ev.data);
    port.start();
    port.postMessage({ t: 'hello_ack', mode, origin: location.origin, isolated: self.crossOriginIsolated === true });
  }
  function handle(m) {
    switch (m.t) {
      case 'start_worker': {
        const url = blobUrl(m.code, 'text/javascript');
        const w = new Worker(url, m.module ? { type: 'module' } : undefined);
        const ch = new MessageChannel();
        w.onerror = (e) => port.postMessage({ t: 'worker_error', message: String(e.message || e) });
        w.postMessage({ t: 'port' }, [ch.port2]);
        ch.port1.onmessage = (ev) => port.postMessage({ t: 'from_worker', data: ev.data }, ev.data && ev.data.transfer ? ev.data.transfer : []);
        ch.port1.start();
        handle.workerPort = ch.port1;
        handle.worker = w;
        port.postMessage({ t: 'worker_started' });
        break;
      }
      case 'to_worker':
        handle.workerPort.postMessage(m.data, m.transfer || []);
        break;
      case 'surface': {
        document.documentElement.innerHTML = m.html;
        for (const s of m.scripts || []) {
          const el = document.createElement('script');
          el.src = blobUrl(s.code, 'text/javascript');
          document.body.appendChild(el);
        }
        port.postMessage({ t: 'surface_ready' });
        break;
      }
      case 'eval_in_frame': {
        let result;
        try { result = { ok: true, value: (0, eval)(m.code) }; } catch (e) { result = { ok: false, error: String(e && e.name) + ': ' + String(e && e.message) }; }
        Promise.resolve(result.value).then(
          (v) => port.postMessage({ t: 'eval_result', id: m.id, ok: result.ok, value: v === undefined ? null : v, error: result.error || null }),
          (e) => port.postMessage({ t: 'eval_result', id: m.id, ok: false, value: null, error: String(e && e.name) + ': ' + String(e && e.message) }),
        );
        break;
      }
    }
  }
  window.hewProbeReport = (data) => port && port.postMessage({ t: 'surface_report', data });
  window.addEventListener('message', function once(ev) {
    if (ev.source !== window.parent) return;
    const d = ev.data;
    if (!d || d.hew !== 'port' || d.nonce !== nonce || !ev.ports || !ev.ports[0]) return;
    window.removeEventListener('message', once);
    onPort(ev.ports[0]);
  });
  window.parent.postMessage({ hew: 'ready', nonce }, '*');
})();`

/** SHA-256 of BOOT_PROBE_SOURCE, base64, as a CSP hash source requires. */
export const BOOT_HASH_LITERAL = 'Egx2DNg4hmltoX81gGTsHxrST87w3aygelblMpjL62Q='

/** Computes the same hash at runtime (the unit test compares the two). */
export async function bootHash(): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(BOOT_PROBE_SOURCE))
  let bin = ''
  for (const b of new Uint8Array(digest)) bin += String.fromCharCode(b)
  return btoa(bin)
}
