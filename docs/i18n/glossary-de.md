# German glossary (de)

Decisions for the German catalog, `app/src/locales/de/messages.po`.
Terms follow the German edition of SketchUp in most cases, so people
coming from SketchUp recognize the tools. The few places where this
catalog departs from SketchUp on purpose are listed under
"Deliberate departures from SketchUp" below.

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
| Zoom / Zoom Extents / Zoom Window | Zoom / Alles zeigen / Zoom-Fenster |
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

The Paint tool is named "Farbeimer", as in SketchUp. The action it
performs is "einfärben" ("Fläche einfärben", "Objekt einfärben"), never
"malen", which reads as painting a picture.

## One English word, two German words

Some English labels are a command in one place and a name in another.
The source gives each sense its own entry (a `msgctxt` in the catalog),
so each gets its own word:

| English | Where | German |
| --- | --- | --- |
| Group | the command (menu, dock, undo) | Gruppieren |
| Group | the kind of thing (Object Info, unnamed group) | Gruppe |
| Paint | the tool | Farbeimer |
| Paint | the action (Library button, undo) | Einfärben |
| Back | the button that goes back | Zurück |
| Back | the camera view | Hinten |
| Center | the Print button that centers the model | Zentrieren |
| Center | the snap point of a circle or arc | Mittelpunkt |

## Deliberate departures from SketchUp

These terms differ from SketchUp's German edition on purpose. Please
keep them unless there is a new reason to change them.

| English | SketchUp (de) | Hew (de) | Why |
| --- | --- | --- | --- |
| Orbit | Rotieren | Orbit | "Rotieren" is too close to the Rotate tool ("Drehen"). "Orbit" is common in German 3D software. |
| Pan | Hand | Schwenken | "Schwenken" names what the tool does. "Hand" only names the icon. |
| Polygon | Vieleck | Polygon | Both are common. "Polygon" matches the English tool name and the search keyword. |
| Zoom Window | Zoombereich | Zoom-Fenster | "Zoom-Fenster" is the term German CAD users know. |
| Pie (arc completion) | Kuchen | Kreisausschnitt | The geometric term. It pairs with "Kreisabschnitt" for Segment. |
| Look Around | Umschauen | Umsehen | Equivalent in meaning. "Umsehen" is the more neutral standard German form. |

SketchUp names were checked against the German SketchUp 2024 Quick
Reference Card.

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
| Lock / Unlock / Locked | Sperren / Entsperren / Gesperrt |
| Library / Library item | Bibliothek / Bibliothekselement |
| part (a thing in the model) | Teil (das Teil) |
| update a part from the Library | aktualisieren / Aktualisierung |
| update (a new version of Hew) | Update |
| release notes | Versionshinweise |
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
