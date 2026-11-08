# Hew — Plugins

The normative reference for Hew's plugin system, from the plugin's side:
the manifest a plugin ships, the permissions it asks for, and (as later
parts land) how it runs, what it contributes, and how it is installed.
The bus every plugin talks over is specified in
[HEW_API.md](HEW_API.md); this document only adds what is particular to
plugins. Code comments cite invariants here as "PLUGINS.md invariant N".

Status: the manifest parser (`crates/plugin-manifest`), grant
enforcement in the dispatcher (HEW_API.md §10), plugin-provided commands
with forwarding (below, and HEW_API.md §10.2), and compounds with
coalescing and compare-and-swap (below, and HEW_API.md §6.1, §6.5) are
implemented. Nothing installs or runs a plugin yet, so the app answers
every forwarded command `plugin_unavailable`, aborting the compound it
opened for a mutating one at once.

## Invariants

These hold for every plugin. The numbers are stable; code cites them.
Where an invariant depends on a part that has not landed yet, it says
so.

1. **No plugin code runs in Hew's process, page, or main thread.** A
   plugin's logic runs in a dedicated Worker inside a sandboxed frame
   with an opaque origin. Surface frames are the one place plugin code
   shares the page's thread. External programs are connected apps, not
   plugins. (Lands with the realm.)
2. **Every document effect is a bus envelope.** A plugin has no other
   channel to the document; stateful UI effects (selection, overlays,
   toasts, windows) are bus commands too. Tool event streams, list
   models, and the surface message pipe are port messages, not
   envelopes.
3. **Grants are enforced at the dispatcher, statically, inside
   transactions.** The dispatcher's grant checks and the transaction
   executor's static pass are the only enforcement points for the bus. A
   permission the manifest does not declare is a `-32001` refusal, never
   a UI-only gate. (Implemented.)
4. **One plugin command is one undo entry.** The host opens a compound
   around every forwarded mutating call; whatever the plugin commits
   while serving it becomes one entry, and nothing else commits until it
   ends: the app refuses the user's edits and queues every other
   client's frames meanwhile. (Implemented; see "Compounds".)
5. **A plugin's identity rides every envelope.** Its connection carries
   the identity `plugin:<id>`, which history records as the origin of
   its changes. (Implemented for the connection; the Changes panel shows
   it later.)
6. **The manifest is the complete description of a plugin's reach.**
   Everything a plugin can contribute or call follows from the manifest
   plus the registry, before the plugin runs.
7. **Declarative before hosted.** A slot takes hosted content only where
   it says so; a declarative contribution always renders with Hew's own
   components.
8. **Headless parity.** Every live-only command refuses typed
   (`host_capability_missing`) on headless hosts, as `hew.view.camera`
   does.
9. **Deterministic registry order.** Commands, Hew's own and every
   plugin's alike, list in name order in one registry, and
   `hew.meta.capabilities` output is a pure function of the registry and
   the grant. (Implemented.)
10. **Crash containment.** A background that throws, hangs, times out, or
    is killed produces a typed error on its in-flight requests, a notice,
    and no other effect. (Lands with the realm.)
11. **Plugins make no network requests of their own.** All outbound
    traffic goes through `hew.net.fetch`, which enforces the manifest's
    host list in the host. (Lands with the network proxy.)
12. **An API major stays served for two years after its successor**,
    through an adapter.
13. **Compute-then-commit is the default write pattern.** A plugin reads,
    computes, and commits one envelope with `expect_generation` set to the
    generation it read (HEW_API.md §6.5), so it never writes on a stale
    model and never holds the document. Compounds are for commands whose
    later writes depend on earlier results, and are the exception.
    (Implemented.)

## Manifest

A plugin is a package with one manifest at its root, `hew-plugin.json`:
JSON, UTF-8, no comments. Every host reads it through the same function,
`plugin_manifest::parse`, in `crates/plugin-manifest` (pure: `serde` and
`semver`, no I/O). Unknown keys are refused anywhere in the typed parts
of the manifest, so a typo cannot silently drop a field; the
`contributes` members other than `commands` and `presave` are read as
opaque JSON until the parts that use them land.

```json
{
  "manifest": 1,
  "id": "com.tinyworkshop.opencutlist",
  "name": "OpenCutList",
  "version": "1.0.0",
  "description": "Cut lists, cutting diagrams, labels, and estimates.",
  "author": { "name": "Tiny Workshop Design", "url": "https://example.com" },
  "license": "GPL-3.0-only",
  "homepage": "https://example.com/opencutlist",
  "source": "https://github.com/example/opencutlist-hew",
  "hew": ">=1.4.0",
  "api": 1,
  "main": "dist/background.js",
  "icon": "icon.svg",
  "requires": [],
  "permissions": ["model.read", "model.write", "attributes.write", "view", "files", "clipboard"],
  "optionalPermissions": ["network:www.lairdubois.fr"],
  "activation": ["on_command", "on_tool", "on_surface_open", "on_document_has_namespace"],
  "data": { "version": 2, "migrate": "data.migrate" },
  "locales": { "default": "en", "catalogs": "locales/" },
  "agent": { "notes": "agent-notes.md" },
  "contributes": { "commands": [], "presave": false }
}
```

| Field | Rule |
|---|---|
| `manifest` | integer, required, must be `1` |
| `id` | required; the plugin id: lowercase reverse-DNS, `label ("." label)+`, each label `[a-z0-9][a-z0-9-]*` of 1 to 63 characters, at most 128 in all. An id whose first label is `hew` is reserved (`org.hew3d.*` is not) |
| `name` | required; 1 to 64 characters. Like every user-visible string in the manifest it may be a plain string, a per-locale object `{ "en": "...", "ja": "..." }` (at least one locale, every text 1 to 64 characters), or a catalog key `{ "$t": "key" }` |
| `version` | required; semver 2.0.0 |
| `description` | optional localizable string |
| `author` | optional `{ name, url? }` |
| `license`, `homepage`, `source` | optional strings |
| `hew` | optional semver range: the minimum app version, for features the API cannot express |
| `api` | required; the protocol major the plugin speaks: `1` |
| `main` | required; the background script, a package path. A package path is non-empty and relative, uses forward slashes, has no `..` segment, has no `:` in its first segment (so no drive letter or scheme), and, for a file, does not end in `/` |
| `icon` | optional package path to a file |
| `requires` | optional `[{ id, range }]`: other plugins, each id following the plugin id grammar, and the semver range of each |
| `permissions` | optional array of permission strings (below), granted at install |
| `optionalPermissions` | optional array, same vocabulary, granted later at the user's say; never pre-granted |
| `activation` | optional; when the background starts: any of `on_command`, `on_tool`, `on_surface_open`, `on_event`, `on_document_has_namespace`, `on_startup`. Default: every one but `on_startup`. `on_startup` requires `activationReason` |
| `activationReason` | optional localizable string shown on the consent sheet |
| `data` | optional `{ version, migrate? }`: `version` (default 1) stamps the documents the plugin writes into; `migrate` names the command that upgrades older data |
| `locales` | optional `{ default, catalogs }`; `catalogs` is a package path to a directory (it may end in `/`) |
| `agent` | optional `{ notes }`: a package path to a markdown file of at most 4 KiB, shown to agents as untrusted plugin text |
| `contributes` | optional; `commands` and `presave` are validated now, every other member is opaque for now |

