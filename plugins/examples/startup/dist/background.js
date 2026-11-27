// The startup example's background script: one classic script, run in the
// plugin's Worker after the SDK runtime has defined `hew`.
//
// When Hew starts it, the plugin reads the model once and remembers how
// many objects it saw. `status` reports that the read happened at startup.

(function () {
  'use strict';

  var state = { ranAtStartup: false, objectsAtStart: 0 };

  function countObjects(nodes) {
    var n = 0;
    for (var i = 0; i < nodes.length; i++) {
      var node = nodes[i];
      if (node && node.kind === 'object') n++;
      if (node && Array.isArray(node.members)) n += countObjects(node.members);
    }
    return n;
  }

  var started = hew.ready.then(function () {
    return hew.api.query.scene({}).then(function (scene) {
      state.objectsAtStart = countObjects(scene.tree || []);
      state.ranAtStartup = true;
    });
  });

  hew.commands.handle('status', function (params, ctx) {
    return started.then(function () {
      return {
        activation: ctx.activation || 'unknown',
        ranAtStartup: state.ranAtStartup,
        objectsAtStart: state.objectsAtStart,
      };
    });
  });
})();
