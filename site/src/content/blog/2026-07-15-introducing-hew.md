---
title: "Introducing Hew"
description: "Why do we need another 3D modeler?"
pubDate: 2026-07-15
draft: false
---

I'm a long-time SketchUp user, going back to 2008 when it was still owned by Google. It was the first 3D modeler that felt truly intuitive to me - all others had large learning curves just to do the same things that SketchUp offered with simple mechanisms. I was hooked.

If you've ever used SketchUp, you know the feeling: click a rectangle tool, draw a shape on a face, pull it up, and suddenly you have a 3D object. No menus, no history tree, no parametric constraint solver to fight with. It's the lowest floor of any serious 3D tool, and it's why I used it for almost two decades.

It's also got two problems that never got fixed.

**Accidental welding.** Leave two pieces of geometry ungrouped in SketchUp and they fuse together the moment they touch — permanently, and usually without me noticing until much later. Untangling a "solid" that turns out to be a dozen surfaces welded into one lump is one of the more miserable experiences in 3D modeling software. Ouch. This prompted me to form a "muscle memory" of always turning every solid into a Group as fast as possible, just to avoid this!

**Hollow face-soup models.** SketchUp doesn't really have solids. It has mesh faces, floating in a scene, that happen to enclose a volume if I've been careful. But delete one face or accidentally delete the wrong line and it's not solid at all anymore - now it's an open shell that I typically wouldn't find until much later.

I lived with both until 2017, when Trimble (the new owners) stopped development of SketchUp Make at version 2017 and started pushing everybody to the web version... or the *very expensive* Pro versions. The free web app was dramatically stripped down, so I continued to use the now-unsupported SketchUp Make 2017. Until now. I primarily use macOS as my primary desktop and Apple is removing support for Intel-based apps (like SketchUp!) in the very near future. I wouldn't be able to use SketchUp Make 2017 even if I wanted to and I'm not about to pay hundreds of dollars a year for the pro subscription.

The obvious step forward is to switch to Blender or FreeCAD or other similar 3D modelers. My primary issue is that I have scores of SketchUp models that would all be abandoned plus I would have to give up almost two decades of muscle memory and learn a completely new 3D paradigm.

**Introducing Hew:** So I created Hew to scratch my own itch. It deliberately feels just like using SketchUp since I kept the mechanism and tools roughly the same, along with the keyboard shortcuts. I can also import my existing SketchUp models since I baked in import support for `.dae` and `.skp` files (the latter via [OpenSKP](https://github.com/hew3d/openskp), a clean-room SketchUp 2017 file reader). But if I'm going to go to that kind of effort, then I also wanted to fix SketchUp's fundamental flaws and make the data model *solids-first*. All extruded Objects are discrete, watertight solids from the core. No more welding to neighbors by accident or making a hollow shell by removing the wrong line.

Hew is also Open Source. The modeling kernel is written in Rust and compiled to WebAssembly, which means it runs identically in a browser tab and inside a native desktop shell. As such, it's available via a web app at [app.hew3d.com](https://app.hew3d.com) (or you can run it in your own web server) plus there are native desktop versions for macOS, Linux, and Windows.

**My Stance on AI:** Hew would not exist without AI - specifically Anthropic's Claude Opus and Fable. It may well
cause the downfall of mankind, but we'll get some very nice software out of it first

**Where things stand today:** Hew is MVP - Minimally Viable Product. The most core features all exist - you can draw, push/pull, orbit and pan, zoom, group, component, tag, import `.skp`/`.dae` models, and export actual STL for 3D printing. You can use it on the web at [app.hew3d.com](https://app.hew3d.com) or download a native desktop app for whatever OS you are running. No iOS or Android (if you don't count a web-app PWA).

**What's next:** More of everything! Hew cannot yet completely replace SketchUp for me, but it's getting darn close. The very next release will have some usability updates, better component support, and Follow Me and Offset tools (at least).

Give it a try! Open up the [web app](https://app.hew3d.com), draw something, and pull it up to make a solid. It's that easy!