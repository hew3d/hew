# SketchUp → Hew helpers

## `hew_export_tags.rb` — bring SketchUp Tags into Hew

SketchUp's COLLADA (`.dae`) exporter **does not export Tag/Layer membership**
(verified: no `layer=` attributes, no `<extra>` metadata). This script smuggles
each entity's Tag through the one thing COLLADA *does* carry — the node **name**,
which Hew already imports and persists.

### Workflow

1. In SketchUp: **Window → Ruby Console**.
2. `load "/full/path/to/hew_export_tags.rb"`
3. `Hew.encode_tags` — rewrites group/component names to embed their tag path.
4. **File → Export → 3D Model…**, choose **COLLADA File (.dae)**, export.
5. `Hew.decode_tags` (or just press **Ctrl/Cmd+Z once**) to restore your names.
6. Import the `.dae` into Hew — the **Tags** panel is populated from the encoded
   paths, and showing/hiding a tag shows/hides every object under it.

`encode_tags` is idempotent (safe to re-run) and wrapped in a single undoable
operation. It only touches entities with a non-default Tag.

### Encoding format (for the Hew importer/UI)

A tagged entity's name becomes:

```
<display name>__HEWTAG__<seg>__HEWSEP__<seg>...
```

- `__HEWTAG__` separates the human display name from the tag data; `__HEWSEP__`
  separates nestable tag-folder segments (root first, e.g.
  `Structure__HEWSEP__Roof`). Folder nesting needs SketchUp 2021+; older versions
  emit a single flat segment.
- SketchUp entities carry exactly one Tag, so there is one suffix per name.

**Why underscore tokens (not `@@`/`/`):** SketchUp's COLLADA exporter sanitizes
node names to `[A-Za-z0-9_]` — it turns spaces and any other character into `_`
and may prepend a `_`. So the delimiter arrives as a *run* of underscores around
the `HEWTAG`/`HEWSEP` letter tokens (e.g. `___HEWTAG__`); Hew matches them
tolerantly as `_+HEWTAG_+` / `_+HEWSEP_+`. One unavoidable consequence: a tag
**name** that contains spaces shows in Hew with underscores instead (e.g.
`Pretty Ceiling` → `Pretty_Ceiling`).

Hew strips everything from the `__HEWTAG__` delimiter on for display.