A contributed command is `{ id, class, summary, label?, permission?,
timeout_ms?, concurrency?, params?, result?, refusals? }`: `class` is
`read_only` or `mutating`; `timeout_ms` defaults to 50000;
`concurrency` is `parallel` (the default) or `serial`; `params` and
`result` are JSON Schemas whose root is `"type": "object"` with
`"additionalProperties": false` (each defaults to the empty closed
object); `permission` is one of `model.read`, `model.write`,
`attributes.write`, `view`, `files`, `clipboard`, `library`, the
permission a caller needs. A plugin command cannot demand a documents or
network permission of its callers.

### Validation order

`parse` checks in this order and reports the first failure, so a file
with several defects always reports the same one:

1. The JSON and its shape. Unknown keys are `manifest_unknown_field`;
   anything else unreadable (bad JSON, a wrong type, a bad semver string,
   an empty locale object) is `manifest_malformed`, even when a value
   happens to contain the words "unknown field".
2. `manifest` is 1, else `manifest_version_unsupported`.
3. `api` is 1, else `api_unsupported`.
4. `id`: `invalid_plugin_id`, or `reserved_plugin_id` for a `hew` id.
5. `name` length, else `name_too_long`.
6. `main` is present and non-empty (`manifest_no_entry`); then `main`,
   `icon`, `locales.catalogs`, and `agent.notes` are package paths, else
   `manifest_invalid_path`.
7. Every permission, `permissions` then `optionalPermissions`:
   `unknown_permission` or `permission_too_broad`; then a name that
   appears twice across both lists is `duplicate_permission`.
8. `on_startup` without `activationReason`: `activation_reason_required`.
9. `contributes.commands`: every `id` matches `label ("." label)*` with
   label `[a-z0-9_]+`, else `invalid_command_id`; then a repeated id is
   `duplicate_command`; then more than 256 commands is
   `too_many_commands`; then, command by command, the `params` and
   `result` schema roots (`invalid_schema`), the `permission`
   (`unknown_permission`), and each refusal name, which must match
   `[a-z][a-z0-9_]*` (`invalid_command_id`).
10. `requires`, entry by entry: the id grammar (`invalid_plugin_id`, or
    `reserved_plugin_id` for a `hew` id); then a repeated id is
    `duplicate_requirement`; then the plugin's own id is
    `self_requirement`.

Every error name above comes with a one-sentence explanation a person
can act on (`ManifestError::explanation`).

## Commands

A plugin contributes bus commands in `contributes.commands` (the entry's
fields are under "Manifest" above). Each becomes the registry command
`<plugin-id>.<id>`, callable by any connection whose grant holds its
`permission`, including other plugins and agents over MCP. The dispatcher
never runs it: it checks the call and forwards it to the plugin, then
checks and shapes the plugin's answer (HEW_API.md §10.2).

| Manifest | Registry |
|---|---|
| `"class": "read_only"` | class `read_only`; changes nothing |
| `"class": "mutating"` | class `solitary`; can change the document; never inside a transaction with anything else |
| `permission` absent | `model.read` for a read-only command, `model.write` for a mutating one |
| `timeout_ms` | the forwarding host's deadline; a value above 50000 is clamped to 50000, not refused |
| `label` | the undo label, in its English form (the `en` text, else the first locale's, else the catalog key) |
| `refusals` | listed as declared; on the wire a refusal is named `<plugin-id>.<name>` |

Registration is atomic: if any command of a plugin is refused, none of
them is added. The registry refuses a plugin id outside the grammar or
reserved, a command id outside the command id grammar, or a refusal name
outside `[a-z][a-z0-9_]*` (`name_outside_plugin_namespace`), a name that
is already registered
(`duplicate_command`), more than 256 commands (`too_many_commands`), and
a schema the dispatcher cannot enforce (`invalid_schema`). A
`permission` a command may not demand (a documents or network
permission, or one that is not a permission at all), which only a
declaration that skipped `plugin_manifest::parse` can carry, is not
refused: it fails closed to `first_party`, so only Hew's own connections
can call that command.

**Schemas.** `params` and `result` are checked by a deliberately small
JSON Schema subset: the dispatcher checks params before forwarding and
the plugin's result after it answers, and refuses at registration any
schema that uses something else, so a declared schema always means what
the check does. The root must be `"type": "object"` with
`"additionalProperties": false`. The keywords:

| Keyword | Meaning |
|---|---|
| `type` | one of `object`, `array`, `string`, `number`, `integer`, `boolean`, `null`, or an array of them; `integer` is a number with no fractional part |
| `properties`, `required`, `additionalProperties` | `additionalProperties` is `false` or a schema |
| `items` | one schema, for every element |
| `enum`, `const` | numbers compare by exact value (`1` equals `1.0`, also above 2^53) |
| `minimum`, `maximum` | inclusive, compared exactly |
| `minLength`, `maxLength` | in characters; the count may be written `2` or `2.0` |
| `minItems`, `maxItems` | |
| `oneOf`, `anyOf` | `oneOf` needs exactly one match |
| `description`, `title`, `default`, `examples`, `$schema`, `$comment` | accepted and ignored |

Anything else, `$ref` and `format` included, is `invalid_schema`. A
mismatch reports the first failure, depth-first, with a JSON pointer into
the value (the root is written `/`). Within an object, members are
checked alphabetically by key, not in the order `properties` lists them,
then `required`, then members `properties` does not name; so when
several members are wrong, the one named is the first by key: `/a: expected number, found
string`, `/: missing required property "a"`, `/: property "x" is not
allowed`, `/v: not one of the allowed values`, `below minimum <n>`,
`above maximum <n>`, `shorter than <n>`, `longer than <n>`, `fewer than
<n> items`, `more than <n> items`, `matches none of oneOf`, `matches more
than one of oneOf`, `matches none of anyOf`. Bad params are the caller's
`-32602`; a bad result is the plugin's `-32003` `plugin_bad_result`, with
the raw result in `data.raw`.

**Refusals.** A plugin refuses with `{name, explanation, detail}`, `name`
being one of its declared refusals; the caller sees `-32000` with the
refusal named `<plugin-id>.<name>` and the plugin's own explanation, so
a plugin's refusal can never pass for one of Hew's.

## Compounds

A mutating command runs inside a **compound**: a bracket the host opens
on the document before routing the call to the plugin, and closes when
the plugin answers (HEW_API.md §6.1). What a handler can rely on:

- **One undo entry.** Every envelope the plugin sends while serving the
  command commits as part of the compound; when the handler returns, all
  of it is one entry in the user's history, labeled with the command's
  `label` (or its name) and attributed to the plugin (`plugin:<id>`). A
  handler that commits nothing adds no entry.
- **Nobody else writes meanwhile.** While the compound is open the user's
  edits are refused, and every other client's frames (socket clients,
  connected apps, other plugins) wait in order until it closes. The
  plugin's own envelopes run because the host stamps them with the
  forward they serve; one stamped for any other forward waits, or is
  refused `-32000` `document_busy` if it reaches the dispatcher. Opening,
  creating, reverting, or closing a document is refused `document_busy`
  from anyone while a compound is open.
