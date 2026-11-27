// Draws a box, then starts a slow mutating command of its own without waiting
// for it, and answers at once: the inner command is still running when the
// outer one ends.
function box(x, label) {
  return hew.transact(
    [
      { method: 'hew.sketch.draw_rect', as: 'r', params: { plane: { ground: true }, corner_a: [x, 0, 0], corner_b: [x + 1, 1, 0] } },
      { method: 'hew.solid.extrude', as: 'e', params: { region: { $ref: 'r#/region_id' }, distance: 1 } },
    ],
    { label: label },
  );
}

hew.commands.handle('run', async function () {
  await box(60, 'Outer box');
  // Not awaited. Its failure is caught so the plugin itself stays well-behaved.
  hew.transact([{ method: 'com.example.hostile-nested-orphan.slow', params: {} }]).catch(function () {});
  return { fired: true };
});

hew.commands.handle('slow', async function () {
  await box(70, 'Inner box');
  await new Promise(function (r) {
    setTimeout(r, 3000);
  });
  return { slow: true };
});
