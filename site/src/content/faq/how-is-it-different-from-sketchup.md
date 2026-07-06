---
question: "How is Hew different from SketchUp?"
order: 3
---

The interaction model is intentionally close to SketchUp's — the same push/pull workflow, the same reliance on inference snapping instead of typed constraints. The difference is underneath: SketchUp models are a soup of individual faces that weld together the moment ungrouped geometry touches, and can silently become hollow if a single face is deleted or flipped. Hew's Objects are watertight solids by construction, never weld to each other by accident, and only ever combine when you explicitly ask them to. You get the same low floor and fast feel, without the two failure modes that make large SketchUp models fragile.
