// Allocates until the Worker dies: kind "heap" keeps JS objects, kind "buffer" keeps touched ArrayBuffers.
// With `topLevel` set it allocates as `main` runs, before the plugin has said hello to the host.
var KEEP = [];
function bomb(kind, capMiB) {
  var mib = 0;
  if (kind === 'buffer') {
    while (mib < capMiB) {
      var b = new Uint8Array(64 << 20);
      b.fill(1);
      KEEP.push(b);
      mib += 64;
    }
  } else {
    while (mib < capMiB) {
      var chunk = [];
      for (var i = 0; i < 1 << 20; i++) chunk.push({ a: i, b: 'x' + i });
      KEEP.push(chunk);
      mib += 64;
    }
  }
  return mib;
}
if (HOSTILE && HOSTILE.topLevel) bomb(HOSTILE.kind || 'heap', HOSTILE.capMiB || 6144);
hew.commands.handle('run', function () {
  var kind = (HOSTILE && HOSTILE.kind) || 'heap';
  var capMiB = (HOSTILE && HOSTILE.capMiB) || 6144;
  return { allocatedMiB: bomb(kind, capMiB) };
});
