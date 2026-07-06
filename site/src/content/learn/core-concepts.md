---
title: "Core concepts"
description: "Objects, watertightness, explicit combining, sticky geometry, and inference snapping — the ideas underneath Hew's SketchUp-like feel."
order: 2
---

Hew's interaction model looks a lot like SketchUp's on the surface. Underneath, a few deliberate rules make it behave very differently once your model gets complicated. Understanding these five ideas will save you more time than memorizing any tool.

## Objects

An Object is Hew's unit of "a solid thing." Extruding a closed 2D profile creates one automatically — you never have to remember a "group it" step. Objects can be nested and instanced (think components), but each one is its own island of geometry with its own watertightness state.

## Watertightness is tracked, not assumed

Every Object knows whether it's a closed, manifold solid. If an operation would open up a shell — deleting a face that leaves a gap, for instance — Hew either prevents it or clearly flags the Object as non-solid. Nothing gets silently patched or "healed" behind your back. What you see is what you'll get when you export.

## Combining Objects is always explicit

Two Objects sitting next to each other, even touching, stay two separate Objects. They do not fuse on contact the way ungrouped SketchUp geometry does. When you actually want two Objects to become one, you reach for an explicit union or merge command. This single rule is what prevents the "accidentally welded my whole model together" problem that haunts long SketchUp projects.

## Sticky geometry, inside an Object

Within a single Object's editing context, familiar SketchUp-style rules still apply: drawing an edge across a face splits it, a closed loop of coplanar edges automatically becomes a new face, and push/pull works on any face region you select. This stickiness is scoped to the Object you're currently editing — it's what makes detail work (like carving a window into a wall) feel exactly like SketchUp, without leaking across Object boundaries.

## Inference snapping

As you draw, Hew's inference engine is constantly looking for meaningful relationships — endpoints, midpoints, points on an edge or face, axis alignment, parallel and perpendicular relationships — across the whole visible scene, and calling them out so you can snap to them. Inference is a query over everything you can see; sticky merging is scoped to the Object you're editing. Keeping those two concepts separate is what lets Hew give you precise, whole-scene snapping without reintroducing accidental welding.
