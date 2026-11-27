// Exercises hew.storage: sets, reads back at once, deletes, lists, and pushes against the key and size limits.
hew.commands.handle('run', async function (params) {
  var op = params && params.op;
  var key = params && params.key;
  try {
    if (op === 'set') {
      await hew.storage.set(key, params.value);
      return { done: true };
    }
    if (op === 'get') {
      var v = await hew.storage.get(key);
      return { value: v === undefined ? null : v };
    }
    if (op === 'delete') {
      await hew.storage.delete(key);
      return { done: true };
    }
    if (op === 'list') return { keys: await hew.storage.list() };
    if (op === 'readYourWrites') {
      await hew.storage.set('ryw', 1);
      var first = await hew.storage.get('ryw');
      var p = hew.storage.set('ryw', 2);
      var second = await hew.storage.get('ryw');
      await p;
      await hew.storage.delete('ryw');
      var gone = await hew.storage.get('ryw');
      return { first: first, second: second, gone: gone === undefined ? null : gone };
    }
    if (op === 'fill') {
      // Values of 900,000 characters until the plugin's 5 MiB is used up.
      var stored = 0;
      var refusal = null;
      for (var i = 0; i < (params.limit || 8); i++) {
        try {
          await hew.storage.set('fill' + i, 'x'.repeat(900000));
          stored++;
        } catch (e) {
          refusal = e && e.name;
          break;
        }
      }
      return { stored: stored, refusal: refusal };
    }
    if (op === 'oneValue') {
      // One value of `chars` characters.
      await hew.storage.set('one', 'x'.repeat(params.chars));
      return { done: true };
    }
    if (op === 'keyOfLength') {
      try {
        await hew.storage.set(params.chars === 0 ? '' : (params.char || 'k').repeat(params.chars), 1);
        return { done: true };
      } catch (e) {
        return { threw: (e && e.name) + ': ' + String(e && e.message).slice(0, 100) };
      }
    }
    if (op === 'notJson') {
      try {
        await hew.storage.set('f', function () {});
        return { done: true };
      } catch (e) {
        return { threw: (e && e.name) + ': ' + String(e && e.message).slice(0, 100) };
      }
    }
  } catch (e) {
    // A refusal from the host carries its name.
    return { refused: (e && e.name) || String(e) };
  }
  return { unknown: String(op) };
});
hew.commands.handle('ping', function () {
  return { alive: true };
});
