---
question: "Can it open and save SketchUp files?"
order: 4
---

Yes, and for every version of SketchUp, from SketchUp 3 through 2027. Hew opens and saves SketchUp's native `.skp` format directly through a clean-room reader and writer: the current format that every SketchUp since 2021 saves (the free web app included), and the older formats from SketchUp 3 to 8 and 2013 to 2020, including the 2017 one that the last free desktop version wrote and that many models are still in. It also reads COLLADA `.dae` files, which every version of SketchUp can export. Imported geometry is healed and classified into watertight Objects where possible, and non-manifold geometry is flagged rather than silently repaired, so you always know what you're starting from. Model Health lists what isn't solid, and the Make Solid wizard closes what can be closed.

An opened `.skp` saves back in the format it came in, and File ▸ Save As can write the format of SketchUp 2021 and later (2021 through 2027) or any release from SketchUp 3 to 8 and 2013 to 2020. Files from SketchUp 2021 to 2027 share one format, so they open and save back the same way, and an option writes it so that every SketchUp since 2021 can open it.

A file is read by its contents rather than its version number: a file of a release newer than any Hew knows opens with a warning if it can be read at all, and is refused with its version only when it cannot; opening it in any current SketchUp and saving it again produces a file Hew reads.
