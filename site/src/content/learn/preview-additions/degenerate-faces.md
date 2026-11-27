---
title: "Remove degenerate faces: passages for other chapters"
description: "Text held back from other chapters until Remove degenerate faces graduates from Preview."
order: 106
preview: [remove-degenerate-faces]
---
This page is not published. It holds the text that describes Remove degenerate faces, taken out of the chapters named below so that the published guide describes exactly what a production build has. When the feature graduates, move each section into the place it names and delete this page. Each section is the exact text to paste, between the fences.


## Section 1

Chapter **Import and export**, section **Combining with large imports**: add after the first sentence ("...on the biggest ones."), before "If it says a face...".

~~~markdown
If Hew refuses because one of the objects has a face with no area, the message says so and offers **Open Model Health**: use **Remove degenerate faces** there, then try again.
~~~

## Section 2

Chapter **Import and export**, section **Making imported shapes solid**: after the list of kinds (the last bullet is **Can't be closed.**) and before **Choose for each item**.

~~~markdown
A closed shape can still carry a few faces with no area, slivers an exporter left behind. Model Health lists such an object under **Has zero-area faces**, and **Remove degenerate faces** deletes them and keeps the shape closed. Hew reports "Removed 3 zero-area faces." It never does this on its own, and it never removes a face that has any area.
~~~
