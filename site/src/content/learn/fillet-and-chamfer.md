---
title: "Fillet and Chamfer"
description: "Round or bevel the edges of a solid, pick router-style edge profiles, and edit a fillet after the fact."
order: 7.1
preview: [fillet]
---

Fillet rounds an edge. Chamfer cuts it flat. Both work on the edges of a solid, on the rim of a face, and on the corners of a line drawing, and both stay editable after you make them: you can come back later, change the size, or take the rounding off.

Press `⇧F` for Fillet, or click Fillet in the tool rail. With a solid selected, the dock at the bottom of the window has a Fillet button too, and it picks every sharp edge of the solid at once. Chamfer is in the **Tools** menu next to Fillet, and it's the same tool with the flat cut chosen. Tap `Alt` at any time to switch between the two.

## Picking what to round

Hover an edge and it lights up. Click it to start.

- **Click an edge.** If the edge runs smoothly into others (the straight sides of a rounded rectangle, or the facets of a circle's rim), the whole run lights up together, because it's one edge as far as you're concerned. Hold `Ctrl` (`Cmd` on macOS) while you hover to pick only the piece under the cursor.
- **Click a face.** Every sharp edge around the face's outline is picked at once, which is how you round the whole top of a table in one go. A hole in a face has its own rim; click near the hole to pick that one.
- **Shift-click to collect.** Shift-click edges or faces to build up a set, then click one of them to size the whole set together. Shift-click a picked edge again to drop it. The set can reach across several objects: pick edges on one object, then Shift-click edges on another, and they all take the same size and shape. The status bar counts what you have picked ("3 edges picked. Shift-click to add more, click one to size them, or type a size.").
- **`Ctrl+A` (`Cmd+A`) over an object** picks every sharp edge it has.
- **Select first.** If you already have objects selected when you choose Fillet, all of their sharp edges are picked and waiting for a size.

A smooth edge, such as the boundary of an existing fillet, can't be filleted again, and the status bar says so. Edges between two *different* objects can't be filleted either: union the objects first, then round where they meet. (Picking edges on several objects at once is fine. That's different from an edge shared by two objects.)

## Setting the size

The moment you click an edge, the rounding appears on it at a starting size: the size of the last one you made, or a tenth of the edge's length if this is the first. A round **handle** sits just off the edge on the outside of the corner, joined to it by a thin line, with its value written beside it. You're not done yet. The pick waits for you.

- **Drag the handle** along its line. The rounding grows and shrinks with it, and the value beside it updates. Let go where you like.
- **Press on the edge and drag** to do it all in one move: the press picks the edge and holds the handle, so drag outward and let go.
- **Click the handle, or press `Enter`, to finish.** A click on the edge right where the handle leaves it does the same.
- **Click anywhere else** to finish too. Click an empty spot or a face and that's all it does. Click another edge and it finishes the first and picks the next at the size you just made, so you can round a run of edges with one click each. Click an existing fillet and it opens for editing.
- **Double-click a fresh edge** to give it the same size as the last one in a single move. After a round with stops or a radius that varies, the double-click repeats the base size as a plain round; the status bar says which.
- **`Esc`** drops the pick and changes nothing.

Moving the pointer around doesn't change the size. Only the handle and the keyboard do. For an exact size, type it and press `Enter`:

| Type | What you get |
|---|---|
| `6` | A fillet of radius 6, or a chamfer that cuts 6 back on both faces |
| `6,3` | A chamfer that cuts 6 back on one face and 3 on the other. The first number goes on the face you were pointing at when you clicked the edge |
| `3,45°` | A chamfer that cuts 3 back on that face, at a 45° angle |
| `8s` | Eight facets around each rounded corner (it sticks until you reload) |

A bare number uses the units you've set for the model, like every other tool. Right after you finish, the number is still live: type another size and the fillet is redone at that size. After a round with stops or a radius that varies, the double-click repeats the base size as a plain round; the status bar says which.

If the size you ask for is more than the edge can take, Hew uses the largest that fits and the status bar tells you what's in the way: "Radius limited to 14 mm by the adjacent face" or "by the neighboring edge," "by the hole in this face," "by the length of this edge," and so on. The preview in the model shows exactly what you'll get before you finish.

## The Edge chip: rounds, bevels, and router profiles

A small **Edge** chip appears in the view while the tool is active. Two buttons to its left, **Round** and **Chamfer**, switch between the everyday shapes in one click, and the one in use is highlighted. Open the chip's menu to choose any shape of the cut:

- **Round** and **Chamfer**, the two everyday shapes.
- **Cove**, a hollow quarter-round cut into an outside edge, like a router's cove bit.
- **Bullnose**, a full half-round across a narrow face, such as the front of a shelf board. It needs two parallel edges of one face.
- **Built-in profiles:** Ogee, Roman ogee, Bead, Thumbnail, Cove and bead, Chamfer and bead, and Cavetto.
- **My profiles:** shapes you drew and saved yourself. See [The Library](/learn/library/#profiles).

A fillet on an inside corner adds material instead of removing it, the way a machinist's fillet does. You don't choose; the edge decides.

When a corner has three edges meeting, a **Corners** chip appears next to the Edge chip. For a round you choose **Rolled** (a smooth ball-shaped corner) or **Mitered** (the three roundings meet in a crisp seam). For a chamfer you choose **Mitered** or **Flat** (a small flat facet cuts the corner).

## Stopping short, and a different size at each end

With one run of edge picked, small diamonds appear at its ends. Drag a diamond along the edge to **stop** the fillet short of the end, or press `Tab` until the measurement box says Stop, type the distance from the end, and press `Enter`. A stop has to be farther from the end than the radius. A second round handle beside each end sets the radius there, so the rounding can grow from one end to the other. **Click along the edge**, away from the main handle, to add another radius point in the middle; drag it along the edge to move it, or drag its sphere to change the radius there.

`Tab` walks through every value you can type: the radius, the start stop, the end stop, the radius at each end, then each radius point's position and radius. The label in the measurement box tells you which one you're on (Radius, Stop, Position). After you press `Enter` on one of them, or after you drag one, typing goes back to the radius. To remove a radius point, `Tab` to its position and press `Delete`.

Stops and varying radius work on straight runs of edge, and not together. A varying radius is for rounds only.

## Editing a fillet

A fillet keeps track of the edges it replaced, so you can change your mind.

- **Hover a fillet** and the status bar reads "Click to resize this fillet. Delete to remove it." Click it and the same round handle appears on the band, at its current size and pointing out along its surface. Drag the handle to resize it, or type a size and press `Enter`. You can also press on the band and drag in one move.
- **Click the handle, or anywhere else, to finish.** A click away from a fillet you haven't changed just closes the edit.
- **Delete** takes the rounding off and brings back the sharp edges. A fillet remembers what it replaced, so this works after you have kept modeling elsewhere, after a save, and when a later fillet runs into it (that one is rebuilt on the sharp corner). If something has cut through the fillet itself since, such as a boolean or a face pushed through it, Hew says so when you click the fillet or try to delete it, and from then on the status bar says it when you hover ("A later edit changed this fillet…"); undo back past that edit to change it.
- **`Alt`** while you're resizing a fillet makes it a chamfer of the same size (or a chamfer a fillet). The edit stays open on the result, so you can keep sizing it or switch it back.
- **`8s`** while you're resizing re-facets the fillet with eight facets at its current size.
- **Push/Pull a fillet's face** and the fillet gets bigger or smaller.
- **Push/Pull the face next to a fillet** and the fillet comes along: the face moves, and the fillet is rebuilt at the new place.
- **Paint** colors a fillet as one face, not as a dozen thin strips.

Undo takes back exactly what you did, in one step.

Rounds along a pipe joint, a slanted cut, or an imported mesh stay editable too. Hover one, resize it, or press Delete to bring back the sharp edges, the same as any other fillet. If a later edit changed the rounding so Hew can't take it back exactly, it refuses and leaves the model as it was, rather than guessing.

If you collected edges on several objects, the whole set is one undo step. If any one object can't take the size, nothing changes anywhere, and the message names the object: "Nothing changed. Object 2 can't take this: ..." followed by the reason.

## Rounding corners in a drawing

Fillet and Chamfer also work on a flat drawing, before you push or pull anything. Hover a corner where two straight lines meet, on the ground or on a face, and the status bar says "Click to round this corner, or type a radius." Click it and a handle appears off the corner: drag it to set the radius, or just type the radius and press Enter. Typing over a hovered corner or edge picks it, the same as a click. The corner becomes a true arc, so it extrudes into a smooth curved wall. Chamfer on a corner takes two distances, like `6,3`. A drawn corner takes only Round or Chamfer, so while you hover one, the Edge chip shows Round in place of any other shape and greys out the rest. Move off the corner and your shape comes back.

Only a corner of two straight lines can be rounded. If the arc would run off the face it's drawn on, or the radius is too big for the shorter side, Hew says so instead of guessing.

## Seeing where a fillet ends

A fillet's rounded surface joins the flat faces next to it without a visible crease, so Hew hides the line where they meet. To see those lines, turn on **View ▸ Tangent Edges**. They show thin and faint, and they print only when the setting is on.

The shading is smooth across the rounding as well. Where a round runs along a pipe joint or meets another round at a ridge, the surface shades as one curve instead of in patches, and it meets the flat faces beside it without a tilt. This only changes how light falls on the surface. The shape itself, and the hard edges where something was cut through a fillet, stay as they are.

## What it can't do yet

- **One size per operation.** Everything you pick together gets the same size and shape. To give edges different sizes, do them in separate passes. When you add a fillet beside one you already made, Hew rebuilds the corner where they meet, so rounding the rim first and the sides second gives the same corner as the other way around.
- **Some corners are too awkward.** Where inside and outside edges meet at a corner that isn't a simple box-like corner, Hew may say "Can't blend this corner" and ask you to do those edges one at a time.
- **Pipe joints, slanted cuts, and imported meshes** (edges that aren't straight, or on curved walls that aren't a simple cylinder) take rounds, bevels, coves (on outside edges, as everywhere), and profiles, around a complete loop or along a run that ends on a flat face at both ends. A bullnose there says "This edge can't be filleted yet." and a run that stops partway along a sharp edge says "Can't blend this corner." Like any other fillet, you can resize these later or delete them to get the sharp edge back.
- **Two objects must be one first.** A fillet needs an edge with a face on each side, in one solid.
- **Only a solid.** A fillet on an object with gaps says "Fillet needs a solid." Fix it with [Model Health](/learn/import-export/#making-imported-shapes-solid) first.
