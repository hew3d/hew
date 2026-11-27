// A plugin with the right grant that draws boxes: each command is one undo step however many calls it makes.
hew.commands.handle('box', async function (params) {
  var n = (params && params.count) || 1;
  var made = [];
  for (var i = 0; i < n; i++) {
    var x = 20 + i * 3;
    var out = await hew.transact(
      [
        { method: 'hew.sketch.draw_rect', as: 'r', params: { plane: { ground: true }, corner_a: [x, 0, 0], corner_b: [x + 1, 1, 0] } },
        { method: 'hew.solid.extrude', as: 'e', params: { region: { $ref: 'r#/region_id' }, distance: 1 } },
      ],
      { label: 'Plugin box ' + (i + 1) },
    );
    made.push(out.results[1].object_id);
  }
  return { made: made };
});

// Says hello again under another name, then draws a box: whose name does the undo entry carry?
hew.commands.handle('spoof', async function () {
  var hello = await RAW.bus('hew.meta.hello', { protocol: 1, client: { name: 'org.hew3d.official', version: '9.9.9' } });
  var out = await hew.transact(
    [
      { method: 'hew.sketch.draw_rect', as: 'r', params: { plane: { ground: true }, corner_a: [40, 0, 0], corner_b: [41, 1, 0] } },
      { method: 'hew.solid.extrude', as: 'e', params: { region: { $ref: 'r#/region_id' }, distance: 1 } },
    ],
    { label: 'Not from Hew' },
  );
  return { hello: hello.error ? hello.error.code : hello.result, made: out.results[1].object_id };
});