- **Abort puts everything back.** A refusal, a timeout, a crash, or the
  user's Cancel aborts the compound: the document returns to exactly what
  it was before the command began, view state included (hidden nodes,
  tag visibility, Scenes), and the history has no trace of it. The epoch
  (HEW_API.md §7) still moves on, so a reader keyed on it sees the
  change.
- **Nested commands nest.** A handler that calls another plugin's
  mutating command (or its own) opens an inner compound; the inner
  command's writes land in it, and it closes into one child of the
  outer. Only the innermost command's envelopes run while it is open.
- **What undo does not reverse.** Undoing the entry reverses every
  undoable change in it. Registry writes that record no undo entry
  anywhere (HEW_API.md §6.4) keep that contract inside a compound:
  `hew.material.create`, `hew.tag.create`, `hew.tag.set_visible`,
  `hew.scenes.*`, and a Library material insert stay after the entry is
  undone. An abort, unlike undo, does take them back.
- **The 50-second ceiling.** A command's `timeout_ms` is clamped to
  50000. A command that runs out of time is answered `-32003` with
  `reason: "timeout"` and its compound aborted. Longer work runs as a job
  that answers the command at once.
- **Cancel.** After a quarter second the app shows a notice naming the
  command (by its label, else the plugin) with Cancel. Cancel aborts the
  compound and answers the caller `-32000` `plugin_command_cancelled`.

**Recording.** A session recording (docs/dev/DIAGNOSTICS.md) holds a
compound as one `plugin_compound` call carrying what the handler sent;
the forwarded call itself is not recorded, and an aborted compound
records nothing. Replay re-creates the compound without the plugin, as
one entry with the same label and origin.

Prefer compute-then-commit (invariant 13): read, compute, and commit one
`hew.doc.transact` with `expect_generation`; a `generation_moved` refusal
means read again and recompute. Use `coalesce` to fold a run of small
edits (field edits in a list, say) into one undo step without holding
the document between them.

## Events

A plugin hears about the document by subscribing:
`hew.event.subscribe {"events": ["document_changed", ...]}` (HEW_API.md
§4.5). Each event comes as a notification, a frame with no `id` whose
method is `hew.event.<name>`. A name the plugin's grant cannot receive
comes back in `denied` instead of failing the call, and the grant is
checked again on every delivery, so a permission withdrawn later stops
the events without a reconnect.

| Event | Needs | What it carries |
|---|---|---|
| `document_changed` | `model.read` | the history clock (`generation`, `epoch`), who made the change (`origin`), its `label`, its `kind` (`commit`, `undo`, `redo`, `abort`, `reset`), and `changes` |
| `context_changed` | `model.read` | the editing context stack and the in-context `fallback` entries, as `hew.query.context` |
| `scenes_changed` | `model.read` | every Scene, as `hew.scenes.list`, and the active one |
| `units_changed` | `model.read` | the display length format |
| `document_will_save`, `document_saved`, `document_opened`, `document_closed` | `model.read` | the document's id and title, the format of a save, and its path only with `documents.save` |
| `selection_changed`, `tool_changed` | `view` | the selection, as `hew.selection.get` answers it; the active tool's name (desktop app; see "Selection and view state") |
| `plugin_state` | nothing | a plugin loaded, unloaded, crashed, or had its grant changed |

Every event names its `document`. A plugin bound to one document hears
only about that one.

**Change sets.** `changes` lists public ids: `added` (created or
placed), `removed` (deleted, consumed by a boolean, exploded), and
`modified` (anything else the change touched), plus `attrs`, the
`{id, ns}` pairs whose attribute dictionaries changed. An undo's lists
are the undo's own: what it brought back is `added`. A transaction, or a
whole plugin command with everything it committed, is one event, sent
when it ends; an aborted one is a single `abort` event. An event a
request causes arrives after that request's reply. When
`full` is `true` the lists are empty and anything may have changed (an
import, a rescale, an aborted command, a document replaced): re-read
what you cache. More than 64 changes between two deliveries also arrive
as one `full` event.

**Catching up.** A plugin that keeps a cache keyed on the history
`generation` asks `hew.query.changes_since {"generation": n}` for
everything since: the union of the change sets, or `full` when the
history no longer reaches back that far. Writes that record no undo
entry (Scenes, tag visibility, palette additions) move neither the
generation nor the epoch and appear in no change set; their own events
(`scenes_changed`) carry them.

## Selection and view state

A plugin holding `view` reads and changes what the person sees
(HEW_API.md §7.3): `hew.selection.get`, `set`, and `clear` for the
selection; `hew.view.hide`, `show`, `show_all`, and `hidden` for hidden
objects, groups, and instances; `hew.view.isolate` and
`clear_isolation` for isolation; `hew.view.zoom_to` to frame entities.
None of them is an edit: they record no undo entry and move neither the
generation nor the epoch, and a transaction that aborts takes back what
they did. So does an aborted plugin command, except for the camera:
framing it moved stays. Subscribe to `selection_changed` to follow the selection and
`tool_changed` to follow the active tool.

- **The selection is ordered.** `ids[0]` is the primary pick. `set`
  combines with what is selected by `mode`: `replace` (the default),
  `add`, `subtract`, or `toggle`, the same rules as the Select tool's
  modifier keys. It holds objects, groups, instances, and sketches, or a
  single guide, or a single annotation.
- **Hidden things cannot be selected.** Naming one refuses
  `selection_target_not_visible`, with the id in `detail`. Show it first
  (`hew.view.show`), or name something else.
- **A drafting selection reads as empty.** While the person has units of
  a drawing selected (lines and regions drawn on a plane before they
  became a sketch), `hew.selection.get` answers no ids, and
  `selection_changed` reports an empty selection.
- **Hiding is saved with the document.** It is the Outliner's eye: a hide
  persists in the file, though it never enters the undo history and
  produces no `document_changed`.
- **Desktop only for now.** The selection, isolation, and framing take
  effect in the desktop app; the browser and Shop Mode do not carry them
  out yet. `hide`, `show`, `show_all`, and `hidden` change and read the
  document itself.

## Entering a node for editing

A plugin holding `view` can put the person inside a node, the way a
double click in Hew's own Outliner does: `hew.view.enter {id}` for a
world object, group, or instance, or `hew.view.enter {path}` for a node
reached through instances (HEW_API.md §6.3). The window opens group and
component sessions level by level, falls back to the in-context entry for
an instance a session cannot take (mirrored, unevenly scaled, or placed in
a group elsewhere), and selects the target. `hew.view.exit` steps out one
level and `hew.view.exit_all` all the way. This is not `hew.context.enter`,
which stays balanced inside a transaction and scopes a plugin's own
edits; the frame `hew.view.enter` opens is the person's.

The recipe for an outliner panel:

1. On a row's double click, call `hew.view.enter` with the row's id, or
   with its occurrence path when the row is a part inside an instance. The
   answer is `{}`; do not read the result for what opened.
2. Listen for `context_changed`. Its `stack` lists the open group and
   component frames, outermost first, and `fallback` lists the in-context
   entries (`{kind: "object" | "instance", id}`) inside the innermost frame.
   Draw the breadcrumb from `stack` followed by `fallback`.
3. Call `hew.query.context` for the same two lists on startup, or after a
   refusal. A refusal the app shows the person as a message leaves the
   context unchanged and sends no event.
