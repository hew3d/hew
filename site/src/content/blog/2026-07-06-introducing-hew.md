---
title: "Introducing Hew"
description: "Why we're building a solids-first 3D modeler with SketchUp's interaction model — and where things stand today."
pubDate:
draft: false
---

If you've ever used SketchUp, you know the feeling: click a rectangle tool, draw a shape on a face, pull it up, and suddenly you have a 3D object. No menus, no history tree, no parametric constraint solver to fight with. It's the lowest floor of any serious 3D tool, and it's why hobbyists, architects, and hackers alike have kept using it for two decades.

It's also got two problems that never got fixed.

**Accidental welding.** Leave two pieces of geometry ungrouped in SketchUp and they fuse together the moment they touch — permanently, and usually without you noticing until much later. Untangling a "solid" that turns out to be a dozen surfaces welded into one lump is one of the more miserable experiences in 3D modeling software.

**Hollow face-soup models.** SketchUp doesn't really have solids. It has faces, floating in a scene, that happen to enclose a volume if you've been careful. Delete one face, flip a normal, leave a sliver gap — and your "solid" quietly becomes an open shell. You won't find out until a 3D print fails halfway through, or a CAD import throws an error you can't trace back to its cause.

Hew is our answer to both. The interaction model is deliberately SketchUp-flavored — draw on faces, push and pull into volumes, inference snapping that finds the endpoint, midpoint, or edge you're reaching for. But underneath, the data model is solids-first, in the spirit of tools like Shapr3D and Plasticity: extruding a closed profile creates a real, discrete, watertight Object automatically. Objects never weld to their neighbors by accident — combining two of them into one is always a deliberate command you choose, never a side effect of geometry touching in space. And an Object's watertightness is tracked and visible, not assumed: an operation that would open a shell is prevented or clearly flagged, never silently patched over.

Hew is open source, and its native file format is documented from day one — a zip container of JSON and binary geometry buffers, not a black box. The modeling kernel is written in Rust and compiled to WebAssembly, which means it runs identically in a browser tab and inside a native desktop shell. It can also read SketchUp's own `.skp` and COLLADA `.dae` files, so existing models aren't stranded.

**Where things stand today:** Hew is pre-release. The web app at [app.hew3d.com](https://app.hew3d.com) is usable now — you can draw, push/pull, orbit and pan and zoom, import `.skp`/`.dae` models, and export a guaranteed-manifold STL for 3D printing. It's rough around the edges in places, and we're actively filling gaps.

**What's next:** native desktop installers for macOS, Windows, and Linux (the web app already runs the same kernel, so these are packaging work rather than a rewrite), a deeper library of learn content, and the usual long tail of modeling tools that any serious 3D app needs. We'll be posting progress here as it lands.

If you want to see where the interaction model and the data model meet, the fastest way is to just open the app and draw something. It only takes a rectangle and a push/pull to feel the difference.
