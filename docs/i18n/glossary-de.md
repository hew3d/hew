# German glossary (de)

Decisions for the German catalog, `app/src/locales/de/messages.po`.
Terms follow the German edition of SketchUp wherever one exists, so
people coming from SketchUp recognize every tool.

## Register

- Formal address ("Sie"), as in SketchUp's German edition and most
  German desktop software. Tool prompts use the imperative with "Sie"
  ("Klicken Sie auf eine Fläche …") or a short infinitive
  ("Klicken zum Platzieren").
- Ellipsis is written with a space before it (`Öffnen …`), following
  German typography.
- Quotation marks are German low-high quotes („…“).
- Key names use the German keyboard labels: Umschalt (Shift), Strg
  (Ctrl), Entf (Delete); Esc, Enter, Tab, and Alt stay as they are.

## Tools

| English | German |
| --- | --- |
| Select | Auswählen |
| Line | Linie |
| Rectangle | Rechteck |
| Circle | Kreis |
| Polygon | Polygon |
| Arc | Bogen |
| Arc completion: Pie / Segment | Kreisausschnitt / Kreisabschnitt |
| Push/Pull | Drücken/Ziehen |
| Follow Me | Folge mir |
| Offset | Versatz |
| Move | Verschieben |
| Rotate | Drehen |
| Scale | Skalieren |
| Flip | Spiegeln |
| Eraser | Radiergummi |
| Paint | Farbeimer |
| Tape Measure | Maßband |
| Protractor | Winkelmesser |
| Dimension | Abmessung |
| Text / 3D Text | Text / 3D-Text |
| Section Plane | Schnittebene |
| Section Cut | Schnitt |
| Axes / Drawing Axes | Achsen / Zeichenachsen |
| Orbit | Orbit |
| Pan | Schwenken |
| Zoom / Zoom Extents / Zoom Window | Zoom / Zoom Grenzen / Zoom-Fenster |
| Position Camera / Look Around / Walk | Kamera positionieren / Umsehen / Gehen |
| Fly | Fliegen |
| Slice | Schneiden |
| Fillet / Chamfer | Abrunden / Fase |
| Shell | Schale |
| Revolve | Rotationskörper |
| Loft / Skin | Loft / Haut |
| Bend / Twist / Taper | Biegen / Verdrehen / Verjüngen |
| Edit Vertex | Eckpunkt bearbeiten |
| Position Texture | Textur positionieren |

## Objects and concepts

| English | German |
| --- | --- |
| solid (noun) | Volumenkörper |
| solid (adjective, "is solid") | geschlossen |
| Make Solid | Volumenkörper erstellen |
| face / edge / vertex | Fläche / Kante / Eckpunkt |
| sketch | Skizze |
| shape | Form |
| object | Objekt |
| group / ungroup | Gruppe / Gruppierung aufheben |
| component / instance / definition | Komponente / Instanz / Definition |
| Make Unique | Eindeutig machen |
| Explode | Auflösen |
| linked copy / independent copy | verknüpfte Kopie / unabhängige Kopie |
| Detach / Detached | Lösen / Gelöst |
| guide (line, point) | Hilfslinie, Hilfspunkt |
| inference / snap | Ableitung / Einrasten |
| Measurements box | Messwertfeld |
| tag | Tag |
| Outliner | Gliederung |
| Scene | Szene |
| Union / Subtract / Intersect | Vereinigen / Subtrahieren / Verschneiden |
| boolean | boolesche Operation |
| sweep / profile / path | Sweep / Profil / Pfad |
| rail (loft) | Führungsschiene |
| opening (in a shape) | Öffnung |
| zero-area face | Fläche ohne Flächeninhalt |
| degenerate face | entartete Fläche |
| flat panel / curved sheet | ebene Platte / gekrümmte Folie |
| Mend / Rejoin | Reparieren / Wieder verbinden |
| Model Health | Modellzustand |
| Purge Unused | Bereinigen (Dialog: „Nicht verwendete Elemente löschen?“) |
| Object Info / Model Info | Objektinformationen / Modellinformationen |
| tray / panel | Ablage / Bereich |
| tool rail | Werkzeugleiste |
| Contextual Dock | Kontextleiste |
| Command Palette | Befehlspalette |
| Shop Mode | Werkstattmodus |
| cut list | Zuschnittliste |
| X-ray / Wireframe | Röntgen / Drahtgitter |
| Parallel Projection / Perspective | Parallelprojektion / Perspektive |
| Report Bug / Sent Reports | Fehler melden / Gesendete Berichte |

## Command-palette keywords

Lowercase entries in `registry.ts` and `curveEntries.ts` are search
keywords. They are translated into the German words a person would type;
English tool names still match, as LOCALIZATION.md describes. Names of
SketchUp extensions (Curviloft, Fredo Scale, Round Corner, Shape
Bender) and file extensions stay as they are.
