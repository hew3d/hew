<!--
GENERATED from crates/api registry — do not edit; regenerate with:
  REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts
-->

# Hew API Reference

This is the mechanically published form of the command registry in
`crates/api/src/registry.rs` (docs/HEW_API.md §9: "published from
it"). It is the per-command companion to HEW_API.md, which defines
the protocol every command obeys; this document lists what each one
actually is. One section per namespace, one entry per command, in
registry order.

New to the API? Read docs/API_GUIDE.md first — how to connect, what
a session looks like, and worked examples of the idioms these
entries assume (transactions, `$ref`, face locators, refusals).

## hew.attr

### `hew.attr.delete`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Delete one attribute key or a whole namespace.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "key": {
      "type": "string"
    },
    "ns": {
      "type": "string"
    },
    "target": {
      "type": "string"
    }
  },
  "required": [
    "target",
    "ns"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `invalid_attr_name` — Attribute names need both a namespace and a key. Give the attribute a non-empty name and try again.
- `reserved_attr_namespace`
- `unknown_attr` — That attribute doesn't exist on this item, so there was nothing to remove.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_guide` — That guide is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_tag` — That tag doesn't exist in this model. Check the tag name and try again.

### `hew.attr.get`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

Read a target's attribute dictionaries.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ns": {
      "type": "string"
    },
    "target": {
      "type": "string"
    }
  },
  "required": [
    "target"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_guide` — That guide is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_tag` — That tag doesn't exist in this model. Check the tag name and try again.

### `hew.attr.set`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Write one attribute key.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "key": {
      "type": "string"
    },
    "ns": {
      "type": "string"
    },
    "target": {
      "type": "string"
    },
    "value": {}
  },
  "required": [
    "target",
    "ns",
    "key",
    "value"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `invalid_attr_name` — Attribute names need both a namespace and a key. Give the attribute a non-empty name and try again.
- `reserved_attr_namespace`
- `non_finite_attr_value` — Attribute numbers can't be NaN or infinity. Use a finite number and try again.
- `attr_value_too_deep` — This attribute value is nested too deeply to store. Flatten it and try again.
- `unrepresentable_attr_value`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_guide` — That guide is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_tag` — That tag doesn't exist in this model. Check the tag name and try again.

## hew.component

### `hew.component.create`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Fold a selection into a definition plus one instance.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "members": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "members"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "component": {
      "type": "string"
    },
    "instance": {
      "type": "string"
    }
  },
  "required": [
    "component",
    "instance"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `empty_component` — Select at least one object to turn into a component.
- `duplicate_member` — The same object is in the selection twice. Reselect and try again.
- `nested_component_unsupported`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.

### `hew.component.explode`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Bake an instance into world geometry.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "instance": {
      "type": "string"
    }
  },
  "required": [
    "instance"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "objects": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "objects"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `cannot_explode_reflected` — A mirrored instance can't be exploded — baking the mirror would turn the solid inside out. Use Make Unique instead.

### `hew.component.make_unique`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Deep-copy an instance's definition into a private one.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "instance": {
      "type": "string"
    }
  },
  "required": [
    "instance"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "component": {
      "type": "string"
    }
  },
  "required": [
    "component"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.

### `hew.component.place`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Place an instance of a definition at a pose.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "component": {
      "type": "string"
    },
    "pose": {}
  },
  "required": [
    "component"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "instance": {
      "type": "string"
    }
  },
  "required": [
    "instance"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `singular` — That transform would scale the object down to nothing, so it was refused.

## hew.context

### `hew.context.enter`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Open a group/component editing frame (transaction-balanced only).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `explode_session_open` — This component is already open for editing further out. Step back out to it (Escape) instead of opening it again — and close the editor before saving.
- `explode_session_nested_group` — This group is nested inside another one, so it can't be opened for editing directly. Enter its enclosing group first, then drill down to this one.
- `explode_session_pose_unsupported` — This instance's pose is scaled unevenly or mirrored, so it can't be opened for direct editing. Even out its scale and unmirror it first.
- `explode_session_grouped_instance` — A placement of this component sits inside a group, so it opens in the in-context editing mode instead.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.

### `hew.context.exit`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Close the innermost frame this envelope opened.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `explode_session_not_open` — No component is currently open for editing.

## hew.doc

### `hew.doc.attach`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** kernel

Bind this connection to one open document.

**Params schema:**

```json
{
  "properties": {
    "document": {
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "type": "object"
}
```

**Refusals:** none.

### `hew.doc.export`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host

Export the attached document — STL, 3MF, or glTF/GLB — solids only, bytes base64, or a path on hosts with filesystem access.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "format": {
      "description": "\"gltf\" is an alias for \"glb\" — every host that implements one implements both",
      "enum": [
        "stl",
        "3mf",
        "glb",
        "gltf"
      ],
      "type": "string"
    },
    "path": {
      "type": "string"
    },
    "segments_per_turn": {
      "maximum": 512,
      "minimum": 8,
      "type": "integer"
    }
  },
  "required": [
    "format"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "bytes_base64": {
      "type": "string"
    },
    "format": {
      "type": "string"
    }
  },
  "required": [
    "format"
  ],
  "type": "object"
}
```

**Refusals:**

- `export_failed`
- `host_capability_missing`
- `nothing_to_export`
- `save_failed`

### `hew.doc.import`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host

Merge a foreign-format file into the attached document through the shared healing pipeline.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "path": {
      "type": "string"
    },
    "units": {
      "enum": [
        "m",
        "mm",
        "cm",
        "in"
      ],
      "type": "string"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "report": {
      "type": "object"
    }
  },
  "required": [
    "report"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `units_required`
- `load_failed`
- `unsupported_format`

### `hew.doc.new`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host

Create a fresh document (headless hosts; live hosts advertise via capabilities).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

### `hew.doc.open`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host

Open a .hew document (headless hosts; live hosts advertise via capabilities).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "path": {
      "type": "string"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `load_failed`

### `hew.doc.save`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host

Save the attached document — written by hosts with filesystem access, bytes base64 by those without.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "path": {
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "bytes_base64": {
      "type": "string"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `path_required`
- `save_failed`

### `hew.doc.transact`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Execute commands in order, atomically, as one labeled undo entry.

**Params schema:**

```json
{
  "properties": {
    "commands": {
      "items": {
        "properties": {
          "as": {
            "type": "string"
          },
          "method": {
            "type": "string"
          },
          "params": {
            "type": "object"
          }
        },
        "required": [
          "method"
        ],
        "type": "object"
      },
      "minItems": 1,
      "type": "array"
    },
    "label": {
      "type": "string"
    }
  },
  "required": [
    "commands"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "label": {
      "type": "string"
    },
    "results": {
      "type": "array"
    }
  },
  "required": [
    "results",
    "label"
  ],
  "type": "object"
}
```

**Refusals:**

- `ref_resolution_failed`

## hew.entity

### `hew.entity.delete`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Delete an entity.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "any public id; a sketch edge id (\"edg_…\") erases just that one edge — the eraser's own kernel path (Sketch::remove_edge) — as one undo entry, rather than the whole sketch",
      "type": "string"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `delete_unsupported`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_guide` — That guide is no longer there — the model changed since it was picked. Click it again.
