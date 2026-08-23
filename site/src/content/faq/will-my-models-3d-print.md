---
question: "Can I 3D-print what I model in Hew?"
order: 7
---

Yes. Hew Objects are watertight solids by construction, so when you export an STL or 3MF the mesh is manifold — no gaps, no flipped normals, no repair step in the slicer. An operation that would open up a shell is prevented or clearly flagged, never allowed to pass silently, so you won't discover a broken model halfway through a print.
