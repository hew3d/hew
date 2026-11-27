---
title: "Curved shapes"
description: "Revolve, twist, loft, bend, hollow out, and project onto curved surfaces, plus what kind of curve each tool gives you."
order: 7.2
preview: [revolve, loft, shell, project, path-array, deform]
---

Push/Pull and Follow Me get you most of the way to a curved model. These tools cover the rest: shapes turned on a lathe, sweeps that twist, hulls that blend one outline into another, solids bent along a line, hollow shells, shapes drawn onto curved surfaces, and copies strung along a curve. They're in the **Tools** menu: Revolve, Loft, Shell, Project, and Array along Path in the **Shape** submenu. [Fillet and Chamfer](/learn/fillet-and-chamfer/) and [Smooth](/learn/smooth/) have their own chapters.

Every one of them makes a normal solid. It's watertight, you can push/pull it, combine it, and export it, and undo takes the whole thing back in one step. When a tool can't do what you asked, it says why and leaves the model alone.

## Which kind of curve you get

Hew's solids are made of flat faces, even the round ones. What differs is how much Hew knows about the curve underneath.

| | Made by | What Hew knows | Snapping and measuring | Export |
|---|---|---|---|---|
| **Exact** | Circles and arcs, anything extruded from them, Follow Me and Revolve around a circle, fillets, chamfers | The true center, axis, and radius of the curve | Snaps to the real center and measures the real radius | Re-faceted as smooth as you ask |
| **Generated** | Loft, twisted or scaled Follow Me, Deform, Follow Me around a bend that isn't a drawn circle, a circle scaled into an oval | The shape and which seams are meant to look smooth | Snaps to the corners and edges of the facets | At the smoothness it was made with |
| **Smoothed** | Smooth | A blocky shape plus how soft to make it | Snaps to the blocky shape | The soft surface, or the blocky shape if you ask |

Exact curves stay exact through combining: union a filleted block with a revolved bowl and each keeps its true curves. Generated curves keep their smooth look through combining too, though they snap and export at the density they were made with. Nothing is ever claimed to be a perfect curve when Hew doesn't know that it is.

## Revolve

Revolve spins a flat shape around a line, the way a lathe turns a bowl or a goblet.

1. Choose **Tools ▸ Shape ▸ Revolve**. Or select a closed shape you've drawn first and click Revolve on the dock; the shape you selected is the one to turn, so the next step is the axis.
2. Click a closed shape you've drawn, or the face of a solid.
3. Click an edge for the axis, or click two points. The axis has to lie in the same plane as the shape, or the status bar says "The axis has to lie in the plane of the shape."
4. A full turn appears at once. Click to accept it, or move to pick a partial turn (it snaps to every 15°) and click. `Enter` takes the full turn.

You can type instead of clicking: `90` or `90°` for the angle, `24s` for the number of facets around the turn. Type again right after and the result is redone.

The axis can run along an edge of the shape, so a half-profile makes a solid with a pole on the axis (a sphere, a cone, a goblet). A partial turn of a shape that touches the axis can't be built, and Hew says so; use a full turn, or move the shape off the axis. An axis that cuts through the middle of the shape is refused too.

Revolve used to be a job for Follow Me with a circle for a path. That still works, but Revolve is now its own tool with its own angle, and it's the quicker way.

## Follow Me with twist and end scale

Follow Me can turn the profile as it travels along a path, and grow or shrink it toward the far end. Pick the path and the profile as usual, then type before you finish:

- `30deg` twists the profile 30° over the length of the path (`30d` and `30°` work too; a bare `30` is a length).
- `40%` scales the profile to 40% of its size at the far end, so a tapered blade or a cone.
- `8s` sets the number of stations along each segment, for more or fewer slices.

You can type these while dragging a partial sweep or at the profile stage, then press `Enter`. The status bar shows what's set, such as "(Twist 30° · End 40%)". The settings reset for each new sweep, and typing again right after finishing redoes the sweep. A twist or an end scale needs an **open** path, since a loop can't twist and still meet itself. A partial sweep of a loop is fine.

## Loft

Loft joins two or more closed shapes into one solid, blending from each outline to the next.

1. Click Loft in the tool rail, or choose **Tools ▸ Shape ▸ Loft**. With a drawn shape selected, the dock has a Loft button too.
2. Click the shapes in order: drawn shapes, or faces of solids. Each gets a number. Click a number again to take it out.
3. To end in a point, click the shapes first, then an existing point after the last one (a pyramid or a nose).
4. To guide the blend along a curve, `Alt`-click a drawn line or curve to add it as a rail. Two rails at most. A rail has to touch every shape you loft between, at a corner or point of each.
5. Press `Enter`.

After two picks a see-through preview shows the result. Shapes with the same number of corners loft corner to corner whatever their sizes, and a circle meets a square's corners square-on. A green grip marks where each shape starts matching. If the shapes pair up wrong, drag it to another point: when every shape has the same number of corners the grip moves from corner to corner, otherwise from point to point.

