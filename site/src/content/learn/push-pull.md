---
title: "Push/Pull"
description: "One gesture turns a profile into a solid, reshapes a face, or cuts a hole clean through. The heart of modeling in Hew."
order: 6
---

Push/Pull (`P`) is the tool that moves your model between 2D and 3D. It does three jobs with one gesture: extrudes a sketch region into a new solid, reshapes an existing solid by moving one of its faces, and cuts holes straight through.

## The gesture

1. Click a face — either a filled sketch region or a face of an existing solid.
2. Move the cursor — a live ghost preview of the result follows, measured perpendicular to the face. The readout shows the current depth.
3. Click again to commit, or type an exact depth (any length format, e.g. `18mm`) and press `Enter`.

Press `Esc` at any point to cancel. Committing with no real distance is refused ("Move more before committing push/pull").

While dragging, you can rest the cursor on any precise snap point in the scene (an endpoint, midpoint, edge, guide, or guide crossing) and the extrusion depth borrows that point's height. That's how you pull one box exactly up to the level of another.

![A sketch region extruded into a box](/docs/first-box.png)

## From sketch to solid

Push/pulling a closed sketch region creates a brand-new **Object** — a watertight solid. This is the only "create geometry" step in Hew; there is no separate group-making or solid-making command. The sketch region is consumed by the extrusion.

## Reshaping a solid

Push/pulling an existing face of a solid moves that face in or out, keeping the object watertight the whole time. Pull the top of a box up to make it taller; push a side inward to make it thinner.

Hew currently extrudes faces whose neighboring faces are perpendicular to them (the common case for box-like geometry). If a push/pull isn't possible on a particular face, Hew refuses with an error rather than producing questionable geometry. Support for arbitrary planar faces is on the roadmap.

## Recesses, bosses, and through-cuts

Draw directly on a solid's face to split it into regions (see [Drawing](/learn/drawing/)), then push/pull the new region:

- **Pull outward** → a boss or pad growing from the face.
- **Push inward** → a recess or pocket.
- **Push all the way through** → a hole. When the cut passes fully through the solid, Hew removes the swept material entirely. If the cut severs the object into disconnected pieces, each piece becomes its own independent, solid Object.

![A bracket-shaped model with a rectangular notch cut from its upright](/docs/bracket-scene.png)

## Inside components

Push/pull works inside a component's editing context too. The edit applies to the component *definition*, so every instance updates together ([Groups and components](/learn/groups-and-components/)).