4. Handle `gesture_in_progress`: a tool gesture (a Move drag) is under way,
   and the call changes nothing. Try again when it ends. `hew.query.context`
   reports `armed`.

Calls are solitary: a multi-command transaction carrying one is refused
`-32602`.

## Reading parts

A plugin holding `model.read` can list every part of a model, measure
each one along its own axes, tell identical and mirrored parts apart,
and read its geometry (HEW_API.md §7.4).

**List the parts.** `hew.query.occurrences` returns one row per
placement: every object, group, and instance, with nested components
expanded through each instance that places them. A row's `path` is the
instances above it, so the same definition member appears once per
placement, each with its own `world_pose`. Pass `kinds: ["object",
"instance"]` for the leaves a cut list counts, and leave `include_hidden`
off for what the person sees. A list shows `name ?? definition_name`.

**From the selection.** `const { ids } = await hew.selection.get();
const nodes = ids.filter((id) => /^(obj|grp|ins)_/.test(id));
const { occurrences } = await hew.query.occurrences({ roots: nodes.length ? nodes : undefined });`
Falls back to the whole model when nothing placeable is selected. A
selection can also hold sketches, guides, or annotations, which `roots`
refuses (`unknown_entity`), hence the filter.

**From the active context.** `const { stack, direct_members } = await
hew.query.context(); const roots = direct_members?.map((m) => m.id);`
`stack` is the editing path a report header records.

**While an editing session is open**, the list is the model as the person
is editing it. Inside a group, the group's members are world rows and
the group itself is a hidden row. Inside a component, its members are
world rows, and the instance the person entered and its siblings are
hidden rows that still list the definition under them, at the pose each
placement would have. `include_hidden: false` is the as-displayed view,
the one a cut list wants.

**Measure a part.** `hew.query.part_frame` with `{ id }` or
`{ path: [...row.path, row.id] }` gives `origin`, `x`, `y`, `z`, and
`extents`. `source` says where the frame came from:

- `definition`: an instance, measured along its definition's own axes,
  scaled by the placement. A mirrored placement keeps a right-handed
  frame (`y` is negated) and reports `mirrored: true`.
- `face`: an object or group, measured along its largest face. `z` is
  that face's outward normal and `x` its longest edge, swapped where
  needed so `extents` read `[length, width, thickness]` (a square part
  keeps its face frame). A board turned at any angle reads its true
  size, and the frame turns with it. A part that is the same after a
  half turn, like a plain box, may come back with an axis reversed after
  it is turned, since nothing in its shape says which way is which.
- `attribute`: an object or group carrying a `hew.part` / `frame`
  attribute, `{ "origin": [x, y, z], "x": [x, y, z], "y": [x, y, z] }`.
  The values are coordinates in the `face` frame `hew.query.part_frame`
  reports for the part without the attribute: `origin` an offset along
  that frame's axes, `x` and `y` directions in it. Echoing that frame
  back (`origin: [0, 0, 0]`, `x: [1, 0, 0]`, `y: [0, 1, 0]`) changes
  nothing. Because the values are relative to the part's own frame, the
  override follows the part through any move or turn. `x` and `y` must
  be unit length and perpendicular within 1e-6, or the attribute is
  ignored. To mark a board's grain running across its width, write
  `x: [0, 1, 0]`, `y: [1, 0, 0]`.

**Read a mesh.** `hew.query.mesh` returns base64 buffers, all
little-endian: f32 positions and normals, u32 indices (counter-clockwise
from outside, mirrored placements included), and a u32 face index per
triangle that matches
`hew.query.topology`'s `faces`. A group or instance concatenates its leaf
objects; `parts` says where each one's indices start. `space: "local"`
reads the part in its own frame (an instance in definition coordinates),
which is what one thumbnail or one 3D preview per definition wants.

**Read topology.** `hew.query.topology` gives one object's vertices,
faces (planes, loops as vertex indices, cylinder surfaces), and edges
(their faces and circle curves). Its indices are valid for the
`generation` it returns only; after any edit, read it again.

### What a row says about a part

Hew does not decide what a part is; a plugin's rule does, from facts each
`hew.query.occurrences` row carries (HEW_API.md §7.4):

- `members`, on every row: `{ objects, groups, instances }`, the node's
  direct members by id, in member order. A group lists its members, an
  instance its definition's, an object three empty lists. Hidden members
  are left out unless `include_hidden`.
- `solid`, on object rows: Model Health's verdict is solid (closed and
  manifold).
- `family` and `family_closed`, on object rows: the seam family the
  object belongs to (an index into `hew.doc.health`'s `families` over the
  whole model, good for the current state only: recompute it from
  `hew.doc.health` after any change, and note that a call with an
  `objects` scope numbers its own families) and whether every shared rim of that family
  can take a cap, so the family can be rejoined with `hew.solid.rejoin`
  (`{family, as: "one_solid"}` answers `{object}`, one undo entry, the
  largest piece's name, tags, and attributes kept; it refuses
  `unknown_family`, `not_cappable`, `rejoin_not_possible`, or a cap or
  union refusal; it takes no generation guard, so run it inside
  `hew.doc.transact` with `expect_generation` when the index was read
  earlier); both `null` for an object in no family.
- `linked`, on instance rows: the definition is a set of linked copies
  rather than a named component.

Group and instance rows carry no `solid` or `family` keys. A typical
rule: a container whose `members` are only objects, every one `solid`,
is one part (a glued-up blank); an object with a `family` whose
`family_closed` is true is a piece of one part split on import; anything
else is read level by level. Hew's own default, in its cut list, stays
"the deepest node whose members are all objects".

### Reading a container's own Objects

`scope: "own"` on `hew.query.part_frame`, `hew.query.measure`,
`hew.view.snapshot`, and `hew.view.line_drawing` takes a container's own
objects: its direct object members and the objects of its groups, never
the members of a nested instance. A door group holding a panel and a
hinge component measures as the panel alone with `scope: "own"`, and as
panel plus hinge with the default `"all"`. On an object both scopes are
the same; a container with no objects of its own refuses as an empty one
does (`no_part_frame` for `part_frame`). The render commands need a
`selection` for it.

`hew.view.snapshot` with `orient: "part"` and one `selection` entry
renders that part in its own frame: `view: "front"` looks along the
frame's `y`, `top` down its `z`, so every board in a parts list is drawn
the same way however it sits in the model.

### Same part, mirrored part

`hew.query.signature { id | path }` answers `{ shape, mirror, symmetric,
frame }` for one object. Two objects with equal `shape` are the same
part in any position; `a.shape == b.mirror` means `b` is `a`'s mirror
image (a left and a right side); `symmetric` means the part is its own
mirror image. The hashes survive a save and reload and any move or turn,
to 1 µm. `frame` is the part's face frame; the pose that carries one
matching part onto another is `frame_b ∘ frame_a⁻¹`. A group or instance
refuses `not_an_object`: ask per solid, from the rows' `members`.

### Probing with rays

`hew.query.raycast` answers "what is there along this line" in terms of
parts. A hit gives the occurrence `path`, the `leaf` object, and a `face`
locator that `hew.material.paint`, `hew.solid.push_pull`, and
`hew.query.resolve` take as is (HEW_API.md §7.4).

