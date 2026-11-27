// Writes a hundred thousand console lines, some of them enormous, in one tick.
hew.commands.handle('run', function () {
  for (var i = 0; i < 100000; i++) console.log('line ' + i, { i: i });
  console.error('x'.repeat(5 << 20));
  return { done: true };
});
hew.commands.handle('ping', function () {
  return { alive: true };
});
