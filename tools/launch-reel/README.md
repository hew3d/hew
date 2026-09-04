# Launch reel

The wordless, fast-cut clip on the hew3d.com landing page, and the animated
excerpt at the top of the README. Every shot is a real capture of the web
app: dark theme, cursor visible, one capability per cut — chrome hidden for
the modeling and hero shots, revealed for the tour of the app itself and
the Scenes, Outliner, and Tags panels.

- `reel-lib.mjs` — the capture harness (a chrome-free sibling of
  `../docs-videos/capture-lib.mjs`)
- `SHOTS.md` — the shot authoring rules; read it before touching a shot
- `shots/*.mjs` — one script per shot, each its own capture into `out/<shot>/`
- `reel.json` — the cut list: shot order, the marks that bound each cut,
  offsets, speed, and which shots make up the README excerpt
- `assemble-reel.py` — cuts and concatenates the shots into
  `site/public/videos/reel.{webm,mp4,jpg}` and `reel-readme.{gif,webp}`

## Rendering

macOS only: the harness hands rendering to the GPU through ANGLE's Metal
backend and the driver reads the load average with `sysctl`.

1. Serve the app on the capture port from a checkout of the app you want
   filmed: `pnpm --dir app exec vite --port 5199 --strictPort`
2. Take every shot on a quiet machine: `./take-all.sh` (or `./take-all.sh
   subtract lathe` for a few). The harness refuses to start when the
   1-minute load average is above its gate, because a busy CPU stretches
   every gesture into the recording, and a capture's own browser and
   encoder push the load past the gate for the take that follows — so the
   driver waits for the load to settle before each take. Its gate defaults
   to 4 (the app's dev server alone holds a quiet machine near 3); the
   harness's bare default of 3 assumes the app is served elsewhere.
3. `python3 assemble-reel.py` — prints each cut and the total; review
   `site/public/videos/reel.mp4`. `--only <shot>` renders one cut for review.
4. Preview the landing page against the local render:
   `HEW_REEL_LOCAL=1 pnpm --dir site dev`
5. Publish (uploads the content-hashed files and pins them in
   `site/src/data/videos.json`): `python3 ../docs-videos/publish-r2.py`.
   Commit the manifest. The publisher prints the README excerpt URL when
   `README.md` does not reference it yet.

Iterating on one shot: `DEBUG_SHOTS=1 node shots/<shot>.mjs` writes a still
per `h.shot()` call into `out/<shot>/`; look at them and correct coordinates
by measurement. `CAPTURE_MAX_LOAD=20` overrides the load gate for layout
runs only — never for the take that ships.

## Models the hero shots need

The hero orbits, the section cut, and the scenes tour drop files from
`~/Documents/Hew/` onto the app (`h.loadHew`), so those files must exist
there, by name: `theater-test-5.hew`, `desk-set.hew` (the Getting Started
tutorial's result), `Guest House - 2021-07-10.hew`, and `Trestle Table.hew`
(with its three scenes: Assembled Table, Cut List, Vertical Section). The
modeling shots build their geometry from a blank document and need
nothing; `inference.mjs` uses the bundled Café Table sample.
