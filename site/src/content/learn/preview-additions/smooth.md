---
title: "Smooth and Soften Edges: passages for other chapters"
description: "Text held back from other chapters until Smooth and Soften Edges graduates from Preview."
order: 103
preview: [smooth, soften]
---
This page is not published. It holds the text that describes Smooth and Soften Edges, taken out of the chapters named below so that the published guide describes exactly what a production build has. When the feature graduates, move each section into the place it names and delete this page. Each section is the exact text to paste, between the fences.


## Section 1

Chapter **Import and export**, under **Importing**, after the paragraph about placing a picture and before **Combining with large imports**.

~~~markdown
### Making an imported mesh look smooth

An STL has no idea which of its edges were meant to be smooth, so a curved part arrives showing every triangle. Select the object and use **Soften edges under** in Object Info: enter an angle (it starts at 20°) and press **Apply**. Edges that bend less than that stop showing as lines and shade smoothly, and the sharp corners stay sharp. It's one undo step and it never changes the shape. See [Smooth](/learn/smooth/#soften-edges-making-an-imported-mesh-look-smooth).
~~~

## Section 2

Chapter **Import and export**, under **Exporting**: after **SVG line drawing** and before **What about STEP?**.

~~~markdown
### Smooth objects

STL, 3MF, glTF, and USDZ files carry an object's [Smooth](/learn/smooth/) surface, the soft shape you see, closed and ready to print. When the model has a smooth object, the export dialog offers **Export cages instead**, which writes the blocky shape you were editing. SketchUp files always get the cage.
~~~
