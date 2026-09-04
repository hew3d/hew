# Docs video fleet — scene authoring guide

One silent, caption-carried clip per Learn chapter, auto-captured from the
real web app. Read this whole file, then the chapter's markdown, then the
two reference scenes before writing anything.

## References (copy their structure exactly)

- `docs-push-pull.mjs` — blank-document scene, geometry built on camera
- `docs-follow-me.mjs` — rail-button tools, multi-stage gestures
- `capture-lib.mjs` — the harness. **READ-ONLY. Never edit it.**

## Pacing rules (viewer-tested on push-pull — match them exactly)

- Modeling-gesture glides run at hand speed: **250–450 ms** (nothing ≥ 500).
- Wait after a tool hotkey: **200–250 ms**.
- After a typed value's Enter, if any keypress follows in the same beat,
  hold **900–1000 ms** first — otherwise the next key stomps the value chip.
- Beat-end result dwells: **1300 ms**.
- Camera *demonstrations* (the viewing chapter's orbit/pan/zoom beats) keep
  their deliberate pace — the too-slow complaint was about modeling
  gestures, not camera moves.
- Build scenes must NOT end on a bare orbit (it drifts the model
  off-frame — `h.orbit` pivots from wherever the cursor sits). Close with:
  Escape → glide to the model's screen center → moderate orbit →
  **Camera ▸ Zoom Extents** via menu glides → 900 ms hold → URL caption.
  Copy the close from `docs-push-pull.mjs` verbatim (there is no Zoom
  Extents hotkey; `z` is the Zoom tool).
- HUD: the default overlay is now caption-above-keys at the bottom
  (`CAPTURE_HUD=bottom`, the default) — captions and key chips read
  together. Don't set CAPTURE_HUD.
- Captures refuse to start when the 1-min load average exceeds 3
  (CAPTURE_MAX_LOAD overrides — throwaway pilots only, never goldens).

## Hard rules

- Scene file: `tools/docs-videos/docs-<slug>.mjs`, one per chapter. Touch ONLY
  your own scene file and your own output dirs.
- Theme: run every capture with `CAPTURE_THEME=dark` (docs media is dark
  by convention). The app must be served: `curl -sf http://localhost:5199/`
  first; if it's down, STOP and report — don't start servers.
- Captions come from the chapter's own prose, compressed to ≤ 90 chars.
  2–6 beats, 35–70 s total. Open with `showMark()` + a title caption;
  close with caption `hew3d.com/learn/<slug>`.
- `h.expectBadge('<n> object', '<beat>')` after EVERY mutating beat — a
  cascade after a silent miss produces garbage that wastes an hour.
  Read-only scenes assert once after load.
- Iterate with `DEBUG_SHOTS=1` + per-beat `shot(n)` screenshots, LOOK at
  them, and correct coordinates by measurement. Captures are
  deterministic: coordinates measured from a previous run stay valid.
  Never guess twice.
- Final run must be clean (no DEBUG_SHOTS) with all badges passing, then
  assemble:
  `python3 assemble-docs.py --meta <out>/capture-meta.json \
     --out ../../site/public/videos/docs-<slug>-dark --h264-crf 22`
  (paths relative to tools/docs-videos/; the output lands in the repo's
  site/public/videos/, which is gitignored — publish-r2.py uploads it).
- Delete nothing outside your output dirs. Do not commit anything.

## Harness facts (hard-won — trust these)

- Tool rail entries are `role=radio`: use `h.clickRail('Follow Me')` /
  `h.clickRail('Polygon')` etc. — never `getByRole('button')` for tools.
  Hotkeys: Space select, L line, R rect, C circle, A arc, P push/pull,
  F offset, M move, Q rotate, S scale, T tape, B paint.
- Menu bar (File Edit Object View Camera Draw Tools Window Help) are real
  buttons at the top: `page.getByRole('button', { name: 'Object' })`,
  then click the menu item by its text.
- Viewport ≈ x 190–1615, y 36–1050. Ground origin ≈ (905, 540). At the
  default centimeter zoom ≈ 22 px/cm on the ground. Right panel starts
  x ≈ 1616 (Object Info name field ≈ (1767, 94)).
- Typed input goes to the active tool: `h.typeSlow('12,8')` +
  `page.keyboard.press('Enter')`. A leading minus pushes inward.
- Keyboard dispatch lags mouse by ~0.2 s — irrelevant here (no metronome).
- Circles: radius in cm; r=2 cm ≈ 45 px — keep circles WELL inside faces.
- `h.iso()` resets the camera (also `Top`/`Front` buttons by role).
  `h.orbit(dx, dy, ms)` = middle-drag. Pan = right-button drag via
  `page.mouse` with `{ button: 'right' }`. Zoom = `page.mouse.wheel`.
- `startCapture({ sample: 'Café Table' })` loads the bundled sample:
  objects `Tabletop`, `Pen Cup`, group `Base`; badge "2 objects ✓ solid";
  document title "Café Table". Textured, ready to film.
- `Escape` exits/steps back any tool. `Cmd-Z` = `page.keyboard.press('Meta+z')`.
- The status bar (bottom) names the active tool + next step — good shot
  material; the solid badge lives bottom-right.

## Voice for captions

Plain, concrete, no exclamation marks. State what the gesture does, not
that it is easy. Reuse the chapter's own phrases where they fit.
