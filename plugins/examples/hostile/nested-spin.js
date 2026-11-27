// Moves a long computation into a nested Worker: the nested Worker computes
// without yielding for `count` ms (10 s by default) while the realm's own
// Worker stays free to answer the host. Not hostile; it shows the way a
// plugin is told to do heavy work.
var RESULT = null;
hew.commands.handle('run', function (params) {
  var ms = (params && params.count) || 10000;
  var source =
    'onmessage = function (e) { var end = Date.now() + e.data; var n = 0; while (Date.now() < end) { n++; } postMessage(n); };';
  var url = URL.createObjectURL(new Blob([source], { type: 'text/javascript' }));
  var w = new Worker(url);
  RESULT = new Promise(function (resolve, reject) {
    w.onmessage = function (e) {
      w.terminate();
      resolve(e.data > 0);
    };
    w.onerror = function (e) {
      reject(new Error('nested worker failed: ' + e.message));
    };
  });
  w.postMessage(ms);
  return { started: true };
});
hew.commands.handle('ping', function () {
  return { alive: true };
});
hew.commands.handle('result', async function () {
  return { computed: await RESULT };
});
