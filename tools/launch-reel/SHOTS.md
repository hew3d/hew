# Launch reel — shot authoring guide

The reel is the wordless, fast-cut clip on the hew3d.com landing page (and,
as a short animated excerpt, the README). Its model is the hero clip on
plasticity.xyz: about three seconds per shot, hard cuts, the viewport filling
the whole frame, no UI, no captions, the cursor visible, every shot one
capability shown mid-action. Read this whole file, then `reel-lib.mjs`, then
`shots/carve.mjs` (a modeling shot), `shots/hero-desk-set.mjs` (a hero
orbit) and `shots/ui-tour.mjs` (chrome revealed mid-shot) before writing a
shot.

## How a shot works

- One script per shot in `shots/<name>.mjs`; it records its own video into
  `out/<name>/` (gitignored). Run it from this directory:
  `DEBUG_SHOTS=1 CAPTURE_MAX_LOAD=20 node shots/<name>.mjs`
- The app must already be served at http://localhost:5199/ (check with
  `curl -sf http://localhost:5199/`). If it is down, STOP and report — do not
  start servers.
- `startCapture({ out: outDir('<name>') })` opens a blank document in dark
  theme with the chrome hidden; `chrome: true` keeps the UI visible, and
  `h.showChrome()` / `h.hideChrome()` flip it mid-shot (the viewport shrinks
  around the same camera, so the model reads as "zooming out" into the
  app); `sample: 'Café Table'` opens the bundled sample instead; `await h.loadHew('/abs/path.hew')` drops a model file onto
  the open document (the hero shots use files in `~/Documents/Hew/` — see
  README.md for the list).
- The canvas is the full 1920×1080 frame. Ground origin ≈ (960, 540) at the
  default camera; at the default zoom the ground is ≈ 30 px/cm near the
  origin (measure from a still — never guess twice).
- `h.mark('<name>')` stamps a cut point. Every shot marks `start` right
  before its first visible action and one or more result marks after; the
  harness marks `end` itself in `h.finish()`. `assemble-reel.py` cuts each
  shot between two marks, so mark generously — the cut list can always
  tighten.
- `h.shot('<label>')` saves a still when `DEBUG_SHOTS=1`. Iterate with
  stills: run, LOOK at them, correct coordinates by measurement, run again.
  Captures are deterministic, so measured coordinates stay valid.
- `await h.expectBadge('<n> object', '<beat>')` after every mutating beat on
  a document built from scratch (a fully grouped hero model shows no badge —
  skip it there). A silent miss cascades into garbage.

## Pacing and framing (the whole point)

- Glides at hand speed or faster: 200–350 ms. Typed values: `h.type('20,12')`
  (70 ms/char) then `h.key('Enter', 400)`.
- Result dwell 600–900 ms — enough for the cut to land, never longer.
- Frame tight. The subject should fill 50–80% of the frame: `h.zoom(-n)` at
  the cursor position zooms toward the cursor, so glide to the subject first.
  Camera moves (orbit) are part of the shot: 2.5–3.5 s, moderate arc
  (`h.orbit(260, 40, 3000)`); glide to the model's screen center before an
  orbit — it pivots from wherever the cursor sits.
- End on the result, cursor parked off the geometry (an inference label
  stuck on a face reads as clutter).
- Hero shots (a loaded model) call `await h.clean()` first (axes and grid
  off). Modeling shots keep the grid; it reads as a drawing surface.

## Working the UI on camera

With the chrome visible, click real UI with the real cursor:
`h.clickUi(page.getByRole('radio', { name: 'Circle' }))` for a rail tool,
`h.clickUi(page.getByTestId('scene-row').nth(1))` for a Scenes row,
`h.clickUi(page.getByText('Object 1', { exact: true }))` for an Outliner
row. The viewport is then x 191–1615, y 34–1050; re-measure coordinates
from a still after `showChrome()`. A refusal toast in the viewport (a
circle drawn over a face edge, say) is a real refusal — fix the gesture.

## Hidden chrome — what still works

- Tool hotkeys: Space select, L line, R rect, C circle, A arc, P push/pull,
  F offset, M move, Q rotate, S scale, T tape, B paint. `Escape` exits.
  `Meta+z` undoes. `ArrowRight/Left/Up` lock the red/green/blue axis.
- Anything without a hotkey goes through the hidden menu bar:
  `h.menu('Object', 'Subtract')`, `h.menu('Tools', 'Follow Me')`,
  `h.menu('Tools', 'Section Plane')`, `h.iso()` (a submenu item:
  `h.menu('Camera', 'Standard Views', 'Iso')`),
  `h.zoomExtents()`, `h.view('Axes', false)`.
- The right-hand panels are hidden too, so the Materials swatches cannot be
  clicked with the mouse. Reach them by DOM click through `page.evaluate`
  and keep the mouse gesture for the paint click itself.
- Dialogs (3D Text, confirmations) are NOT hidden. A shot that needs one is
  not a reel shot — report it instead of filming a dialog.
- Do not edit `reel-lib.mjs`. If the harness needs something, say exactly
  what in your report.

## Deliverable per shot

The script, the still that proves the framing, and a one-line note with the
marks and the seconds between them. Leave the final clean take to the
integrator — draft captures run under load and stretch.
