# Visual-regression goldens

The **render** regression net — the only place we diff pixels against a committed
golden PNG. Everything else in `e2e/` asserts *logic* (state hashes, object
counts) via the harness; this catches what logic can't see: shading,
tessellation, and camera drift in the WebGL viewport.

Kept deliberately tiny (docs/DEVELOPMENT.md, strategy 3 — "visual goldens, sparingly").
Most regressions belong one layer down, in the harness specs.

## How it's pinned

Goldens are reproducible because the render is **fully software and version-locked**:

- **Project `visual`** (`playwright.config.ts`) — fixed `800×600` viewport,
  `deviceScaleFactor: 1`, and Chromium forced onto **ANGLE-SwiftShader**
  (`--use-angle=swiftshader`). No real GPU is involved.
- That SwiftShader ships *inside* Playwright's pinned Chromium, so two machines
  on the same `@playwright/test` version rasterize byte-for-byte the same — host
  GPU, drivers, and OS are out of the loop.
- The scene is built by the harness (`drawBox` + `pushPull`) under a fixed
  camera, so geometry and framing are deterministic.
- No masks (since the re-baseline): the pre- floating panels
  the specs used to mask are gone, and the Studio chrome overlapping the
  canvas crop (contextual dock, viewport HUD) is deterministic per scene, so
  it is part of each golden. An intended chrome change therefore *is* a
  golden refresh — that's working as designed.

A small tolerance (`maxDiffPixelRatio: 0.02`, `threshold: 0.2`, animations off)
in the global `expect.toHaveScreenshot` absorbs sub-pixel AA noise.

## Running

```bash
pnpm --dir app e2e:visual          # check against committed goldens
pnpm --dir app e2e:visual:update   # regenerate goldens (see caveat below)
```

Goldens live in `box.spec.ts-snapshots/<name>-visual-linux.png`. The `-linux`
suffix is the runner platform — CI is Linux, so commit the Linux goldens.

## The golden contract (read before `--update`)

Regenerate **only** when a render change is intended (and reviewed by eye), or
when bumping `@playwright/test` to a Chromium build that rasterizes differently.
A golden refresh is a reviewable change to a checked-in PNG — never a reflex to
make CI green.

This lane **blocks** CI, so a diff it flags stops the build until the goldens
are either fixed or refreshed. That is the point of it — and it is what lets
three.js and Playwright automerge at all (`.github/renovate.json5`).

When the diff is not a real regression (most often after a Playwright bump),
regenerate **on the CI runner**, so the committed golden matches what CI
rasterizes. Trigger the **Regen Visual Goldens** workflow
(`.github/workflows/regen-visual-goldens.yml`) from the Actions tab, selecting
the branch that is red. It regenerates on the runner, proves the fresh PNGs
pass against themselves, and uploads them as a `visual-goldens` artifact. It
does **not** commit or push anything: download the artifact, unzip it over
`app/e2e/visual/`, read the diff, and commit it to that branch yourself.
Manual equivalent, on the runner:

```bash
pnpm --dir app exec playwright test --project=visual --update-snapshots
```

(A dev-box regen on the same `@playwright/test` version is normally
byte-identical — SwiftShader is bundled with the pinned Chromium — but the
runner is authoritative when they disagree.)

Playwright writes the actual + diff PNGs into `test-results/` on failure; the
`playwright-report` artifact (uploaded on failure in CI) shows the three-up
expected/actual/diff view.
