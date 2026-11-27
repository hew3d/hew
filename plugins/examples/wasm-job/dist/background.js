// A real, single-threaded WebAssembly job run in nested Workers.
//
// The module below is hand-assembled (there is no build step): `count(start,
// stop, step)` counts the primes among start, start + step, ... below stop by
// trial division, so a call is CPU-bound for as long as its range is long.
// A job is cut into chunks of 20,000 numbers and slice k of N takes every
// Nth chunk, so every slice costs about the same. The plugin compiles the
// module ONCE, then starts one nested Worker per slice from a blob and posts
// the compiled module to each. No bytes are ever sent to a
// nested Worker, so none of them can have compiled anything itself; each
// reports what it saw so the caller can check that.
(function () {
  'use strict';

  var BYTES = new Uint8Array([
    0, 97, 115, 109, 1, 0, 0, 0, 1, 8, 1, 96, 3, 127, 127, 127, 1, 127, 3, 2, 1, 0, 7, 9, 1, 5, 99, 111, 117, 110, 116, 0, 0, 10, 95, 1, 93, 1,
    4, 127, 32, 0, 33, 3, 2, 64, 3, 64, 32, 3, 32, 1, 79, 13, 1, 32, 3, 65, 2, 79, 33, 6, 65, 2, 33, 4, 2, 64, 3, 64, 32, 6, 69, 13, 1, 32, 4,
    32, 4, 108, 32, 3, 75, 13, 1, 32, 3, 32, 4, 112, 69, 4, 64, 65, 0, 33, 6, 11, 32, 4, 65, 1, 106, 33, 4, 12, 0, 11, 11, 32, 5, 32, 6, 106,
    33, 5, 32, 3, 32, 2, 106, 33, 3, 12, 0, 11, 11, 32, 5, 11,
  ]);

  // The nested Worker: counts how often it is asked to compile (never), checks
  // what it was sent is a compiled module, and runs one slice.
  var CHILD = [
    'var compiles = 0;',
    'var wrap = function (o, k) { var f = o[k]; if (typeof f === "function") o[k] = function () { compiles++; return f.apply(this, arguments); }; };',
    'wrap(WebAssembly, "compile"); wrap(WebAssembly, "compileStreaming");',
    'onmessage = function (e) {',
    '  var d = e.data;',
    '  var isModule = d.module instanceof WebAssembly.Module;',
    '  var exportsNamed = isModule ? WebAssembly.Module.exports(d.module).map(function (x) { return x.name; }) : [];',
    '  var t0 = performance.now();',
    '  var i = new WebAssembly.Instance(d.module);',
    '  var n = 0;',
    '  for (var from = d.index * d.chunk; from < d.stop; from += d.chunk * d.workers) n += i.exports.count(from, Math.min(from + d.chunk, d.stop), 1);',
    '  postMessage({ index: d.index, primes: n, ms: performance.now() - t0, isModule: isModule, exportsNamed: exportsNamed,',
    '    compiles: compiles, receivedKeys: Object.keys(d).sort(), isolated: self.crossOriginIsolated });',
    '};',
  ].join('\n');

  var compiles = 0;
  var cached = null;
  function compiled() {
    if (cached === null) {
      compiles++;
      cached = WebAssembly.compile(BYTES);
    }
    return cached;
  }

  function slice(module, url, index, workers, limit) {
    return new Promise(function (resolve, reject) {
      var w = new Worker(url);
      w.onmessage = function (e) {
        w.terminate();
        resolve(e.data);
      };
      w.onerror = function (e) {
        w.terminate();
        reject(new Error('nested worker failed: ' + e.message));
      };
      // The module is the only thing about the code that is sent.
      w.postMessage({ module: module, index: index, workers: workers, chunk: 20000, stop: limit });
    });
  }

  function clamp(value, min, max, fallback) {
    if (typeof value !== 'number' || !isFinite(value)) return fallback;
    return Math.min(max, Math.max(min, Math.floor(value)));
  }

  hew.commands.handle('run', async function (params) {
    // The manifest's schema bounds both; the code holds to the same bounds,
    // so a copy of this example without the schema is no less safe.
    var limit = clamp(params && params.limit, 1, 100000000, 4000000);
    var workers = clamp(params && params.workers, 1, 16, Math.min(8, navigator.hardwareConcurrency || 1));
    var t0 = performance.now();
    var module = await compiled();
    var url = URL.createObjectURL(new Blob([CHILD], { type: 'text/javascript' }));
    var all = [];
    // `only` runs a single slice, for timing one slice alone.
    for (var i = 0; i < workers; i++) {
      if (params && typeof params.only === 'number' && params.only !== i) continue;
      all.push(slice(module, url, i, workers, limit));
    }
    var slices = await Promise.all(all);
    URL.revokeObjectURL(url);
    var total = 0;
    for (var k = 0; k < slices.length; k++) total += slices[k].primes;
    return { limit: limit, workers: workers, primes: total, slices: slices, compiles: compiles, wallMs: performance.now() - t0, bytes: BYTES.length };
  });

  hew.commands.handle('ping', function () {
    return { alive: true, at: performance.now() };
  });

  hew.commands.handle('iso', async function () {
    var url = URL.createObjectURL(new Blob(['postMessage({ isolated: self.crossOriginIsolated, sab: typeof SharedArrayBuffer });'], { type: 'text/javascript' }));
    var nested = await new Promise(function (resolve, reject) {
      var w = new Worker(url);
      w.onmessage = function (e) {
        w.terminate();
        resolve(e.data);
      };
      w.onerror = function (e) {
        reject(new Error('nested worker failed: ' + e.message));
      };
    });
    URL.revokeObjectURL(url);
    return { isolated: self.crossOriginIsolated, nested: nested.isolated, sharedArrayBuffer: typeof SharedArrayBuffer, cores: navigator.hardwareConcurrency };
  });
})();
