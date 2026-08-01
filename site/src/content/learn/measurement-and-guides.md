---
title: "Precision, measurement, and guides"
description: "Everything about exact dimensions: display units, the typed-input grammar, the Tape Measure and Protractor, and guides."
order: 10
---

Models made in Hew tend to get manufactured: 3D printed, cut, assembled. So exact dimensions are first-class, and they come from typed input on every tool, from the display units you choose, and from construction guides that extend the snapping system.

## Units

Model geometry is always stored in meters internally; **Settings ▸ Units** controls how lengths are *displayed and interpreted*:

| System | Format | Example |
|---|---|---|
| Metric | Meters | `1.238 m` |
| Metric | Centimeters | `123.75 cm` |
| Metric | Millimeters | `1237.5 mm` |
| Imperial | Architectural | `5' 3-1/8"` |
| Imperial | Fractional inches | `63-1/8"` |
| Imperial | Decimal inches | `63.125"` |

Imperial fractions round to the nearest 1/16". Every readout — tool previews, the Tape Measure, the typed-input echo — follows this setting.

## Typed input, in full

While a tool is mid-gesture, type a value and press `Enter`. The rules:

- **Bare numbers** are read in the current display unit (in imperial modes, bare numbers are inches).
- **Explicit units always win**, in any display mode: `250mm`, `3.5cm`, `1.2m`, `2km`, `6"`, `6in`, `2'`, `2ft`.
- **Feet-inches-fractions** parse whenever `'` or `"` appears: `5'3"`, `5' 3-1/2"`, `3 1/2"`, `5/8"`.
- **Rectangle** takes two comma- or `x`-separated dimensions, mixable: `1cm,100mm`, `2' x 18"`; one value makes a square.
- **Rotate and Protractor** take plain degrees; **Scale** takes a plain positive factor as-typed (not a length in the display unit) — or an explicit length (`50mm`, `8"`) as a target dimension for the axis being dragged.

## Tape Measure (`T`)

The Tape Measure does two jobs, chosen by where your *first* click lands:

**Measure a distance.** Click any point, move, and read the live distance; click again to finish. If the second click lands on empty space, Hew drops a **guide point** there and — when the two points aren't the same — an infinite **guide line** through both of them, the same pair SketchUp drops. If the second click lands on real geometry, you get only the measurement; nothing is created. Hold `⌘`/`Ctrl` through either kind of click to measure without dropping anything, for whenever you only want the readout.

**Drop a parallel guide.** Click on an **edge** — of a solid, a sketch you've drawn, a group member, or a component instance — then move sideways: a guide line parallel to that edge follows at the offset shown in the readout. Click to place it, or type an exact offset and press `Enter`. This is the classic SketchUp workflow for laying out a design before drawing it. A **world axis** works as the source edge too: click anywhere along the red, green, or blue axis (the On Axis cue confirms the pick) and pull sideways. So does an existing guide line — one guide can father a whole ladder of parallels.

Neither job is stuck on the ground plane. Starting from a hovered sketch keeps the guide or measurement in that sketch's plane, the same as the draw tools; with nothing hovered, press an arrow key while the tool is idle to lock the next click to a world-axis plane (`→`/`←`/`↑` for red/green/blue, `↓` or the same arrow again to release) — useful for measuring across, or guiding off, a face that isn't the ground and has nothing else to hover first. For a parallel guide whose source edge doesn't lie flat in whatever plane applies, Hew derives the nearest plane that *does* contain the edge, rather than dropping the constraint outright. Absent a hovered sketch or plane lock, clicking an edge that also sits on a face offers that face's own plane instead — but only for as long as the cursor stays over that same face; drag off it and the guide falls back to the plain sideways offset, rather than staying locked to whichever face happened to be under your very first click. And once a plane is locked (from any of these sources), a guide or measurement can still reach a point that sits slightly off it — an endpoint or midpoint that doesn't lie exactly in the plane — instead of refusing it: a measurement projects that point onto the plane, and a parallel guide keeps only the point's component along the guide's own direction; the inference chip marks the snap "projected" either way, so you know it isn't landing on the point's literal position.

Arrow keys also lock the guide or measurement's own direction, mid-gesture. For a parallel guide, `→`/`←`/`↑` locks the offset to red/green/blue (`↓` releases); an axis that runs along the source edge itself is refused, with a status-bar note explaining why, rather than silently doing nothing. For a plain measurement, every axis is fair game, since there's no edge to run along. `Shift` does the same job without committing to it: held mid-gesture, it latches onto whichever axis the guide or measurement is *currently* running closest to, and lets go again the moment you release it — an arrow-key lock, once set, survives a Shift tap the same way. Whichever axis is active, the measurement preview colors to match it — and for a parallel guide, so does the connector line back to the source edge, though the guide line itself stays neutral, since it always runs parallel to the edge rather than to the locked axis. Either way, the lock reads at a glance instead of off the numbers.

The corner widget keeps showing your last reading after you finish a measurement or place a guide — it doesn't clear until you switch to a different tool, so there's time to actually read the number instead of catching it out of the corner of your eye. And you don't have to type the target length while the gesture is still live: measure two real points normally, look at the finished distance sitting in the widget, and *then* type a new length and press `Enter` — Hew offers the same "resize the model?" confirmation this produces mid-drag, just triggered after the fact instead of during. `Esc` at that point drops the typed number and puts the original reading back, without disturbing anything; typing again still offers to resize against the same two points.

## Protractor

The Protractor (Tools ▸ Protractor) measures an angle and drops an **angular guide line** through a point:

1. Position the on-screen disk. It lies on the face under your cursor, or the ground plane. Hold `Shift` to lock the current plane, or press an arrow key to force the plane's axis (`→` X, `←` Y, `↑`/`↓` Z).
2. Click the **apex** (where the angle's corner sits).
3. Click along a **baseline** (the zero direction).
4. Sweep to the angle (it snaps near the axes) and click, or type degrees and press `Enter`.

The result is a guide line through the apex at that angle.

## Working with guides

![A box with construction guide lines and a guide point placed around it](/docs/guides.png)

Guides are construction geometry: dashed lines and point markers that are never part of your solids, never export, but always snap. Use them to pre-plan positions, then draw to them.

- The purple "On Guide" snap cue appears whenever the cursor is on a guide.
- Where a guide **crosses** an edge, a sketch line, or another guide, the amber Intersection cue appears — click there to land exactly on the crossing, which is usually the whole reason the guide exists.
- **View ▸ Guides** hides or shows all guides at once; hidden guides don't snap.
- To delete one guide, select it with the Select tool and press `Delete`. **Edit ▸ Delete Guide Lines** clears every guide in one undoable step.
- Guides are saved in the `.hew` file, and guides in imported SketchUp files come across.

## Reading precision from the model

For a quick sanity check without any tool gymnastics: select an object and read Object Info, or use the Tape Measure between two snap points. Endpoint-to-endpoint measurements come from the model's exact geometry.
