# Hew — Brand

Source of truth: **Hew Brand Sheet v1 · locked** (`brand/Hew-Brand-Sheet.pdf`).
Raw assets (SVG + PNG) live in `brand/`; see `brand/README.md` for the file
inventory. This document is the written guideline — update it in the same
commit as any brand asset change.

## The mark

Hew's mark is an **open isometric cube** — the universal symbol for 3D, drawn
as a clean single-stroke wireframe (a hexagon outline with three spokes to the
centre). It reads instantly at any size and stays unmistakable in a single
colour. **Classic wireframe in Terracotta is the primary lockup.**

Geometry (for regeneration / inlining): a 100-unit viewBox `-50 -50 100 100`,
hexagon points `0,-34 29.44,-17 29.44,17 0,34 -29.44,17 -29.44,-17`, three
spokes from the origin to the top and two upper vertices, `stroke-width` 4.6,
round joins and caps. Thicken the stroke for very small renders (the titlebar
glyph uses 7 at ~15px).

## Colour

| Name       | Hex       | Role                                    |
| ---------- | --------- | --------------------------------------- |
| Terracotta | `#C45D3C` | Primary — the mark, app icon, brand chrome |
| Charcoal   | `#1B1A17` | Dark — dark app-icon tile, PWA/OS chrome |
| Cream      | `#F3EDE3` | Light — light app-icon tile, paper backgrounds |

> **Note on the app UI accent.** The running Studio UI keeps its own accent
> (blue `#5b8cff`) for selection, active tools, focus rings, and kbd chips —
> a deliberate scope boundary: the brand palette governs the mark, icons, and
> OS/PWA chrome, not the in-app themed accent.

## Typography

**Hanken Grotesk** (Google Fonts, open license).

| Use                  | Weight            | Notes           |
| -------------------- | ----------------- | --------------- |
| Wordmark & headlines | ExtraBold 800     | tracking −0.03em |
| Subheads & labels    | Bold 700          |                 |
| UI & emphasis        | SemiBold 600      |                 |
| Body text            | Regular 400       |                 |

The lockup wordmark is set in Hanken Grotesk ExtraBold. Hanken is **not**
bundled into the app UI today — the Studio UI runs on the system font stack
(`--font-family-ui`). `brand/hew-lockup-outlined*.svg` carry the wordmark as
vector outlines so the lockup renders identically without the font installed.

## App icon

Three colourways, rounded-square tile with the wireframe cube:

- **Terracotta · primary** — white cube on a Terracotta tile. This is the
  shipped desktop (Tauri) + web (PWA/favicon) icon.
- **Charcoal · dark** — Terracotta cube on a Charcoal tile.
- **Cream · light** — Terracotta cube on a Cream tile.

Corner radius on the 1024 tile is 224px (~22%), an Apple-style squircle
approximation.

## Where the brand shows up in the repo

| Surface                     | File(s)                                          | Source asset |
| --------------------------- | ------------------------------------------------ | ------------ |
| Desktop app icon (all OSes) | `shells/tauri/src-tauri/icons/*` (generated)     | `brand/png/hew-appicon-1024.png` via `tauri icon` |
| Web favicon (SVG + PNG)     | `app/public/favicon.svg`, `app/public/favicon.png` | `hew-mark-terracotta.svg`, `hew-favicon-32.png` |
| Apple touch icon            | `app/public/apple-touch-icon.png`                | `hew-appicon-180.png` |
| PWA icons (192/512/maskable)| `app/public/pwa-*.png`                           | `hew-appicon-512/1024.png` |
| PWA / OS chrome colour      | `app/vite.config.ts` manifest, `app/index.html`  | Charcoal `#1B1A17` |
| In-app titlebar glyph       | `app/src/TitleBar.tsx`                            | inlined mark, Terracotta stroke |

### Regenerating icons

- **Desktop:** `cd shells/tauri && pnpm exec tauri icon ../../brand/png/hew-appicon-1024.png`
- **Web:** the `app/public/pwa-*.png`, `favicon.png`, and `apple-touch-icon.png`
  are derived from `brand/png/*` (downscales; the maskable variant is the
  appicon composited at 80% onto a full-bleed Terracotta square so the OS mask
  never clips the cube).

## Not yet used

Captured now, wired up when the surfaces exist:

- **Logo lockup** (`brand/hew-lockup*.svg`) — for a README header, docs site,
  splash/about screen. Needs Hanken Grotesk, or use the `-outlined` variants
  offline.
- **Push/pull animation** — listed on the brand sheet as a separate asset;
  not yet produced.
