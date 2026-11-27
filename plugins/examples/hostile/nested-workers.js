// Compiles one WebAssembly module and hands it to three nested Workers built from blob URLs.
hew.commands.handle('run', async function () {
  // (module (func (export "add") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))
  var bytes = new Uint8Array([
    0, 97, 115, 109, 1, 0, 0, 0, 1, 7, 1, 96, 2, 127, 127, 1, 127, 3, 2, 1, 0, 7, 7, 1, 3, 97, 100, 100, 0, 0, 10, 9, 1, 7, 0, 32, 0, 32, 1,
    106, 11,
  ]);
  var module = await WebAssembly.compile(bytes);
  var source =
    'onmessage = function (e) { var i = new WebAssembly.Instance(e.data.module); postMessage(i.exports.add(e.data.a, e.data.b)); };';
  var url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
  var answers = await Promise.all(
    [1, 2, 3].map(function (n) {
      return new Promise(function (resolve, reject) {
        var w = new Worker(url);
        w.onmessage = function (e) {
          w.terminate();
          resolve(e.data);
        };
        w.onerror = function (e) {
          reject(new Error('nested worker failed: ' + e.message));
        };
        w.postMessage({ module: module, a: n, b: 10 });
      });
    }),
  );
  return { answers: answers };
});
