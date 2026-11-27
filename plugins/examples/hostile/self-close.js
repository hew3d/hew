// Closes its own Worker from inside a command, so it never answers or says goodbye.
hew.commands.handle('run', function () {
  setTimeout(function () {
    self.close();
  }, 0);
  return new Promise(function () {});
});
