// Counts the model changes it is told about, in memory and in its own
// storage, and remembers each one's generation so a repeat shows.
(function () {
  'use strict';

  var received = 0;
  var generations = [];

  hew.events.on('document_changed', function (params) {
    received++;
    generations.push(params && typeof params.generation === 'number' ? params.generation : -1);
    void hew.storage.set('received', received);
  });

  hew.commands.handle('status', async function (params, ctx) {
    var stored = await hew.storage.get('received');
    return { activation: ctx.activation, received: received, generations: generations.slice(), stored: stored === undefined ? null : stored };
  });
})();