A rail also decides which corners pair up. The first rail you add sets the pairing: Hew turns each shape's start so the rail meets all of them at matching corners, which is why a rail between two corners that the automatic match would not have paired still works. Because the rail decides, the green start grips go away while a rail is picked, and any start you dragged earlier is reset. A second rail has to agree with the first. If it would pair the corners a different way, Loft refuses it: "Each rail must run through a matching corner or point on every shape you loft between, and two rails must line the shapes up the same way." A rail that Loft turns down isn't counted, so you still have both slots.

The shapes you picked stay where they are. Every shape needs to be closed and without holes, and the tool says so otherwise ("Loft needs closed shapes. Close this one first."). A shape with a hole is turned away the moment you click it ("Loft needs shapes without holes."). For a hollow loft, loft the solid and then Shell it. Two circles around the same axis loft into an exact cylinder or cone.

## Deform

Deform reshapes a whole solid. Choose one of **Twist**, **Bend**, **Taper**, or **Bend to Path** from **Tools ▸ Deform**.

For the first three, click the solid. A box appears around it. Click the face at the end you want to move (the opposite end stays put), move the pointer, and click. Or type the amount: `25°` for Twist and Bend, `60%` for Taper. A typed bend goes the way you moved the pointer; type a minus sign to bend the other way. If the result would pass through itself, the shape stops at the biggest amount that doesn't, and the status bar says "Angle limited to ...". A typed amount past that is refused with the same limit named, so you can type it, unless it is within 1% of the limit, in which case it goes in at exactly the limit. Twist takes up to 720° and Bend up to 360°, and the status bar says so if you type more.

A dense solid, or a big twist of one, can take too long to show while you move, and so can finding the limit of a solid you have already twisted or bent. Then the status bar says the preview was skipped, the amount still follows the pointer, and a click or a typed amount applies it as usual. Past the limit, the click is refused with the limit named, just like a typed amount.

**Bend to Path** takes the solid and a drawn curve. Click the solid, click an edge or two points for the line it bends from, then click the path and press `Enter`. The solid is laid along the curve and stretched to its length, like a molding following a wavy edge.

Deform works on a whole solid, not on a group or component. Open or make the solid first.

## Shell

Shell hollows a solid into a thin-walled one, like turning a block into a box or a cup.

1. Choose **Tools ▸ Shape ▸ Shell**, or click Shell on the dock with a solid selected, and click a face of the solid. That face is left open, and a preview of the hollow result appears at once. Click more faces to leave them open too, or click an open face again to close it. (Select the object before you choose the tool and it starts with every face closed.)
2. Type the wall thickness and press `Enter`. The preview starts at about a twentieth of the object's smallest side. A negative number puts the wall on the outside instead.

With no face left open you get a closed hollow body. If the wall is too thick for the shape, Hew says "A 2 cm wall leaves no room here." and nothing changes.

An open box leaves one clean rim around the opening, not a ring of little faces, so a shelled cup looks like a cup.

You can also open the floor of a pocket you pushed into the solid. The pocket's walls get the same thickness, and the hollow opens through the floor. If the pocket is as deep as the wall or shallower, the walls carry on down and the opening becomes a hole straight through the top. A wall so thick that the walls around the pocket would run into the outside walls, or into another pocket's, is refused with the same "leaves no room" message.

## Project

Project draws a shape onto a surface that isn't flat, then cuts or raises it.

1. Draw a closed shape (a circle for a porthole, a logo outline) on a plane facing the object.
2. Choose **Tools ▸ Shape ▸ Project** and click the shape, then click the object. (Or select the shape before you choose the tool, and just click the object.) The shape is drawn onto its surface, along the direction the shape faces. That's one undo step; the depth below is another.
3. Move the pointer to set a depth, or type one, and click or press `Enter`. A positive depth cuts, a negative one raises. Press `Esc` to keep the drawn shape and skip the depth.

`Alt` switches between two ways of cutting. **Straight** cuts straight in along the direction of projection, so a deep enough cut goes through a wall: a porthole. A hole cut from a drawn circle is round and smooth, like a drilled one. **Follow the surface** keeps the depth constant as the surface curves, like engraving a ring. A depth that would cut the object into separate pieces is refused: "That depth would cut the object apart."

## Array along Path

Array along Path places copies of the things you've selected along a curve.

1. Select what to copy.
2. Choose **Tools ▸ Shape ▸ Array along Path** and click a drawn line or curve, or the edge of a solid's face.
3. Type how many (the original counts as one) and press `Enter`. Add a scale ramp like `100%→40%` to make the copies shrink along the path.

The copies turn to follow the path. Press `Alt` to keep them all facing the same way. On a closed path the copies divide the loop evenly. Typing a new count right after redoes it. If the copies sit closer together than the thing you copied is long, the status bar says "Copies overlap at this count." The array is made anyway; Hew warns about overlap and never refuses it, because overlapping copies are what you want for chain links.

The copies keep the same offset from the path that your object has. If your object sits to one side of the path, the copies sit the same distance to that side all the way along, so they trace a copy of the path moved to where the object is, not the path itself. To put copies right on the path, move the object onto its start first.
