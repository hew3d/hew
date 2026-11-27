// Needs the optional `view` permission for `hidden`. Counts, in the Worker's
// own memory, how many calls this run has answered, so a restart shows, and
// how many times it was told its grant widened.
(function () {
  'use strict';

  var boot = Math.random().toString(36).slice(2);
  var calls = 0;
  var grants = 0;
  var lastGrant = null;

  hew.lifecycle.on('grant', function (grant) {
    grants++;
    lastGrant = grant;
  });

  function refusalOf(e) {
    // A refusal carries its name in `refusal`; anything else is named by its class and code.
    var named = e && e.refusal ? JSON.stringify(e.refusal) : (e && e.name) + ' ' + (e && e.code);
    return named + ': ' + String((e && e.message) || e).slice(0, 120);
  }

  async function read() {
    try {
      await hew.api.view.hidden({});
      return { ok: true };
    } catch (e) {
      return { ok: false, refusal: refusalOf(e) };
    }
  }

  hew.commands.handle('hidden', async function () {
    calls++;
    var r = await read();
    return { ok: r.ok, refusal: r.refusal === undefined ? null : r.refusal, calls: calls, grants: grants, boot: boot };
  });

  hew.commands.handle('ask', async function () {
    calls++;
    var asked = null;
    var refusal = null;
    try {
      asked = await hew.api.permissions.request({ permissions: ['view'], reason: 'To read which parts are hidden.' });
    } catch (e) {
      refusal = refusalOf(e);
    }
    var after = await read();
    return { asked: asked, refusal: refusal, ok: after.ok, after: after.refusal === undefined ? null : after.refusal, calls: calls, grants: grants, boot: boot };
  });

  hew.commands.handle('state', function () {
    return { calls: calls, grants: grants, boot: boot, grant: lastGrant };
  });
})();
