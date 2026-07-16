---
question: "Can it import SketchUp files?"
order: 4
---

Yes. Hew can import SketchUp's native `.skp` format (2017-era files) via a clean-room reader, as well as COLLADA `.dae` files, which every version of SketchUp can export. Imported geometry is healed and classified into watertight Objects where possible, and non-manifold geometry is flagged rather than silently repaired, so you always know what you're starting from.

The SketchUp native import is limited to 2017 for now since that's the last freely available desktop app version and newer SketchUp versions should be able to save-as 2017.