- `unknown_edge` — That edge is no longer there — the model changed since it was picked. Click it again.

### `hew.entity.move`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Translate (with copy/array forms) by vector or from→to points.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "copy": {
      "additionalProperties": false,
      "properties": {
        "count": {
          "maximum": 1000,
          "minimum": 1,
          "type": "integer"
        }
      },
      "type": "object"
    },
    "from": {},
    "ids": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    },
    "to": {},
    "translation": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    }
  },
  "required": [
    "ids"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `mixed_selection_unsupported`
- `sketch_copy_unsupported`
- `array_count_too_large`
- `empty_selection` — The selection has nothing visible to transform — everything in it is hidden or empty. Unhide its contents, or select something visible.
- `duplicate_member` — The same object is in the selection twice. Reselect and try again.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.

### `hew.entity.rename`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Rename an entity.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    },
    "name": {
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `rename_unsupported`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.

### `hew.entity.rotate`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Rotate about a pivot and axis by an angle.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "angle": {
      "type": "number"
    },
    "axis": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "copy": {
      "additionalProperties": false,
      "properties": {
        "count": {
          "maximum": 1000,
          "minimum": 1,
          "type": "integer"
        }
      },
      "type": "object"
    },
    "ids": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    },
    "pivot": {}
  },
  "required": [
    "ids",
    "pivot",
    "axis",
    "angle"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `mixed_selection_unsupported`
- `sketch_copy_unsupported`
- `array_count_too_large`
- `empty_selection` — The selection has nothing visible to transform — everything in it is hidden or empty. Unhide its contents, or select something visible.
- `duplicate_member` — The same object is in the selection twice. Reselect and try again.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.

### `hew.entity.scale`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Scale about an anchor with per-axis factors.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "anchor": {},
    "factors": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "ids": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "ids",
    "anchor",
    "factors"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `mixed_selection_unsupported`
- `empty_selection` — The selection has nothing visible to transform — everything in it is hidden or empty. Unhide its contents, or select something visible.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.

## hew.group

### `hew.group.create`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Group sibling nodes non-destructively.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "members": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "members"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "group": {
      "type": "string"
    }
  },
  "required": [
    "group"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `empty_group` — Select at least one object to group.
- `duplicate_member` — The same object is in the selection twice. Reselect and try again.
- `mixed_parents` — Only siblings can be grouped — everything selected must be top-level, or all inside the same group. Move them to one level first.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.

### `hew.group.explode`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Dissolve a group, re-homing its members.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.

## hew.guide

### `hew.guide.angular`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Add an angular construction guide.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "angle": {
      "description": "radians, right-handed about plane_normal",
      "type": "number"
    },
    "base_dir": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "origin": {},
    "plane_normal": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    }
  },
  "required": [
    "origin",
    "plane_normal",
    "base_dir",
    "angle"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "guide": {
      "type": "string"
    }
  },
  "required": [
    "guide"
  ],
  "type": "object"
}
```

**Refusals:**

- `degenerate_guide` — The guide needs a definite direction. Drag a little further before dropping it.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `no_such_point`

### `hew.guide.clear`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Delete all guides.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:** none.

### `hew.guide.line`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Add an infinite construction guide line.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "direction": {},
    "origin": {}
  },
  "required": [
    "origin",
    "direction"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "guide": {
      "type": "string"
    }
  },
  "required": [
    "guide"
  ],
  "type": "object"
}
```

**Refusals:**

- `degenerate_guide` — The guide needs a definite direction. Drag a little further before dropping it.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `no_such_point`

### `hew.guide.point`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Add a construction guide point.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "position": {}
  },
  "required": [
    "position"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "guide": {
      "type": "string"
    }
  },
  "required": [
    "guide"
  ],
  "type": "object"
}
```

**Refusals:**

- `degenerate_guide` — The guide needs a definite direction. Drag a little further before dropping it.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `no_such_point`

## hew.history

### `hew.history.redo`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** kernel

Redo the most recently undone entry.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `nothing_to_redo` — Nothing to redo.
- `inverse_failed` — This step couldn't be undone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.
- `inverse_diverged` — Undo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.

### `hew.history.status`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** kernel

History depth and the top entry's label and origin.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "redo_depth": {
      "type": "integer"
    },
    "top": {},
    "undo_depth": {
      "type": "integer"
    }
  },
  "required": [
    "undo_depth",
    "redo_depth",
    "top"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.history.undo`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** kernel

