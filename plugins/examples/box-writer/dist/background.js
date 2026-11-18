// Draws one box. A refusal is caught and reported, not thrown: a handler that
// fails with anything but a refusal of its own is stopped.
(function () {
  'use strict';

  hew.commands.handle('box', async function () {
    try {
      await hew.transact(
        [
          { method: 'hew.sketch.draw_rect', as: 'r', params: { plane: { ground: true }, corner_a: [30, 0, 0], corner_b: [31, 1, 0] } },
          { method: 'hew.solid.extrude', as: 'e', params: { region: { $ref: 'r#/region_id' }, distance: 1 } },
        ],
        { label: 'Box from a plugin' },
      );
      return { ok: true, code: null, message: null, refusal: null };
    } catch (e) {
      return {
        ok: false,
        code: e && typeof e.code === 'number' ? e.code : null,
        message: String((e && e.message) || e).slice(0, 200),
        refusal: e && e.refusal ? JSON.stringify(e.refusal).slice(0, 300) : null,
      };
    }
  });
})();
