// Prints what it is told to, so the pane's console can be read, and throws
// an error nothing catches on request.
(function () {
  'use strict';

  hew.commands.handle('say', function (p) {
    var level = p && ['log', 'info', 'warn', 'error'].indexOf(p.level) >= 0 ? p.level : 'log';
    console[level](String(p && p.text));
    return { done: true };
  });

  hew.commands.handle('crash', function () {
    setTimeout(function () {
      throw new Error('on purpose');
    }, 0);
    return new Promise(function () {});
  });
})();