- **Cavity probing.** From a point inside a cavity, shoot along each axis
  with `all: true` and `max_distance` set to the largest gap that
  matters. The nearest hit in each direction is a face bounding the
  cavity; its `path` says which placed part it belongs to and `normal`
  which way it faces.
- **Contact search.** Shoot from a hardware anchor along the direction
  it attaches, with `exclude` listing the hardware's own placement
  (`[{ path: [...] }]`), so the first hit is the neighbor's face.
- **Occupied-space check.** Shoot with `all: true` along a groove's
  center line. Crossings come in entry and exit pairs per part, so an odd
  count means the line starts inside a part. For volumes, intersect a
  candidate solid with the part using `hew.compute.boolean` with `op:
  "intersect"` (see "Checking before committing"): nothing is created,
  and `measure.volume` is the overlap.

Hidden parts fall through unless `include_hidden: true`.

### Measuring

`hew.query.measure` with `{ id }` or `{ path }` returns a part's `volume`
(m³), `area` (m²), world-axis `bbox`, and `extents`; groups and
instances sum the parts under them. Volume is defined for closed meshes:
an open part adds nothing and is counted in `open_leaves`, and `volume`
is `null` when nothing under the node is closed. Sizes along the part's
own axes come from `hew.query.part_frame`, not from `extents`.

### Thumbnails

A picture of one definition for a parts list or a picker is one
snapshot: `hew.view.snapshot` with `selection: ["cmp_…"]`,
`background: "transparent"`, and `width` and `height` of 128. The
definition renders once at the identity pose however many times it is
placed, framed in a fitted isometric view. Cache the result by the
definition id and the document `generation`; a definition that has not
changed does not need a new picture. The render is synchronous on the
app's single wasm thread, so keep thumbnails small, request them one at
a time, and never request one inside a tool gesture. The picture is the
software rasterizer's shaded look: no textures, edges, or section planes.

### Outlines

A 2D outline of a part in its own frame, for nesting irregular parts on
a sheet, is one call: `hew.compute.outline { id | path, which }`, in the
part frame by default. `which: "largest_face"` gives the loops of the
part's largest face; `which: "projection"` gives its silhouette seen along
the frame's `z`. Both keep arcs and circles seen face-on analytic, so a
hole drilled through the part comes back as one `circle` edge. Outer loops
run counter-clockwise and holes clockwise. `hew.compute.offset` grows or
shrinks such loops for a tool radius or a kerf.

The same outline can also be built from two reads:

- **Any part.** Read the frame with `hew.query.part_frame`, then ask
  `hew.view.line_drawing` for `selection: [the part]`, `format:
  "segments"`, and a parallel camera looking down the frame's `z`:
  `camera: { eye: origin + z * d, target: origin, up: y, projection:
  "parallel", parallel_height: extents[1] }`, with `d` larger than
  `extents[2]`. Segments come back in view-plane meters with `y` up and
  the origin at the camera's target, which is already the part frame's
  `xy`. The outline is the outermost of the `hard` and `silhouette`
  segments; drop the interior edges yourself.
- **Slab parts**, the common case. Read `hew.query.topology` and take
  the face whose `plane.normal` is parallel to the frame's `z` with the
  largest `area`. Its `outer` loop and `holes` are the outline, and its
  edges' `curve` keeps true arcs intact, which is what a polygon library
  wants.

### Setting axes

A plugin holding `model.write` can change how a part is measured
(HEW_API.md §7.4). Each call is one undo step.

- **A component's axes: `hew.component.rebase`.** Give the definition
  (or any instance of it) a new `origin`, `x`, and `y` in its current
  local coordinates; `x` and `y` must be unit length and perpendicular,
  and `z` is `x × y`. The definition's geometry moves into the new
  frame and every placement's pose takes the frame back, so nothing
  moves on screen: every copy stays exactly where it was, and only the
  axes a person sees when they open the component, and the axes
  `part_frame` reports for its instances, turn. Use it to make a board's
  length run along the definition's `x` once, for every copy. The
  result counts the instances it updated.
- **A plain part's axes: `hew.entity.set_frame`.** Objects and groups
  have no axes of their own, so their frame is an override:
  `{ id, origin, x, y }` in the frame `hew.query.part_frame` reports for
  the part without one (the `face` frame, after its length-width swap),
  or `{ id, clear: true }` to go back to it. Echoing that frame (`origin:
  [0, 0, 0]`, `x: [1, 0, 0]`, `y: [0, 1, 0]`) changes nothing; to turn a
  board's grain across its width, write `x: [0, 1, 0]`, `y: [1, 0, 0]`.
  The override follows the part through any move or turn. A component
  instance refuses `not_a_part`; rebase its definition instead.
- **One mirrored copy: `hew.component.flip_instance`.** `{ instance,
  axis }` mirrors one world placement across its definition's local
  `x`, `y`, or `z` plane, leaving the definition and every other copy
  alone, and answers the new pose and `mirrored`.
- **A mirrored part: `hew.entity.flip`.** `{ ids, axis, copy: {} }`
  makes one mirrored copy across the plane normal to `axis` through the
  selection's center (`about: "origin"` or a point moves the plane;
  `plane: { point, normal }` names any plane). The copy is a linked
  placement at a mirrored pose, so a left and a right side share one
  solid; `copy: { linked: false }` bakes independent geometry, and no
  `copy` flips the selection in place.

`hew.entity.move` also copies into a grid in one undo step:
`copy: { grid: [{ count, step }, …] }` with one to three axes, `count`
the number of slots along an axis counting the original, so `[{ count:
2, step: [w, 0, 0] }, { count: 3, step: [0, d, 0] }]` fills the other
five slots of a 2 × 3 field. A grid takes no `translation`; `copy.count`
keeps its own meaning (that many copies at the step and its multiples).

## Linked copies

A copy made over the bus is linked by default, whatever the person's
"Copies are linked" preference says: the object or group copied becomes
the one member of a new definition (it keeps its id, name, tags, and
attributes), a placement takes its spot, and each copy is another
placement. Editing one copy through its definition edits them all.

- **Copy.** `hew.entity.move` and `hew.entity.rotate` take `copy: {
  count?, grid?, linked? }`. The result's `ids` are the new placements
  (`ins_…`) and `linked` lists each wrap, `{ source, instance,
  definition }`, so a plugin holding `source` can switch to `instance`.
  Several boards copied together make one set each; a group makes one
  set. `linked: false` copies independent geometry. The wrap and the
  copies are one undo step.
- **Edit through any copy.** A face or edge locator takes `{ path:
  [...row.path, row.id], at }` in place of `{ object, at }`, `at` in the
  object's own frame as `hew.query.raycast` reports it, so
  `hew.solid.push_pull` and `hew.material.paint` by path change every
  copy. `hew.solid.subtract` with `a: { path }` cuts every copy.
- **Unlink.** `hew.component.make_unique { path, cascade?, deep? }` gives
  the placement at the end of `path` its own definition and answers `{
  component, instance, remap }`, `remap` mapping each copied id to its
  new id. The copy keeps its name and tags and stays a linked set of one.
- **Make component.** `hew.component.promote { component, name }` turns a
  linked set into a named component; a named component refuses
  `not_linked`.
