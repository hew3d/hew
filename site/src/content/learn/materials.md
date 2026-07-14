---
title: "Materials"
description: "Paint faces or fill whole objects from a per-document palette of colors and textures that survives every modeling operation, and adjust opacity for glass and other see-through materials."
order: 11
---

Materials in Hew are simple and per-document: a palette of flat colors and image textures that you paint onto faces or assign to whole objects.

## The Materials panel

Expand **Materials** in the right-hand tray (or Window ▸ Materials, `⇧⌘C` / `Ctrl+Shift+C`).

![The Materials panel expanded, showing the Default swatch, two named materials, and the Add color controls](/docs/materials-panel.png)

The panel contains:

- **Default (unpainted)** is the built-in neutral gray. Select it to "paint with nothing," i.e. to reset faces back to unpainted.
- Your **material swatches**: click one to make it the current material and pick up the **Paint** tool in one move — the very next click paints with it (see [Painting](#painting) below). Texture materials show a thumbnail.
- **Opacity**: drag the slider to make the selected swatch translucent — glass, screens, scrims, anything meant to be seen through. Works the same for colors and textures.
- **Add color**: pick a color with the color picker, optionally name it, and add it to the palette.
- **Add texture**: choose a PNG or JPEG image and give it a real-world size (width × height in meters). The image tiles across faces at that physical scale.

## Opacity

Every material has an opacity, from fully opaque (100%) down to fully transparent (0%) — 255 shades either way. Select the swatch you want to change, then drag the **Opacity** slider underneath the swatch list. The percentage next to it tracks where you are.

![The Materials panel with the Slate swatch selected and its Opacity slider dragged down to 66%, visibly lightening the slate base object in the viewport](/docs/materials-opacity.png)

The change applies to every face and object currently painted with that material, updates the viewport immediately, and is undoable like any other edit.

## Painting

Click the swatch you want — that makes it the current material and switches you to the **Paint** tool automatically (you can also pick Paint yourself with `B`). Then:

- **Click a face** to paint just that face.
- **`⌘`/`Ctrl`-click** to set the whole object's **base material** in one go.

The hovered face highlights so you can see what you're about to paint. To un-paint, select the **Default** swatch and paint again.

## Face paint vs. object base material

Every object has an optional **base material** — the color its faces show when they haven't been painted individually. Individually painted faces override the base. The base is why painted models stay painted as you keep working: when an operation creates new faces (pulling a boss out of a painted box, slicing, booleans), the new faces inherit the object's base material instead of reverting to gray.

Set the base material with `⌘`/`Ctrl`-click in the Paint tool.

## Materials survive modeling

Painted faces keep their materials through splitting, push/pull, slicing, and boolean operations — material assignments follow the surviving geometry. Painting is undoable like any other edit.

## Materials and export

glTF/GLB export carries your colors and embedded textures with the model. Use it when the receiving app should see materials. STL has no concept of color; it exports bare geometry. See [Import and export](/learn/import-export/).
