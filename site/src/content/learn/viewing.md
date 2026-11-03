---
title: "Viewing your model"
description: "How to get around: mouse navigation, the dedicated camera tools, standard views, Zoom Extents, visibility controls, and the non-destructive Section Plane."
order: 4
---

Camera navigation never interrupts your work. With any tool active, at any moment, you can orbit, pan, or zoom, even halfway through drawing a line.

<docs-video theme="dark" slug="viewing" label="viewing demonstration"></docs-video>

## With the mouse

| Action | Input |
|---|---|
| Orbit | Drag with the **middle mouse button** |
| Pan | Hold `Shift` and drag with the **middle mouse button** |
| Zoom | **Scroll wheel** — zooms toward the cursor |
| Fly | Hold the **right mouse button** and press `W` to move forward — see [Fly](#fly) below |

Zooming follows the cursor, so point at what you want to get closer to and scroll. Orbiting pivots around whatever is under the cursor when you start the drag, and each scroll step covers a fixed fraction of the distance to the surface you're pointing at — so a small detail on a large model is a few scrolls away, and orbiting around it keeps it on screen. The camera has gentle inertia. Hold `⌘` (Mac) or `Ctrl` (Windows, Linux) during a drag for a precise orbit or pan instead: the camera tracks the pointer 1:1 with no inertia for as long as the key is held. Pressing it while the view is still coasting from a released drag stops it dead.

## With the camera tools

If you're on a trackpad or prefer left-button navigation, activate a dedicated camera tool — the left mouse button then drives the camera instead of the current tool:

- **Orbit** — press `O`, or Camera ▸ Orbit
- **Pan** — press `H`, or Camera ▸ Pan
- **Zoom** — press `Z` (drag up/down), or Camera ▸ Zoom. Hold `Shift` while dragging (or Shift-scroll) to change the field of view instead — see below.

While the Orbit tool is active, holding `Shift` temporarily pans instead, the same convention SketchUp users know. This also works partway through a drag: press `Shift` mid-orbit to switch to panning, and release it to go back to orbiting, without letting go of the mouse button.

When you're done navigating, press `Space` to return to the Select tool (or the shortcut of whatever tool you were using).

## Position Camera, Look Around, and Walk

These three walk you through a model at eye level instead of orbiting around it — useful for checking a doorway width, a sightline, or how a room actually feels to stand in.

- **Camera ▸ Position Camera** — click a point to stand there, at eye height, facing whatever direction you were already looking; drag instead to stand at the press point and look toward wherever you release. Either way, it hands off straight to Look Around so you can immediately look around from where you landed.
- **Camera ▸ Look Around** — drag to look around from a fixed spot, like turning your head; looking straight up or down is clamped short of true vertical so the view never flips.
- **Camera ▸ Walk** — drag up or down to walk forward or back, left or right to turn; hold `Shift` to strafe sideways and change height instead. Walk keeps you at a constant eye height above the ground plane; there's no collision detection, so you can walk through geometry.

Eye height defaults to 1.68 m (SketchUp's classic 5′6″) and is shared across all three: type a new height and press Enter in any of them, and it carries over to the others for the rest of the session. `Esc` returns to the Select tool from any of the three.

## Fly

Walk keeps your feet on the ground. Fly is for moving through a house the way a game spectator does: hold the **right mouse button** anywhere in the viewport, move the mouse to look, and use the keys to move. The cursor disappears while you fly and comes back where it was when you let go, so a long look never runs it into the edge of the screen. Let go and you are back in whatever tool you were using. It works under every tool, in the middle of any gesture, and the left button stays with the tool the whole time — fly up to a window, then click its sill with Push/Pull without letting go.

| Key | Motion |
|---|---|
| `W` / `S` | Forward / back along the direction you are looking |
| `A` / `D` | Sideways |
| `E` / `Q` | Up / down, always straight up and down |
| Arrow keys | Same as `W` `A` `S` `D` |
| `Shift` | Four times faster while held |
| Scroll wheel | Faster or slower; the new speed stays for the rest of the session |

The keys are read by their position on the keyboard, so the same cluster works on an AZERTY or Dvorak layout. Movement starts and stops with a short glide rather than a jolt. The status bar lists the keys the moment the button goes down, and the measurement box shows the current speed, which starts at a brisk walk of 2 m/s. Flying switches the view to perspective and leaves it there. There is no collision detection: walls do not stop you.

If you would rather not hold a button, **Camera ▸ Fly** (or "Fly" in the command palette) switches Fly on until you press `Esc`. The cursor disappears, and from then on the mouse looks around with nothing held, the same keys move you, and the scroll wheel sets the speed. On a laptop trackpad it is the same: a finger on the pad looks, a two-finger scroll or a pinch changes the speed. Holding the right button inside it still works. Press `Esc` to get the cursor back where it was.

If the browser will not hide the cursor (it has to be asked from a click or a key press, and some will not do it at all), Fly still switches on: a two-finger scroll looks around and a pinch sets the speed, and the status bar says to click the view to lock the mouse. That click is for the lock, not the tool. Switching to another window gives the cursor back; click the view when you return.

Right-drag used to pan. Pan is now `Shift` plus a middle-button drag, or the Pan tool (`H`).

## Parallel projection

**Camera ▸ Parallel Projection** switches between the normal perspective view and an orthographic (parallel) one, where parallel edges in the model stay parallel on screen instead of converging toward a vanishing point — useful for reading true proportions, or for a flat, technical-drawing look on a standard view. Toggling holds the view steady on whatever you're currently looking at; toggle back and you're exactly where you started. Everything else — orbiting, panning, snapping, the tool gizmos — works the same in either projection.

## Field of view

There's no separate Field of View command — it's reachable only through the Zoom tool, in perspective.

While Zoom is active, the current field of view shows in the corner the whole time. The most direct way to change it is to hold `Shift` while dragging: drag up to narrow the view (a longer, more zoomed-in lens), drag down to widen it. `Shift`-scroll does the same thing in smaller steps. Whichever way you started the drag decides what it does for its whole length — starting a plain drag and pressing `Shift` partway through keeps dollying, and vice versa.

You can also type a value directly and press Enter: degrees (`45` or `45deg`) or a 35mm-equivalent focal length (`50mm`), if you're used to thinking in camera lenses. Either way the corner shows both units, so you always know where you landed.

None of this does anything under Parallel Projection — a parallel view has no lens to speak of, so a Shift-drag there dollies like a plain one.

## Zoom Window

**Camera ▸ Zoom Window** drags a rectangle over the part of the model you want to fill the screen, then returns you to the Select tool — a one-shot camera move, not a mode you have to remember to leave.

## Standard views and framing

- The viewport's top-left chips jump straight to **Top**, **Iso**, or **Front**.
- **Camera ▸ Standard Views** offers all seven: Top, Bottom, Front, Back, Left, Right, and Iso. All are also in the command palette ("Standard View: …"). Top, Bottom, Front, Back, Left, and Right only turn the camera about the point you're orbiting and keep your zoom, so the model stays the same size on screen; Iso re-frames the whole model.

![The Top button looking straight down at a two-part bracket](/docs/views-top.webp)
![The Front button switching the same bracket to a straight-on front view](/docs/views-front.webp)

- **Camera ▸ Zoom Extents** (palette: "zoom to fit") frames every visible thing in the model — solids and sketches alike (guides don't count) — the fastest way back when you've orbited into a corner. Hew also zooms to fit on its own when a model is opened, and a small unit chosen on the welcome screen starts the blank view zoomed in to match.

## Controlling what you see

- **View ▸ Axes** shows or hides the world axes.
- **View ▸ Grid** shows or hides the ground grid, independently of the axes. The ground is a virtual backdrop — it never hides your model, so a Bottom view sees the model straight through it, and geometry lying exactly on the ground stays visible.
- **View ▸ Guides** shows or hides all construction guides (see [Precision, measurement, and guides](/learn/measurement-and-guides/)). Hidden guides also stop participating in snapping.
- **View ▸ Section Cut** turns a placed section's cut on or off (below); unlike the other three, it's grayed out until you've placed one.
- **View ▸ X-ray** draws every face see-through, so edges and parts hidden behind other faces show through. Materials that are already see-through (glass) get fainter still. Selection and hover highlights look the same. Nothing hides anything in X-ray: every edge, endpoint, and midpoint snaps and can be selected, hidden or not. Faces still count wherever the cursor is not on an edge or point: clicking one selects as usual and push/pull works on it, and where faces stack up the front one is picked. While a group, component, or object is open for editing, the tools reach its faces through whatever is in front of them, in Shaded as well as in X-ray: bury a component partly inside a box, double-click the component, and Push/Pull can grab its buried face directly while the box is skipped. Everything outside the open thing is drawn translucent while you edit, so the tools skip it. If none of the open thing is behind the cursor, the tool acts as it did before. The top level is unchanged.
- **View ▸ Wireframe** draws only the edges, including the ones a face would normally hide. Faces are not there for clicking either: hovering where a face would be finds no face to snap to, select, push/pull, or paint, and drawing lands on the ground, a locked plane, or an edge or point. To select an object, click one of its edges.
- X-ray and Wireframe are one setting: choosing one turns the other off, and choosing the one that's on returns to the normal view. On the dark theme both draw edges in a light gray so they read against the background. They apply to this window only, are not saved in the file, and start off in a new session. A print or image capture shows the model the way it looks on screen.
- The **Outliner** and **Tags** panels have per-object and per-tag visibility toggles ([Organizing your model](/learn/organizing/)).
- **Scenes** save a whole view — camera, hidden objects and tags, section cut, and these toggles — and bring it back in one click ([Scenes](/learn/scenes/)).
- When you double-click into a group, component, or object to edit it, the rest of the scene dims so your editing context is unmistakable. Press `Esc` to step back out one level.

## Looking inside with a Section Plane

The **Section Plane** tool (Tools ▸ Section Plane) is a view aid, not a modeling operation: it clips away part of the model so you can look inside — check wall thickness, verify clearance between mating parts, spot a hidden void — without touching any geometry. It's the opposite of [Slice](/learn/combining-solids/#slice), which actually cuts a solid into two separate objects; a section plane changes only what the viewport draws. It is saved with the file as view state (like the camera, outside undo), and a [Scene](/learn/scenes/) can capture it.

![A hollow chest cut open by an active section plane, showing an interior shelf](/docs/section-cut.webp)

1. Activate the tool and click a face — the section plane is created coincident with, and normal to, that face, and becomes active immediately. Click empty ground instead for a horizontal section at ground level. The tool stays active so you can adjust the section right away; press `Space` to return to the Select tool when you're done.
2. Everything on the back side of the plane disappears, and cut walls render from the inside, so wall thickness reads directly off the exposed edges (the cut itself isn't filled in — there's no solid cap).
3. The widget marking the plane reads mostly as an outline plus a small arrow along its normal — solid when the cut is active, dashed when it's turned off — with enough fill to read as glass rather than empty space. It's always sized to frame the whole visible model, from any angle, and grows automatically as you add geometry beyond its current edges.
4. **Sweep the cut** — with the Section Plane tool active, click the widget, then move the cursor to slide the plane along its own normal arrow through the model; click again to set it there (or type an exact distance and press Enter). This is the main way to inspect a design, front to back. Click a face instead to re-place the section somewhere new.
5. **Turn it off without losing it** — check **View ▸ Section Cut** (also in the command palette as "Section Cut") to see the whole model again. The widget stays put, drawn dashed, ready to switch back on when you check the box again.
6. **Remove it** — with the Section Plane tool active, press Delete or Backspace. The model returns to whole.

Only one section plane exists at a time — placing a new one replaces whichever was there before. To keep several cuts of the same model, capture each in a [Scene](/learn/scenes/#scenes-and-the-section-plane).

## Reading the axes

The world axes use the conventional colors (**X red, Y green, Z blue**), drawn solid in the positive direction and dashed in the negative. Inference cues reuse these colors: when a drawing or move operation locks to an axis, the cue and preview take on that axis's color. Hew is a Z-up application: "up" in your model is the blue axis.
