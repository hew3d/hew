// A plugin granted only to read and to change the view, trying every other command it can name, by every route.
hew.commands.handle('run', async function () {
  var methods = (HOSTILE && HOSTILE.methods) || [];
  var direct = {};
  var inside = {};
  for (var i = 0; i < methods.length; i++) {
    var m = methods[i];
    var r = await RAW.bus(m, {});
    direct[m] = r.error ? r.error.code : 'RESULT';
  }
  // Each again as the one command of a transaction.
  for (var j = 0; j < methods.length; j++) {
    var n = methods[j];
    var t = await RAW.bus('hew.doc.transact', { label: 'sneaky', commands: [{ method: n, params: {} }] });
    inside[n] = t.error ? t.error.code : 'RESULT';
  }
  // A rehearsal of a write, and a write next to a read, in one transaction.
  var dry = await RAW.bus('hew.doc.transact', {
    dry_run: true,
    commands: [{ method: 'hew.tag.create', params: { path: ['sneaky'] } }],
  });
  var mixed = await RAW.bus('hew.doc.transact', {
    commands: [
      { method: 'hew.query.scene', params: {} },
      { method: 'hew.tag.create', params: { path: ['sneaky'] } },
    ],
  });
  // Saying hello again, attaching to another document, and a batch of writes.
  var hello = await RAW.bus('hew.meta.hello', { protocol: 1, client: { name: 'com.example.somebody-else', version: '9' } });
  var attach = await RAW.bus('hew.doc.attach', { document: 'doc_2' });
  var batch = await new Promise(function (resolve) {
    var id = 'batch1';
    RAW.post({
      t: 'bus',
      forward: RAW.forward,
      frame: JSON.stringify([
        { jsonrpc: '2.0', id: id, method: 'hew.tag.create', params: { path: ['batched'] } },
        { jsonrpc: '2.0', id: 'batch2', method: 'hew.entity.delete', params: { ids: [] } },
      ]),
    });
    resolve('sent');
  });
  // What is still allowed: reading, and the view.
  var read = await RAW.bus('hew.query.scene', {});
  var view = await RAW.bus('hew.view.zoom_extents', {});
  return {
    direct: direct,
    inside: inside,
    dry: dry.error ? dry.error.code : 'RESULT',
    mixed: mixed.error ? { code: mixed.error.code, data: mixed.error.data === undefined ? null : mixed.error.data } : 'RESULT',
    hello: hello.error ? hello.error.code : 'RESULT',
    helloGrant: hello.error ? null : hello.result.grant,
    attach: attach.error ? attach.error.code : 'RESULT',
    batch: batch,
    read: read.error ? read.error.code : 'RESULT',
    view: view.error ? view.error.code : 'RESULT',
  };
});
