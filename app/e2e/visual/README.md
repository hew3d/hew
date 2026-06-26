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
- The floating info panels are **masked** (`data-testid="floating-panel"`): their
  text is incidental to a render golden, so masking keeps a UI-copy change from
  reding the image.

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

If CI ever flags a diff that isn't a real regression (e.g. after a Playwright
bump), regenerate **on the CI runner** — not a dev GPU box — and commit the
result, so the committed golden matches what CI rasterizes:

```bash
pnpm --dir app exec playwright test --project=visual --update-snapshots
```

Playwright writes the actual + diff PNGs into `test-results/` on failure; the
`playwright-report` artifact (uploaded on failure in CI) shows the three-up
expected/actual/diff view.
