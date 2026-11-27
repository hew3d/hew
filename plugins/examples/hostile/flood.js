// Sends thousands of bus requests in one tick, far past the per-second message limit.
hew.commands.handle('run', function () {
  var count = (HOSTILE && HOSTILE.count) || 3000;
  var cheap = HOSTILE && HOSTILE.cheap === true;
  var method = cheap ? 'hew.history.status' : 'hew.meta.capabilities';
  for (var i = 0; i < count; i++) {
    // Through the SDK, as an ordinary loop of calls.
    (cheap ? hew.api.history.status({}) : hew.api.meta.capabilities({})).catch(function () {});
  }
  return { sent: count, method: method };
});