Undo the top history entry (optionally guarded by expected_label).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "expected_label": {
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `expected_label_mismatch`
- `nothing_to_undo` — Nothing to undo.
- `inverse_failed` — This step couldn't be undone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.
- `inverse_diverged` — Undo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.

## hew.material

### `hew.material.create`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Add a color or texture material to the palette. Registry-state: records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "color": {
      "items": {
        "maximum": 255,
        "minimum": 0,
        "type": "integer"
      },
      "maxItems": 4,
      "minItems": 3,
      "type": "array"
    },
    "name": {
      "type": "string"
    }
  },
  "required": [
    "name",
    "color"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "material": {
      "type": "string"
    }
  },
  "required": [
    "material"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.material.paint`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Paint a face or entity.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "face": {},
    "id": {
      "type": "string"
    },
    "material": {
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "material"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.
- `locator_missed`
- `ambiguous_locator`

### `hew.material.set_default`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Set an object's default material.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    },
    "material": {
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "id",
    "material"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.

### `hew.material.set_opacity`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Set a material's opacity.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "alpha": {
      "maximum": 255,
      "minimum": 0,
      "type": "integer"
    },
    "material": {
      "type": "string"
    }
  },
  "required": [
    "material",
    "alpha"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_material` — That material is no longer in the palette. Pick another swatch.

## hew.meta

### `hew.meta.capabilities`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

The registry as data: every granted command's schemas, summary, and refusal inventory.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "commands": {
      "items": {
        "properties": {
          "class": {
            "enum": [
              "model_mutating",
              "read_only",
              "solitary"
            ],
            "type": "string"
          },
          "implemented": {
            "type": "boolean"
          },
          "name": {
            "type": "string"
          },
          "params": {
            "type": "object"
          },
          "refusals": {
            "items": {
              "type": "string"
            },
            "type": "array"
          },
          "result": {
            "type": "object"
          },
          "summary": {
            "type": "string"
          },
          "version": {
            "type": "integer"
          }
        },
        "required": [
          "name",
          "version",
          "summary",
          "class",
          "params",
          "result",
          "refusals",
          "implemented"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "commands"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.meta.documents`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** host

The host's open documents.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "documents": {
      "type": "array"
    }
  },
  "required": [
    "documents"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.meta.hello`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

Open the connection: negotiate protocol and encoding, learn the granted profile and open documents.

**Params schema:**

```json
{
  "properties": {
    "client": {
      "properties": {
        "name": {
          "type": "string"
        },
        "version": {
          "type": "string"
        }
      },
      "type": "object"
    },
    "encodings": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "protocol": {
      "type": "integer"
    },
    "token": {
      "type": "string"
    }
  },
  "required": [
    "protocol"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "app": {
      "properties": {
        "name": {
          "type": "string"
        },
        "version": {
          "type": "string"
        }
      },
      "required": [
        "name",
        "version"
      ],
      "type": "object"
    },
    "documents": {
      "type": "array"
    },
    "encoding": {
      "type": "string"
    },
    "profile": {
      "enum": [
        "core",
        "app"
      ],
      "type": "string"
    },
    "protocol": {
      "type": "integer"
    }
  },
  "required": [
    "protocol",
    "app",
    "profile",
    "encoding",
    "documents"
  ],
  "type": "object"
}
```

**Refusals:** none.

## hew.query

### `hew.query.context`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

The open editing-context frame stack.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "direct_members": {},
    "stack": {
      "type": "array"
    }
  },
  "required": [
    "stack"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.query.entity`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

One entity's details.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "any public id, including a sketch's own edge id (\"edg_…\", HEW_API.md §5.2) — a sketch's `hew.query.scene`/`hew.query.entity` listing hands these out, and this command answers them directly with `{kind:\"edge\", sketch, from, to, length, curve}`",
      "type": "string"
    }
  },
  "required": [
    "id"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "id": {
      "type": "string"
    },
    "kind": {
      "type": "string"
    }
  },
  "required": [
    "kind",
    "id"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`

### `hew.query.faces`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

A solid's faces: planes, areas, centroids, boundary loops.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "object": {
      "type": "string"
    }
  },
  "required": [
    "object"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "faces": {
      "type": "array"
    },
    "object": {
      "type": "string"
    }
  },
  "required": [
    "object",
    "faces"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`

### `hew.query.measure`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

Distances and angles between points, edges, and faces.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "from": {},
    "to": {}
  },
  "required": [
    "from",
    "to"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "delta": {
      "type": "array"
    },
    "distance": {
      "type": "number"
    }
  },
  "required": [
    "distance",
    "delta"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `no_such_point`

### `hew.query.raycast`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

First hit along a ray — the programmatic form of clicking.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "dir": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "origin": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    }
  },
  "required": [
    "origin",
    "dir"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "distance": {
      "type": "number"
    },
    "kind": {
      "enum": [
        "object",
        "instance"
      ],
      "type": "string"
    },
    "normal": {
      "type": "array"
    },
    "object": {
      "description": "the world object's or, for an instance hit, the instance's public id",
      "type": "string"
    },
    "point": {
      "type": "array"
    }
  },
  "required": [
    "object",
    "kind",
    "point",
    "distance",
    "normal"
  ],
  "type": "object"
}
```

**Refusals:**

- `locator_missed`
- `ambiguous_locator`

### `hew.query.resolve`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

Resolve any locator (point, face, edge) to its concrete value without mutating.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "edge": {
      "description": "HEW_API.md §5.2's edge locator: a solid edge by {object,at}, a sketch edge's own public id (\"edg_…\") as a bare string, or a sketch edge by {sketch,at} / {sketch,from,to}"
    },
    "face": {},
    "point": {}
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "a resolved \"edge\" carries \"kind\": \"solid\" ({object,from,to}) or \"sketch\" ({id,sketch,from,to,curve})",
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `no_such_point`
- `face_token_unknown`
- `face_token_stale`

### `hew.query.scene`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel

The document tree with per-entity summaries.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "components": {
      "type": "array"
    },
    "document": {
      "type": "object"
    },
    "guides": {
      "type": "array"
    },
    "materials": {
      "type": "array"
    },
    "sketches": {
      "type": "array"
    },
    "tags": {
      "type": "array"
    },
    "tree": {
      "type": "array"
    }
  },
  "required": [
    "document",
    "tree",
    "sketches",
    "guides",
    "materials",
    "tags",
    "components"
  ],
  "type": "object"
}
```

