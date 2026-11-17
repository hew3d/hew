// The hello plugin's background script: one classic script, run in the
// plugin's Worker after the SDK runtime has defined `hew`.
//
// `greet` reads the model through the bus, counts its objects, and keeps a
// count of how many times it was asked in the plugin's own storage.

(function () {
  'use strict';

  function countObjects(nodes) {
    var n = 0;
    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];
      if (node && node.kind === 'object') n++;
      if (node && Array.isArray(node.children)) n += countObjects(node.children);
    }
    return n;
  }

  hew.commands.handle('greet', async function (params) {
    var scene = await hew.api.query.scene({});
    var times = ((await hew.storage.get('times')) || 0) + 1;
    await hew.storage.set('times', times);
    var name = params && typeof params.name === 'string' && params.name !== '' ? params.name : 'Hew';
    return { greeting: 'Hello, ' + name + '!', objects: countObjects(scene.tree || []), times: times };
  });
})();
