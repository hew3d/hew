# Hew web E2E (Playwright)

End-to-end tests for the **web build** of the app. Strategy and rationale:
`docs/DEVELOPMENT.md` (the test pyramid) and decision ****. This directory is
the *top* of the pyramid — keep it thin; most interaction-logic regression
belongs in Vitest (`src/**/*.test.ts`) and, once it lands, the headless
replay/semantic layer.

## Layout

- `*.spec.ts` — Playwright tests (run by `pnpm --dir app e2e`).
- `helpers/` — shared test utilities. `*.test.ts` here are **Vitest** unit tests
  for the pure helpers (e.g. the projection math), run by `pnpm --dir app test`.

The two runners are split by file extension so neither loads the other's files:
**Vitest owns `*.test.ts`, Playwright owns `*.spec.ts`** (`vitest.config.ts`
`include` + `playwright.config.ts` `testMatch`).

## Running

```bash
# one-off local run (builds the app, previews it, runs the smoke)
pnpm --dir app e2e

# first time on a machine: fetch browser binaries (+ system deps; needs sudo)
pnpm --dir app e2e:install
# or, binaries only (no sudo): pnpm --dir app exec playwright install chromium webkit
```

In CI the build is produced by an earlier pipeline step and `CI=1` makes the
Playwright `webServer` just `vite preview` the existing `dist` (no rebuild).

## Browsers

- **webkit** — primary. Approximates the macOS Tauri WKWebView, the target CI
  can't otherwise reach (`tauri-driver` has no WKWebView support —).
- **chromium** — the input/render path; headless CI uses the SwiftShader
  software-GL fallback (launch flags in `playwright.config.ts`) so the WebGL2
  viewport context still initializes.

Firefox is omitted from the default set to keep CI lean; add it when a
regression warrants it.

## The canvas problem (docs/DEVELOPMENT.md)

The viewport is an opaque WebGL `<canvas>` — DOM selectors can't "click the
edge." Three strategies, in priority order:

1. **Semantic harness (primary)** — drive `window.__hew_test`
   (`selectEdge`/`pushPull`/`hoverPoint`/…). Deterministic, no pixel math. Most
   tests should use this once it lands.
2. **Pixel interaction (secondary)** — to validate that *dragging a screen pixel
   hits the right geometry*: pin the camera to a fixed matrix, then project a
   known world point to a page pixel and `mouse.move(px, py)` there. The pure
   projection math lives in `helpers/projectWorldToScreen.ts` (`worldToPagePixel`
   + `PINNED_CAMERA`), unit-tested in `helpers/projectWorldToScreen.test.ts`.
   The actual camera-pinning hook is part of the harness; until then a
   pixel test builds the view-projection from `PINNED_CAMERA` itself.
3. **Visual goldens (sparingly)** — `toHaveScreenshot` on a single pinned
   runner (fixed viewport/DPR, software GL, tolerance). GPU variance makes these
   flaky, so they stay rare and runner-pinned.

## Scope today

- `smoke.spec.ts` — proves the web build boots: React mounts, the WASM
  kernel loads, the WebGL2 canvas goes live, no console errors.
- `harness.spec.ts` — drives `window.__hew_test` to prove the semantic
  harness is wired to the live kernel + app reconcile (kernel ops, picking,
  recording, error surfacing).
- `web-smoke.spec.ts` — the canonical lifecycle: launch → draw
  rectangle → push/pull → **save/reload** → screenshot. The modeling +
  persistence logic goes through the harness; the **pixel** channel is used only
  as its own subject (the viewport rendered the solid, and still shows it after
  a save/open round-trip). The strong fidelity guarantee is logical, not visual:
  `state_hash` + object count survive save→reload exactly. Screenshots are
  attached as artifacts but **not** pixel-compared — pinned visual goldens are
   (GPU variance needs a pinned runner).
- `tools.spec.ts` — pixel-free behavior spec per modeling tool, driven
  through the harness's semantic methods (which call the kernel directly).
- `session.spec.ts` — shell/session journeys: save/load fidelity,
  autosave recovery dialog, docked-tray defaults + persistence, unit
  persistence, undo-then-save.
- `ui-chrome.spec.ts` (2026-07) — the DOM chrome the semantic specs bypass:
  tool-rail radio activation, bare-letter shortcuts, the contextual
  dock (verbs, honest active-tool highlight via `aria-pressed`, context
  swap), the unified File ▸ Export… dialog, and the command palette.
- `input-pipeline.spec.ts` (2026-07) — strategy 2 made live, and the only
  place the REAL input path (keyboard → tool switch, canvas pointer →
  raycast/snap, typed VCB → commit) is exercised end to end:
  `harness.setCamera` pins the pose, `helpers/projectWorldToScreen.ts`'s
  `buildViewProjection` + `worldToPagePixel` turn world points into
  `page.mouse` targets. Covers the Rectangle→Push/Pull typed journey, Arc
  typed-bulge entry, Escape cancel, click-selection (SelectTool ray-pick +
  dock context follow), the Delete-key handler, and the Ctrl+Z /
  Ctrl+Shift+Z bindings.
  Keep it small: geometry correctness stays in kernel tests; per-op behavior
  stays in `tools.spec.ts` — only *wiring* belongs here. Gotcha discovered
  while writing it: a chord on a world axis axis-snaps the bulge cursor back
  onto the chord (flat sagitta), so pixel tests draw off-axis.
- `visual/` — the render-regression goldens, run only by the pinned
  `visual` Playwright project (fixed viewport/DPR + SwiftShader). The functional
  projects skip this dir (`testIgnore`). See `e2e/visual/README.md`.
