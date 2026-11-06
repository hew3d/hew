---
question: "Can it open SketchUp files?"
order: 4
---

Yes. Hew opens SketchUp's native `.skp` format directly through a clean-room reader: the current 2026 format that every SketchUp in use today saves (the free web app included), and the older formats from SketchUp 3 to 8 and 2013 to 2020, including the 2017 one that the last free desktop version wrote and that many models are still in. It also reads COLLADA `.dae` files, which every version of SketchUp can export. Imported geometry is healed and classified into watertight Objects where possible, and non-manifold geometry is flagged rather than silently repaired, so you always know what you're starting from. Model Health lists what isn't solid, and the Make Solid wizard closes what can be closed.

An opened `.skp` saves back in the format it came in, and File ▸ Save As can write the 2026 format or any release from SketchUp 3 to 8 and 2013 to 2020. Files from SketchUp 2021 to 2025 are read as the 2026 format and saved back in it.

A file is read by its contents rather than its version number: a stream or 2026-format file of a release that is not on the list opens with a warning if it can be read at all, and is refused with its version only when it cannot; opening it in any current SketchUp and saving it again produces a file Hew reads.
