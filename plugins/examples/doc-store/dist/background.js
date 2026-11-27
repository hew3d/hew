// Per-document storage and the plugin's own storage, side by side, plus a
// record of every document id the plugin has been told about.
(function () {
  'use strict';

  // Different on every start of the realm, so a restart shows.
  var boot = Math.random().toString(36).slice(2);
  var seen = [];

  hew.lifecycle.on('document', function (id) {
    seen.push(id);
  });

  function orNull(v) {
    return v === undefined ? null : v;
  }

  hew.commands.handle('docset', async function (p) {
    await hew.storage.document.set(p.key, p.value);
    return { done: true };
  });
  hew.commands.handle('docget', async function (p) {
    return { value: orNull(await hew.storage.document.get(p.key)) };
  });
  hew.commands.handle('globalset', async function (p) {
    await hew.storage.set(p.key, p.value);
    return { done: true };
  });
  hew.commands.handle('globalget', async function (p) {
    return { value: orNull(await hew.storage.get(p.key)) };
  });
  hew.commands.handle('info', async function () {
    await hew.ready;
    return { documentId: hew.plugin.documentId, document: hew.plugin.document, seen: seen.slice(), keys: await hew.storage.document.list(), boot: boot };
  });
})();
