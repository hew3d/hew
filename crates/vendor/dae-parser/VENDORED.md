# Vendored: dae-parser 0.11.0

Source: https://github.com/digama0/dae-parser (crates.io `dae-parser` v0.11.0),
licensed MIT OR Apache-2.0 (see `LICENSE-MIT` / `LICENSE-APACHE`).

Why vendored:  / docs/DEVELOPMENT.md specify a **vendored, version-pinned**
COLLADA parser so Hew can own fixes against a frozen format. The crates.io
release ships a bug that makes it unusable for real SketchUp exports, so the
fix has to live in our tree.

## Local patches (Hew)

- `src/core/geom.rs` — `impl ParseGeom for PolygonGeom`: `NAME` was `"polygon"`
  (singular), which never matches the real `<polygons>` element. `Primitive::parse`
  therefore returned `None` for every `<polygons>`, and the unconsumed element
  tripped the `<extra>` parser with `unexpected element polygons`, aborting the
  whole document. Fixed to `"polygons"`. SketchUp emits `<polygons>` for any face
  with holes, so this is load-bearing for real models. Marked inline with
  `// HEW PATCH`.

## Manifest changes

- Dropped the optional `nalgebra` dependency (unused; Hew has its own math —
  ).