- **Joining.** `hew.solid.union` or `intersect` with a placement operand
  refuses `would_unlink`, since joining splits it from its copies; pass
  `unlink: true` to explode it into plain solids first (the result lists
  `unlinked`).
- **Change sets.** Every `document_changed` and `hew.query.changes_since`
  change set carries `linked` (the wraps) and `copied_from` (`[new,
  source]` pairs); both are empty for an undo.

```js
const r = await hew.entity.move({ ids: [board], translation: [0.5, 0, 0], copy: { count: 3 } });
const set = r.linked[0].definition;       // the shared solid
const { occurrences } = await hew.query.occurrences({ kinds: ["instance"] });
const copies = occurrences.filter((o) => o.definition === set);
console.log(copies.length, copies.every((o) => o.linked)); // 4 true
await hew.component.make_unique({ path: [copies[3].id] });  // one copy goes its own way
```

## Part libraries

A hardware or cabinet-module library is a folder of files the plugin
ships or the user points it at. `hew.component.import_definition
{ bytes_base64, format, name? }` turns one file's bytes (`hew`, `skp`,
`glb`, `gltf`, `stl`, or `dae`) into one component definition with no
placement, and answers `{ definition: "cmp_…", reused, report }`. Place
it as often as needed with `hew.component.place`: a hundred placements
are one definition. The plugin needs only `model.write`; the call
carries the bytes, so no file permission is involved. The plugin supplies
bytes it already holds: a file bundled with it, or bytes it obtained by
whatever means it is permitted (the app has no command that reads a file
for a plugin).

The definition is keyed by a hash of the format, the units, the bytes,
and any images, so the same file with other units is a different part. Importing
the same file again, in the same session or a later one, returns the same
definition with `reused: true` and changes nothing, so a plugin can call
it every time it needs the part and does not track what it already
brought in. The first import names the definition; to rename it, use
`hew.entity.rename`. A new definition is one undo step; a reuse adds
none. An STL carries no units, so pass `units`; a picture is refused
(`unsupported_for_definition`).

## Checking before committing

Two ways to ask "what would this do?" without changing the document:

- **A dry run** rehearses a whole envelope: `hew.doc.transact` with
  `dry_run: true` runs your commands on a copy and answers exactly what a
  commit would, the same results (the ids a commit would mint included)
  or the same refusal at the same index, with `dry_run: true` beside
  them. Put reads after the edits to read the rehearsed geometry back in
  the same call: a `hew.query.mesh` or `hew.query.measure` on the
  extruded object through `$ref`. Nothing changes: no undo entry, no
  event, no generation move. Use it once, just before you commit, to
  check that the commit will go through and what it will produce;
  `expect_generation` works as it does for a commit.
- **A computation** answers one geometry question: `hew.compute.boolean`,
  `slice`, `shells`, `measure`, `outline`, and `offset` take solids
  (an object by id or path, possibly re-posed, or a prism or box you
  describe inline) and return a volume, a mesh, pieces, or loops. Only the
  operands are copied, so this is the call for a preview on every pointer
  move: "what is left of this board if I cut this groove", "do these two
  parts overlap".

A dry run needs only `model.read`: a command that would need
`model.write` or `attributes.write` counts as a read inside it, because
nothing is committed, so a read-only plugin can check an edit it is not
allowed to make. Your attribute namespace still applies: a rehearsed
write under a namespace you do not own refuses as the commit would.

A dry run cannot rehearse what is not an edit of the model: files, the
library, the screen, the selection, hiding and showing, a Scene's apply,
or another plugin's command. Those refuse `dry_run_host_command`; run them
for real. Undo and the other solitary commands are `-32602` in a dry
run. A dry run copies the whole document, so do not send one per pointer
move.

One computation runs at a time per connection; a second one sent while
the first runs is refused `compute_busy`. Do not queue previews behind
the pointer: when the pointer moves again before an answer arrives, drop
the stale request and send the newest. A computation over more than
2,000,000 triangles is refused `compute_too_large`; compute on one part
at a time.

To cut a part where it sits, in every placement, without creating a
cutter object: `hew.solid.subtract` with `a` as the part's path and `b` a
`{ prism }` or `{ box }` in world coordinates. It is one undo entry, and
undo brings the part back as it was.

## Moving a nested part

A Smart Handle moves, turns, scales, copies, or cuts a part wherever it
is nested, in the same world terms the person drags in, and every copy of
the part follows (HEW_API.md §7, "Transforming a part through a path").

1. **Pick with a path.** `hew.query.raycast` or `hew.query.occurrences`
   gives the part's `path` (the instances above it) and its id. The
   locator is `{ path: [...row.path, row.id] }`.
2. **Move with the same path, by a world delta.**
   `await hew.entity.move({ ids: [{ path: [...row.path, row.id] }], translation: [0.1, 0, 0] })`
   moves the part 100 mm along world +x under that placement. Hew
   applies the move inside the definition, so every other placement of
   it moves too, each through its own pose: a mirrored copy moves the
   mirror image. `rotate` (world `pivot` and `axis`) and `scale` (world
   `anchor`) work the same way. Do not convert to definition space; Hew
   does that.
3. **Copy inside the part's definition** with `copy: { count }` (or a
   grid) on the same call: the copies are new members, so they show in
   every placement. The result's `ids` are the copies.
4. **Cut through a placement.** `hew.solid.subtract({ a: { path: [...row.path, row.id] }, b: cutter })`
   cuts a world cutter body out of the part as it sits in that
   placement; the cutter is consumed, and every placement shows the cut.
   `hew.solid.slice` and `hew.solid.push_pull` take the same path form.

All the parts in one call must be members of one definition reached by
the same path; to move a world part and a nested one together, send two
commands in one `hew.doc.transact`. Each command is one undo entry. While
the person has that definition open for editing, its parts are world
rows, the path form refuses `explode_session_scope`, and the plugin moves
the world rows instead. A copy, a subtract, a boolean between two parts,
or a slice through a path also refuses `explode_session_scope` while the
person has any definition or group open for editing, even an unrelated
one: closing it would gather the new parts into it. Ask the person to
finish editing, then retry.

## Materials

Paint lives in three places, and a face shows the first one it finds:

1. the face's own material (`hew.query.faces` `material`);
2. its object's base material (`material_default` on the object,
   `material` on an object row);
3. for a member of a component, the material of the nearest instance
   above it that has one, walking outward from the member: a nested
   instance's own material wins over its outer instance's.

With none of the three, the face draws in the default gray. This is
SketchUp's rule: painting a component colors only the faces nobody
painted, so a painted face or a painted base keeps its color in every
copy. One definition placed twice can show two colors, one per
placement.

**Writing.** `hew.material.paint` sets each kind (one undo step each):
`{ face }` paints one face; `{ id: "obj_…" }` sets an object's base;
`{ id: "ins_…" }` sets that placement's own material, nested placements
included; `{ id: "cmp_…" }`, or `{ id: "ins_…", scope: "definition" }`,
sets the base of every object the definition owns (its members and the
objects in its member groups, not the geometry of components nested in
it). `material: null` clears. A definition id takes only `scope:
"definition"`, an object id no scope; anything else is `-32602`.
`hew.material.replace` without `object` also switches every instance
whose own material is `from`.

