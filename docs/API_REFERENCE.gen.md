<!--
GENERATED from crates/api registry — do not edit; regenerate with:
  REGENERATE_API_ARTIFACTS=1 cargo test -p api --test generate_artifacts
-->

# Hew API Reference

This is the mechanically published form of the command registry in
`crates/api/src/registry.rs` (docs/agents/HEW_API.md §9: "published from
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
- **Permission:** attributes.write

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
      "oneOf": [
        {
          "description": "an entity's public id, or \"document\"",
          "type": "string"
        },
        {
          "additionalProperties": false,
          "properties": {
            "face": {
              "description": "a face locator: {object, at}, {object, ray}, or {\"$face\": \"label#key\"}"
            }
          },
          "required": [
            "face"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "edge": {
              "description": "a solid edge locator: {object, at}"
            }
          },
          "required": [
            "edge"
          ],
          "type": "object"
        }
      ]
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
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_edge` — That edge is no longer there — the model changed since it was picked. Click it again.
- `not_a_solid_edge`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`
- `invalid_attr_name` — Attribute names need both a namespace and a key. Give the attribute a non-empty name and try again.
- `reserved_attr_namespace`
- `attr_namespace_not_granted`
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
- **Permission:** model.read

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
      "oneOf": [
        {
          "description": "an entity's public id, or \"document\"",
          "type": "string"
        },
        {
          "additionalProperties": false,
          "properties": {
            "face": {
              "description": "a face locator: {object, at}, {object, ray}, or {\"$face\": \"label#key\"}"
            }
          },
          "required": [
            "face"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "edge": {
              "description": "a solid edge locator: {object, at}"
            }
          },
          "required": [
            "edge"
          ],
          "type": "object"
        }
      ]
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
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_edge` — That edge is no longer there — the model changed since it was picked. Click it again.
- `not_a_solid_edge`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`
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
- **Permission:** attributes.write

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
      "oneOf": [
        {
          "description": "an entity's public id, or \"document\"",
          "type": "string"
        },
        {
          "additionalProperties": false,
          "properties": {
            "face": {
              "description": "a face locator: {object, at}, {object, ray}, or {\"$face\": \"label#key\"}"
            }
          },
          "required": [
            "face"
          ],
          "type": "object"
        },
        {
          "additionalProperties": false,
          "properties": {
            "edge": {
              "description": "a solid edge locator: {object, at}"
            }
          },
          "required": [
            "edge"
          ],
          "type": "object"
        }
      ]
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
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_edge` — That edge is no longer there — the model changed since it was picked. Click it again.
- `not_a_solid_edge`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`
- `invalid_attr_name` — Attribute names need both a namespace and a key. Give the attribute a non-empty name and try again.
- `reserved_attr_namespace`
- `attr_namespace_not_granted`
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
- **Permission:** model.write

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
- **Permission:** model.write

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

### `hew.component.flip_instance`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Mirror one world instance across one of its definition's local axes.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "axis": {
      "description": "the definition-local axis to mirror across (the plane normal to it, through the local origin)",
      "enum": [
        "x",
        "y",
        "z"
      ],
      "type": "string"
    },
    "instance": {
      "description": "a world instance (\"ins_…\")",
      "type": "string"
    }
  },
  "required": [
    "instance",
    "axis"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "instance": {
      "type": "string"
    },
    "mirrored": {
      "description": "whether the new pose's determinant is negative",
      "type": "boolean"
    },
    "pose": {
      "description": "the new pose, row-major 3×4",
      "items": {
        "type": "number"
      },
      "maxItems": 12,
      "minItems": 12,
      "type": "array"
    }
  },
  "required": [
    "instance",
    "pose",
    "mirrored"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`

### `hew.component.make_unique`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** model.write

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

### `hew.component.rebase`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Re-express a definition in a new frame; every placement is compensated, so nothing moves on screen.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "a definition (\"cmp_…\"), or an instance (\"ins_…\") meaning its definition",
      "type": "string"
    },
    "origin": {
      "description": "the new origin, in the definition's current local coordinates",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "x": {
      "description": "the new x axis, unit length, in the definition's current local coordinates",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "y": {
      "description": "the new y axis, unit length and perpendicular to x; z is x × y",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    }
  },
  "required": [
    "id",
    "origin",
    "x",
    "y"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "component": {
      "type": "string"
    },
    "instances_updated": {
      "description": "how many instances place the definition, world or nested in another definition; each pose was compensated",
      "minimum": 0,
      "type": "integer"
    }
  },
  "required": [
    "component",
    "instances_updated"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `invalid_frame` — That frame is not a valid right-handed set of axes. Give three unit-length, mutually perpendicular directions.
- `rebase_during_session` — Finish editing the component before changing its axes.

## hew.context

### `hew.context.enter`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** model.write

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
- **Permission:** always

Bind this connection to one open document, named by `document` (id, path, label, or title; see hew.meta.documents). With no `document`, the connection's current document.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "document": {
      "description": "a document id (doc_N), or its exact path, label, or title when unambiguous",
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "document": {
      "properties": {
        "attached": {
          "description": "this is the connection's current document",
          "type": "boolean"
        },
        "dirty": {
          "type": "boolean"
        },
        "document_id": {
          "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
          "type": [
            "string",
            "null"
          ]
        },
        "front": {
          "description": "this is the user's front-most document window",
          "type": "boolean"
        },
        "id": {
          "description": "doc_N, unique for the host's lifetime and never reused",
          "type": "string"
        },
        "label": {
          "description": "the client-chosen label given at hew.doc.new/open, if any",
          "type": "string"
        },
        "path": {
          "description": "the file last opened from or saved to; null while unsaved",
          "type": [
            "string",
            "null"
          ]
        },
        "title": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "title",
        "path",
        "dirty",
        "document_id"
      ],
      "type": "object"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `unknown_document`
- `ambiguous_document`
- `document_not_granted`

### `hew.doc.close`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** documents.discard

Close an open document window (the attached one, or the named `document`). Refuses unsaved changes unless `discard` is set.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "discard": {
      "description": "close even with unsaved changes (they are lost); default false",
      "type": "boolean"
    },
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
  "properties": {
    "closed": {
      "properties": {
        "attached": {
          "description": "this is the connection's current document",
          "type": "boolean"
        },
        "dirty": {
          "type": "boolean"
        },
        "document_id": {
          "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
          "type": [
            "string",
            "null"
          ]
        },
        "front": {
          "description": "this is the user's front-most document window",
          "type": "boolean"
        },
        "id": {
          "description": "doc_N, unique for the host's lifetime and never reused",
          "type": "string"
        },
        "label": {
          "description": "the client-chosen label given at hew.doc.new/open, if any",
          "type": "string"
        },
        "path": {
          "description": "the file last opened from or saved to; null while unsaved",
          "type": [
            "string",
            "null"
          ]
        },
        "title": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "title",
        "path",
        "dirty",
        "document_id"
      ],
      "type": "object"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_document`
- `ambiguous_document`
- `unsaved_changes`
- `last_document`
- `document_busy` — A plugin is still working on this model. Wait for it to finish, or cancel it from the notice at the top of the window.

### `hew.doc.export`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host
- **Permission:** documents.save

Export the attached document — STL, 3MF, glTF/GLB, USDZ (solids only), or a SketchUp .skp ("skp", 2026 container; "skp2017") — bytes base64, or a path on hosts with filesystem access.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "format": {
      "description": "\"gltf\" is an alias for \"glb\" — every host that implements one implements both. \"skp\" is the SketchUp 2026 container, \"skp2017\" the 2017 format; exporting never changes the document's own file",
      "enum": [
        "stl",
        "3mf",
        "glb",
        "gltf",
        "usdz",
        "skp",
        "skp2017"
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
    },
    "selection": {
      "description": "draw only these: node ids, occurrence locators {path}, or a definition's cmp_ id (its leaves at the identity pose)",
      "items": {
        "oneOf": [
          {
            "type": "string"
          },
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
        ]
      },
      "minItems": 1,
      "type": "array"
    },
    "units": {
      "description": "SketchUp formats only: the display units the file opens in (hew.view.units's formats). Absent: the host's display units (the app's preference live; meters headless).",
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
- `unknown_entity`
- `unknown_occurrence`
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.doc.focus`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** documents.other

Raise a document's window to the front. Every other command leaves the user's focus alone; this is the explicit opt-in.

**Params schema:**

```json
{
  "additionalProperties": false,
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
  "properties": {
    "document": {
      "properties": {
        "attached": {
          "description": "this is the connection's current document",
          "type": "boolean"
        },
        "dirty": {
          "type": "boolean"
        },
        "document_id": {
          "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
          "type": [
            "string",
            "null"
          ]
        },
        "front": {
          "description": "this is the user's front-most document window",
          "type": "boolean"
        },
        "id": {
          "description": "doc_N, unique for the host's lifetime and never reused",
          "type": "string"
        },
        "label": {
          "description": "the client-chosen label given at hew.doc.new/open, if any",
          "type": "string"
        },
        "path": {
          "description": "the file last opened from or saved to; null while unsaved",
          "type": [
            "string",
            "null"
          ]
        },
        "title": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "title",
        "path",
        "dirty",
        "document_id"
      ],
      "type": "object"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_document`
- `ambiguous_document`

### `hew.doc.health`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

Model health: every open object (world or component member) with its verdict, open rims and seam family, and counts by kind.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "objects": {
      "description": "obj_ ids to diagnose; omitted = every live object, component members included",
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "counts": {
      "description": "objects, solids, flat_sheets, bent_sheets, cappable, holes, mixed, non_manifold, seam_families",
      "type": "object"
    },
    "families": {
      "description": "objects an importer split from one shape, joined by shared rims",
      "items": {
        "properties": {
          "members": {
            "items": {
              "type": "string"
            },
            "type": "array"
          },
          "shared_rims": {
            "type": "integer"
          }
        },
        "type": "object"
      },
      "type": "array"
    },
    "rows": {
      "description": "one per open (non-solid) object; a component member is one row however often it is placed",
      "items": {
        "properties": {
          "accepted": {
            "description": "marked as a surface with hew.solid.mark_surface",
            "type": "boolean"
          },
          "branching_vertices": {
            "type": "integer"
          },
          "definition": {
            "description": "cmp_ id when the object is a component member",
            "type": [
              "string",
              "null"
            ]
          },
          "definition_name": {
            "type": [
              "string",
              "null"
            ]
          },
          "faces": {
            "description": "how many faces the object has",
            "type": "integer"
          },
          "family": {
            "description": "index into families",
            "type": [
              "integer",
              "null"
            ]
          },
          "instances": {
            "description": "outermost ins_ ids that place the member",
            "items": {
              "type": "string"
            },
            "type": "array"
          },
          "islands": {
            "type": "integer"
          },
          "name": {
            "type": [
              "string",
              "null"
            ]
          },
          "object": {
            "type": "string"
          },
          "placements": {
            "type": "integer"
          },
          "rims": {
            "description": "open rims; their order is what hew.solid.cap's rims index",
            "items": {
              "properties": {
                "cappable": {
                  "description": "a face can close it: closed, planar, not the edge of a flat part of the surface, no branching vertex",
                  "type": "boolean"
                },
                "closed": {
                  "type": "boolean"
                },
                "edges": {
                  "type": "integer"
                },
                "extent": {
                  "type": "number"
                },
                "perimeter": {
                  "type": "number"
                },
                "planar": {
                  "type": "boolean"
                },
                "role": {
                  "enum": [
                    "outer",
                    "hole"
                  ],
                  "type": "string"
                },
                "shared": {
                  "description": "another row's rim coincides with it: a seam between pieces of one shape the importer split (see family)",
                  "type": "boolean"
                },
                "sheet_edge": {
                  "description": "planar, but runs along the edge of a flat part of the surface on its plane (a basin's lip), so a face there would lie on that part's back",
                  "type": "boolean"
                }
              },
              "type": "object"
            },
            "type": "array"
          },
          "verdict": {
            "properties": {
              "holes": {
                "description": "flat_sheet: holes in the sheet",
                "type": "integer"
              },
              "kind": {
                "enum": [
                  "flat_sheet",
                  "bent_sheet",
                  "cappable",
                  "holes",
                  "mixed",
                  "non_manifold"
                ],
                "type": "string"
              },
              "planar": {
                "description": "holes: how many are flat (a face can close each)",
                "type": "integer"
              },
              "warped": {
                "description": "holes: how many are not flat",
                "type": "integer"
              }
            },
            "required": [
              "kind"
            ],
            "type": "object"
          }
        },
        "required": [
          "object",
          "verdict",
          "rims",
          "accepted",
          "placements"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "rows",
    "families",
    "counts"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`

### `hew.doc.import`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** documents.save

Merge a foreign-format file (STL, glTF/GLB, COLLADA, SketchUp) into the attached document through the shared healing pipeline, or place a PNG/JPEG picture (a component holding one face painted with it, at origin/u/v, width meters wide). Additive: existing geometry is untouched.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "bytes_base64": {
      "description": "the file's bytes, for a host with no filesystem (the desktop app); `path` then only names the file. hew-cli supplies this for you",
      "type": "string"
    },
    "images": {
      "additionalProperties": {
        "type": "object"
      },
      "description": "COLLADA texture images by the uri the file uses: {bytes_base64, format: png|jpeg}",
      "type": "object"
    },
    "origin": {
      "description": "a picture's (.png/.jpg/.jpeg) lower-left corner, meters; default [0,0,0]",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "path": {
      "type": "string"
    },
    "u": {
      "description": "a picture's width direction; default [1,0,0]",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "units": {
      "enum": [
        "m",
        "mm",
        "cm",
        "in"
      ],
      "type": "string"
    },
    "v": {
      "description": "toward a picture's top (not parallel to u); default [0,1,0]",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "width": {
      "description": "a picture's width in meters (its height follows from its proportions); default 1",
      "exclusiveMinimum": 0,
      "type": "number"
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
- **Permission:** documents.other

Create a fresh document. The desktop app opens it in a new window — behind the user's current one unless `focus` says otherwise — and attaches the connection to it; headless hosts replace the working document.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "focus": {
      "description": "where the new window goes; background (the default) never takes the user's focus",
      "enum": [
        "background",
        "front"
      ],
      "type": "string"
    },
    "label": {
      "description": "a name to find the document by later, resolvable by hew.doc.attach and listed by hew.meta.documents",
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "document": {
      "properties": {
        "attached": {
          "description": "this is the connection's current document",
          "type": "boolean"
        },
        "dirty": {
          "type": "boolean"
        },
        "document_id": {
          "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
          "type": [
            "string",
            "null"
          ]
        },
        "front": {
          "description": "this is the user's front-most document window",
          "type": "boolean"
        },
        "id": {
          "description": "doc_N, unique for the host's lifetime and never reused",
          "type": "string"
        },
        "label": {
          "description": "the client-chosen label given at hew.doc.new/open, if any",
          "type": "string"
        },
        "path": {
          "description": "the file last opened from or saved to; null while unsaved",
          "type": [
            "string",
            "null"
          ]
        },
        "title": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "title",
        "path",
        "dirty",
        "document_id"
      ],
      "type": "object"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `window_not_ready`
- `document_busy` — A plugin is still working on this model. Wait for it to finish, or cancel it from the notice at the top of the window.

### `hew.doc.open`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host
- **Permission:** documents.other

Open a .hew or SketchUp .skp document. The desktop app opens it in a new window (or attaches to the window that already has that file open), behind the user's current window unless `focus` says otherwise; headless hosts replace the working document.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "focus": {
      "description": "where the new window goes; background (the default) never takes the user's focus",
      "enum": [
        "background",
        "front"
      ],
      "type": "string"
    },
    "label": {
      "description": "a name to find the document by later, resolvable by hew.doc.attach and listed by hew.meta.documents",
      "type": "string"
    },
    "path": {
      "description": "a .hew file, or a SketchUp .skp file (either container) when the extension is .skp; a later hew.doc.save with no format writes the same container back",
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
    "already_open": {
      "description": "the file was already open in a window; the connection was attached to it instead of opening a second copy",
      "type": "boolean"
    },
    "document": {
      "properties": {
        "attached": {
          "description": "this is the connection's current document",
          "type": "boolean"
        },
        "dirty": {
          "type": "boolean"
        },
        "document_id": {
          "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
          "type": [
            "string",
            "null"
          ]
        },
        "front": {
          "description": "this is the user's front-most document window",
          "type": "boolean"
        },
        "id": {
          "description": "doc_N, unique for the host's lifetime and never reused",
          "type": "string"
        },
        "label": {
          "description": "the client-chosen label given at hew.doc.new/open, if any",
          "type": "string"
        },
        "path": {
          "description": "the file last opened from or saved to; null while unsaved",
          "type": [
            "string",
            "null"
          ]
        },
        "title": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "title",
        "path",
        "dirty",
        "document_id"
      ],
      "type": "object"
    }
  },
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `load_failed`
- `window_not_ready`
- `document_busy` — A plugin is still working on this model. Wait for it to finish, or cancel it from the notice at the top of the window.

### `hew.doc.purge_unused`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Delete every unused material and component definition, as one undo entry.

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
  "properties": {
    "definitions": {
      "type": "integer"
    },
    "materials": {
      "type": "integer"
    }
  },
  "required": [
    "materials",
    "definitions"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.doc.revert`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** documents.discard

Revert to Saved: re-read the document's saved file in place, discarding every change since the last save. The undo and redo history is cleared and the document reads clean. Refuses `nothing_to_revert` when the document has no saved file or has no changes since the last save. There is no confirmation step: the caller is deliberate.

**Params schema:**

```json
{
  "additionalProperties": false,
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
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `load_failed`
- `nothing_to_revert`
- `unknown_document`
- `ambiguous_document`
- `document_busy` — A plugin is still working on this model. Wait for it to finish, or cancel it from the notice at the top of the window.

### `hew.doc.save`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** host
- **Permission:** documents.save

Save the attached document (Save / Save As) as .hew or SketchUp .skp: the file is written, the unsaved mark cleared, and the document bound to it. `copy` writes a copy without any of that. With no `path` it re-saves in place; a document with no file yet, in a host that cannot ask where, hands the bytes back instead.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "copy": {
      "description": "write a copy to `path` — the document is not rebound to it and keeps its unsaved mark; requires `path`",
      "type": "boolean"
    },
    "format": {
      "description": "\"skp\" is the 2026 container (what current SketchUp saves), \"skp2017\" the 2017 format (opens in SketchUp Make 2017 and later). Absent: a .skp path keeps the working file's own .skp container (the 2026 one for a document that did not come from a .skp), any other path writes .hew, and no path uses the working file's own format. A format that contradicts the path's extension is refused (format_mismatch).",
      "enum": [
        "hew",
        "skp",
        "skp2017"
      ],
      "type": "string"
    },
    "path": {
      "type": "string"
    },
    "return_bytes": {
      "description": "hand the document's bytes back and write nothing: no file, no binding, no saved mark; takes no `path`",
      "type": "boolean"
    },
    "units": {
      "description": "The display units a SketchUp file opens in (hew.view.units's formats), a save back over a .skp included. Absent: a save back keeps the file's own units and a new file takes the host's display units (the app's preference live; meters headless). A .hew ignores it.",
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
    },
    "format": {
      "description": "the format the document was saved in",
      "enum": [
        "hew",
        "skp",
        "skp2017"
      ],
      "type": "string"
    },
    "simplified": {
      "description": "present for a .skp: what the file could not hold exactly",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "writer_notes": {
      "description": "present for a .skp: what the 2017 writer dropped",
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

- `file_open_elsewhere`
- `format_mismatch`
- `host_capability_missing`
- `path_required`
- `save_failed`

### `hew.doc.transact`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** always

Execute commands in order, atomically, as one labeled undo entry.

**Params schema:**

```json
{
  "properties": {
    "coalesce": {
      "description": "merge into the top undo entry when it was committed under the same key by a connection with this connection's identity (docs/agents/HEW_API.md §6.5)",
      "maxLength": 64,
      "minLength": 1,
      "type": "string"
    },
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
    "expect_generation": {
      "description": "refuse with generation_moved, before any command runs, unless the document's history generation is this value (docs/agents/HEW_API.md §6.5)",
      "minimum": 0,
      "type": "integer"
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
- `generation_moved`

## hew.entity

### `hew.entity.delete`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_guide` — That guide is no longer there — the model changed since it was picked. Click it again.
- `unknown_edge` — That edge is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `definition_nested_in_definition` — This component is used inside another component's definition. Make that component's instances unique or explode them first, or delete the outer component.
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.

### `hew.entity.move`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
          "description": "copies at step, 2·step, …, count·step along the translation",
          "maximum": 1000,
          "minimum": 1,
          "type": "integer"
        },
        "grid": {
          "description": "a grid copy: one entry per axis; count is the number of slots along it, the original's included, so [{count: 4, step}] makes three copies. Takes the place of translation/from/to and of copy.count; the product of counts is at most 1000",
          "items": {
            "additionalProperties": false,
            "properties": {
              "count": {
                "minimum": 1,
                "type": "integer"
              },
              "step": {
                "items": {
                  "type": "number"
                },
                "maxItems": 3,
                "minItems": 3,
                "type": "array"
              }
            },
            "required": [
              "count",
              "step"
            ],
            "type": "object"
          },
          "maxItems": 3,
          "minItems": 1,
          "type": "array"
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
      "description": "the copies, placement-major, each placement's block in ids order",
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
- **Permission:** model.write

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
- `unknown_material` — That material is no longer in the palette. Pick another swatch.

### `hew.entity.rotate`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** model.write

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

### `hew.entity.set_frame`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Set or clear the part frame override (hew.part / frame) of an object or group.

**Params schema:**

```json
{
  "additionalProperties": false,
  "description": "origin, x, and y are coordinates in the face frame hew.query.part_frame reports for the part without an override (after its [length, width, thickness] swap): origin an offset along that frame's axes, x and y unit, perpendicular directions in it. Give all three, or clear: true.",
  "properties": {
    "clear": {
      "description": "true removes the override; give it alone",
      "type": "boolean"
    },
    "id": {
      "description": "an object (\"obj_…\") or group (\"grp_…\")",
      "type": "string"
    },
    "origin": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "x": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "y": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
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
  "properties": {
    "id": {
      "type": "string"
    },
    "source": {
      "description": "where hew.query.part_frame now takes the frame from",
      "enum": [
        "attribute",
        "face"
      ],
      "type": "string"
    }
  },
  "required": [
    "id",
    "source"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `not_a_part`
- `invalid_frame` — That frame is not a valid right-handed set of axes. Give three unit-length, mutually perpendicular directions.

## hew.event

### `hew.event.subscribe`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** always

Receive the named events on this connection as notification frames. Names the grant may not receive come back in `denied`.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "events": {
      "items": {
        "enum": [
          "document_changed",
          "selection_changed",
          "context_changed",
          "document_will_save",
          "document_saved",
          "document_opened",
          "document_closed",
          "units_changed",
          "scenes_changed",
          "tool_changed",
          "plugin_state",
          "slot_changed",
          "settings_changed",
          "ui_cancel",
          "window_closed",
          "surface_closed"
        ],
        "type": "string"
      },
      "maxItems": 32,
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "events"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "denied": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "subscribed": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "subscribed",
    "denied"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.event.unsubscribe`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** always

Stop receiving the named events on this connection.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "events": {
      "items": {
        "enum": [
          "document_changed",
          "selection_changed",
          "context_changed",
          "document_will_save",
          "document_saved",
          "document_opened",
          "document_closed",
          "units_changed",
          "scenes_changed",
          "tool_changed",
          "plugin_state",
          "slot_changed",
          "settings_changed",
          "ui_cancel",
          "window_closed",
          "surface_closed"
        ],
        "type": "string"
      },
      "maxItems": 32,
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "events"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "denied": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "subscribed": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "subscribed",
    "denied"
  ],
  "type": "object"
}
```

**Refusals:** none.

## hew.group

### `hew.group.create`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** model.write

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

### `hew.group.reparent`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Move live nodes into a group, or out to the top level, without touching their geometry.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "parent": {
      "type": [
        "string",
        "null"
      ]
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
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `empty_ids`
- `group_cycle` — A group can't be moved into itself or into one of its own members. Pick a different group, or move it out to the top level.
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.

## hew.guide

### `hew.guide.angular`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** model.write

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
- **Permission:** model.write

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
- **Permission:** model.write

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
- **Permission:** model.write

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
- `inverse_failed` — This step couldn't be undone or redone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.
- `inverse_diverged` — Undo or redo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.

### `hew.history.status`

- **Version:** 1
- **Tier:** Required
- **Class:** solitary
- **Served:** kernel
- **Permission:** model.read

History depth, the top entry's label and origin, the saved depth, and every undo/redo entry's label and origin.

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
    "entries": {
      "properties": {
        "redo": {
          "items": {
            "properties": {
              "bookkeeping": {
                "type": "boolean"
              },
              "label": {
                "type": "string"
              },
              "origin": {}
            },
            "required": [
              "label",
              "origin",
              "bookkeeping"
            ],
            "type": "object"
          },
          "type": "array"
        },
        "undo": {
          "items": {
            "properties": {
              "bookkeeping": {
                "type": "boolean"
              },
              "label": {
                "type": "string"
              },
              "origin": {}
            },
            "required": [
              "label",
              "origin",
              "bookkeeping"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "undo",
        "redo"
      ],
      "type": "object"
    },
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "redo_depth": {
      "type": "integer"
    },
    "saved_depth": {},
    "top": {},
    "undo_depth": {
      "type": "integer"
    }
  },
  "required": [
    "undo_depth",
    "redo_depth",
    "top",
    "saved_depth",
    "entries",
    "generation",
    "epoch"
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
- **Permission:** model.write

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
- `inverse_failed` — This step couldn't be undone or redone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.
- `inverse_diverged` — Undo or redo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.

## hew.library

### `hew.library.describe`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** model.read

One library item's full metadata, manifest summary, and document attributes.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "item": {
      "description": "the item's id or relative path",
      "type": "string"
    }
  },
  "required": [
    "item"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "attrs": {
      "description": "the item's full document attribute dictionaries, namespace -> key -> value",
      "type": "object"
    },
    "category": {
      "enum": [
        "component",
        "material",
        "model"
      ],
      "type": "string"
    },
    "collection": {
      "type": "string"
    },
    "error": {
      "description": "present only when the file failed to parse",
      "type": "string"
    },
    "id": {
      "type": "string"
    },
    "keywords": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "mtime_ms": {
      "type": "integer"
    },
    "name": {
      "type": "string"
    },
    "path": {
      "type": "string"
    },
    "saved_at": {
      "type": "string"
    },
    "size": {
      "type": "integer"
    },
    "summary": {
      "description": "manifest counts (objects, materials, components, ...); absent when error is present",
      "type": "object"
    }
  },
  "required": [
    "path",
    "name",
    "category",
    "keywords",
    "size",
    "mtime_ms",
    "attrs"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_library_item`

### `hew.library.insert`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** library

Insert a library item into the attached document, wired for idempotent re-insert exactly like the UI's own placement; a material item goes into the palette instead, recording no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "at": {
      "description": "a translation applied to the item's world roots; identity when omitted",
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
          "type": "object"
        }
      ]
    },
    "bytes_base64": {
      "description": "the item's raw .hew bytes — exactly one of item/bytes_base64",
      "type": "string"
    },
    "content_hash": {
      "description": "sha256 hex of bytes_base64, for idempotent re-insert matching; ignored (and provenance is skipped) without it",
      "type": "string"
    },
    "item": {
      "description": "the item's id or relative path — exactly one of item/bytes_base64",
      "type": "string"
    },
    "name": {
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "annotations_skipped": {
      "type": "integer"
    },
    "definitions_added": {
      "type": "integer"
    },
    "definitions_reused": {
      "type": "integer"
    },
    "guides_added": {
      "type": "integer"
    },
    "materials": {
      "description": "a material item only: its palette materials, added or reused, in item order",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "materials_added": {
      "type": "integer"
    },
    "materials_reused": {
      "type": "integer"
    },
    "objects_added": {
      "type": "integer"
    },
    "roots": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "world_sketches_skipped": {
      "type": "integer"
    }
  },
  "required": [
    "roots",
    "definitions_added",
    "definitions_reused",
    "materials_added",
    "materials_reused",
    "objects_added",
    "guides_added",
    "world_sketches_skipped",
    "annotations_skipped"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_library_item`
- `load_failed`
- `component_expansion_exceeded` — That would multiply past a million rendered component parts. Reduce how many copies the nested components repeat — explode or thin out a level — and try again.
- `singular` — That transform would scale the object down to nothing, so it was refused.
- `degenerate_axis` — The rotation axis needs two distinct points. Pick a second point further from the first.
- `reflection` — This would turn the object inside out (a mirror), which can't be baked into a solid. Mirror a component instance instead.
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.

### `hew.library.list`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** model.read

List items in the Hew Library, optionally filtered by category, collection, or a text query.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "category": {
      "enum": [
        "component",
        "material",
        "model"
      ],
      "type": "string"
    },
    "collection": {
      "description": "matches this collection path and its subtree",
      "type": "string"
    },
    "query": {
      "description": "case-insensitive substring match against name and keywords",
      "type": "string"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "folder": {
      "type": "string"
    },
    "items": {
      "items": {
        "properties": {
          "category": {
            "enum": [
              "component",
              "material",
              "model"
            ],
            "type": "string"
          },
          "collection": {
            "type": "string"
          },
          "error": {
            "description": "present only when the file failed to parse",
            "type": "string"
          },
          "id": {
            "type": "string"
          },
          "keywords": {
            "items": {
              "type": "string"
            },
            "type": "array"
          },
          "mtime_ms": {
            "type": "integer"
          },
          "name": {
            "type": "string"
          },
          "path": {
            "type": "string"
          },
          "saved_at": {
            "type": "string"
          },
          "size": {
            "type": "integer"
          },
          "summary": {
            "description": "manifest counts (objects, materials, components, ...); absent when error is present",
            "type": "object"
          }
        },
        "required": [
          "path",
          "name",
          "category",
          "keywords",
          "size",
          "mtime_ms"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "folder",
    "items"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

### `hew.library.remove`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** library

Delete a library item.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "item": {
      "description": "the item's id or relative path",
      "type": "string"
    }
  },
  "required": [
    "item"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "removed": {
      "type": "string"
    }
  },
  "required": [
    "removed"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_library_item`

### `hew.library.save`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** library

Save a selection, one material, or the whole document to the library. Records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "category": {
      "description": "defaults to component (a selection), material (a material), or model (the whole document)",
      "enum": [
        "component",
        "material",
        "model"
      ],
      "type": "string"
    },
    "collection": {
      "type": "string"
    },
    "keywords": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "material": {
      "description": "one palette material to save alone as a material item, as the palette's Save to Library does — not with selection",
      "type": "string"
    },
    "name": {
      "type": "string"
    },
    "return_bytes": {
      "description": "also return the saved item's bytes base64",
      "type": "boolean"
    },
    "selection": {
      "description": "node ids to save as a component item; omitted saves the whole document as a model item",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "source_doc": {
      "description": "display-only \"saved from\" bookkeeping; defaults to the host's current document path when known (e.g. hew-cli --file), else omitted",
      "type": "string"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "bytes_base64": {
      "description": "present only when return_bytes was true",
      "type": "string"
    },
    "id": {
      "type": "string"
    },
    "path": {
      "type": "string"
    }
  },
  "required": [
    "path",
    "id"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_entity`
- `empty_component` — Select at least one object to turn into a component.
- `duplicate_member` — The same object is in the selection twice. Reselect and try again.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.
- `save_failed`

### `hew.library.update_meta`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** library

Edit a library item's name, keywords, or collection in place.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "collection": {
      "type": "string"
    },
    "item": {
      "description": "the item's id or relative path",
      "type": "string"
    },
    "keywords": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "name": {
      "type": "string"
    }
  },
  "required": [
    "item"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "category": {
      "enum": [
        "component",
        "material",
        "model"
      ],
      "type": "string"
    },
    "collection": {
      "type": "string"
    },
    "id": {
      "type": "string"
    },
    "keywords": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "name": {
      "type": "string"
    },
    "saved_at": {
      "type": "string"
    }
  },
  "required": [
    "name",
    "category",
    "keywords"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `unknown_library_item`
- `save_failed`

## hew.material

### `hew.material.create`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Add a color or texture material to the palette. Registry-state: records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "color": {
      "description": "required without a texture; with one it tints the image (default white: the image as-is)",
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
    },
    "texture": {
      "additionalProperties": false,
      "description": "an image texture: a PNG or JPEG, inline or a file path (hew-cli reads it), and the meters one tile covers",
      "properties": {
        "bytes_base64": {
          "description": "the encoded PNG/JPEG — exactly one of bytes_base64/path",
          "type": "string"
        },
        "path": {
          "description": "an image file, read by hew-cli — exactly one of bytes_base64/path",
          "type": "string"
        },
        "world_size": {
          "description": "[width, height] in meters of one tile",
          "items": {
            "exclusiveMinimum": 0,
            "type": "number"
          },
          "maxItems": 2,
          "minItems": 2,
          "type": "array"
        }
      },
      "required": [
        "world_size"
      ],
      "type": "object"
    }
  },
  "required": [
    "name"
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

**Refusals:**

- `load_failed`
- `host_capability_missing`

### `hew.material.paint`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Paint a face or entity.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "face": {},
    "id": {
      "description": "an object (its base material), an instance (the placement's own material), or a definition (every object it owns)",
      "type": "string"
    },
    "material": {
      "type": [
        "string",
        "null"
      ]
    },
    "scope": {
      "description": "for an instance id: the placement (default) or its definition; a definition id takes only \"definition\"",
      "enum": [
        "instance",
        "definition"
      ],
      "type": "string"
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
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `unknown_material` — That material is no longer in the palette. Pick another swatch.
- `locator_missed`
- `ambiguous_locator`

### `hew.material.position_texture`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Position a face's texture by three tile-corner pins, or reset it to the default projection.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "face": {
      "description": "a face locator (§5.2)"
    },
    "pins": {
      "additionalProperties": false,
      "description": "three corners of one texture tile, in the face's object frame (as a locator's `at`), projected onto the face: origin, one tile along the width (u), one tile along the height (v) — exactly one of pins/reset",
      "properties": {
        "origin": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        "u": {
          "items": {
            "type": "number"
          },
          "maxItems": 3,
          "minItems": 3,
          "type": "array"
        },
        "v": {
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
        "u",
        "v"
      ],
      "type": "object"
    },
    "reset": {
      "description": "true: back to the default projection at the material's tile size — exactly one of pins/reset",
      "type": "boolean"
    }
  },
  "required": [
    "face"
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
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`
- `degenerate_pins`

### `hew.material.replace`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Replace one material with another everywhere it is painted, or within one object.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "from": {
      "description": "the material to replace; null: unpainted object bases",
      "type": [
        "string",
        "null"
      ]
    },
    "object": {
      "description": "only within this object; omitted: the whole document, instance materials included",
      "type": "string"
    },
    "to": {
      "description": "the material it becomes; null: unpaint",
      "type": [
        "string",
        "null"
      ]
    }
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
  "additionalProperties": false,
  "properties": {},
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.

### `hew.material.set_default`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** model.write

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
- **Permission:** always

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
- **Permission:** model.read

The host's open documents: id, title, path, dirty, and — in the desktop app — label, front, and attached.

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
      "items": {
        "properties": {
          "attached": {
            "description": "this is the connection's current document",
            "type": "boolean"
          },
          "dirty": {
            "type": "boolean"
          },
          "document_id": {
            "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
            "type": [
              "string",
              "null"
            ]
          },
          "front": {
            "description": "this is the user's front-most document window",
            "type": "boolean"
          },
          "id": {
            "description": "doc_N, unique for the host's lifetime and never reused",
            "type": "string"
          },
          "label": {
            "description": "the client-chosen label given at hew.doc.new/open, if any",
            "type": "string"
          },
          "path": {
            "description": "the file last opened from or saved to; null while unsaved",
            "type": [
              "string",
              "null"
            ]
          },
          "title": {
            "type": "string"
          }
        },
        "required": [
          "id",
          "title",
          "path",
          "dirty",
          "document_id"
        ],
        "type": "object"
      },
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
- **Permission:** always

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
      "items": {
        "properties": {
          "attached": {
            "description": "this is the connection's current document",
            "type": "boolean"
          },
          "dirty": {
            "type": "boolean"
          },
          "document_id": {
            "description": "the persistent UUID the document's file carries (lowercase, hyphenated); null until the document is first saved",
            "type": [
              "string",
              "null"
            ]
          },
          "front": {
            "description": "this is the user's front-most document window",
            "type": "boolean"
          },
          "id": {
            "description": "doc_N, unique for the host's lifetime and never reused",
            "type": "string"
          },
          "label": {
            "description": "the client-chosen label given at hew.doc.new/open, if any",
            "type": "string"
          },
          "path": {
            "description": "the file last opened from or saved to; null while unsaved",
            "type": [
              "string",
              "null"
            ]
          },
          "title": {
            "type": "string"
          }
        },
        "required": [
          "id",
          "title",
          "path",
          "dirty",
          "document_id"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "encoding": {
      "type": "string"
    },
    "grant": {
      "type": "object"
    },
    "profile": {
      "enum": [
        "core",
        "app",
        "plugin"
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

## hew.print

### `hew.print.pdf`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** model.read

Print the document to a PDF the way File ▸ Print… does: standard (one page, the view as-is) or scaled (parallel projection at an exact drawing scale, tiled across pages with overlap bands, crop/trim marks, a scale bar, and a title block). Line art is vector (hidden lines removed); shaded is a software-rasterized bitmap per page. Bytes base64 inline, or written to path.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "camera": {
      "description": "as hew.view.snapshot; scaled prints use its direction with parallel projection",
      "type": "object"
    },
    "include_hidden": {
      "description": "line art: dashed hidden lines",
      "type": "boolean"
    },
    "margin_mm": {
      "description": "defaults to 12.7 (½ in)",
      "type": "number"
    },
    "marks": {
      "description": "crop/trim marks and neighbour labels; defaults to true",
      "type": "boolean"
    },
    "mode": {
      "description": "defaults to scaled",
      "enum": [
        "scaled",
        "standard"
      ],
      "type": "string"
    },
    "orientation": {
      "description": "defaults to auto (fewer tiles; standard follows the view's aspect)",
      "enum": [
        "auto",
        "portrait",
        "landscape"
      ],
      "type": "string"
    },
    "overlap_mm": {
      "description": "tile overlap band, defaults to 10; 0 for none",
      "type": "number"
    },
    "paper": {
      "description": "\"letter\" | \"legal\" | \"tabloid\" | \"a5\" | \"a4\" | \"a3\" (default a4) or {w_mm, h_mm}",
      "oneOf": [
        {
          "type": "string"
        },
        {
          "properties": {
            "h_mm": {
              "type": "number"
            },
            "w_mm": {
              "type": "number"
            }
          },
          "required": [
            "w_mm",
            "h_mm"
          ],
          "type": "object"
        }
      ]
    },
    "path": {
      "description": "write the PDF here instead of returning it inline",
      "type": "string"
    },
    "scale": {
      "description": "scaled: paper/model, defaults to 0.1 (1:10)",
      "type": "number"
    },
    "scale_bar": {
      "description": "defaults to true",
      "type": "boolean"
    },
    "scale_label": {
      "description": "title-block scale text; defaults to the ratio (\"1:10\")",
      "type": "string"
    },
    "scene": {
      "type": "string"
    },
    "style": {
      "description": "defaults to line_art (vector)",
      "enum": [
        "line_art",
        "shaded"
      ],
      "type": "string"
    },
    "title": {
      "description": "title-block document name; defaults to the document's",
      "type": "string"
    },
    "title_block": {
      "description": "defaults to true",
      "type": "boolean"
    },
    "units": {
      "description": "scale-bar family, defaults to metric",
      "enum": [
        "metric",
        "imperial"
      ],
      "type": "string"
    },
    "view": {
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
  "properties": {
    "cols": {
      "type": "integer"
    },
    "pages": {
      "type": "integer"
    },
    "path": {
      "type": "string"
    },
    "pdf_base64": {
      "description": "present only when path was not given",
      "type": "string"
    },
    "rows": {
      "type": "integer"
    }
  },
  "required": [
    "pages",
    "cols",
    "rows"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `nothing_to_render`
- `too_complex`
- `save_failed`
- `unknown_scene` — That Scene no longer exists — it may have been deleted.

## hew.query

### `hew.query.changes_since`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

What changed since a history generation: the union of the change sets since then, or `full` when the history no longer reaches back that far.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "generation": {
      "minimum": 0,
      "type": "integer"
    }
  },
  "required": [
    "generation"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "changes": {
      "additionalProperties": false,
      "properties": {
        "added": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "attrs": {
          "items": {
            "additionalProperties": false,
            "properties": {
              "id": {
                "type": "string"
              },
              "ns": {
                "type": "string"
              },
              "scope": {
                "enum": [
                  "entity",
                  "face",
                  "edge"
                ]
              }
            },
            "required": [
              "id",
              "ns",
              "scope"
            ],
            "type": "object"
          },
          "type": "array"
        },
        "document": {
          "type": "boolean"
        },
        "full": {
          "type": "boolean"
        },
        "modified": {
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "removed": {
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "added",
        "removed",
        "modified",
        "attrs",
        "document",
        "full"
      ],
      "type": "object"
    },
    "epoch": {
      "type": "integer"
    },
    "from": {
      "type": "integer"
    },
    "generation": {
      "type": "integer"
    }
  },
  "required": [
    "generation",
    "epoch",
    "from",
    "changes"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.query.context`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

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
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "stack": {
      "type": "array"
    }
  },
  "required": [
    "stack",
    "generation",
    "epoch"
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
- **Permission:** model.read

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
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "id": {
      "type": "string"
    },
    "kind": {
      "type": "string"
    }
  },
  "required": [
    "kind",
    "id",
    "generation",
    "epoch"
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
- **Permission:** model.read

A solid's faces: planes, areas, centroids, boundary loops.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "object": {
      "description": "an object id; give this or path",
      "type": "string"
    },
    "path": {
      "description": "an occurrence path ending at an object ([...instances, object]); each face then also reports material_effective through it",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "faces": {
      "description": "per face: object, plane, area, centroid, outer, holes, material, surface, texture_pins, attrs (the face's attribute dictionary, namespace -> key -> value, {} for none), and material_effective when asked through a path",
      "type": "array"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "object": {
      "type": "string"
    }
  },
  "required": [
    "object",
    "faces",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_occurrence`

### `hew.query.measure`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

Distances between points, edges, and faces; or the volume, area, and bounding box of a part, group, or instance.

**Params schema:**

```json
{
  "oneOf": [
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
    },
    {
      "additionalProperties": false,
      "properties": {
        "id": {
          "description": "a world object, group, or instance; a definition member needs path",
          "type": "string"
        }
      },
      "required": [
        "id"
      ],
      "type": "object"
    },
    {
      "additionalProperties": false,
      "properties": {
        "path": {
          "description": "an occurrence path [...instances, node]",
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
        "delta": {
          "type": "array"
        },
        "distance": {
          "type": "number"
        },
        "epoch": {
          "minimum": 0,
          "type": "integer"
        },
        "generation": {
          "minimum": 0,
          "type": "integer"
        }
      },
      "required": [
        "distance",
        "delta",
        "generation",
        "epoch"
      ],
      "type": "object"
    },
    {
      "description": "meters, square meters, cubic meters; groups and instances sum their parts. extents are the sizes of the world-axis-aligned bbox (part-frame extents are hew.query.part_frame's). volume is null when no part is watertight; open parts add nothing to it and are counted in open_leaves",
      "properties": {
        "area": {
          "type": "number"
        },
        "bbox": {
          "properties": {
            "max": {
              "type": "array"
            },
            "min": {
              "type": "array"
            }
          },
          "required": [
            "min",
            "max"
          ],
          "type": "object"
        },
        "epoch": {
          "minimum": 0,
          "type": "integer"
        },
        "extents": {
          "type": "array"
        },
        "generation": {
          "minimum": 0,
          "type": "integer"
        },
        "leaf_count": {
          "type": "integer"
        },
        "open_leaves": {
          "type": "integer"
        },
        "volume": {
          "type": [
            "number",
            "null"
          ]
        }
      },
      "required": [
        "volume",
        "area",
        "bbox",
        "extents",
        "leaf_count",
        "open_leaves",
        "generation",
        "epoch"
      ],
      "type": "object"
    }
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_occurrence`
- `locator_missed`
- `ambiguous_locator`
- `no_such_point`
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.query.mesh`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

One placement's triangle mesh: positions, normals, indices, and per-triangle face indices, base64.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "a world node's public id",
      "type": "string"
    },
    "path": {
      "description": "an occurrence path, instances first and the node last",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    },
    "space": {
      "description": "defaults to world; local is the node's own frame (an instance's definition coordinates)",
      "enum": [
        "world",
        "local"
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
  "properties": {
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "face_indices_base64": {
      "description": "u32 little-endian per triangle: the face's index in hew.query.topology's faces, plus its part's face_index_base",
      "type": "string"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "indices_base64": {
      "description": "u32 little-endian, three per triangle, counter-clockwise from outside",
      "type": "string"
    },
    "normals_base64": {
      "description": "f32 little-endian xyz per vertex, unit length",
      "type": "string"
    },
    "parts": {
      "items": {
        "additionalProperties": false,
        "properties": {
          "face_index_base": {
            "minimum": 0,
            "type": "integer"
          },
          "id": {
            "type": "string"
          },
          "index_base": {
            "minimum": 0,
            "type": "integer"
          },
          "index_count": {
            "minimum": 0,
            "type": "integer"
          }
        },
        "required": [
          "id",
          "face_index_base",
          "index_base",
          "index_count"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "positions_base64": {
      "description": "f32 little-endian xyz per vertex",
      "type": "string"
    },
    "triangle_count": {
      "minimum": 0,
      "type": "integer"
    },
    "vertex_count": {
      "minimum": 0,
      "type": "integer"
    }
  },
  "required": [
    "vertex_count",
    "triangle_count",
    "positions_base64",
    "normals_base64",
    "indices_base64",
    "face_indices_base64",
    "parts",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_occurrence`
- `mesh_too_large`
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.query.occurrences`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

Every placement in the model with its path and world pose: objects, groups, and instances, nested definitions expanded through each instance.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "include_hidden": {
      "description": "defaults to false: hidden placements are skipped and hidden instances are not descended",
      "type": "boolean"
    },
    "kinds": {
      "description": "which rows to return (never what is traversed); defaults to all three",
      "items": {
        "enum": [
          "object",
          "group",
          "instance"
        ],
        "type": "string"
      },
      "type": "array"
    },
    "roots": {
      "description": "where to start: node ids (world nodes) or occurrence locators {path}; absent is the whole world. Rows come in root order, each root's subtree depth-first, a placement listed once even when two roots reach it",
      "items": {
        "oneOf": [
          {
            "type": "string"
          },
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
        ]
      },
      "minItems": 1,
      "type": "array"
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
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "occurrences": {
      "items": {
        "additionalProperties": false,
        "properties": {
          "definition": {
            "type": [
              "string",
              "null"
            ]
          },
          "definition_name": {
            "type": [
              "string",
              "null"
            ]
          },
          "depth": {
            "minimum": 0,
            "type": "integer"
          },
          "hidden": {
            "description": "effective: the node's own flag and tags, its groups', and every instance on the path",
            "type": "boolean"
          },
          "id": {
            "type": "string"
          },
          "kind": {
            "enum": [
              "object",
              "group",
              "instance"
            ],
            "type": "string"
          },
          "material": {
            "description": "an object's base material, an instance's own; null for a group",
            "type": [
              "string",
              "null"
            ]
          },
          "material_effective": {
            "description": "what the node's unpainted faces show: an object's base, else the nearest painted instance on the path; an instance's own, else the nearest painted instance above it; null for a group",
            "type": [
              "string",
              "null"
            ]
          },
          "mirrored": {
            "type": "boolean"
          },
          "name": {
            "type": [
              "string",
              "null"
            ]
          },
          "parent": {
            "type": [
              "string",
              "null"
            ]
          },
          "path": {
            "description": "the instances above this node, outermost first; {path: [...path, id]} locates it",
            "items": {
              "type": "string"
            },
            "type": "array"
          },
          "tags": {
            "items": {
              "items": {
                "type": "string"
              },
              "type": "array"
            },
            "type": "array"
          },
          "user_hidden": {
            "type": "boolean"
          },
          "world_pose": {
            "description": "row-major 3×4",
            "items": {
              "type": "number"
            },
            "maxItems": 12,
            "minItems": 12,
            "type": "array"
          }
        },
        "required": [
          "path",
          "id",
          "kind",
          "definition",
          "parent",
          "world_pose",
          "mirrored",
          "hidden",
          "user_hidden",
          "material",
          "material_effective",
          "tags",
          "name",
          "definition_name",
          "depth"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "occurrences",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_occurrence`
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.query.part_frame`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

A part measured along its own axes: origin, axes, and extents of one placement.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "a world node's public id",
      "type": "string"
    },
    "path": {
      "description": "an occurrence path, instances first and the node last",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
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
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "extents": {
      "description": "along x, y, z: [length, width, thickness] for a face frame; the definition's own axis order for an instance",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "mirrored": {
      "type": "boolean"
    },
    "origin": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "source": {
      "enum": [
        "attribute",
        "definition",
        "face"
      ],
      "type": "string"
    },
    "x": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "y": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "z": {
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
    "x",
    "y",
    "z",
    "extents",
    "source",
    "mirrored",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_occurrence`
- `no_part_frame` — That part has no faces to measure.
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.query.raycast`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

First hit along a ray, or every crossing, through any depth of nesting — the programmatic form of clicking.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "all": {
      "description": "list every face crossing along the ray, nearest first (front and back faces both), instead of only the nearest; an empty list is a success",
      "type": "boolean"
    },
    "dir": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "exclude": {
      "description": "world node ids or occurrence locators {path}; each entry and everything under it is ignored by the ray",
      "items": {
        "oneOf": [
          {
            "type": "string"
          },
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
        ]
      },
      "maxItems": 64,
      "type": "array"
    },
    "include_hidden": {
      "description": "let the ray hit parts that are effectively hidden (user-hidden or on a hidden tag); each hit reports hidden. Default false: hidden parts fall through, as rendering does",
      "type": "boolean"
    },
    "max_distance": {
      "description": "meters along the ray; farther crossings are dropped",
      "exclusiveMinimum": 0,
      "type": "number"
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
  "oneOf": [
    {
      "properties": {
        "distance": {
          "type": "number"
        },
        "epoch": {
          "minimum": 0,
          "type": "integer"
        },
        "face": {
          "description": "a face locator {object, at} with at in the leaf Object's own frame; feed it to hew.query.resolve, hew.material.paint, or hew.solid.push_pull",
          "type": "object"
        },
        "generation": {
          "minimum": 0,
          "type": "integer"
        },
        "hidden": {
          "description": "effective visibility including tags; false unless include_hidden",
          "type": "boolean"
        },
        "kind": {
          "enum": [
            "object",
            "instance"
          ],
          "type": "string"
        },
        "leaf": {
          "description": "the hit Object's public id",
          "type": "string"
        },
        "normal": {
          "type": "array"
        },
        "object": {
          "description": "the world object's or, for a hit inside an instance, the outermost instance's public id",
          "type": "string"
        },
        "path": {
          "description": "the hit part's occurrence path without the part itself (the form hew.query.occurrences uses); [] for a world object",
          "items": {
            "type": "string"
          },
          "type": "array"
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
        "normal",
        "path",
        "leaf",
        "face",
        "hidden",
        "generation",
        "epoch"
      ],
      "type": "object"
    },
    {
      "properties": {
        "epoch": {
          "minimum": 0,
          "type": "integer"
        },
        "generation": {
          "minimum": 0,
          "type": "integer"
        },
        "hits": {
          "items": {
            "properties": {
              "distance": {
                "type": "number"
              },
              "face": {
                "description": "a face locator {object, at} with at in the leaf Object's own frame; feed it to hew.query.resolve, hew.material.paint, or hew.solid.push_pull",
                "type": "object"
              },
              "hidden": {
                "description": "effective visibility including tags; false unless include_hidden",
                "type": "boolean"
              },
              "kind": {
                "enum": [
                  "object",
                  "instance"
                ],
                "type": "string"
              },
              "leaf": {
                "description": "the hit Object's public id",
                "type": "string"
              },
              "normal": {
                "type": "array"
              },
              "object": {
                "description": "the world object's or, for a hit inside an instance, the outermost instance's public id",
                "type": "string"
              },
              "path": {
                "description": "the hit part's occurrence path without the part itself (the form hew.query.occurrences uses); [] for a world object",
                "items": {
                  "type": "string"
                },
                "type": "array"
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
              "normal",
              "path",
              "leaf",
              "face",
              "hidden"
            ],
            "type": "object"
          },
          "maxItems": 1000,
          "type": "array"
        },
        "raycast_hits_capped": {
          "description": "present and true when more than 1,000 crossings were found and the farthest were dropped",
          "type": "boolean"
        }
      },
      "required": [
        "hits",
        "generation",
        "epoch"
      ],
      "type": "object"
    }
  ],
  "type": "object"
}
```

**Refusals:**

- `locator_missed`
- `ambiguous_locator`
- `unknown_entity`
- `unknown_occurrence`
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.query.resolve`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

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
    "occurrence": {
      "description": "an occurrence locator {path: [instance, …, leaf]}; answers {occurrence: OccurrenceJson}",
      "type": "object"
    },
    "point": {}
  },
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "a resolved \"edge\" carries \"kind\": \"solid\" ({object,from,to}) or \"sketch\" ({id,sketch,from,to,curve})",
  "properties": {
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    }
  },
  "required": [
    "generation",
    "epoch"
  ],
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
- `unknown_occurrence`

### `hew.query.scene`

- **Version:** 1
- **Tier:** Required
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

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
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
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
    "components",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.query.topology`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

One object's vertices, faces, and edges with their adjacency, planes, and analytic curves.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "description": "a world node's public id",
      "type": "string"
    },
    "path": {
      "description": "an occurrence path, instances first and the node last",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    },
    "space": {
      "description": "defaults to world; local is the node's own frame (an instance's definition coordinates)",
      "enum": [
        "world",
        "local"
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
  "properties": {
    "edges": {
      "items": {
        "description": "{index, a, b (vertex indices), faces: [face indices, 1 or 2], curve: {kind: \"circle\", center, axis, radius} | null, attrs: the edge's attribute dictionary, {} for none}",
        "type": "object"
      },
      "type": "array"
    },
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "faces": {
      "items": {
        "description": "{index, plane: {point, normal}, outer: [vertex indices], holes: [[...]], material, back_material, surface: {kind: \"cylinder\", axis_point, axis, radius} | null, area, attrs: the face's attribute dictionary, {} for none}",
        "type": "object"
      },
      "type": "array"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "object": {
      "type": "string"
    },
    "vertices": {
      "items": {
        "items": {
          "type": "number"
        },
        "maxItems": 3,
        "minItems": 3,
        "type": "array"
      },
      "type": "array"
    }
  },
  "required": [
    "object",
    "vertices",
    "faces",
    "edges",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `unknown_occurrence`
- `not_an_object`

## hew.scenes

### `hew.scenes.add`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Add a Scene capturing the document's current view state. Records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "after": {
      "description": "insert after this Scene's id; appended at the end when omitted",
      "type": "string"
    },
    "camera": {
      "additionalProperties": false,
      "description": "an explicit camera to capture — no named-view shorthand, a Scene captures a concrete eye/target, not a fitted view; when omitted and the camera property is captured, falls back to the document's own saved working camera",
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
        "parallel_height": {
          "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there",
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
    "description": {
      "type": "string"
    },
    "display": {
      "additionalProperties": false,
      "description": "opaque editor display toggles: stored and returned, never interpreted by the kernel",
      "properties": {
        "axes": {
          "type": "boolean"
        },
        "grid": {
          "type": "boolean"
        },
        "guides": {
          "type": "boolean"
        }
      },
      "required": [
        "grid",
        "axes",
        "guides"
      ],
      "type": "object"
    },
    "name": {
      "description": "must be non-empty and unused; auto-named \"Scene N\" when omitted",
      "type": "string"
    },
    "properties": {
      "additionalProperties": false,
      "description": "which of the five capturable properties to (re-)capture; each defaults to true",
      "properties": {
        "camera": {
          "type": "boolean"
        },
        "display": {
          "type": "boolean"
        },
        "hidden_nodes": {
          "type": "boolean"
        },
        "hidden_tags": {
          "type": "boolean"
        },
        "section": {
          "type": "boolean"
        }
      },
      "type": "object"
    }
  },
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
    "name": {
      "type": "string"
    },
    "sid": {
      "type": "integer"
    }
  },
  "required": [
    "id",
    "sid",
    "name"
  ],
  "type": "object"
}
```

**Refusals:**

- `duplicate_scene_name` — A Scene with that name already exists. Choose a different name.
- `empty_scene_name` — A Scene needs a name.
- `unknown_scene` — That Scene no longer exists — it may have been deleted.

### `hew.scenes.apply`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Apply a Scene: write its captured camera/hidden-set/section state into the document. Records no undo entry (§6.4).

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
  "description": "each key present only when the Scene captured that property; hidden_object_ids/hidden_instance_ids appear as a pair (possibly empty arrays) whenever hidden_nodes or hidden_tags is captured, and are OMITTED entirely — not empty-arrayed — when neither is, so a partial-capture Scene can never read as \"show everything\"",
  "properties": {
    "camera": {
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
          "type": "number"
        },
        "parallel_height": {
          "description": "the height the view spans at the target, meters",
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
        "target",
        "up",
        "projection",
        "fov_deg",
        "parallel_height"
      ],
      "type": "object"
    },
    "hidden_instance_ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "hidden_object_ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "section": {
      "description": "null means captured-but-no-plane-placed",
      "oneOf": [
        {
          "type": "null"
        },
        {
          "properties": {
            "active": {
              "type": "boolean"
            },
            "normal": {
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
            "normal",
            "active"
          ],
          "type": "object"
        }
      ]
    }
  },
  "type": "object"
}
```

**Refusals:**

- `unknown_scene` — That Scene no longer exists — it may have been deleted.

### `hew.scenes.describe`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.read

Set a Scene's free-text description. Records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "description": {
      "type": "string"
    },
    "id": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "description"
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

- `unknown_scene` — That Scene no longer exists — it may have been deleted.

### `hew.scenes.list`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

Every Scene, in tab order.

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
    "scenes": {
      "items": {
        "description": "camera/display are present only when that property is captured AND has something to report; section is present (possibly null) whenever the section property is captured",
        "properties": {
          "camera": {
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
                "type": "number"
              },
              "parallel_height": {
                "description": "the height the view spans at the target, meters",
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
              "target",
              "up",
              "projection",
              "fov_deg",
              "parallel_height"
            ],
            "type": "object"
          },
          "description": {
            "type": "string"
          },
          "display": {
            "additionalProperties": false,
            "description": "opaque editor display toggles: stored and returned, never interpreted by the kernel",
            "properties": {
              "axes": {
                "type": "boolean"
              },
              "grid": {
                "type": "boolean"
              },
              "guides": {
                "type": "boolean"
              }
            },
            "required": [
              "grid",
              "axes",
              "guides"
            ],
            "type": "object"
          },
          "id": {
            "type": "string"
          },
          "name": {
            "type": "string"
          },
          "props": {
            "properties": {
              "camera": {
                "type": "boolean"
              },
              "display": {
                "type": "boolean"
              },
              "hidden_nodes": {
                "type": "boolean"
              },
              "hidden_tags": {
                "type": "boolean"
              },
              "section": {
                "type": "boolean"
              }
            },
            "required": [
              "camera",
              "hidden_nodes",
              "hidden_tags",
              "section",
              "display"
            ],
            "type": "object"
          },
          "section": {
            "description": "null means captured-but-no-plane-placed",
            "oneOf": [
              {
                "type": "null"
              },
              {
                "properties": {
                  "active": {
                    "type": "boolean"
                  },
                  "normal": {
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
                  "normal",
                  "active"
                ],
                "type": "object"
              }
            ]
          },
          "sid": {
            "type": "integer"
          },
          "view_style": {
            "description": "the look the Scene restores with its display property: backdrop (sRGB background, sky and ground or null) and whether the model axes are drawn; captured from the document's own view style, so absent for a document shown Hew's way",
            "properties": {
              "axes": {
                "type": "boolean"
              },
              "background": {
                "items": {
                  "type": "integer"
                },
                "maxItems": 3,
                "minItems": 3,
                "type": "array"
              },
              "ground": {
                "oneOf": [
                  {
                    "items": {
                      "type": "integer"
                    },
                    "maxItems": 3,
                    "minItems": 3,
                    "type": "array"
                  },
                  {
                    "type": "null"
                  }
                ]
              },
              "sky": {
                "oneOf": [
                  {
                    "items": {
                      "type": "integer"
                    },
                    "maxItems": 3,
                    "minItems": 3,
                    "type": "array"
                  },
                  {
                    "type": "null"
                  }
                ]
              }
            },
            "required": [
              "background",
              "sky",
              "ground",
              "axes"
            ],
            "type": "object"
          }
        },
        "required": [
          "id",
          "sid",
          "name",
          "description",
          "props"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "scenes"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.scenes.remove`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Delete a Scene. Records no undo entry (§6.4).

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

- `unknown_scene` — That Scene no longer exists — it may have been deleted.

### `hew.scenes.rename`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Rename a Scene. Records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    },
    "name": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "name"
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

- `unknown_scene` — That Scene no longer exists — it may have been deleted.
- `duplicate_scene_name` — A Scene with that name already exists. Choose a different name.
- `empty_scene_name` — A Scene needs a name.

### `hew.scenes.reorder`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Move a Scene to a new position in tab order. Records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "id": {
      "type": "string"
    },
    "index": {
      "description": "tab-order position; clamped to the end for an out-of-range index, never refused",
      "minimum": 0,
      "type": "integer"
    }
  },
  "required": [
    "id",
    "index"
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

- `unknown_scene` — That Scene no longer exists — it may have been deleted.

### `hew.scenes.update`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Re-capture a Scene's properties from the document's current state. Records no undo entry (§6.4).

**Params schema:**

```json
{
  "additionalProperties": false,
  "description": "properties defaults to the Scene's currently captured set (re-capture, never widen) when omitted",
  "properties": {
    "camera": {
      "additionalProperties": false,
      "description": "an explicit camera to capture — no named-view shorthand, a Scene captures a concrete eye/target, not a fitted view; when omitted and the camera property is captured, falls back to the document's own saved working camera",
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
        "parallel_height": {
          "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there",
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
    "display": {
      "additionalProperties": false,
      "description": "opaque editor display toggles: stored and returned, never interpreted by the kernel",
      "properties": {
        "axes": {
          "type": "boolean"
        },
        "grid": {
          "type": "boolean"
        },
        "guides": {
          "type": "boolean"
        }
      },
      "required": [
        "grid",
        "axes",
        "guides"
      ],
      "type": "object"
    },
    "id": {
      "type": "string"
    },
    "properties": {
      "additionalProperties": false,
      "description": "which of the five capturable properties to (re-)capture; each defaults to true",
      "properties": {
        "camera": {
          "type": "boolean"
        },
        "display": {
          "type": "boolean"
        },
        "hidden_nodes": {
          "type": "boolean"
        },
        "hidden_tags": {
          "type": "boolean"
        },
        "section": {
          "type": "boolean"
        }
      },
      "type": "object"
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

- `unknown_scene` — That Scene no longer exists — it may have been deleted.

## hew.selection

### `hew.selection.clear`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** view

Clear the app's selection.

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
  "properties": {
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "ids": {
      "description": "public ids in pick order; index 0 is the primary",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "primary": {
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "ids",
    "primary",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

### `hew.selection.get`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** view

The app's selection: public ids in pick order, the primary first. A drafting selection reads as empty.

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
  "properties": {
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "ids": {
      "description": "public ids in pick order; index 0 is the primary",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "primary": {
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "ids",
    "primary",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`

### `hew.selection.set`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** view

Change the app's selection: replace it (the default), or add, subtract, or toggle the named entities. Nodes and sketches mix; a guide or an annotation is selected alone. Hidden entities refuse.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ids": {
      "description": "objects, groups, instances, and sketches; or exactly one guide; or exactly one annotation",
      "items": {
        "type": "string"
      },
      "maxItems": 10000,
      "type": "array"
    },
    "mode": {
      "description": "defaults to replace",
      "enum": [
        "replace",
        "add",
        "subtract",
        "toggle"
      ],
      "type": "string"
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
  "additionalProperties": false,
  "properties": {
    "epoch": {
      "minimum": 0,
      "type": "integer"
    },
    "generation": {
      "minimum": 0,
      "type": "integer"
    },
    "ids": {
      "description": "public ids in pick order; index 0 is the primary",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "primary": {
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "ids",
    "primary",
    "generation",
    "epoch"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `selection_target_not_visible`
- `selection_mixed_kinds`
- `host_capability_missing`

## hew.sketch

### `hew.sketch.copy`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Copy drawing units on a plane by an offset.

**Params schema:**

```json
{
  "properties": {
    "offset": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "units": {
      "description": "unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges)",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "plane",
    "units",
    "offset"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "the plane units the edit made (new keys after the edit); empty for delete",
  "properties": {
    "units": {
      "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
      "items": {
        "properties": {
          "key": {
            "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
            "type": "string"
          },
          "kind": {
            "enum": [
              "edge",
              "region"
            ],
            "type": "string"
          },
          "object_id": {
            "description": "the owning solid, for topology and owned",
            "type": "string"
          },
          "sketch_id": {
            "description": "the sketch holding the drawing, for owned and free when it has one",
            "type": "string"
          },
          "store": {
            "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
            "enum": [
              "topology",
              "owned",
              "free"
            ],
            "type": "string"
          }
        },
        "required": [
          "kind",
          "key",
          "store"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "units"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_unit`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.

### `hew.sketch.delete`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Delete drawing units from a plane.

**Params schema:**

```json
{
  "properties": {
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "units": {
      "description": "unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges)",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "plane",
    "units"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "the plane units the edit made (new keys after the edit); empty for delete",
  "properties": {
    "units": {
      "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
      "items": {
        "properties": {
          "key": {
            "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
            "type": "string"
          },
          "kind": {
            "enum": [
              "edge",
              "region"
            ],
            "type": "string"
          },
          "object_id": {
            "description": "the owning solid, for topology and owned",
            "type": "string"
          },
          "sketch_id": {
            "description": "the sketch holding the drawing, for owned and free when it has one",
            "type": "string"
          },
          "store": {
            "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
            "enum": [
              "topology",
              "owned",
              "free"
            ],
            "type": "string"
          }
        },
        "required": [
          "kind",
          "key",
          "store"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "units"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_unit`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.

### `hew.sketch.draw_arc`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
      "description": "on-face drawing (plane spec {\"face\": <locator>}) draws through the drafting engine instead of creating a sketch region; `units` lists the plane units the draw touched; mints transaction-scoped face tokens \"a\"/\"b\" (an open sweep — a plain boundary-to-boundary cut) or \"face\"/\"parent\" (a full-turn sweep, or a pie/segment close — closed, imprinted like a circle) (HEW_API.md §5.2/§5.4) when the draw produced a face",
      "properties": {
        "object_id": {
          "type": "string"
        },
        "units": {
          "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
          "items": {
            "properties": {
              "key": {
                "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
                "type": "string"
              },
              "kind": {
                "enum": [
                  "edge",
                  "region"
                ],
                "type": "string"
              },
              "object_id": {
                "description": "the owning solid, for topology and owned",
                "type": "string"
              },
              "sketch_id": {
                "description": "the sketch holding the drawing, for owned and free when it has one",
                "type": "string"
              },
              "store": {
                "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
                "enum": [
                  "topology",
                  "owned",
                  "free"
                ],
                "type": "string"
              }
            },
            "required": [
              "kind",
              "key",
              "store"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "object_id",
        "units"
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
- **Permission:** model.write

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
      "description": "on-face drawing (plane spec {\"face\": <locator>}) draws through the drafting engine instead of creating a sketch region; `units` lists the plane units the draw touched; mints transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4) when the draw produced a face",
      "properties": {
        "object_id": {
          "type": "string"
        },
        "units": {
          "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
          "items": {
            "properties": {
              "key": {
                "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
                "type": "string"
              },
              "kind": {
                "enum": [
                  "edge",
                  "region"
                ],
                "type": "string"
              },
              "object_id": {
                "description": "the owning solid, for topology and owned",
                "type": "string"
              },
              "sketch_id": {
                "description": "the sketch holding the drawing, for owned and free when it has one",
                "type": "string"
              },
              "store": {
                "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
                "enum": [
                  "topology",
                  "owned",
                  "free"
                ],
                "type": "string"
              }
            },
            "required": [
              "kind",
              "key",
              "store"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "object_id",
        "units"
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
- **Permission:** model.write

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
      "description": "on-face drawing (plane spec {\"face\": <locator>}) draws through the drafting engine instead of creating a sketch region; `units` lists the plane units the draw touched; mints transaction-scoped face tokens \"a\"/\"b\" naming the faces either side of the path's last cut — for a single cut, the two faces it produced (HEW_API.md §5.2/§5.4) when the draw produced a face",
      "properties": {
        "object_id": {
          "type": "string"
        },
        "units": {
          "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
          "items": {
            "properties": {
              "key": {
                "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
                "type": "string"
              },
              "kind": {
                "enum": [
                  "edge",
                  "region"
                ],
                "type": "string"
              },
              "object_id": {
                "description": "the owning solid, for topology and owned",
                "type": "string"
              },
              "sketch_id": {
                "description": "the sketch holding the drawing, for owned and free when it has one",
                "type": "string"
              },
              "store": {
                "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
                "enum": [
                  "topology",
                  "owned",
                  "free"
                ],
                "type": "string"
              }
            },
            "required": [
              "kind",
              "key",
              "store"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "object_id",
        "units"
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
- **Permission:** model.write

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
      "maximum": 120,
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
      "description": "on-face drawing (plane spec {\"face\": <locator>}) draws through the drafting engine instead of creating a sketch region; `units` lists the plane units the draw touched; mints transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4) when the draw produced a face",
      "properties": {
        "object_id": {
          "type": "string"
        },
        "units": {
          "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
          "items": {
            "properties": {
              "key": {
                "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
                "type": "string"
              },
              "kind": {
                "enum": [
                  "edge",
                  "region"
                ],
                "type": "string"
              },
              "object_id": {
                "description": "the owning solid, for topology and owned",
                "type": "string"
              },
              "sketch_id": {
                "description": "the sketch holding the drawing, for owned and free when it has one",
                "type": "string"
              },
              "store": {
                "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
                "enum": [
                  "topology",
                  "owned",
                  "free"
                ],
                "type": "string"
              }
            },
            "required": [
              "kind",
              "key",
              "store"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "object_id",
        "units"
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
- `sides_above_cap`
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
- **Permission:** model.write

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
      "description": "on-face drawing (plane spec {\"face\": <locator>}) draws through the drafting engine instead of creating a sketch region; `units` lists the plane units the draw touched; mints transaction-scoped face tokens \"face\" (the new sub-face) and \"parent\" (the reshaped parent, now carrying the loop as a hole) (HEW_API.md §5.2/§5.4) when the draw produced a face",
      "properties": {
        "object_id": {
          "type": "string"
        },
        "units": {
          "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
          "items": {
            "properties": {
              "key": {
                "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
                "type": "string"
              },
              "kind": {
                "enum": [
                  "edge",
                  "region"
                ],
                "type": "string"
              },
              "object_id": {
                "description": "the owning solid, for topology and owned",
                "type": "string"
              },
              "sketch_id": {
                "description": "the sketch holding the drawing, for owned and free when it has one",
                "type": "string"
              },
              "store": {
                "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
                "enum": [
                  "topology",
                  "owned",
                  "free"
                ],
                "type": "string"
              }
            },
            "required": [
              "kind",
              "key",
              "store"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "object_id",
        "units"
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
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `point_not_on_face` — Part of the line leaves the face. Keep every point on the face being split.
- `would_corrupt` — That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.

### `hew.sketch.move`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Move drawing units (islands, edges, circles, regions) on a plane by an offset.

**Params schema:**

```json
{
  "properties": {
    "offset": {
      "description": "translation [x, y, z] in meters; must stay in the plane",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "units": {
      "description": "unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges)",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "plane",
    "units",
    "offset"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "the plane units the edit made (new keys after the edit); empty for delete",
  "properties": {
    "units": {
      "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
      "items": {
        "properties": {
          "key": {
            "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
            "type": "string"
          },
          "kind": {
            "enum": [
              "edge",
              "region"
            ],
            "type": "string"
          },
          "object_id": {
            "description": "the owning solid, for topology and owned",
            "type": "string"
          },
          "sketch_id": {
            "description": "the sketch holding the drawing, for owned and free when it has one",
            "type": "string"
          },
          "store": {
            "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
            "enum": [
              "topology",
              "owned",
              "free"
            ],
            "type": "string"
          }
        },
        "required": [
          "kind",
          "key",
          "store"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "units"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_unit`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.

### `hew.sketch.offset`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Offset a region boundary within its sketch, or a face or plane region by its unit key.

**Params schema:**

```json
{
  "properties": {
    "distance": {
      "description": "positive grows the material, negative shrinks it",
      "type": "number"
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "region": {
      "description": "a sketch region id; with `plane`, a region unit key rgn_<hex16> from hew.sketch.units (a face or plane region)",
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
  "oneOf": [
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
    },
    {
      "description": "the unit-key form (`plane` given): the plane units the offset made",
      "properties": {
        "units": {
          "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
          "items": {
            "properties": {
              "key": {
                "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
                "type": "string"
              },
              "kind": {
                "enum": [
                  "edge",
                  "region"
                ],
                "type": "string"
              },
              "object_id": {
                "description": "the owning solid, for topology and owned",
                "type": "string"
              },
              "sketch_id": {
                "description": "the sketch holding the drawing, for owned and free when it has one",
                "type": "string"
              },
              "store": {
                "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
                "enum": [
                  "topology",
                  "owned",
                  "free"
                ],
                "type": "string"
              }
            },
            "required": [
              "kind",
              "key",
              "store"
            ],
            "type": "object"
          },
          "type": "array"
        }
      },
      "required": [
        "units"
      ],
      "type": "object"
    }
  ]
}
```

**Refusals:**

- `unknown_region` — That profile is no longer there — the model changed since it was picked. Click it again.
- `malformed_region` — This profile's outline couldn't be traced. Redraw the shape; if it keeps failing, use Report Bug.
- `offset_too_small` — That offset distance is too small to make a new boundary. Drag further, or type an exact distance.
- `offset_collapsed` — The shape can't absorb that offset — its boundary would collapse, cross itself, or spike out of a sharp corner. Try a smaller distance, or soften the sharpest corner.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `unknown_unit`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.

### `hew.sketch.rotate`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Rotate drawing units on a plane about the plane normal through a point.

**Params schema:**

```json
{
  "properties": {
    "angle": {
      "description": "radians, counter-clockwise seen from the plane normal side",
      "type": "number"
    },
    "axis_point": {
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
    "units": {
      "description": "unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges)",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "plane",
    "units",
    "axis_point",
    "angle"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "the plane units the edit made (new keys after the edit); empty for delete",
  "properties": {
    "units": {
      "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
      "items": {
        "properties": {
          "key": {
            "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
            "type": "string"
          },
          "kind": {
            "enum": [
              "edge",
              "region"
            ],
            "type": "string"
          },
          "object_id": {
            "description": "the owning solid, for topology and owned",
            "type": "string"
          },
          "sketch_id": {
            "description": "the sketch holding the drawing, for owned and free when it has one",
            "type": "string"
          },
          "store": {
            "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
            "enum": [
              "topology",
              "owned",
              "free"
            ],
            "type": "string"
          }
        },
        "required": [
          "kind",
          "key",
          "store"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "units"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_unit`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.

### `hew.sketch.scale`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Scale drawing units on a plane about a center, uniformly or per axis.

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
    "factor": {
      "description": "uniform scale; give this or `factors`",
      "type": "number"
    },
    "factors": {
      "description": "per-axis [sx, sy, sz] in world axes; must keep the drawing in its plane",
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    },
    "units": {
      "description": "unit keys from hew.sketch.units: isl_<hex16> (an island: all its edges), edg_<hex16> (one edge), crv_<hex16> (every facet of one circle), rgn_<hex16> (a region's ring edges that are drawing edges)",
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
    }
  },
  "required": [
    "plane",
    "units",
    "center"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "description": "the plane units the edit made (new keys after the edit); empty for delete",
  "properties": {
    "units": {
      "description": "the plane units the call touched: every drawing edge lying along what was drawn, then every region whose boundary carries it",
      "items": {
        "properties": {
          "key": {
            "description": "edg_<hex16> or rgn_<hex16>, as hew.sketch.units lists them",
            "type": "string"
          },
          "kind": {
            "enum": [
              "edge",
              "region"
            ],
            "type": "string"
          },
          "object_id": {
            "description": "the owning solid, for topology and owned",
            "type": "string"
          },
          "sketch_id": {
            "description": "the sketch holding the drawing, for owned and free when it has one",
            "type": "string"
          },
          "store": {
            "description": "topology: the solid's own face structure; owned: drawing kept on the solid's face; free: drawing over no face",
            "enum": [
              "topology",
              "owned",
              "free"
            ],
            "type": "string"
          }
        },
        "required": [
          "kind",
          "key",
          "store"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "units"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_unit`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `point_off_plane` — That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.
- `degenerate_segment` — The line's two ends are the same point. Click a second, different point.

### `hew.sketch.units`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

List a plane's drawing as units: islands of edges (with where each is stored) and regions.

**Params schema:**

```json
{
  "properties": {
    "plane": {
      "description": "HEW_API.md §7 plane spec: {ground:true} | {origin,normal[,x_axis]} | {face:<locator>} | {sketch:<id>}",
      "type": "object"
    }
  },
  "required": [
    "plane"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "regions": {
      "items": {
        "properties": {
          "extrudable": {
            "description": "true when the region lies over no face and hew.solid.extrude takes it as a new Object",
            "type": "boolean"
          },
          "face": {
            "description": "a face locator ({object, at}) for the region when it IS a face of that solid — pass it to hew.solid.push_pull",
            "type": "object"
          },
          "holes": {
            "items": {
              "items": {
                "items": {
                  "type": "number"
                },
                "maxItems": 3,
                "minItems": 3,
                "type": "array"
              },
              "type": "array"
            },
            "type": "array"
          },
          "key": {
            "description": "rgn_<hex16>",
            "type": "string"
          },
          "object_id": {
            "description": "the solid under the region, when one is",
            "type": "string"
          },
          "outer": {
            "items": {
              "items": {
                "type": "number"
              },
              "maxItems": 3,
              "minItems": 3,
              "type": "array"
            },
            "type": "array"
          }
        },
        "required": [
          "key",
          "outer",
          "holes",
          "extrudable"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "units": {
      "items": {
        "properties": {
          "edges": {
            "items": {
              "properties": {
                "a": {
                  "items": {
                    "type": "number"
                  },
                  "maxItems": 3,
                  "minItems": 3,
                  "type": "array"
                },
                "b": {
                  "items": {
                    "type": "number"
                  },
                  "maxItems": 3,
                  "minItems": 3,
                  "type": "array"
                },
                "curve": {
                  "description": "present when the edge is a facet of an analytic circle",
                  "properties": {
                    "center": {
                      "items": {
                        "type": "number"
                      },
                      "maxItems": 3,
                      "minItems": 3,
                      "type": "array"
                    },
                    "key": {
                      "description": "crv_<hex16>",
                      "type": "string"
                    },
                    "radius": {
                      "type": "number"
                    }
                  },
                  "type": "object"
                },
                "key": {
                  "description": "edg_<hex16>",
                  "type": "string"
                },
                "object_id": {
                  "type": "string"
                },
                "sketch_id": {
                  "type": "string"
                },
                "store": {
                  "enum": [
                    "topology",
                    "owned",
                    "free"
                  ],
                  "type": "string"
                }
              },
              "required": [
                "key",
                "a",
                "b",
                "store"
              ],
              "type": "object"
            },
            "type": "array"
          },
          "key": {
            "description": "isl_<hex16>",
            "type": "string"
          }
        },
        "required": [
          "key",
          "edges"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "units",
    "regions"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_sketch` — That sketch is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`

## hew.solid

### `hew.solid.cap`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Close an open object's flat openings with faces; an object closed all round becomes a solid.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "object": {
      "description": "obj_ id (a world object or a component member)",
      "type": "string"
    },
    "rims": {
      "description": "indices into the object's hew.doc.health rims; omitted = every cappable rim (when none is, all of them, so the refusal says why)",
      "items": {
        "minimum": 0,
        "type": "integer"
      },
      "type": "array"
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
    "closed": {
      "description": "the object is now a solid",
      "type": "boolean"
    },
    "faces": {
      "description": "faces added (face tokens cap.1, cap.2, ... in a transaction)",
      "type": "integer"
    },
    "object_id": {
      "type": "string"
    },
    "reversed": {
      "description": "an island the caps closed was inside out, so that island's faces were turned over (a cavity inside another island is left as is)",
      "type": "boolean"
    }
  },
  "required": [
    "object_id",
    "faces",
    "closed",
    "reversed"
  ],
  "type": "object"
}
```

**Refusals:**

- `object_is_solid` — This is already solid, so there's nothing to close. Use Push/Pull to change its shape.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.
- `invalid` — Hew couldn't make this change without damaging the object, so nothing changed. Please send the file with Help ▸ Report Bug.
- `unknown_rim` — That opening is already closed. Check the list again for what is left.
- `rim_not_planar` — This opening isn't flat, so one face can't close it. Draw faces across it by hand.
- `rim_self_intersecting` — The edge of this opening crosses itself, so one face can't close it. Draw faces across it by hand.
- `cap_would_be_coplanar` — This is a flat panel, not an open shape. Keep it as a panel, or add thickness to make it solid.
- `non_manifold_rim` — The edges around this opening branch off in more than one direction, so Hew can't tell where a face goes. Fix those edges by hand first.
- `caps_overlap` — A face here would overlap another part of this object. Close all of the object's openings together, or draw faces across this one by hand.

### `hew.solid.delete_faces`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Remove faces from an open object; islands cut loose become objects of their own. Solids refuse.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "faces": {
      "description": "faces of that object; omitted = every face, which deletes the object (a component member too, unless it is its component's last object: last_definition_member)",
      "items": {
        "description": "HEW_API.md §5.2 face locator: {object,at} | {object,ray} | {\"$face\":\"label#key\"}",
        "type": "object"
      },
      "minItems": 1,
      "type": "array"
    },
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
    "deleted": {
      "description": "every face went, so the object was deleted",
      "type": "boolean"
    },
    "object_id": {
      "type": "string"
    },
    "object_ids": {
      "description": "islands cut loose, now objects of their own",
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "object_id",
    "deleted",
    "object_ids"
  ],
  "type": "object"
}
```

**Refusals:**

- `object_is_solid` — This is already solid, so there's nothing to close. Use Push/Pull to change its shape.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.
- `invalid` — Hew couldn't make this change without damaging the object, so nothing changed. Please send the file with Help ▸ Report Bug.
- `unknown_face` — That face is no longer there — the model changed since it was picked. Click it again.
- `last_definition_member` — This is the component's only member — deleting it would leave every instance empty. Delete the instances instead, or add another member first.
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`

### `hew.solid.delete_imprint`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Kept for compatibility: delete a drawn shape (sub-face or chord island) back into its face; hew.sketch.delete is the general form.

**Params schema:**

```json
{
  "properties": {
    "imprint": {
      "description": "HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {\"$face\":\"label#key\"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point)",
      "type": "object"
    }
  },
  "required": [
    "imprint"
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

- `not_in_plane` — A shape on a face can only move within the face's plane. Slide, turn, or scale it on the face, or delete it and draw it again where you want it.
- `not_an_imprint`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`

### `hew.solid.extrude`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Extrude a region into a new Object, consuming the profile.

**Params schema:**

```json
{
  "properties": {
    "distance": {
      "type": "number"
    },
    "plane": {
      "description": "HEW_API.md §7 plane spec naming the drafting context a unit-key `region` belongs to. A region over no face extrudes as a new Object; a region that is a face of a solid push/pulls that face (result as hew.solid.push_pull); a region over a face that cannot hold it refuses region_not_pushable",
      "type": "object"
    },
    "region": {
      "description": "a sketch region id; with `plane`, a region unit key rgn_<hex16> from hew.sketch.units",
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
  "description": "mints face tokens \"base\", \"top\", \"side.<n>\" (boundary-loop order) — HEW_API.md §5.4's normative example — when a new Object results",
  "properties": {
    "object_id": {
      "type": "string"
    },
    "object_ids": {
      "description": "a through-cut's resulting pieces (unit-key form over a face, as hew.solid.push_pull)",
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

- `distance_too_small` — That distance is too small to build anything. Drag further, or type an exact length.
- `degenerate_geometry` — This profile can't be extruded into a valid solid. Simplify the shape and try again.
- `unknown_region` — That profile is no longer there — the model changed since it was picked. Click it again.
- `region_has_no_face` — This loop of lines has no face to push or pull. Draw over one of its lines with the Line tool to give it a face.
- `unknown_entity`
- `unknown_unit`
- `region_not_pushable` — That shape can't be pushed or pulled: part of it is drawn on the solid's face and part is loose, so the face can't hold it. Move the drawing clear of the edge, or push/pull the neighboring pieces instead.
- `region_touches_hole` — This shape touches another shape at a point, so it can't be pulled as one piece. Pull the shape inside it instead, or move them a hair apart.

### `hew.solid.follow_me`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- `profile_crosses_axis` — This profile crosses the circle's center axis, so revolving it would lose the geometry on one side. Draw the profile entirely on one side of the axis instead.
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

### `hew.solid.imprints`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** model.read

Kept for compatibility: list an object's drawn shapes (sub-faces and chords) per drawing plane; hew.sketch.units is the general form.

**Params schema:**

```json
{
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
    "curve_units": {
      "description": "drawn circles and arcs (whole or cut by other drawing into several runs) on the object's drawing planes: each {closed, curve: {center, radius}, runs: [{at, path}]}, a run that is a chord above carrying that chord's at locator — the whole drawn curve, a circle when closed, else an arc",
      "items": {
        "properties": {
          "closed": {
            "type": "boolean"
          },
          "curve": {
            "type": "object"
          },
          "runs": {
            "items": {
              "type": "object"
            },
            "type": "array"
          }
        },
        "required": [
          "closed",
          "curve",
          "runs"
        ],
        "type": "object"
      },
      "type": "array"
    },
    "imprints": {
      "items": {
        "properties": {
          "at": {
            "description": "sub_face: an interior point clear of any nested sub-face; chord: the midpoint of the run's first segment — the locator to pass to move_imprint/rotate_imprint/scale_imprint/delete_imprint",
            "items": {
              "type": "number"
            },
            "maxItems": 3,
            "minItems": 3,
            "type": "array"
          },
          "curve": {
            "description": "sub_face only: {center, radius} when the whole loop is one drawn circle, else null"
          },
          "curves": {
            "description": "per edge of the loop (sub_face) or run (chord), in order: {center, radius} when that edge is a facet of a drawn circle or arc, else null — a pie, segment, or edge-to-edge arc reports its arc here",
            "type": "array"
          },
          "holes": {
            "description": "sub_face only: the hole loops' positions",
            "items": {
              "type": "array"
            },
            "type": "array"
          },
          "kind": {
            "enum": [
              "sub_face",
              "chord"
            ],
            "type": "string"
          },
          "loop": {
            "description": "sub_face only: the outer loop's positions, cycle order",
            "items": {
              "type": "array"
            },
            "type": "array"
          },
          "nested": {
            "description": "sub_face only: count of sub-faces lying inside this one",
            "type": "integer"
          },
          "normal": {
            "description": "the drawing plane's normal",
            "items": {
              "type": "number"
            },
            "type": "array"
          },
          "path": {
            "description": "chord only: the run's vertex positions, first to last",
            "items": {
              "type": "array"
            },
            "type": "array"
          },
          "region": {
            "description": "sub_face only: the region's unit key (rgn_…) on that plane, as hew.sketch.units reports it",
            "type": "string"
          },
          "unit": {
            "description": "the island's unit key (isl_…) on that plane, as hew.sketch.units reports it",
            "type": "string"
          }
        },
        "required": [
          "kind",
          "at"
        ],
        "type": "object"
      },
      "type": "array"
    }
  },
  "required": [
    "imprints",
    "curve_units"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.intersect`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- `boolean_operand_not_solid` — Something in that selection isn't solid, so it can't be combined. To make it solid, select it and choose Object ▸ Make Solid…
- `boolean_operand_empty` — That selection has no solids to combine. Pick a solid object or a group of solids.
- `grouped_operand` — This operation can't target an object inside a group. Ungroup it, or leave the group context, first.
- `degenerate_contact` — The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.mark_surface`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Mark an open object as a surface on purpose (or clear the mark); hew.doc.health keeps listing it, flagged accepted.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "accepted": {
      "default": true,
      "description": "false clears the mark",
      "type": "boolean"
    },
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
    "accepted": {
      "type": "boolean"
    },
    "object_id": {
      "type": "string"
    }
  },
  "required": [
    "object_id",
    "accepted"
  ],
  "type": "object"
}
```

**Refusals:**

- `object_is_solid` — This is already solid, so there's nothing to close. Use Push/Pull to change its shape.
- `unknown_entity`

### `hew.solid.move_imprint`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Kept for compatibility: slide a drawn shape (sub-face or chord island) in its plane; hew.sketch.move is the general form.

**Params schema:**

```json
{
  "properties": {
    "imprint": {
      "description": "HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {\"$face\":\"label#key\"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point)",
      "type": "object"
    },
    "offset": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    }
  },
  "required": [
    "imprint",
    "offset"
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

- `not_in_plane` — A shape on a face can only move within the face's plane. Slide, turn, or scale it on the face, or delete it and draw it again where you want it.
- `not_an_imprint`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`

### `hew.solid.push_pull`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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

- `object_not_solid` — This object isn't solid, so it can't be pushed or pulled. To make it solid, select it and choose Object ▸ Make Solid…
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

### `hew.solid.rotate_imprint`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Kept for compatibility: turn a drawn shape about its plane's normal through a pivot; hew.sketch.rotate is the general form.

**Params schema:**

```json
{
  "properties": {
    "about": {
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
    "angle": {
      "description": "radians",
      "type": "number"
    },
    "imprint": {
      "description": "HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {\"$face\":\"label#key\"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point)",
      "type": "object"
    }
  },
  "required": [
    "imprint",
    "angle",
    "about"
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

- `not_in_plane` — A shape on a face can only move within the face's plane. Slide, turn, or scale it on the face, or delete it and draw it again where you want it.
- `not_an_imprint`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`
- `no_such_point`

### `hew.solid.scale_imprint`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Kept for compatibility: uniformly scale a drawn shape about an anchor in its plane; hew.sketch.scale is the general form.

**Params schema:**

```json
{
  "properties": {
    "about": {
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
    "factor": {
      "description": "uniform scale factor",
      "exclusiveMinimum": 0,
      "type": "number"
    },
    "imprint": {
      "description": "HEW_API.md §5.2 face locator ({object,at} | {object,ray} | {\"$face\":\"label#key\"}) naming a sub-face; {object,at} also picks a CHORD when the point lies on one of its lines (a chord is tried first, then the sub-face containing the point)",
      "type": "object"
    }
  },
  "required": [
    "imprint",
    "factor",
    "about"
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

- `not_in_plane` — A shape on a face can only move within the face's plane. Slide, turn, or scale it on the face, or delete it and draw it again where you want it.
- `not_an_imprint`
- `draft_boundary_edge_touched` — A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `locator_missed`
- `ambiguous_locator`
- `face_token_unknown`
- `face_token_stale`
- `no_such_point`

### `hew.solid.slice`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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

- `not_solid` — Only a solid can be sliced. To make it solid, select it and choose Object ▸ Make Solid…
- `plane_misses_solid` — The slicing plane doesn't pass through the object. Position the cut so it goes through the solid.
- `degenerate` — The cut lines up exactly with an existing face or edge, so it wouldn't create two pieces. Move the cut slightly.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.subtract`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- `boolean_operand_not_solid` — Something in that selection isn't solid, so it can't be combined. To make it solid, select it and choose Object ▸ Make Solid…
- `boolean_operand_empty` — That selection has no solids to combine. Pick a solid object or a group of solids.
- `grouped_operand` — This operation can't target an object inside a group. Ungroup it, or leave the group context, first.
- `degenerate_contact` — The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.
- `unknown_object` — That object is no longer there — the model changed since it was picked. Click it again.
- `unknown_group` — That group is no longer there — the model changed since it was picked. Click it again.
- `unknown_instance` — That component instance is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`

### `hew.solid.thicken`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Pull a flat surface into a solid of the given thickness.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "centered": {
      "default": false,
      "description": "split the thickness evenly on both sides of the surface",
      "type": "boolean"
    },
    "distance": {
      "description": "meters along each face's front normal (negative: behind it)",
      "type": "number"
    },
    "object": {
      "type": "string"
    }
  },
  "required": [
    "object",
    "distance"
  ],
  "type": "object"
}
```

**Result schema:**

```json
{
  "properties": {
    "object_id": {
      "description": "the surface, now a solid",
      "type": "string"
    },
    "object_ids": {
      "description": "further separate pieces of the surface, each now a solid of its own",
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "object_id",
    "object_ids"
  ],
  "type": "object"
}
```

**Refusals:**

- `object_is_solid` — This is already solid, so there's nothing to close. Use Push/Pull to change its shape.
- `unknown_component` — That component is no longer there — the model changed since it was picked. Click it again.
- `unknown_entity`
- `explode_session_scope` — That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.
- `invalid` — Hew couldn't make this change without damaging the object, so nothing changed. Please send the file with Help ▸ Report Bug.
- `not_a_flat_sheet` — Only flat panels can get a thickness this way. Close the openings of this shape instead.
- `thickness_too_small` — That thickness is too small. Enter at least 0.001 mm.

### `hew.solid.union`

- **Version:** 1
- **Tier:** Required
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- `boolean_operand_not_solid` — Something in that selection isn't solid, so it can't be combined. To make it solid, select it and choose Object ▸ Make Solid…
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
- **Permission:** model.write

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
- **Permission:** model.write

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
- **Permission:** model.write

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

### `hew.tag.rename`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

Rename a tag path (and every tag nested under it), keeping its identity.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "new_path": {
      "items": {
        "type": "string"
      },
      "minItems": 1,
      "type": "array"
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
    "path",
    "new_path"
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
- `duplicate_tag` — A tag with that name already exists. Choose a different name.
- `invalid_tag_path` — That isn't a valid tag name — it can't be empty, and a tag can't be moved inside itself.

### `hew.tag.set_visible`

- **Version:** 1
- **Tier:** Standard
- **Class:** model-mutating
- **Served:** kernel
- **Permission:** model.write

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
- **Permission:** view

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
        "parallel_height": {
          "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there",
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

### `hew.view.clear_isolation`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** view

End the live viewport's isolation; a no-op when nothing is isolated.

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

### `hew.view.hidden`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** view

What is hidden: the hidden objects, groups, and instances, the hidden tags, and the live viewport's isolation (null when nothing is isolated or the host has no viewport).

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
  "properties": {
    "hidden": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "isolated": {
      "oneOf": [
        {
          "items": {
            "oneOf": [
              {
                "description": "an object, group, or instance id",
                "type": "string"
              },
              {
                "additionalProperties": false,
                "properties": {
                  "instance": {
                    "description": "an instance that places it",
                    "type": "string"
                  },
                  "object": {
                    "description": "a definition member object",
                    "type": "string"
                  }
                },
                "required": [
                  "object",
                  "instance"
                ],
                "type": "object"
              }
            ]
          },
          "type": "array"
        },
        {
          "type": "null"
        }
      ]
    },
    "tags_hidden": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "hidden",
    "tags_hidden",
    "isolated"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.view.hide`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** view

Hide objects, groups, and component instances. View state: saved with the document, never undoable. Tags hide through hew.tag.set_visible.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ids": {
      "items": {
        "type": "string"
      },
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
  "additionalProperties": false,
  "properties": {
    "hidden": {
      "description": "every hidden object, group, and instance",
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "hidden"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `not_a_node`

### `hew.view.isolate`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** view

Show only the named objects, groups, instances, or definition members (each with an instance that places it) in the live viewport, and frame them. View-only: nothing in the document changes.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ids": {
      "items": {
        "oneOf": [
          {
            "description": "an object, group, or instance id",
            "type": "string"
          },
          {
            "additionalProperties": false,
            "properties": {
              "instance": {
                "description": "an instance that places it",
                "type": "string"
              },
              "object": {
                "description": "a definition member object",
                "type": "string"
              }
            },
            "required": [
              "object",
              "instance"
            ],
            "type": "object"
          }
        ]
      },
      "maxItems": 10000,
      "minItems": 1,
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
  "additionalProperties": false,
  "properties": {
    "isolated": {
      "items": {
        "oneOf": [
          {
            "description": "an object, group, or instance id",
            "type": "string"
          },
          {
            "additionalProperties": false,
            "properties": {
              "instance": {
                "description": "an instance that places it",
                "type": "string"
              },
              "object": {
                "description": "a definition member object",
                "type": "string"
              }
            },
            "required": [
              "object",
              "instance"
            ],
            "type": "object"
          }
        ]
      },
      "type": "array"
    }
  },
  "required": [
    "isolated"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `host_capability_missing`

### `hew.view.line_drawing`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** model.read

Hidden-line drawing of the visible document, or of a selection, from a camera (crates/hlr), served identically in the app and in hew-cli: hard edges, curved-wall silhouettes, section-cut outlines, optionally dashed hidden lines — as a true-size SVG at a drawing scale (inline or written to path), or as raw segments in view-plane meters.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "camera": {
      "description": "identical vocabulary to hew.view.snapshot's camera; mutually exclusive with view and scene",
      "type": "object"
    },
    "format": {
      "description": "defaults to svg",
      "enum": [
        "svg",
        "segments"
      ],
      "type": "string"
    },
    "include_hidden": {
      "description": "defaults to false; when true, hidden pieces are returned too (kind \"hidden\", dashed in SVG)",
      "type": "boolean"
    },
    "include_soft": {
      "description": "defaults to false; when true, non-silhouette curved-wall facet seams are returned (kind \"soft\")",
      "type": "boolean"
    },
    "path": {
      "description": "svg only: write the file here instead of returning it inline (hosts with filesystem access)",
      "type": "string"
    },
    "scale": {
      "description": "svg only: paper/model drawing scale, defaults to 1 (full size); the SVG's width/height are millimeters at that scale",
      "type": "number"
    },
    "scene": {
      "description": "a Scene's id: its camera and hidden sets, as hew.view.snapshot",
      "type": "string"
    },
    "selection": {
      "description": "draw only these: node ids, occurrence locators {path}, or a definition's cmp_ id (its leaves at the identity pose)",
      "items": {
        "oneOf": [
          {
            "type": "string"
          },
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
        ]
      },
      "minItems": 1,
      "type": "array"
    },
    "view": {
      "description": "a named standard view (parallel projection); mutually exclusive with camera and scene",
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
  "properties": {
    "bounds": {
      "description": "[min_x, min_y, max_x, max_y] in view-plane meters; null when empty",
      "items": {
        "type": "number"
      },
      "maxItems": 4,
      "minItems": 4,
      "type": "array"
    },
    "count": {
      "type": "integer"
    },
    "ids": {
      "description": "public id of the entity each segment belongs to",
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "kinds": {
      "items": {
        "enum": [
          "hard",
          "silhouette",
          "soft",
          "section",
          "hidden"
        ],
        "type": "string"
      },
      "type": "array"
    },
    "path": {
      "description": "format svg with path",
      "type": "string"
    },
    "segments": {
      "description": "format segments: [ax, ay, bx, by] in view-plane meters, y up, origin at the camera target",
      "items": {
        "items": {
          "type": "number"
        },
        "maxItems": 4,
        "minItems": 4,
        "type": "array"
      },
      "type": "array"
    },
    "svg": {
      "description": "format svg without path",
      "type": "string"
    }
  },
  "required": [
    "count"
  ],
  "type": "object"
}
```

**Refusals:**

- `host_capability_missing`
- `nothing_to_render`
- `too_complex`
- `save_failed`
- `unknown_scene` — That Scene no longer exists — it may have been deleted.
- `unknown_entity`
- `unknown_occurrence`
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.view.show`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** view

Show hidden objects, groups, and component instances again.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ids": {
      "items": {
        "type": "string"
      },
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
  "additionalProperties": false,
  "properties": {
    "hidden": {
      "description": "every hidden object, group, and instance",
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "hidden"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `not_a_node`

### `hew.view.show_all`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** kernel
- **Permission:** view

Show every hidden object, group, and component instance. Hidden tags stay hidden.

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
  "properties": {
    "hidden": {
      "items": {
        "type": "string"
      },
      "maxItems": 0,
      "type": "array"
    }
  },
  "required": [
    "hidden"
  ],
  "type": "object"
}
```

**Refusals:** none.

### `hew.view.snapshot`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** model.read

Render the attached document to PNG, rendered by the host's software rasterizer, identically in the app and in hew-cli, optionally of a selection on a transparent background — bytes base64 by default, or a path on hosts with filesystem access.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "background": {
      "description": "\"transparent\", #rrggbb, or #rrggbbaa; the fill of pixels no surface touches (defaults to the rasterizer's light gray). The id, depth, and normals passes are unaffected.",
      "type": "string"
    },
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
        "parallel_height": {
          "description": "the height the view spans at the target, meters (what a parallel view frames); defaults to what the perspective view spans there",
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
    "depth_bits": {
      "description": "bits per pixel of the depth PNG (default 8); only with include_depth",
      "enum": [
        8,
        16
      ],
      "type": "integer"
    },
    "height": {
      "description": "defaults to 512; out-of-range values are clamped, not refused",
      "maximum": 2048,
      "minimum": 16,
      "type": "integer"
    },
    "include_depth": {
      "description": "defaults to false; when true, also returns a depth pass as a grayscale PNG — NEAR is WHITE, background 0 — with depth_near/depth_far (meters from the eye along the view axis) so absolute depth is recoverable: depth = depth_far - (v - 1) / (max - 1) * (depth_far - depth_near), max = 255 or 65535. The conditioning input depth-aware image models expect",
      "type": "boolean"
    },
    "include_ids": {
      "description": "defaults to false; when true, also returns a per-pixel id-buffer and its palette",
      "type": "boolean"
    },
    "include_normals": {
      "description": "defaults to false; when true, also returns a camera-space normals pass as an RGB PNG: (n * 0.5 + 0.5) * 255 per axis, x right, y up, z toward the viewer, background (128, 128, 255)",
      "type": "boolean"
    },
    "path": {
      "description": "when given, the PNG is written here instead of returned inline, honored by hosts with filesystem access and refused typed elsewhere (mirrors hew.doc.export)",
      "type": "string"
    },
    "scene": {
      "description": "a Scene's id: renders through its resolved camera and hidden sets (Document::resolve_scene) instead of the document's own — falls back to the usual cameraless resolution when the Scene captures no camera. Mutually exclusive with camera and view. The Scene's section plane, if any, is NOT rendered headlessly at 1.0.",
      "type": "string"
    },
    "selection": {
      "description": "render only these: node ids, occurrence locators {path}, or a definition's cmp_ id (its members at the identity pose, one picture however many times it is placed). With no camera or view the camera is a fitted isometric over the selection, never the document's saved camera; view fits that standard view to the selection. Mutually exclusive with scene.",
      "items": {
        "oneOf": [
          {
            "type": "string"
          },
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
        ]
      },
      "minItems": 1,
      "type": "array"
    },
    "view": {
      "description": "a named standard view fitted to the scene bounding box; mutually exclusive with camera and scene",
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
  "description": "two shapes depending on whether path was given: bytes base64 inline (default), or a path plus a sidecar path for each extra pass asked for (id-buffer, depth, normals)",
  "properties": {
    "depth_bits": {
      "description": "with a depth pass: 8 or 16",
      "type": "integer"
    },
    "depth_far": {
      "description": "with a depth pass: the farthest drawn surface",
      "type": "number"
    },
    "depth_near": {
      "description": "with a depth pass: meters from the eye, along the view axis, of the nearest drawn surface",
      "type": "number"
    },
    "depth_path": {
      "description": "present only when include_depth and path were both given: \"<path>.depth.png\"",
      "type": "string"
    },
    "depth_png_base64": {
      "description": "present only when include_depth was true and path was not given",
      "type": "string"
    },
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
    "normals_path": {
      "description": "present only when include_normals and path were both given: \"<path>.normals.png\"",
      "type": "string"
    },
    "normals_png_base64": {
      "description": "present only when include_normals was true and path was not given",
      "type": "string"
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
- `unknown_entity`
- `unknown_occurrence`
- `unknown_scene` — That Scene no longer exists — it may have been deleted.
- `too_many_occurrences` — This model has more placed parts than Hew can list at once.

### `hew.view.units`

- **Version:** 1
- **Tier:** Standard
- **Class:** solitary
- **Served:** host
- **Permission:** view

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
- **Permission:** view

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

### `hew.view.zoom_to`

- **Version:** 1
- **Tier:** Standard
- **Class:** read-only
- **Served:** host
- **Permission:** view

Frame the named entities in the live viewport. padding widens the box on every side by that fraction of its largest extent (default 0.1). Answers the box before padding.

**Params schema:**

```json
{
  "additionalProperties": false,
  "properties": {
    "ids": {
      "items": {
        "type": "string"
      },
      "type": "array"
    },
    "padding": {
      "description": "defaults to 0.1",
      "maximum": 2,
      "minimum": 0,
      "type": "number"
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
  "additionalProperties": false,
  "properties": {
    "max": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    },
    "min": {
      "items": {
        "type": "number"
      },
      "maxItems": 3,
      "minItems": 3,
      "type": "array"
    }
  },
  "required": [
    "min",
    "max"
  ],
  "type": "object"
}
```

**Refusals:**

- `unknown_entity`
- `nothing_to_frame`
- `host_capability_missing`

