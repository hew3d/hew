// Fails a command in each way a handler can: throws, refuses, returns what cannot be sent, answers too big or never.
hew.commands.handle('run', function (params) {
  var how = (params && params.how) || (HOSTILE && HOSTILE.how);
  if (how === 'throw') throw new Error('on purpose');
  if (how === 'reject') return Promise.reject(new Error('rejected on purpose'));
  if (how === 'refuse') throw new HewRefusal('not_today', 'This plugin declines.', { why: 'test' });
  if (how === 'bigint') return { n: 10n };
  if (how === 'cycle') {
    var o = {};
    o.self = o;
    return o;
  }
  if (how === 'function') return function () {};
  if (how === 'undefined') return undefined;
  if (how === 'huge') return { text: 'x'.repeat(2 << 20) };
  if (how === 'never') return new Promise(function () {});
  if (how === 'ok') return { fine: true };
  return { how: String(how) };
});