**Refusals:** none.

## hew.sketch

### `hew.sketch.draw_arc`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Draw an arc on a plane spec.

**Params schema:**

```json
{
  "properties": {
    "center": {
      "oneOf": [
        {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        {
          "description": "a derived-point locator (HEW_API.md §5.3)",
          "type": "object"
        }
      ]
    },
    "close": {
      "description": "how the arc's ends are closed: \"open\" (default, a bare arc), \"pie\" (closed wedge — two radii to the center), or \"segment\" (closed circular segment — the chord). \"pie\"/\"segment\" commit a closed profile (a region in plane/sketch mode, a SplitFaceInner loop in face mode) like draw_rect/draw_circle, and need at least 2 segments (see \"segments\"). Must be \"open\" when the sweep is a full turn (already closed).",
      "enum": [
        "open",
        "pie",
        "segment"
      ],
      "type": "string"
    },
    "end_angle": {
      "description": "radians",
      "type": "number"
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "radius": {
      "exclusiveMinimum": 0,
      "type": "number"
    },
    "segments": {
      "description": "facet count; defaults proportionally to the sweep, capped at MAX_CIRCLE_SEGMENTS = 1024. A single chord (1) is fine for close: \"open\", but close: \"pie\"/\"segment\" needs at least 2 — a single chord can't form a non-degenerate closed loop — and the proportional default is floored at 2 for those modes too.",
      "type": "integer"
    },
    "start_angle": {
      "description": "radians",
      "type": "number"
    }
  },
  "required": [
    "plane",
    "center",
    "radius",
    "start_angle",
    "end_angle"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "oneOf": [
    {
      "properties": {
        "curve_id": {
          "description": "present when this command began a curve chain",
          "type": "string"
        },
        "region_id": {
          "description": "present when exactly one region resulted (HEW_API.md §6.1's example)",
          "type": "string"
        },
        "region_ids": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "sketch": {
          "type": "string"
        }
      },
      "required": [
        "sketch",
        "region_ids"
      ],
      "type": "object"
    },
    {
      "description": "on-face drawing (plane spec {\"face\": <locator>}) imprints the solid's face instead of creating a sketch region; mints transaction-scoped face tokens \"a\"/\"b\" (an open sweep — a plain boundary-to-boundary cut) or \"face\"/\"parent\" (a full-turn sweep, or a pie/segment close — closed, imprinted like a circle) (HEW_API.md §5.2/§5.4)",
      "properties": {
        "object_id": {
          "type": "string"
        }
      },
      "required": [
        "object_id"
      ],
      "type": "object"
    }
  ]
}
```

**Refusals:**

- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_curve` — That circle or arc is too small to keep. Drag a larger radius, or type an exact one.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `segments_above_cap` — That is more segments than a circle can hold. Enter a smaller count.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `path_too_short` — The cut needs at least two points. Click a start and an end.
- `endpoint_not_on_boundary` — A splitting line must start and end on the face's edges. Snap both ends to the face boundary.
- `path_not_simple` — The line crosses itself or touches the face's edge partway along. Draw a simple path from edge to edge.
- `loop_not_strictly_inside` — The shape must sit fully inside the face, clear of its edges. Draw it a little smaller or further from the boundary.
- `loop_self_intersects` — The shape's outline crosses itself. Draw a simple, non-crossing outline.
- `curve_claim_off_loop` — The drawn outline and its circle disagree, so the imprint was refused. Redraw the circle; if it keeps failing, use Report Bug.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `point_not_on_face` — Part of the line leaves the face. Keep every point on the face being split.
- `would_corrupt` — That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.

### `hew.sketch.draw_circle`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Draw a circle on a plane spec.

**Params schema:**

```json
{
  "properties": {
    "center": {
      "oneOf": [
        {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        {
          "description": "a derived-point locator (HEW_API.md §5.3)",
          "type": "object"
        }
      ]
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "radius": {
      "exclusiveMinimum": 0,
      "type": "number"
    },
    "segments": {
      "description": "facet count; defaults to 48, must fall in [MIN_CIRCLE_SEGMENTS, MAX_CIRCLE_SEGMENTS] = [24, 1024]",
      "type": "integer"
    }
  },
  "required": [
    "plane",
    "center",
    "radius"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "oneOf": [
    {
      "properties": {
        "curve_id": {
          "description": "present when this command began a curve chain",
          "type": "string"
        },
        "region_id": {
          "description": "present when exactly one region resulted (HEW_API.md §6.1's example)",
          "type": "string"
        },
        "region_ids": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "sketch": {
          "type": "string"
        }
      },
      "required": [
        "sketch",
        "region_ids"
      ],
      "type": "object"
    },
    {
      "description": "on-face drawing (plane spec {\"face\": <locator>}) imprints the solid's face instead of creating a sketch region; mints transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4)",
      "properties": {
        "object_id": {
          "type": "string"
        }
      },
      "required": [
        "object_id"
      ],
      "type": "object"
    }
  ]
}
```

**Refusals:**

- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_curve` — That circle or arc is too small to keep. Drag a larger radius, or type an exact one.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `segments_below_floor` — A circle needs at least 24 segments — below that it stops being a circle and becomes a polygon. Use the Polygon tool for a coarser shape.
- `segments_above_cap` — That is more segments than a circle can hold. Enter a smaller count.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `loop_not_strictly_inside` — The shape must sit fully inside the face, clear of its edges. Draw it a little smaller or further from the boundary.
- `loop_self_intersects` — The shape's outline crosses itself. Draw a simple, non-crossing outline.
- `curve_claim_off_loop` — The drawn outline and its circle disagree, so the imprint was refused. Redraw the circle; if it keeps failing, use Report Bug.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `point_not_on_face` — Part of the line leaves the face. Keep every point on the face being split.
- `would_corrupt` — That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.

### `hew.sketch.draw_line`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Draw a line (chain) on a plane spec.

**Params schema:**

```json
{
  "properties": {
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "points": {
      "items": {
        "oneOf": [
          {
            "items": {
              "type": "number"
            },
            "maxItems": 3,
            "minItems": 3,
            "type": "array"
          },
          {
            "description": "a derived-point locator (HEW_API.md §5.3)",
            "type": "object"
          }
        ]
      },
      "minItems": 2,
      "type": "array"
    }
  },
  "required": [
    "plane",
    "points"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "oneOf": [
    {
      "properties": {
        "curve_id": {
          "description": "present when this command began a curve chain",
          "type": "string"
        },
        "region_id": {
          "description": "present when exactly one region resulted (HEW_API.md §6.1's example)",
          "type": "string"
        },
        "region_ids": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "sketch": {
          "type": "string"
        }
      },
      "required": [
        "sketch",
        "region_ids"
      ],
      "type": "object"
    },
    {
      "description": "on-face drawing (plane spec {\"face\": <locator>}) imprints the solid's face instead of creating a sketch region; mints transaction-scoped face tokens \"a\"/\"b\" naming the two faces the cut produced (HEW_API.md §5.2/§5.4)",
      "properties": {
        "object_id": {
          "type": "string"
        }
      },
      "required": [
        "object_id"
      ],
      "type": "object"
    }
  ]
}
```

**Refusals:**

- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `path_too_short` — The cut needs at least two points. Click a start and an end.
- `endpoint_not_on_boundary` — A splitting line must start and end on the face's edges. Snap both ends to the face boundary.
- `path_not_simple` — The line crosses itself or touches the face's edge partway along. Draw a simple path from edge to edge.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `point_not_on_face` — Part of the line leaves the face. Keep every point on the face being split.
- `would_corrupt` — That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.

### `hew.sketch.draw_polygon`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Draw a regular N-gon on a plane spec.

**Params schema:**

```json
{
  "properties": {
    "center": {
      "oneOf": [
        {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        {
          "description": "a derived-point locator (HEW_API.md §5.3)",
          "type": "object"
        }
      ]
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "radius": {
      "exclusiveMinimum": 0,
      "type": "number"
    },
    "sides": {
      "minimum": 3,
      "type": "integer"
    }
  },
  "required": [
    "plane",
    "center",
    "radius",
    "sides"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "oneOf": [
    {
      "properties": {
        "curve_id": {
          "description": "present when this command began a curve chain",
          "type": "string"
        },
        "region_id": {
          "description": "present when exactly one region resulted (HEW_API.md §6.1's example)",
          "type": "string"
        },
        "region_ids": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "sketch": {
          "type": "string"
        }
      },
      "required": [
        "sketch",
        "region_ids"
      ],
      "type": "object"
    },
    {
      "description": "on-face drawing (plane spec {\"face\": <locator>}) imprints the solid's face instead of creating a sketch region; mints transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4)",
      "properties": {
        "object_id": {
          "type": "string"
        }
      },
      "required": [
        "object_id"
      ],
      "type": "object"
    }
  ]
}
```

**Refusals:**

- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_curve` — That circle or arc is too small to keep. Drag a larger radius, or type an exact one.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `loop_not_strictly_inside` — The shape must sit fully inside the face, clear of its edges. Draw it a little smaller or further from the boundary.
- `loop_self_intersects` — The shape's outline crosses itself. Draw a simple, non-crossing outline.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `point_not_on_face` — Part of the line leaves the face. Keep every point on the face being split.
- `would_corrupt` — That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.

### `hew.sketch.draw_rect`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Draw an axis-aligned rectangle on a plane spec.

**Params schema:**

```json
{
  "properties": {
    "corner_a": {
      "oneOf": [
        {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        {
          "description": "a derived-point locator (HEW_API.md §5.3)",
          "type": "object"
        }
      ]
    },
    "corner_b": {
      "oneOf": [
        {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        {
          "description": "a derived-point locator (HEW_API.md §5.3)",
          "type": "object"
        }
      ]
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    }
  },
  "required": [
    "plane",
    "corner_a",
    "corner_b"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "oneOf": [
    {
      "properties": {
        "curve_id": {
          "description": "present when this command began a curve chain",
          "type": "string"
        },
        "region_id": {
          "description": "present when exactly one region resulted (HEW_API.md §6.1's example)",
          "type": "string"
        },
        "region_ids": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "sketch": {
          "type": "string"
        }
      },
      "required": [
        "sketch",
        "region_ids"
      ],
      "type": "object"
    },
    {
      "description": "on-face drawing (plane spec {\"face\": <locator>}) imprints the solid's face instead of creating a sketch region; mints transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4)",
      "properties": {
        "object_id": {
          "type": "string"
        }
      },
      "required": [
        "object_id"
      ],
      "type": "object"
    }
  ]
}
```

**Refusals:**

- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `loop_not_strictly_inside` — The shape must sit fully inside the face, clear of its edges. Draw it a little smaller or further from the boundary.
- `loop_self_intersects` — The shape's outline crosses itself. Draw a simple, non-crossing outline.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `point_not_on_face` — Part of the line leaves the face. Keep every point on the face being split.
- `would_corrupt` — That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.

### `hew.sketch.offset`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Offset a region boundary within its sketch.

**Params schema:**

```json
{
  "properties": {
    "distance": {
      "description": "positive grows the material, negative shrinks it",
      "type": "number"
    },
    "region": {
      "type": "string"
    }
  },
  "required": [
    "region",
    "distance"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "curve_ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "region_id": {
      "description": "present when exactly one region resulted",
      "type": "string"
    },
    "region_ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "sketch": {
      "type": "string"
    }
  },
  "required": [
    "sketch",
    "region_ids",
    "curve_ids"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_region` — That profile is no longer there — the model changed since it was picked. Click it again.
- `malformed_region` — This profile's outline couldn't be traced. Redraw the shape; if it keeps failing, use Report Bug.
- `offset_too_small` — That offset distance is too small to make a new boundary. Drag further, or type an exact distance.
- `offset_collapsed` — The shape can't absorb that offset — its boundary would collapse, cross itself, or spike out of a sharp corner. Try a smaller distance, or soften the sharpest corner.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

## hew.solid

### `hew.solid.extrude`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Extrude a region into a new Object, consuming the profile.

**Params schema:**

```json
{
  "properties": {
    "distance": {
      "type": "number"
    },
    "region": {
      "type": "string"
    }
  },
  "required": [
    "region",
    "distance"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "mints face tokens \"base\", \"top\", \"side.<n>\" (boundary-loop order) — HEW_API.md §5.4's normative example",
  "properties": {
    "object_id": {
      "type": "string"
    }
  },
  "required": [
    "object_id"
  ],
  "type": "object"
}
```

**Refusals:**

- `distance_too_small` — That distance is too small to build anything. Drag further, or type an exact length.
- `degenerate_geometry` — This profile can't be extruded into a valid solid. Simplify the shape and try again.
- `unknown_region` — That profile is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.follow_me`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Sweep a profile along an edge-chain path, as the tool does.

**Params schema:**

```json
{
  "properties": {
    "path": {
      "oneOf": [
        {
          "description": "Reserved: no kernel mapping yet — refuses unimplemented",
          "properties": {
            "edges": {
              "type": "array"
            }
          },
          "required": [
            "edges"
          ],
          "type": "object"
        },
        {
          "properties": {
            "face": {
              "description": "HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {\"$face\":\"label#key\"}",
              "type": "object"
            }
          },
          "required": [
            "face"
          ],
          "type": "object"
        },
        {
          "properties": {
            "curve": {
              "type": "string"
            }
          },
          "required": [
            "curve"
          ],
          "type": "object"
        }
      ]
    },
    "profile": {
      "oneOf": [
        {
          "description": "a sketch region id",
          "type": "string"
        },
        {
          "properties": {
            "face": {
              "description": "HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {\"$face\":\"label#key\"}",
              "type": "object"
            }
          },
          "required": [
            "face"
          ],
          "type": "object"
        }
      ]
    }
  },
  "required": [
    "profile",
    "path"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "object_id": {
      "type": "string"
    }
  },
  "required": [
    "object_id"
  ],
  "type": "object"
}
```

**Refusals:**

- `empty_path` — Follow Me needs a path. Select or click the line, curve, or face edge to follow.
- `unknown_path_edge` — That path line is no longer there — the model changed since it was picked. Click it again.
- `path_branches` — The path forks — Follow Me needs one continuous run of lines. Select a single chain with no branches.
- `path_disconnected` — The path is in separate pieces. Select one connected run of lines.
- `path_segment_too_short` — Part of the path is too short to follow. Redraw the path without tiny segments.
- `profile_not_perpendicular` — Hew tries to stand the profile up automatically, but couldn't square it to the path here. Move the profile closer to where the sweep should start — on a drawn circle, near the rim, in line with the circle's center.
- `follow_me_in_component_unsupported`
- `path_detached_from_profile` — The path doesn't start on the profile's surface. Start the path at the profile — or, on a loop, cross the profile partway along a straight run, not at a corner.
- `path_reverses` — The path doubles back on itself — or turns nearly all the way around — so there is no clean corner to turn. Remove the reversal, or soften the corner.
- `path_too_tight` — The path turns tighter than the profile is wide, so the sweep would fold into itself. Use a smaller profile or gentler turns.
- `profile_crosses_axis` — This profile crosses the circle's centre axis, so revolving it would lose the geometry on one side. Draw the profile entirely on one side of the axis instead.
- `partial_sweep_on_pole` — A partial sweep can't cut open this shape — its profile touches the turning axis, so the pole only exists in the full revolution. Sweep the whole path, or move the profile off the axis.
- `sweep_self_intersects` — Following this path would make the shape run into itself. Shorten the path or use a smaller profile.
- `sweep_degenerate` — This profile can't be swept along that path into a valid solid. Simplify the profile or the path and try again.
- `unknown_region` — That profile is no longer there — the model changed since it was picked. Click it again.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `unimplemented`

### `hew.solid.intersect`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Boolean intersection of two solids.

**Params schema:**

```json
{
  "properties": {
    "a": {
      "description": "a node id: obj_/grp_/ins_",
      "type": "string"
    },
    "b": {
      "description": "a node id: obj_/grp_/ins_",
      "type": "string"
    }
  },
  "required": [
    "a",
    "b"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "result": {
      "type": "string"
    }
  },
  "required": [
    "result"
  ],
  "type": "object"
}
```

**Refusals:**

- `boolean_operand_has_instance` — Combining can't consume a component instance — its geometry is shared with every copy. Explode the instance (or Make Unique, then Explode) first.
- `boolean_operand_not_solid` — Something in that selection is not a watertight solid, so it can't be combined. Check each object's solid badge in Object Info and fix or remove the leaky one.
- `boolean_operand_empty` — That selection has no solids to combine. Pick a solid object or a group of solids.
- `grouped_operand` — This operation can't target an object inside a group. Ungroup it, or leave the group context, first.
- `degenerate_contact` — The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.push_pull`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Push/pull a face of a solid with the tool's full semantics.

**Params schema:**

```json
{
  "properties": {
    "distance": {
      "type": "number"
    },
    "face": {
      "description": "HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {\"$face\":\"label#key\"}",
      "type": "object"
    }
  },
  "required": [
    "face",
    "distance"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "exactly one of object_id / object_ids is present, depending on the tool's three-way branch (HEW_API.md §7 semantics notes)",
  "properties": {
    "object_id": {
      "description": "the pushed/pulled object, still standing (in-place or sub-face case)",
      "type": "string"
    },
    "object_ids": {
      "description": "a through-cut's resulting pieces, replacing the source object",
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `object_not_solid` — This object isn't a watertight solid, so it can't be pushed or pulled. Check its solid status in the Object Info panel.
- `distance_too_small` — That distance is too small to build anything. Drag further, or type an exact length.
- `would_vanish` — Pushing that far would remove the whole object. Push a shorter distance, or delete the object instead.
- `non_manifold_result` — The walls this would create run into the object's other geometry. Try a different distance, or reshape the surrounding faces first.
- `not_a_sub_face` — Push/Pull here needs a shape drawn on the face. Draw a closed outline on the face first.
- `radius_vanishes` — Pushing that far would shrink the curved wall to nothing. Push a shorter distance.
- `wall_neighbor_non_planar` — Offsetting this curved wall would bend a neighboring face out of flat, so it was refused. Adjust or simplify the touching faces first.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `grouped_operand` — This operation can't target an object inside a group. Ungroup it, or leave the group context, first.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`

### `hew.solid.slice`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Slice a solid by a plane into two solids.

**Params schema:**

```json
{
  "properties": {
    "object": {
      "type": "string"
    },
    "plane": {
      "properties": {
        "normal": {
          "oneOf": [
            {
              "items": {
                "type": "number"
              },
              "maxItems": 3,
              "minItems": 3,
              "type": "array"
            },
            {
              "description": "a derived-point locator (HEW_API.md §5.3)",
              "type": "object"
            }
          ]
        },
        "origin": {
          "oneOf": [
            {
              "items": {
                "type": "number"
              },
              "maxItems": 3,
              "minItems": 3,
              "type": "array"
            },
            {
              "description": "a derived-point locator (HEW_API.md §5.3)",
              "type": "object"
            }
          ]
        }
      },
      "required": [
        "origin",
        "normal"
      ],
      "type": "object"
    }
  },
  "required": [
    "object",
    "plane"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "negative": {
      "type": "string"
    },
    "positive": {
      "type": "string"
    }
  },
  "required": [
    "positive",
    "negative"
  ],
  "type": "object"
}
```

**Refusals:**

- `not_solid` — Only a watertight solid can be sliced. Check its solid status in the Object Info panel.
- `plane_misses_solid` — The slicing plane doesn't pass through the object. Position the cut so it goes through the solid.
- `degenerate` — The cut lines up exactly with an existing face or edge, so it wouldn't create two pieces. Move the cut slightly.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.subtract`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Boolean subtraction of two solids.

**Params schema:**

```json
{
  "properties": {
    "a": {
      "description": "a node id: obj_/grp_/ins_",
      "type": "string"
    },
    "b": {
      "description": "a node id: obj_/grp_/ins_",
      "type": "string"
    }
  },
  "required": [
    "a",
    "b"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "result": {
      "type": "string"
    }
  },
  "required": [
    "result"
  ],
  "type": "object"
}
```

**Refusals:**

- `boolean_operand_has_instance` — Combining can't consume a component instance — its geometry is shared with every copy. Explode the instance (or Make Unique, then Explode) first.
- `boolean_operand_not_solid` — Something in that selection is not a watertight solid, so it can't be combined. Check each object's solid badge in Object Info and fix or remove the leaky one.
- `boolean_operand_empty` — That selection has no solids to combine. Pick a solid object or a group of solids.
- `grouped_operand` — This operation can't target an object inside a group. Ungroup it, or leave the group context, first.
- `degenerate_contact` — The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.union`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel

Boolean union of two solids.

**Params schema:**

```json
{
  "properties": {
    "a": {
      "description": "a node id: obj_/grp_/ins_",
      "type": "string"
    },
    "b": {
      "description": "a node id: obj_/grp_/ins_",
      "type": "string"
    }
  },
  "required": [
    "a",
    "b"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "result": {
      "type": "string"
    }
  },
  "required": [
    "result"
  ],
  "type": "object"
}
```

**Refusals:**

- `boolean_operand_has_instance` — Combining can't consume a component instance — its geometry is shared with every copy. Explode the instance (or Make Unique, then Explode) first.
- `boolean_operand_not_solid` — Something in that selection is not a watertight solid, so it can't be combined. Check each object's solid badge in Object Info and fix or remove the leaky one.
- `boolean_operand_empty` — That selection has no solids to combine. Pick a solid object or a group of solids.
- `grouped_operand` — This operation can't target an object inside a group. Ungroup it, or leave the group context, first.
- `degenerate_contact` — The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

## hew.tag

### `hew.tag.assign`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Assign a tag to nodes.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    },
    "path": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    },
    "remove": {
      "type": "boolean"
    }
  },
  "required": [
    "id",
    "path"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.

### `hew.tag.create`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Register a tag path. Registry-state: records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "hidden": {
      "type": "boolean"
    },
    "path": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "tag": {
      "type": "string"
    }
  },
  "required": [
    "tag"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.tag.delete`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Delete a tag path, unassigning it everywhere.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "path": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "path"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_tag` — That tag doesn't exist in this model. Check the tag name and try again.

### `hew.tag.set_visible`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel

Toggle a tag's visibility. Registry-state: records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "path": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    },
    "visible": {
      "type": "boolean"
    }
  },
  "required": [
    "path",
    "visible"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:** none.

## hew.view

### `hew.view.camera`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host

Set the live desktop viewport's camera. A host effect on the view, not a document edit (mutates_document = false: never recorded, never resyncs the document). Headless clients pass a camera per hew.view.snapshot call instead.

**Params schema:**

```json
{
  "additionalProperties": false,
  "description": "exactly one of camera or view is required",
  "properties": {
    "camera": {
      "additionalProperties": false,
      "description": "mutually exclusive with view; identical vocabulary to hew.view.snapshot's camera",
      "properties": {
        "eye": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        "fov_deg": {
          "description": "perspective only; defaults to 35",
          "type": "number"
        },
        "projection": {
          "enum": [
            "perspective",
            "parallel"
          ],
          "type": "string"
        },
        "target": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        "up": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        }
      },
      "required": [
        "eye",
        "target"
      ],
      "type": "object"
    },
    "view": {
      "description": "a named standard view; mutually exclusive with camera",
      "enum": [
        "iso",
        "front",
        "back",
        "left",
        "right",
        "top",
        "bottom"
      ],
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

### `hew.view.snapshot`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host

Render the attached document to PNG, headless-rendered via a software rasterizer (a live host may render through its viewport instead) — bytes base64 by default, or a path on hosts with filesystem access.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "camera": {
      "additionalProperties": false,
      "description": "mutually exclusive with view",
      "properties": {
        "eye": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        "fov_deg": {
          "description": "perspective only; defaults to 35",
          "type": "number"
        },
        "projection": {
          "enum": [
            "perspective",
            "parallel"
          ],
          "type": "string"
        },
        "target": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        "up": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        }
      },
      "required": [
        "eye",
        "target"
      ],
      "type": "object"
    },
    "height": {
      "description": "defaults to 512; out-of-range values are clamped, not refused",
      "maximum": 2048,
      "minimum": 16,
      "type": "integer"
    },
    "include_ids": {
      "description": "defaults to false; when true, also returns a per-pixel id-buffer and its palette",
      "type": "boolean"
    },
    "path": {
      "description": "when given, the PNG is written here instead of returned inline, honored by hosts with filesystem access and refused typed elsewhere (mirrors hew.doc.export)",
      "type": "string"
    },
    "view": {
      "description": "a named standard view fitted to the scene bounding box; mutually exclusive with camera",
      "enum": [
        "iso",
        "front",
        "back",
        "left",
        "right",
        "top",
        "bottom"
      ],
      "type": "string"
    },
    "width": {
      "description": "defaults to 512; out-of-range values are clamped, not refused",
      "maximum": 2048,
      "minimum": 16,
      "type": "integer"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "two shapes depending on whether path was given: bytes base64 inline (default), or a path plus, when include_ids was true, a sidecar path for the id-buffer",
  "properties": {
    "height": {
      "type": "integer"
    },
    "id_buffer_base64": {
      "description": "present only when include_ids was true and path was not given: u16 little-endian per pixel, index into id_palette (0 = background)",
      "type": "string"
    },
    "id_buffer_path": {
      "description": "present only when include_ids and path were both given: \"<path>.ids.bin\", the same u16 little-endian per-pixel encoding written to disk",
      "type": "string"
    },
    "id_palette": {
      "description": "public ids; id_palette[i] is what the id-buffer (inline or on disk) reports as index i+1",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "path": {
      "description": "present only when path was given: echoes it back",
      "type": "string"
    },
    "png_base64": {
      "description": "present only when path was not given",
      "type": "string"
    },
    "width": {
      "type": "integer"
    }
  },
  "required": [
    "width",
    "height"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `nothing_to_render`
- `save_failed`

### `hew.view.units`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host

Set the app's displayed length-unit format (app/src/settings/units.ts's LengthFormat) — an app-level display PREFERENCE, never document state or file-format data.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "format": {
      "description": "metric: m, cm, mm; imperial: arch (feet+inches, e.g. 5' 3-1/8\"), frac_in (fractional inches), dec_in (decimal inches)",
      "enum": [
        "m",
        "cm",
        "mm",
        "arch",
        "frac_in",
        "dec_in"
      ],
      "type": "string"
    }
  },
  "required": [
    "format"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

### `hew.view.zoom_extents`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host

Frame all visible geometry in the live viewport (View > Zoom Extents). A view effect, not a document edit (mutates_document = false).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

