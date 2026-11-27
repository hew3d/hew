// Probes everything a Worker could reach beyond the bus: IPC globals, network, script loading, navigation, storage.
function describeError(e) {
  return e && e.name ? e.name + ': ' + String(e.message).slice(0, 100) : String(e).slice(0, 100);
}
async function attempt(fn) {
  try {
    var v = await fn();
    return 'ok' + (v === undefined ? '' : ': ' + String(v).slice(0, 100));
  } catch (e) {
    return 'threw ' + describeError(e);
  }
}
// Resolves with what the first of the events says, or "silent" after `ms`.
function firstEvent(target, names, ms) {
  return new Promise(function (resolve) {
    var done = false;
    var finish = function (what) {
      if (!done) {
        done = true;
        resolve(what);
      }
    };
    names.forEach(function (n) {
      target.addEventListener(n, function (ev) {
        finish(n + (ev && ev.message ? ': ' + ev.message.slice(0, 80) : ''));
      });
    });
    setTimeout(function () {
      finish('silent');
    }, ms);
  });
}

hew.commands.handle('run', async function () {
  var origin = (HOSTILE && HOSTILE.origin) || 'http://127.0.0.1:4391';
  var out = {};

  // What exists.
  var names = [
    '__TAURI__', '__TAURI_INTERNALS__', '__TAURI_IPC__', '__TAURI_METADATA__', 'ipc', 'window', 'parent', 'top', 'document',
    'localStorage', 'sessionStorage', 'XMLHttpRequest', 'WebSocket', 'EventSource', 'fetch', 'importScripts', 'indexedDB', 'caches',
    'BroadcastChannel', 'Worker', 'SharedWorker', 'Notification', 'RTCPeerConnection', 'WebTransport', 'cookieStore',
  ];
  names.forEach(function (n) {
    out['typeof ' + n] = typeof self[n];
  });
  out['typeof navigator.sendBeacon'] = typeof navigator.sendBeacon;
  out['typeof navigator.serviceWorker'] = typeof navigator.serviceWorker;
  out['typeof navigator.storage'] = typeof navigator.storage;
  out['typeof navigator.clipboard'] = typeof navigator.clipboard;
  out['isSecureContext'] = String(self.isSecureContext);
  out['crossOriginIsolated'] = String(self.crossOriginIsolated);
  out['origin'] = String(self.origin);
  out['location.protocol'] = String(self.location && self.location.protocol);

  // Network.
  out['fetch http'] = await attempt(function () {
    return fetch(origin + '/__plugin_probe__/');
  });
  out['fetch data'] = await attempt(function () {
    return fetch('data:text/plain,hi');
  });
  var own = URL.createObjectURL(new Blob(['hi']));
  out['fetch blob'] = await attempt(function () {
    return fetch(own);
  });
  out['XMLHttpRequest'] = await attempt(function () {
    var x = new XMLHttpRequest();
    x.open('GET', origin + '/__plugin_probe__/');
    var seen = firstEvent(x, ['load', 'error', 'abort', 'timeout'], 2000);
    x.send();
    return seen;
  });
  out['WebSocket'] = await attempt(function () {
    var ws = new WebSocket('ws://' + origin.replace(/^https?:\/\//, '') + '/__plugin_probe__/');
    return firstEvent(ws, ['open', 'error', 'close'], 2000);
  });
  out['EventSource'] = await attempt(function () {
    var es = new EventSource(origin + '/__plugin_probe__/');
    return firstEvent(es, ['open', 'error'], 2000);
  });
  out['sendBeacon'] = await attempt(function () {
    return navigator.sendBeacon(origin + '/__plugin_probe__/', 'x');
  });

  // Loading script.
  out['importScripts http'] = await attempt(function () {
    importScripts(origin + '/__plugin_probe__/script.js');
  });
  out['importScripts blob'] = await attempt(function () {
    importScripts(URL.createObjectURL(new Blob(['1'], { type: 'text/javascript' })));
  });
  out['importScripts data'] = await attempt(function () {
    importScripts('data:text/javascript,1');
  });
  out['import() data'] = await attempt(function () {
    return import('data:text/javascript,export default 1');
  });
  out['import() http'] = await attempt(function () {
    return import(origin + '/__plugin_probe__/script.js');
  });
  out['import() blob'] = await attempt(function () {
    return import(URL.createObjectURL(new Blob(['export default 1'], { type: 'text/javascript' })));
  });
  out['eval'] = await attempt(function () {
    return eval('1+1');
  });
  out['new Function'] = await attempt(function () {
    return new Function('return 1')();
  });
  out['setTimeout string'] = await attempt(function () {
    setTimeout('1', 0);
  });
  out['WebAssembly.compile'] = await attempt(function () {
    return WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
  });
  out['Worker http'] = await attempt(function () {
    var w = new Worker(origin + '/__plugin_probe__/script.js');
    return firstEvent(w, ['error', 'message'], 2000).then(function (r) {
      w.terminate();
      return r;
    });
  });
  out['Worker data'] = await attempt(function () {
    var w = new Worker('data:text/javascript,postMessage(1)');
    return firstEvent(w, ['error', 'message'], 2000).then(function (r) {
      w.terminate();
      return r;
    });
  });
  out['Worker blob'] = await attempt(function () {
    var w = new Worker(URL.createObjectURL(new Blob(['postMessage(1)'], { type: 'text/javascript' })));
    return firstEvent(w, ['error', 'message'], 2000).then(function (r) {
      w.terminate();
      return r;
    });
  });
  out['SharedWorker blob'] = await attempt(function () {
    var w = new SharedWorker(URL.createObjectURL(new Blob(['onconnect=function(e){e.ports[0].postMessage(1)}'], { type: 'text/javascript' })));
    return firstEvent(w.port, ['message', 'messageerror'], 2000).then(function (r) {
      return r;
    });
  });

  // Navigation and the Worker's other end.
  out['location = url'] = await attempt(function () {
    self.location = origin + '/__plugin_probe__/';
  });
  out['location.href = url'] = await attempt(function () {
    self.location.href = origin + '/__plugin_probe__/';
  });
  out['self.postMessage'] = await attempt(function () {
    self.postMessage({ t: 'error', message: 'forged by the plugin' });
  });

  // Storage.
  out['indexedDB.open'] = await attempt(function () {
    var req = indexedDB.open('probe');
    return firstEvent(req, ['success', 'error', 'blocked'], 2000);
  });
  out['caches.open'] = await attempt(function () {
    return caches.open('probe');
  });
  out['navigator.storage.getDirectory'] = await attempt(function () {
    return navigator.storage.getDirectory();
  });
  out['localStorage'] = await attempt(function () {
    return localStorage.getItem('hew.settings.preview');
  });
  out['cookie'] = await attempt(function () {
    return document.cookie;
  });
  out['Notification.requestPermission'] = await attempt(function () {
    return Notification.requestPermission();
  });
  out['clipboard.readText'] = await attempt(function () {
    return navigator.clipboard.readText();
  });
  // The lockdown: what the runtime removed before main, here, in a nested
  // Worker, and in a Worker that one starts, and the ways back tried.
  var REMOVED = [
    'caches', 'CacheStorage', 'Cache', 'indexedDB', 'IDBFactory', 'BroadcastChannel', 'SharedWorker', 'WebSocket',
    'EventSource', 'XMLHttpRequest', 'fetch', 'importScripts', 'WebTransport', 'Notification', 'PushManager',
    'ServiceWorker', 'LockManager', 'StorageManager', 'FontFace', 'webkitRequestFileSystem',
  ];
  var NAV = ['locks', 'serviceWorker', 'storage', 'sendBeacon'];
  function left() {
    var l = [];
    REMOVED.forEach(function (n) { if (typeof self[n] !== 'undefined') l.push(n); });
    NAV.forEach(function (n) { if (typeof navigator[n] !== 'undefined') l.push('navigator.' + n); });
    return l.join(' ') || '(none)';
  }
  out['lockdown: left in the realm'] = left();
  var nestedSource =
    'var REMOVED=' + JSON.stringify(REMOVED) + ';var NAV=' + JSON.stringify(NAV) + ';' + left.toString() + ';' +
    'onmessage=function(e){var here=left();if(!e.data.deeper){postMessage({here:here});return}' +
    'try{var w=new Worker(URL.createObjectURL(new Blob([e.data.src])));w.onmessage=function(m){postMessage({here:here,deeper:m.data.here})};' +
    'w.onerror=function(x){postMessage({here:here,deeper:"error "+x.message})};w.postMessage({})}catch(err){postMessage({here:here,deeper:"threw "+err.name})}}';
  var nested = await attempt(function () {
    return new Promise(function (resolve) {
      var w = new Worker(URL.createObjectURL(new Blob([nestedSource], { type: 'text/javascript' })));
      w.onmessage = function (m) { w.terminate(); resolve(JSON.stringify(m.data)); };
      w.onerror = function (e) { resolve('error ' + e.message); };
      w.postMessage({ deeper: true, src: nestedSource });
    });
  });
  try {
    var parsed = JSON.parse(nested.replace(/^ok: /, ''));
    out['lockdown: left in a nested Worker'] = parsed.here;
    out['lockdown: left in a nested-nested Worker'] = parsed.deeper;
  } catch (e) {
    out['lockdown: left in a nested Worker'] = nested;
  }
  out['recover: prototype walk'] = await attempt(function () {
    var found = [];
    for (var o = self; o; o = Object.getPrototypeOf(o)) {
      REMOVED.forEach(function (n) { if (Object.prototype.hasOwnProperty.call(o, n) && o[n] !== undefined) found.push(n); });
    }
    return found.join(' ') || '(none)';
  });
  out['recover: self.constructor.prototype.caches'] = await attempt(function () { return typeof self.constructor.prototype.caches; });
  out['recover: Reflect descriptors of caches'] = await attempt(function () {
    var d = [];
    for (var o = self; o; o = Object.getPrototypeOf(o)) {
      var x = Reflect.getOwnPropertyDescriptor(o, 'caches');
      if (x) d.push((x.get ? 'getter' : typeof x.value) + (x.configurable ? '' : ' fixed'));
    }
    return d.join(', ') || '(none)';
  });
  out['recover: navigator descriptors of locks'] = await attempt(function () {
    var d = [];
    for (var o = navigator; o; o = Object.getPrototypeOf(o)) {
      var x = Reflect.getOwnPropertyDescriptor(o, 'locks');
      if (x) d.push(x.get ? 'getter' : typeof x.value);
    }
    return d.join(', ') || '(none)';
  });
  out['recover: delete self.caches'] = await attempt(function () {
    try { delete self.caches; } catch (e) { /* fixed */ }
    return typeof self.caches;
  });
  out['recover: Worker from data:'] = await attempt(function () { new Worker('data:text/javascript,postMessage(1)'); });
  out['recover: module Worker'] = await attempt(function () {
    new Worker(URL.createObjectURL(new Blob(['postMessage(1)'], { type: 'text/javascript' })), { type: 'module' });
  });
  out['recover: Worker.prototype.constructor'] = await attempt(function () {
    var w = new SAVED_WORKER(URL.createObjectURL(new Blob([''], { type: 'text/javascript' })));
    var same = Object.getPrototypeOf(w).constructor === SAVED_WORKER;
    w.terminate();
    return same ? 'the wrapper' : 'another';
  });
  out['side door: OffscreenCanvas, createImageBitmap'] = typeof self.OffscreenCanvas + ', ' + typeof self.createImageBitmap;
  return out;
});
// Taken while main runs: the runtime has already replaced it.
var SAVED_WORKER = self.Worker;
hew.commands.handle('ping', function () {
  return { alive: true };
});