**Reading.** `material_effective` is what a node's unpainted faces show,
before any face's own paint:

- on an `hew.query.occurrences` row: for an object, its base, else the
  nearest painted instance on the row's `path`; for an instance, its
  own material, else the nearest painted instance above it; `null` for
  a group. `material` on the row is an object's base or an instance's
  own.
- on `hew.query.entity` for an instance: its own material. An id names
  no path, so a nested placement's inherited material comes from its
  occurrence rows.
- per face from `hew.query.faces` called with `path` (an occurrence
  path ending at the object): the face's own, else the base, else the
  inherited material. Called with `object`, faces report only their own
  `material`.

A cut list grouping parts by material reads `material_effective` from
the occurrence rows and lets a face's own paint (edge banding, veneer)
override it per face.

**In the app.** At the top of the editing context, the Paint tool
paints the placement a click lands on, and outlines it while the
pointer is over it; inside that component's editing context, a click
paints the face, which changes it in every copy. Alt-click samples the
material the clicked face inherits, the nearest painted placement on
its path. Exploding a painted placement gives every object without a
base, and every nested placement without a material of its own, the
placement's material, so nothing changes color.

## Attributes

A plugin keeps its own data on the model in attribute dictionaries
(HEW_API.md §8): namespace → key → value, written under the plugin's id
or a namespace its grant names. Every entity and the document carry
them, and so do a solid's faces and edges, which have no ids: address
one by locator, `{ "face": { object, at } }` or `{ "edge": { object, at
} }`, as the target of `hew.attr.set`, `hew.attr.get`, and
`hew.attr.delete`. `hew.query.faces` and `hew.query.topology` report
each face's and edge's dictionary as `attrs` (`{}` for none), and a
change set names a write under the object with `scope: "face"` or
`"edge"`. A sketch edge has no edge dictionary (`not_a_solid_edge`);
keep sketch data on the sketch.

**How a face or edge dictionary travels.** It moves with its element
through every edit, by one rule per kind of change:

- **Kept.** An element that keeps its identity keeps its dictionary: a
  face push/pull moves, an edge an operation reuses, and everything a
  transform, copy, explode, make unique, or library insert carries.
