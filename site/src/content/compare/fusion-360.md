---
title: "Hew for Fusion 360 users"
package: "Fusion 360"
description: "Direct modeling with no timeline, no cloud, and no account — and none of Fusion's assemblies, drawings, or CAM."
order: 3
---

You know Fusion 360: sketch, constrain, extrude, and every feature lands on a parametric timeline you can revisit. Hew has no timeline and no parameters. It's pure direct modeling — you push and pull the solid itself, and undo is the only history. For work where you'd want to change a dimension at step 3 and have steps 4 through 30 regenerate, that's a real loss. For the quick jobs where sketch-constrain-extrude always felt like ceremony — a bracket, a jig, a cabinet — it's the overhead gone.

The other difference is where your work lives. Fusion is an Autodesk account and cloud-resident documents, with a personal-use license whose limits have tightened over the years. Hew is a local application with local files in an open, documented format — free and open source, no account, no tiers, nothing to revoke. Precision survives the simplification: typed exact lengths, inference snapping, guides, dimensions, printed templates at exact scale.

Hew is not a Fusion replacement. There are no assemblies or joints, no drawings module, no CAM, no simulation. If your parts get machined or your designs are constraint-driven, keep Fusion; Hew competes for the modeling you do *around* that. (Much the same story applies coming from SolidWorks or Onshape — swap the account model and the price.)

## Feature comparison

| | Fusion 360 | Hew |
| --- | --- | --- |
| Price & account | Subscription; free personal-use tier with real limits; Autodesk account required | Free, open source, no account |
| Where files live | Autodesk cloud | Your disk, in an open documented format |
| Modeling paradigm | Parametric timeline + constrained sketches (direct editing available) | Direct modeling only — no timeline, no constraints |
| Changing an early decision | Edit the feature, model regenerates | Push/pull the geometry again; undo history only |
| Precision | Constraint solver, typed dimensions | Typed unit-aware lengths, inference snapping, guides, dimensions |
| Solids | Solid by kernel | Solid by construction; watertight status shown live, exports gated on it |
| Platforms | Windows, macOS | Windows, macOS, Linux, full app in a browser |
| Export | STEP and many CAD formats | STL, 3MF, glTF, USDZ, SVG — no STEP |
| Assemblies, joints, drawings, CAM, simulation | Core Fusion | None |

The table stops at that last row deliberately: everything below it is Fusion-only territory, and listing it line by line would just repeat "Hew doesn't."
