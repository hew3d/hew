// Calls its own command from inside that command over the bus, and again through a transaction.
hew.commands.handle('run', async function (params) {
  var depth = (params && params.depth) || 0;
  if (depth >= 1) return { depth: depth, reached: true };
  var self_ = await RAW.bus('com.example.hostile-reentrant.run', { depth: depth + 1 });
  var other = await RAW.bus('hew.doc.transact', { commands: [{ method: 'com.example.hostile-reentrant.run', params: { depth: 1 } }] });
  return {
    direct: self_.error ? { code: self_.error.code, message: String(self_.error.message).slice(0, 120) } : self_.result.r,
    viaTransact: other.error ? { code: other.error.code, message: String(other.error.message).slice(0, 120) } : 'RESULT',
  };
});
hew.commands.handle('ping', function () {
  return { alive: true };
});