- **Inherited.** A face built from another one takes that one's
  dictionary: both halves of a split, the new face of an imprinted shape,
  every piece of a re-trace, each face of a boolean or slice result (from
  the operand face it came from; where both operands contribute the same
  patch, the first operand's), both faces of a thickened sheet, and each
  face of a solid split into pieces. Both pieces of a split edge take its
  dictionary.
- **New.** An element built from nothing has none: push/pull and boss
  walls, extrusion and Follow Me faces, capping faces, and the edges of a
  cut or an imprinted loop.
- **Merged.** When two faces or two edges become one, the survivor keeps
  its own dictionary and the other one's is dropped; there is no size
  comparison. Undo brings it back. A seam between faces whose
  dictionaries differ is part of the solid, as a seam between differently
  painted faces is: cleaning up seams after a boolean leaves it, and the
  drawing tools treat it as the solid's own edge, not as a line that can
  be erased.
- **Rebuilt edges.** Booleans, slices, thickening a sheet, and splitting
  a solid into pieces rebuild every edge from positions, so edge
  dictionaries do not survive them. Face dictionaries do. A plugin that needs an edge marked across a
  boolean marks it again afterwards (or keeps the mark on the faces).

A cut list marking edge banding therefore keeps the band on the edge
through push/pull, copies, and cuts across a face; after a boolean, a
slice, or any other edit that rebuilds edges, the band is gone and the
plugin marks the edge again.

### The document id

A saved document carries a persistent id, `document_id`: a UUID in
lowercase hyphenated text (HEW_FILE_FORMAT.md §2). It exists so state a
plugin keeps per model but not in the model, such as per-document
storage and provenance, can key on the document across sessions. Hew
mints it the first time the document is saved, Save As to a new name
and copies of the file keep it, and it never changes after. A document
that has never been saved has none: `hew.meta.documents` and the
`document_opened` and `document_saved` events report `null` for it, so
anything a plugin keeps for an unsaved document lasts only for the
session, until a save gives the document its id. A library item made
from a whole document does not carry the source's id; a document opened
from it gets its own on its first save.

## Permissions

A plugin's permissions become its grant (HEW_API.md §10), which the
dispatcher enforces on every envelope, inside transactions too. A
permission the manifest does not declare is a `-32001` refusal from the
bus, never a UI-only gate.

| Permission | What it lets a plugin do |
|---|---|
| `model.read` | read the model: queries, attributes, scenes and library listings, snapshots and line drawings, `hew.print.pdf` |
| `model.write` | change the model: every modeling command, scene edits, undo and redo |
| `attributes.write` | write attributes under the plugin's own id and names below it |
| `attributes.write:<ns>` | also write under `<ns>` (a reverse-DNS name, never `hew` or `hew.*`); satisfies `attributes.write` for the command check |
| `view` | move the camera, zoom, change display units; read and change the selection, hide and show, isolate, frame (and hear `selection_changed`, `tool_changed`) |
| `documents.save` | save, export, and import the bound document, and see document paths |
| `documents.discard` | close or revert the bound document |
| `documents.other` | reach documents other than the bound one: attach to them, address them per request, open and create documents, raise windows |
| `files` | the file pickers (with the commands that add them) |
| `clipboard` | the clipboard (with the commands that add it) |
| `library` | insert into, save to, and edit the Hew Library |
| `network:<host>` | talk to one host through the host's network proxy; `<host>` is a lowercase hostname with no port, scheme, or `*` (`network:*` is `permission_too_broad`) |

`all` is not a manifest permission; it marks first-party connections.

**Ceiling.** A plugin's grant runs at the `app` ceiling if it holds
`view`, any `documents.*` permission, `files`, or `clipboard` (all
live-host effects), and at `core` otherwise. A grant never reaches a
command outside its ceiling, whatever it holds.

**Per-command permissions.** Every command in the registry declares the
one permission it needs; the full list is the `Permission` line of each
entry in [API_REFERENCE.gen.md](../API_REFERENCE.gen.md), and HEW_API.md
§10 summarizes it. `always` marks the handshake, attaching to the bound
document, `hew.meta.capabilities`, and the transaction container, which
every grant reaches.

**Also enforced for every plugin:** the plugin's grant is bound to one
document (the envelope `document` member, attaching elsewhere, and the
`document` parameter of `hew.doc.close`, `revert`, and `focus` need
`documents.other`); `all` is never honored on a plugin's grant; parameters that name a file on disk are refused
(`path_not_granted`); attribute writes outside the plugin's namespaces
are refused (`attr_namespace_not_granted`). HEW_API.md §10 and §8 have
the exact shapes.

## Engine behavior record

How this record was produced. The Chromium (Chrome 151) and WebKit
(Playwright, Safari 26.5 build) columns come from
`pnpm --dir app exec playwright test e2e/plugin-realm --project chromium --project webkit`
(plus `PROBE_SPIN=1 ... -g surface-spin` for the explicit-only row), run on
a tree whose parent is commit f7734a5 with the P0 change applied. The Linux
WebKitGTK column comes from `pnpm --dir shells/tauri e2e:build:probe` then
`xvfb-run pnpm --dir shells/tauri e2e:probe` (tauri-driver) on the same
tree. Each run writes `results/<project>.json`; the cells below are copied
from those files, shortened. The Firefox column is produced on demand with
`HEW_PROBE_FIREFOX=1 pnpm --dir app exec playwright test e2e/plugin-realm --project firefox`.
macOS WKWebView and Windows WebView2 have not been run; those cells are
open and the stage is not done until they are filled.

Deviations from the proposed parent policy in the web runs: Vite's dev
server inlines a React-refresh preamble, so the Playwright spec adds the
SHA-256 of each inline script in the served document to `script-src`
alongside the boot hash; nothing else in the policy changes. The desktop
run uses the policy in `shells/tauri/src-tauri/probe.conf.json` unmodified.
The web runs also add `access-control-allow-origin: *` to responses so a
fetch from the opaque-origin realm can only fail by policy, not by CORS.

"pass" means the design's assumption holds. "recorded" means the cell is a
measurement, or the check could not establish a pass (the cell says why).
"fallback" names what the realm does on that engine instead; the
fallbacks are listed under "Fallbacks in force". The desktop run cannot
click, grant permissions, cut the network or count requests, so the rows
that need those are skipped or recorded as not exercised there.

| Row | Chromium | WebKit (Playwright) | Firefox | macOS WKWebView | Windows WebView2 | Linux WebKitGTK |
|---|---|---|---|---|---|---|
| boot-hash | pass | pass | not run | not run | not run | pass |
| worker-blob | pass | pass | not run | not run | not run | pass |
| worker-module-type | recorded: module worker does not run | recorded: module worker works | not run | not run | not run | recorded: module worker works |
| worker-env | pass | pass | not run | not run | not run | pass |
| worker-denials | recorded: Cache API absent in the worker, so the two Cache.add entries were not exercised; nothing allowed | recorded: nothing allowed; every block lacks a CSP violation report; caches.open("x") allowed (accepted); both Cache.add entries blocked | not run | not run | not run | recorded: nothing allowed; every block lacks a CSP violation report; caches.open("x") allowed (accepted); both Cache.add entries blocked |
| worker-wasm | pass | pass | not run | not run | not run | pass |
| worker-isolation | recorded: host=false frame=false worker=false sab=undefined | recorded: host=false frame=false worker=false sab=undefined | not run | not run | not run | recorded: host=false frame=false worker=false sab=undefined |
| eval-blocked | pass | pass | not run | not run | not run | pass |
| worker-spin-host-alive | pass | pass | not run | not run | not run | pass |
| surface-boot | pass | pass | not run | not run | not run | pass |
| surface-styles | pass | pass | not run | not run | not run | pass |
| surface-img-self | pass | pass | not run | not run | not run | pass |
| surface-navigate-remote | pass | pass | not run | not run | not run | recorded: loadCount=1; request count and frame URL are not observable on desktop |
| surface-navigate-self | recorded: loadCount 0; request for /?probe-escape=1 observed (the frame navigated to the host) | recorded: loadCount 0; request for /?probe-escape=1 observed (the frame navigated to the host) | not run | not run | not run | recorded: loadCount 1 |
| surface-click-activation | pass | recorded: writeText ok; readText refused (NotAllowedError); no clipboard permission API | not run | not run | not run | skipped: needs a driver (not available on desktop) |
| surface-webgl2 | pass | pass | not run | not run | not run | pass |
| surface-hit-regions | pass | pass | not run | not run | not run | skipped: needs a driver (not available on desktop) |
| offline-boot | pass | recorded as FAILED: setOffline(true) breaks the surface handshake on this engine (see rows that failed with no fallback); with all http(s) requests aborted instead, both handshakes completed and 0 requests were made | not run | not run | not run | recorded: offline not exercised (no network cut or request count on desktop); both handshakes completed |
| latency | recorded: worker p50=0.1 p95=0.2 ms; surface p50=0.1 p95=0.2 ms | recorded: worker p50=1 p95=1 ms; surface p50=0 p95=1 ms | not run | not run | not run | recorded: worker p50=1 p95=1 ms; surface p50=0 p95=1 ms |
| throughput | recorded: 1000 messages in 19.9 ms (1000 items) | recorded: 1000 messages in 16.0 ms (1000 items) | not run | not run | not run | recorded: 1000 messages in 10.0 ms (1000 items) |
| transfer | pass | pass | not run | not run | not run | pass |
| surface-spin | recorded: HOST FROZEN: the page stopped answering while the spinning surface was alive (see fallbacks) | recorded: HOST FROZEN: the page stopped answering while the spinning surface was alive (see fallbacks) | not run | not run | not run | not run (explicit-only) |

### Fallbacks in force

- All engines, worker-module-type: the boot script starts a classic Worker, never a module Worker. A module Worker from a blob URL in an opaque-origin frame does not run on Chromium.
- Chromium, worker-denials: the Cache API does not exist in the blob worker, so the two required `Cache.add` entries could not be shown blocked; the API being absent is accepted as the denial (nothing can be called), and the cell stays "recorded", not "pass".
- WebKit (Playwright) and Linux WebKitGTK, worker-denials: no `securitypolicyviolation` was reported for any entry, so the policy is not shown to be what blocked them (several blocks are fetch load failures, an invalid-URL error for relative URLs, or SecurityError). Accepted as denials in effect: `Cache.add` of a same-origin and of a remote URL, and every fetch/socket/IndexedDB entry, failed. `caches.open("x")` itself succeeds (an ephemeral per-realm Cache API); it is not a network or cross-plugin path.
- WebKit (Playwright) and Linux WebKitGTK, surface-navigate-remote: Linux only has the load count (1) and cannot show whether a request left; the WebKit run showed zero requests and a frame URL of `about:srcdoc`. The Linux cell therefore stays "recorded" rather than "pass".
- All engines, surface-navigate-self: the surface's own navigation to the host origin is not blocked by the sandbox (Playwright saw the `/?probe-escape=1` request). The realm must treat a surface navigation to a same-origin URL as reachable and rely on the host's own rules for that URL.
- All engines, surface-spin: a spinning surface script freezes the host page on both Playwright engines (the driver's evaluate gets no answer and the row never finishes). A surface cannot be assumed to run off the host's thread; the realm design must not rely on a frozen surface leaving the host responsive (kill by removing the frame is only possible when the host itself still runs).
- WebKit (Playwright), surface-click-activation: `navigator.clipboard.readText()` is refused (no permission API on this engine), so the read half is recorded, not passed.
- Cross-origin isolation is not used on any engine: the host sends no COOP/COEP, `hew.supports("wasm.threads")` is false everywhere, and a Worker is required on every engine (no `background.worker` fallback).

### Rows that failed with no fallback

- WebKit (Playwright), offline-boot: the stage's procedure (Playwright `setOffline(true)`) breaks the surface handshake on this engine (`surface-boot` timed out; `boot-hash` passed). The stage specifies no fallback for this row. A substitute run that aborts every http(s) request instead completed both handshakes with zero requests, and that outcome is kept as evidence, but the row is recorded as failed as specified and needs the maintainer's call (accept the substitute procedure, or find a different offline emulation for this engine).
- Linux WebKitGTK, offline-boot: no network cut or request counting is available in the desktop run, so the zero-request criterion was not tested there.
