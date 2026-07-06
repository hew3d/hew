---
title: "Hew — a solids-first 3D modeler with SketchUp's feel"
description: "Draw on faces, push and pull into solids, and export a watertight model every time. Hew is a free, open-source 3D modeler that keeps SketchUp's interaction model and fixes its two worst failure modes."
hero:
  eyebrow: "Now in early access"
  title: "Model like SketchUp. Export like it's guaranteed to print."
  tagline: "Hew keeps the push/pull workflow and inference snapping that made SketchUp beloved — on a solids-first data model that never lets your model fall apart into a face soup."
  primaryCta:
    label: "Try it in your browser"
    href: "https://app.hew3d.com"
  secondaryCta:
    label: "Download for desktop"
    href: "/download"
features:
  - title: "Solids by default"
    body: "Extrude a closed 2D profile and Hew automatically creates a discrete, watertight Object — no menu to remember, no \"make group\" step. Every shape you pull off a sketch starts life as a real solid."
  - title: "Objects never weld by accident"
    body: "Draw a second box next to the first and they stay two separate Objects, permanently, until you explicitly combine them. No more discovering months later that your whole model is secretly one fused blob."
  - title: "Explicit combine, on purpose"
    body: "Union and merge are deliberate commands you reach for when you actually want two Objects to become one — never an accident of geometry touching in space."
  - title: "Watertightness you can see"
    body: "Objects track whether they're a closed solid. If an operation would open a shell, Hew flags it or blocks it outright — it never silently \"fixes\" your geometry behind your back."
  - title: "SketchUp semantics where they belong"
    body: "Inside a single Object, sticky geometry still works the way you expect: edges split faces, closed coplanar loops become faces, push/pull acts on any face region, all backed by the same inference snapping you already know."
  - title: "Open format, open source"
    body: "Hew's native .hew format is a documented zip of JSON and binary buffers — no lock-in, no black box. The whole modeler, kernel included, is open source."
comparisonTitle: "The two things SketchUp never fixed"
comparisonIntro: "SketchUp's low floor and push/pull feel are genuinely great. Its data model is the part that keeps biting people. Hew keeps the first and replaces the second."
comparison:
  - title: "Accidental welding"
    pain: "In SketchUp, any geometry left outside a group or component sticks to whatever touches it — permanently, and often silently. Untangling a model where everything welded together is close to impossible."
    fix: "In Hew, extruding a profile creates its own Object automatically. Objects are islands: they never weld to their neighbors unless you explicitly union or merge them."
  - title: "Hollow face-soup models"
    pain: "SketchUp models are collections of individual faces. Delete or flip one face by mistake and a \"solid\" silently becomes an open shell — which breaks 3D printing, CAD interop, and any tool downstream that assumes a closed volume."
    fix: "Hew Objects are watertight solids by construction. An operation that would open a shell is prevented or the Object is clearly flagged non-solid — so the STL you export is guaranteed manifold, not a hope."
closingCta:
  title: "Model in the browser today. No install required."
  body: "Hew runs as a static web app — the same Rust kernel compiled to WASM, the same file format, the same tools. Desktop installers for macOS, Windows, and Linux are on the way."
  primaryCta:
    label: "Try it in your browser"
    href: "https://app.hew3d.com"
  secondaryCta:
    label: "See what's coming to desktop"
    href: "/download"
---
