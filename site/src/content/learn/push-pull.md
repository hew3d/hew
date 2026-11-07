---
title: "Push/Pull"
description: "One gesture turns a profile into a solid, reshapes a face, or cuts a hole clean through. The heart of modeling in Hew."
order: 6
---

Push/Pull (`P`) is the tool that moves your model between 2D and 3D. It does three jobs with one gesture: extrudes a sketch region into a new solid, reshapes an existing solid by moving one of its faces, and cuts holes straight through.

<docs-video theme="dark" slug="push-pull" label="push pull demonstration"></docs-video>

## The gesture

1. Click a face — either a filled sketch region or a face of an existing solid.
2. Move the cursor — a live ghost preview of the result follows, measured perpendicular to the face. The readout shows the current depth.
3. Click again to commit, or type an exact depth (any length format, e.g. `18mm`) and press `Enter`.

You can also press on the face, drag, and let go: letting go commits the push/pull at that depth, and a distance typed right after still redoes it.

Press `Esc` at any point to cancel. Committing with no real distance is refused ("Move more before committing push/pull").

Type a distance right after that click commits, too: the push/pull you just made redoes at the new distance — a positive number keeps the direction you just pushed or pulled, a negative one flips it — as many times as you like, until you click another face, press `Esc`, or switch tools. A double-click repeats the last distance in one step. One `Undo` afterward removes the whole push/pull, no matter how many times you retyped it.

While dragging, you can rest the cursor on any precise snap point in the scene (an endpoint, midpoint, edge, guide, or guide crossing) and the extrusion depth borrows that point's height. That's how you pull one box exactly up to the level of another.

![A sketch region extruded into a box](/docs/first-box.webp)

## From sketch to solid

Push/pulling a closed sketch region creates a brand-new **Object** — a watertight solid. This is the only "create geometry" step in Hew; there is no separate group-making or solid-making command. The preview ghost is orange when the solid grows, red when it loses material, and blue when a new object appears (every region of a free sketch, pushed either way, and any push with the `Ctrl` toggle on). The outline you drew becomes the solid's base face and leaves the sketch: a sketch is the larval form of a solid, and once it extrudes there is no separate 2D copy lying around. Undo reverses the whole step — the solid disappears and the outline returns, ready to re-extrude. A shape drawn on a face with [Detached](/learn/drawing/#detaching-a-shape) turned on pushes into a new Object, never into the face.

Solids interpenetrate freely in Hew, so a region drawn over a standing solid extrudes into a second solid coinciding with the first — nothing stops you overlapping or stacking solids. Merging two into one is always a separate, explicit step (a boolean union). If you overlapped something by accident, undo, or move one of the solids apart.

Push/pull also turns an imported **flat panel** into a solid. A single flat face from SketchUp (a pane, a sign, a tabletop drawn as one face) isn't solid, but drag it and it becomes a slab with its paint on both sides, the same as **Add thickness…** in [Make Solid](/learn/import-export/#making-imported-shapes-solid). Other imported shapes that aren't solid can't be pushed or pulled until you close them with **Object ▸ Make Solid…**.

## Reshaping a solid

Push/pulling an existing face of a solid moves that face in or out, keeping the object watertight the whole time. Pull the top of a box up to make it taller; push a side inward to make it thinner.

Any plain object's face works directly, no editing step needed. Faces inside a **group or component** are the exception, here as everywhere: double-click in first, then push/pull — the same rule the drawing tools follow.

This works on angled faces too, not just box-like geometry. The face moves straight along its own normal, and where a neighbor meets it at an angle a new side wall grows to connect them — pull the cut face of a sliced wedge outward and the wedge gains a prism of material along its slope; pull a facet of a circle-based solid and it grows a small pad.

Pulling outward always works: you are adding a block of material, so there is no limit. Pushing inward works only as far as the shape physically allows — Hew refuses the moment the moving face would run into the rest of the solid, leaving the object unchanged. A wedge's sloped face, for instance, can't be pushed in at all: there is nowhere for it to go. A curved wall other than a circle or an arc (see below) moves one facet at a time, not as a whole.

## Curved walls: push/pull changes the radius

The wall of an extruded circle or arc is a special case. Pushing or pulling any facet of it acts on the **whole wall**: the radius changes by exactly the distance you drag, using the exact circle the wall remembers. Pull a cylinder's side outward and the whole cylinder gets fatter; push a hole's wall toward its center and the hole shrinks. Caps and attached flat walls follow along.

The same refusal rule applies here: if changing the radius would bend a neighboring face or collapse the wall into its own axis, Hew refuses with an error and leaves the model untouched. A wall that a boolean has cut into (say, a cylinder with a flat slice taken off) still resizes — the cut face slides along with the new radius.

## Recesses, bosses, and through-cuts

Draw directly on a solid's face to split it into regions (see [Drawing](/learn/drawing/)) — or let the Offset tool inset the face's own boundary in one gesture — then push/pull the new region:

- **Pull outward** → a boss or pad growing from the face.
- **Push inward** → a recess or pocket.
- **Push all the way through** → a hole. When the cut passes fully through the solid, Hew removes the swept material entirely. If the cut severs the object into disconnected pieces, each piece becomes its own independent, solid Object.
- **Push past a neighboring face** → a notch. On an L- or P-shaped solid, pushing one face inward past the plane of a face that points the same way (the wider part's end past the narrower part's end) carves the material behind it, the same way a through-cut does, instead of stopping at the plane.
- **Pull past a neighboring face** → the reverse. Pulling the narrower part's end outward past the wider part's end grows it straight through, and pulling a notched face back out to where it started restores the original shape. Pull exactly flush with the neighboring face and the two merge into one. The same goes for a face pulled across a gap into the face looking back at it, such as one arm of a U pulled into the other: the gap closes and the two become one. A face whose corner sits on a lower step (the exposed end of a thick part where a thinner part joins it) grows or recedes the same way, whatever the distance.

A shape drawn half over a face and half past its edge splits at the edge, and each part pushes on its own terms. Push/Pull the part over the face and the solid grows (or is cut, if you push in); Push/Pull the part past the edge and you get a new, separate object. Do them in either order: the second part is still there, waiting, after the first.

Shapes that touch an edge or another shape at a single point push and pull like any other: a circle drawn with its rim on a box's edge pushes into a pocket or through to the far side. A shape the face can't hold — one that leaves a sliver too thin to keep — is still drawn, but can't be pushed. Push/Pull says so and says why, instead of doing nothing. Move the shape clear of the problem and try again.

![A box with a circular hole punched clean through it, the opening visible at the back face](/docs/pushpull-throughcut.webp)

You can type the depth instead of dragging to choose the direction: a plain number grows the boss outward, and a leading minus (`-6mm`) cuts the recess inward.

![A bracket-shaped model with a rectangular notch cut from its upright](/docs/bracket-scene.webp)

## Inside groups and components

Push/pull works while you're editing a group or a component, through-cuts included — draw on a member's face and cut straight through it, the same gesture as at the top level. Inside a component, the edit applies to the *definition*, so every instance updates together; inside a group, the result folds back in as a member when you step out ([Groups and components](/learn/groups-and-components/)).
