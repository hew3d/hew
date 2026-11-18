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
implemented. A person can install, switch off, roll back, and remove a
plugin from Settings (see "Installing"), but nothing runs one yet, so the
app answers every forwarded command `plugin_unavailable`, aborting the
compound it opened for a mutating one at once.

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
`contributes` members other than `commands`, `presave` and `events` are
read as opaque JSON until the parts that use them land.

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
| `contributes` | optional; `commands`, `presave` and `events` are validated now, every other member is opaque for now |
| `contributes.events` | optional; 1 to 32 distinct event names from the table in "Events" (`document_changed`, `selection_changed`, ...): the events that start the plugin when `activation` includes `on_event` (see "How a plugin runs"). An unknown, repeated or surplus name makes the file `manifest_malformed`, as an unknown activation value does |

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

**MCP.** `hew-cli mcp`'s `hew_query` tool admits every command that does
not change the document: the commands whose class is `read_only` (a
plugin's `read_only` commands included), plus `hew.view.snapshot`,
`hew.view.line_drawing`, and `hew.history.status` by name, minus any
command that mutates the document (`hew.library.save`). View and
selection state commands in that class (`hew.view.hide`/`show`/
`show_all`/`isolate`/`zoom_to`, `hew.selection.set`) change what is
hidden, framed, or selected, not the geometry, and are admitted. Anything else,
and any unknown method, is refused; `hew_transact` runs the rest. A raw
socket client that sends `path` with a snapshot or line drawing gets
`host_capability_missing`; `hew-cli dispatch hew.view.snapshot
'{"path":…}' --live` strips it, asks the app for the bytes once, and
writes the file.

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
  An inner command still running when its outer command ends (the
  handler answered without waiting for it, or the outer was cancelled,
  ran out of time, or its plugin stopped) is ended first: its caller is
  answered `plugin_command_cancelled`, its plugin is sent the cancel, and
  its compound is aborted. Then the outer one commits or aborts. Await
  every command a handler calls.
- **A call made for a command that has ended is refused.** A bus call
  that names a forward that was answered, cancelled, or ended under its
  outer command is answered `-32000` `forward_ended` and nothing runs;
  it never lands as a write of its own. A handler that carries on after
  its `ctx.signal` aborts gets this refusal, and its failure from then on
  is not reported (the caller was already answered).
- **What undo does not reverse.** Undoing the entry reverses every
  undoable change in it. Registry writes that record no undo entry
  anywhere (HEW_API.md §6.4) keep that contract inside a compound:
  `hew.material.create`, `hew.tag.create`, `hew.tag.set_visible`,
  `hew.scenes.*`, and a Library material insert stay after the entry is
  undone. An abort, unlike undo, does take them back.
- **The 50-second ceiling.** A command's `timeout_ms` is clamped to
  1000 .. 50000 (50000 when it declares none), counted from the moment
  its plugin is handed the call ("Limits" below). A command that runs out
  of time is answered `-32003` with `reason: "timeout"`, its compound
  aborted, and its plugin stopped. Longer work runs as a job
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

**Starting on an event.** A plugin that lists events in
`contributes.events` and has `on_event` in `activation` is started by
the first of them, and is handed that event (and any that came while it
started) as soon as it has started. Its connection is already subscribed
to those of its events its grant receives, so `hew.events.on` for one of
them hears every one from the first; subscribing again is harmless. An
event its grant cannot receive does not start it. See "How a plugin
runs".

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
an instance a session cannot take (unevenly scaled, or placed in a group
elsewhere; a mirrored instance opens a session like any other), and selects the target. `hew.view.exit` steps out one
level and `hew.view.exit_all` all the way. This is not `hew.context.enter`,
which stays balanced inside a transaction and scopes a plugin's own
edits; the frame `hew.view.enter` opens is the person's.

The recipe for an outliner panel:

1. On a row's double click, call `hew.view.enter` with the row's id, or
   with its occurrence path when the row is a part inside an instance. The
   answer is `{}`; do not read the result for what opened. If the window
   declines the entry, the answer is a refusal (`nested_component_in_context`
   or `enter_refused`) and the window is where it was.
2. Listen for `context_changed`. Its `stack` lists the open group and
   component frames, outermost first, and `fallback` lists the in-context
   entries (`{kind: "object" | "instance", id}`) inside the innermost frame.
   Draw the breadcrumb from `stack` followed by `fallback`.
3. Call `hew.query.context` for the same two lists on startup, or after a
   refusal. A refused entry leaves the context as it was. A refused path may
   still send `context_changed` for the frames it opened and then took back,
   so draw from the latest event, which shows where the window is.
4. Handle `gesture_in_progress`: a tool gesture (a Move drag) is under way,
   and the call changes nothing. Try again when it ends. `hew.query.context`
   reports `armed`.

Each step the window takes is reported as it happens: entering through N
session frames sends N `context_changed` events, one per frame, and
`hew.view.exit_all` sends one per frame it closes. Every event carries the
whole `stack` and `fallback`, so draw from the latest one and do not count
events. `hew.view.exit` at the top level (no frame and no in-context
entry open) refuses `nothing_to_exit`; `hew.view.exit_all` there answers
`{}` and sends no event.

What a plugin can rely on: the call answers after the window has acted. A
node whose definition nests other groups or components, placed unevenly
scaled or inside a group, cannot be entered; the answer is the refusal
`nested_component_in_context`, and the person sees a message saying so. Any
other entry the window declines answers `enter_refused`, with the reason in
`detail.code`. In both cases the window is where it was: steps of a path
taken before the declined one are taken back. The limit that stays: the
window's Redo then holds the step that was taken back, and (when the
window had opened a frame first) any redo history the person had before the
call is gone. If the window could not be put back,
`detail.restored` is `false`; call `hew.query.context` to see where it is.

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

A row's `path` lists instances only, and never the row's own id. A member
of an instance has the path `[...instance.path, instance.id]`. A member of
a group has the group's own `path`, with `parent` naming the group.
`members` returns bare ids, so a member's locator follows that rule. A
door group `grp_d` in the world holding a hinge instance `ins_h`, whose
definition holds `obj_pin`:

```
grp_d    path: []         parent: null
ins_h    path: []         parent: "grp_d"   (a group member: the group's path)
obj_pin  path: ["ins_h"]  parent: null      (an instance member: [...path, instance])
```

`obj_pin`'s locator is `{ path: ["ins_h", "obj_pin"] }`, `[...row.path, row.id]`.

An object row's `world_pose` is its placement's pose (the pose of the
path, with the instance's own pose included), not the object's geometry.
Moving the Object inside its definition leaves `world_pose` as it was,
because the geometry moved, not the placement. To see such an edit, read
`hew.query.measure { path }` and compare its `bbox`.

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

### Where a locator's `at` is read

A face or edge locator reads `at` (and `ray`) in the object's own frame,
as `hew.query.faces` and `hew.query.raycast` report them. For an Object
with no placement above it, that is the world frame. For a member reached
through a placement (`{ path: [instance, …, object], at }`) it is the
definition's coordinates, not world coordinates, so the same point is
valid through every placement. When a locator whose path goes through a
placement misses, the `locator_missed` refusal names the frame.

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

`scope: "own"` includes own Objects that are not closed solids (a stray
face, an imported surface), so a part's frame and extents grow to cover
them. Pass `solids_only: true` on `hew.query.part_frame` or
`hew.query.measure` to leave those out: an Object counts when its row's
`solid` is true. It works with any `scope` and composes with `exclude`.
When it leaves nothing to measure, the answer is the one `exclude` gives
when it leaves nothing (`no_part_frame` for `part_frame`, `-32602` for
`measure`). The default is `false`, which changes no existing answer.

`hew.view.snapshot` with `orient: "part"` and one `selection` entry
renders that part in its own frame: `view: "front"` looks along the
frame's `y`, `top` down its `z`, so every board in a parts list is drawn
the same way however it sits in the model.

### Same part, mirrored part

`hew.query.signature { id | path }` answers `{ shape, mirror, symmetric,
frame: { origin, x, y, z, extents } }` for one object. Two objects with equal `shape` are the same
part in any position; `a.shape == b.mirror` means `b` is `a`'s mirror
image (a left and a right side); `symmetric` means the part is its own
mirror image. The hashes survive a save and reload and any move or turn,
to 1 µm. What a face or edge claims is part of the shape: corners drawn
as a circle, as a regular polygon, or as plain lines are three different
parts, and a part with a cylinder wall, a fillet, a crease or a Smooth
level is not the same part without it. "Link identical copies" compares the
same things, so two parts with equal `shape` can be linked. `frame` is
the part's face frame, `origin` its minimum corner and `extents` its size along the axes; the pose that carries one
matching part onto another is `frame_b ∘ frame_a⁻¹`, and onto its
mirror twin `frame_b ∘ C ∘ R ∘ C⁻¹ ∘ frame_a⁻¹` with `R` a reflection
and `C` the shift to the frame's center, `origin + extents / 2` (a
reflection about `origin` would land the part off by its own size). A
group or instance
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
document, `hew.meta.capabilities`, the transaction container, the event
subscriptions, and (a Preview command) `hew.permissions.request`, which
every grant reaches.

**Also enforced for every plugin:** the plugin's grant is bound to one
document (the envelope `document` member, attaching elsewhere, and the
`document` parameter of `hew.doc.close`, `revert`, and `focus` need
`documents.other`); `all` is never honored on a plugin's grant; parameters that name a file on disk are refused
(`path_not_granted`); attribute writes outside the plugin's namespaces
are refused (`attr_namespace_not_granted`). HEW_API.md §10 and §8 have
the exact shapes.

### Optional permissions

A Preview feature. `optionalPermissions` are granted later, at the
person's say, two ways:

- **In Settings > Plugins.** The person grants one, or takes one back,
  with a switch for each optional permission on the plugin's detail page
  (Overview). Granting asks first (the desktop app's own confirmation
  through `plugin_set_grant`, where a no leaves the switch off quietly and
  `confirmation_open` is a plain notice; on the web a sheet, the consent
  sheet in its grant mode, titled "Allow more for this plugin?", whose
  lines come from the kernel's `plugin_grant_lines` and never from
  TypeScript, with Cancel focused, Enter accepting nothing and "Allow" the
  yes); taking back asks nothing. The switch shows the stored record, not
  the click. The web store's `setGrant(id, grant, optional)` only writes,
  under the store lock, refusing names the manifest does not declare
  (`unknown_permission`), and answers with the record as stored. The same
  sheet, `decideGrant(id, add)` in `app/src/plugins/grantSheet.tsx`, is what
  a running plugin's `hew.permissions.request` shows on the web, over the
  document window; it refuses `plugin_changed` when the record moved while
  the person decided.
- **When the plugin asks.** `hew.permissions.request {permissions,
  reason?}` (HEW_API.md §7) from the plugin's own connection. Every name
  must be one of the manifest's `optionalPermissions`
  (`permission_not_requestable` names the others). The plugin must ask
  right after the person acted in the plugin's own window or panel
  (`no_user_activation` otherwise; the act counts for 5 seconds and for
  one request). No plugin has a window or panel yet, so for now every
  request a plugin makes is refused `no_user_activation`; the test
  harness stands in for the act (`__hew_test.plugins.activate(id)`, which
  counts only while the plugin runs). A request still waiting when the
  plugin stops or reloads is dropped with it: the next run is not told
  `request_pending`, and its answer goes nowhere.
  One request at a time (`request_pending`); after the person says no,
  a minute of `request_declined` in that window (each window's host keeps
  its own minute, so the same plugin in another document window may ask
  at once; a reload of the plugin in the same window does not reset it);
  names the plugin already holds are answered at once, without asking. Shop Mode refuses every request
  `not_available_in_shop`, before anything else. While the person
  decides, the clock of the command the request was made in is stopped,
  for at most 5 minutes; past that the request is refused
  `request_declined` and the clock runs again. The question stays on the
  screen, though: if the person then says yes to it, the permission is
  granted after all, and the plugin learns of it the way it learns of a
  grant made in Settings (`{t: 'grant'}`, `hew.plugin.grant`,
  `hew.lifecycle.on('grant', fn)`), not from the request it was already
  told no to. No minute of refusals follows a request that ran out of
  time. Only a running plugin's
  own connection is answered: any other caller (a socket client, an
  agent, a connected app) gets `host_capability_missing`.

**What a plugin can rely on.** A grant that only adds optional
permissions reaches the running plugin in place: its connection holds
the new grant from the next call on (inside the command it is serving
too), the realm receives `{t: 'grant'}` and `hew.plugin.grant` changes,
`hew.lifecycle.on('grant', fn)` handlers run with the new grant, and
events the old grant refused at `hew.events.on` are asked for again. A
yes to `hew.permissions.request` is applied before the answer arrives,
and the answer, `{granted}`, lists the names asked for that the plugin
now holds. Anything else that changes the grant (a permission taken
back, a required permission, a new version) reloads the plugin
(`unload` `reload`); after a permission is taken back, calls that need
it are refused `-32001` as for any permission not held.

## Installing

Installing is a person's job, done in Settings ▸ Plugins. It is a Preview
feature: it exists only in a preview build and only while the Plugins
switch in Settings ▸ Preview is on. With the switch off, or in a
production build, the tab is absent and the stores below are never read,
written, or listed. A plugin that is turned on runs in a sandbox in each
document window ("How a plugin runs" below).

**An agent cannot install or manage plugins.** No bus command, `hew-cli`
subcommand or MCP tool installs, changes or removes one. In a preview
desktop build the shell answers four bus commands itself, and none
completes anything that manages a plugin: `hew.plugin.list` and
`hew.plugin.activity` read, `hew.plugin.dev` only asks the app to show a
request ("Developer mode and folder plugins" below), and
`hew.plugin.reload` only asks the windows to reload a developer-folder
plugin that is already running ("Plugin data" below). A plugin package is never opened by File ▸ Open or by dropping
it on the window, and Hew registers no `.hewplugin` file type or link
handler. The one door is the Settings pane
(`app/src/plugins/installer.ts`).

### What can be installed

| Tier | Sources |
|---|---|
| Desktop | a `.hewplugin` file, or (with developer mode on) a developer folder, each chosen with the native picker |
| Web | a `.hewplugin` file chosen in the pane's file input, or a copy of a folder chosen with the browser's folder picker where it has one |

**A folder on the web is a copy.** There is no developer mode and no
live folder on the web. Where the browser has a folder picker
(`showDirectoryPicker`; Chromium-based browsers), the pane offers "From
folder…": the page walks the picked folder under the package reader's
own name rules (printable ASCII; none of `\ : < > " | ? *`; no segment
ending in a dot or a space; no Windows device name; no two names equal
ignoring case; paths up to 1,024 bytes) and caps (4,096 files, 32 MB in
one file, 64 MB in all, the zip's own headers counted), measured on the
sizes the browser reports before a byte is read, so an oversized folder
is refused without being read into memory. The walk itself is bounded:
every entry it visits, directories included, counts against the 4,096
cap, and nesting deeper than 512 levels (all a 1,024-byte path can hold)
is refused, both as `package_too_large`, so a tree of empty folders or a
listing that never ends is cut off. A file whose size changed between
the measure and the read is refused `package_unreadable` before its
bytes are read, and the bytes are read through a stream with a running
count that stops at the measured size, so a file that lies about its
size is never held beyond that cap. The
files are written into a stored zip in memory
(`app/src/plugins/storedZip.ts`) and passed through the same package
path as a file, so the kernel's reader judges the result as it judges
any package, then the ordinary consent sheet. It is recorded as a file
install named after the folder (`{kind: "file", name: "<folder>"}`),
with an `origin: "folder"` field in the stored record only; the kernel
reader ignores the field, the app reads it back, and the row says
"Copied from a folder" with the cleaned name set apart on its own line.
Nothing is read from the folder again: to take a change, pick it again.
The browser's picker does not show symbolic links, so the page cannot
refuse one; it reads what the link points to.

A package is a zip whose entries are all stored with no compression
(`zip -0 -r ../tool.hewplugin .` from inside the plugin folder), with
`hew-plugin.json` at its top level. The reader refuses a compressed
entry (`package_not_store_only`); a path that is not plain ASCII, is
absolute, has a `.` or `..` or empty segment, a backslash, a `:`, a
segment ending in a dot or a space, or a Windows device name
(`package_path_invalid`); two entries that are equal after case folding,
a repeated name, or a symbolic link (also `package_path_invalid`); more
than 4096 entries, 32 MB in one file, or 64 MB in all
(`package_too_large`, measured on the bytes read); and a manifest that
names a `main`, `icon`, `agent.notes`, or `locales.catalogs` the package
does not hold (`manifest_file_missing`). An id that starts `org.hew3d.`
is kept for plugins Hew publishes and is refused from a file or folder
(`reserved_id`); a `hew` range the running Hew is outside
(`hew_version_unsupported`), a `requires` entry that is not installed in
a matching version (`dependency_missing`), or one that would make a cycle
(`dependency_cycle`) refuse the install before the sheet appears.

A folder install reads the folder in place, so its files can change
after the install; the sheet says so.

### Developer mode and folder plugins

A folder install is for someone writing a plugin, and runs code straight
from a folder, so the desktop offers it only in developer mode. Developer
mode belongs to the shell: it is kept in the file
`plugin-developer-mode` in the app's config folder, read once at start,
and anything in that file but `on` (with or without one line break)
reads as off, so a damaged file turns nothing on. The page's copy is for
display only. Desktop commands:

| Command | Arguments | Returns |
|---|---|---|
| `plugin_dev_mode` | none | `boolean` |
| `plugin_set_dev_mode` | `on` | `boolean`, the value now |
| `plugin_dev_review` | `id` | `{token, consent}` |
| `plugin_dev_request` | none | `{id, path}` (as the request event carries it) or `null` |
| `plugin_dev_request_dismiss` | `id` | nothing |
| `plugin_host_present` | `present` | nothing; the window's plugin host started or stopped |
| `plugin_pick_dir` | `useRequest?: string`, a request's id | the picked folder or `null` |

Turning developer mode on asks in a native dialog: "Turn on developer
mode?", "Hew will run plugin code straight from a folder, where the files
can change at any time. Turn this on only if you are writing a plugin.",
accept "Turn On" (a no is `developer_mode_declined`; `confirmation_open`
while another dialog shows). It is written to its file before it takes
effect. Turning it off asks nothing and takes effect at once: every
folder stops being watched, folder installs waiting for an answer and any
live API request are dropped. Then it is kept: the file is written as
off, or removed if that fails (no file reads as off); if that fails too
it stays off for this run and the answer is `developer_mode_not_saved`,
"Developer mode is off for now, but Hew could not save that, so it may be
on again the next time Hew starts.", which the pane shows. Either way
`plugins-changed` is emitted.

With developer mode off, picking a folder, staging one, reading a folder
plugin's file, watching it and reviewing it are all refused
`developer_mode_off`, for a folder installed before as much as a new one.
Such a plugin stays listed and does not run. Packages are not affected.

**The folder.** The canonical path recorded at the pick is the only
root. Every read of a folder plugin's file, the icon included, goes
through one reader (`plugin_host::store::read_jailed`): the root must
still be a folder (`folder_missing` when it is gone or is no longer a
folder), must not be a symbolic link, and must canonicalize to exactly
the recorded path, so no folder above it has become a link either
(`package_path_invalid`). Under it no segment may be a link, the file is
opened once, and what was opened must be a regular file whose canonical
path is inside the root; on Unix it must also be the very file the
checks saw and the one its canonical path names now (device and inode).
A review reads the folder through the same reader.

**Saving reloads; asking for more stops.** While developer mode is on
and the page has listed plugins, the shell polls the folder of every
enabled folder plugin about twice a second, comparing each entry's
path, size and modification time, without following links and stopping
at the package caps (an entry whose name is not valid UTF-8 is
`package_path_invalid`, as in a package); a burst of saves is one change
after about 300 ms of quiet. Then it reads the folder's
`hew-plugin.json` through the reader and compares it with the stored
copy. A field is reviewed when it carries authority or the consent sheet
showed it; every other change refreshes the stored copy:

| What changed | What happens |
|---|---|
| code or other files only | nothing stored changes; `plugins-dev-changed` with `manifest_changed: false` |
| fields that need no review: `version`, `main`, `icon`, `locales`, `description`, `author`, `license`, `homepage`, `source`, `api`, `data`, `agent`; activation events other than a new `on_startup`; commands added (with their class's default permission) or removed; a command's `summary`, `label`, `timeout_ms`, `concurrency`, `params`, `result`, `refusals`; `hew` or `requires` that are met | the stored copy and the record's `version` follow the folder; the grant is kept; `plugins-changed`, then `plugins-dev-changed` with `manifest_changed: false` |
| `permissions` or `optionalPermissions` differ as sets, either way | the record is marked `needs_consent` (`permissions_changed`) |
| `on_startup` newly asked | marked (`startup_added`) |
| `name` or `activationReason` | marked (`consent_text_changed`) |
| a kept command's `class` or `permission`; an added command whose `permission` is not its class's default; `presave` or any other `contributes` kind | marked (`authority_changed`) |
| a command whose name (`<plugin-id>.<command-id>`) another installed plugin declares | marked (`command_name_taken`) |
| the manifest no longer reads | marked (`manifest_unreadable`) |
| it names another id | marked (`id_changed`) |
| its `hew` range or `requires` changed and are not met | marked (`requirements_unmet`) |

When it is marked, `plugins-changed` and then `plugins-dev-changed` with
`manifest_changed: true` and the reason are emitted, and every read of
the folder is refused `dev_consent_pending` until the change is reviewed.
The stored manifest and the grant are not touched, so what the plugin
may do never widens before the person says so. The folder stays
watched: a mark describes a difference, so when the folder's manifest no
longer differs from the stored one in anything reviewed (the typo is
fixed, the permission taken out again) the mark is cleared by itself,
with `plugins-changed` and `plugins-dev-changed` with `manifest_changed:
false`, and the host starts the plugin as before. Nothing else clears a
mark but an accepted review or a fresh install. On `manifest_changed:
false` the host reloads the plugin, and starts it again if it was
running or starts with Hew. `plugins-dev-changed` with a `problem`
(`folder_missing`, `package_path_invalid`, `package_too_large`) says the
folder cannot be watched right now; it is sent once until the folder
lists again, and `plugin_list` carries the same name as `dev_problem` on
the folder plugin's row (`null` when there is none), so a window that
opens later shows it too; the row says `folder_not_checked` while the
plugin is not ready (below).

**Compared when watching starts.** A folder edited while nobody watched
it (Hew closed, developer mode off, the plugin turned off) is compared
the same way when the plugin starts being watched: the first
`plugin_list` after Hew starts (before it answers), developer mode
turned on, the plugin turned on, installed, reinstalled, reviewed or
rolled back. For turning on, installing, a review and a rollback, the
folder's manifest is read before the command writes and compared under
the same hold of the store lock as the write, so the record a host can
read is already marked; otherwise the comparison runs before
`plugins-changed` is sent. The watcher compares again on its first
listing of the folder.

**Nothing is read before a comparison completes.** From the moment a
folder plugin would start being watched until one comparison of its
folder has completed, it is unchecked: every read of its folder is
refused `folder_missing` ("Hew is still checking this plugin's
folder."), so the host holds it as not startable rather than crashed;
`plugin_list` gives it `dev_problem: "folder_not_checked"` (or the
watcher's own problem, if it has one) and `hew.plugin.list` gives it
`state: "checking"`. The set of checked plugins lives in the shell's
memory: it starts empty with Hew, is emptied when developer mode goes
off, loses a plugin that stops being watched, and loses a plugin whose
record a command rewrites (turning it on, an install, a review, a
rollback) until that command's own comparison completes. A comparison
whose read stalls or fails leaves it unchecked, and the watcher tries
again every pass, whether the listing changed or not, until one
completes; then the host is told to try the plugin again. A second
window that lists plugins while the first window's comparison runs, or a
host that reads just after developer mode turns on, gets the refusal,
not the folder. A
rollback of a marked folder plugin brings back the older record and
compares the folder with the older manifest, so it is marked again when
the folder asks for more than that version did.

**Bounded.** Each folder's walk runs on its own short-lived thread and
is waited for about a second; a folder that takes longer (a network
mount that stopped answering) is reported once as `folder_missing` and
not walked again until that walk returns, so it holds up the others for
one interval at most. Walks stop between entries when the watcher is
stopped, and stopping waits for the watcher at most two seconds before
letting it go, so turning developer mode off and quitting are never held
by a folder. A command's own reads of folder manifests are bounded the
same way. Bytes read from a folder while developer mode went off are
dropped (`developer_mode_off`), and a folder stage that read its folder
while it went off is refused and holds no slot.

Event payload: `{id, manifest_changed, reason, problem}`, the last two a
string or `null`.

**Review.** `plugin_dev_review(id)` reads the recorded folder again
(no new pick) and stages it, for a folder plugin with the marker only
(`no_review_pending` otherwise, `folder_id_changed` when the folder now
names another plugin, and any refusal a folder install gets). The
consent payload has `review: true` and lists what was added and what was
removed, and is shown whatever the version. `plugin_commit` then asks
natively, "Allow this plugin's changes?" with accept "Allow"; a yes
replaces the stored manifest and the grant in place (no `previous/`, and
`installed_at_ms`, `enabled`, `data_version` and saved data kept; the
optional grants it still declares kept; starting with Hew allowed again
only by turning it on, if it newly asks to), clears the marker, and emits
`plugins-changed` and `plugins-dev-changed` with `manifest_changed:
false`. A no, a dropped review and one left past ten minutes all leave
the marker; Review can be asked again. A fresh install of the folder
through the ordinary sheet also clears it.

**The live API asks; the person picks.** `hew.plugin.list` (read-only)
answers `{plugins: [{id, name, version, source, enabled, state}],
developer_mode, request_waiting}`: `name` is the cleaned display name, `source` the
source kind, and `state` one of `needs_developer_mode` (a folder plugin
while developer mode is off), `needs_review`, `checking` (an enabled
folder plugin whose folder has not been compared yet), `plugins_off`
(the same while no window runs a plugin host), `unreadable` or
`installed`; `request_waiting` is whether a live API request still waits. Whether a plugin is running is the page's to know and is
not in it, nor is any path. `hew.plugin.dev {path}` keeps at most one
request, with a random id, for ten minutes. It is refused, recording
nothing:

| Refusal | When |
|---|---|
| `invalid_params` (`-32602`) | no string path, empty, over 4096 bytes, or nothing left once cleaned |
| `plugins_off` | no open window runs a plugin host now (the page reports it with `plugin_host_present` as its Plugins switch turns the host on and off) |
| `developer_mode_off` | developer mode is off |
| `confirmation_open` | a native dialog (a confirmation or a picker) is showing |
| `request_dismissed` | the person dismissed a request less than 60 seconds ago |
| `request_waiting` | a request for another path is waiting |
| `dry_run_host_command` | sent inside a `hew.doc.transact` with `dry_run: true` |

The same path while its request waits answers `awaiting_user` again and
shows nothing new, so a client that asks in a loop cannot bring a
notice back. Otherwise it emits `plugins-dev-request {id, path}` (the
path cleaned and cut to 512 characters) to every window and answers
`{status: "awaiting_user"}`. It never picks, stages, or reads the
folder, and the path is not recorded as picked. `plugins-dev-request`
with `null` tells every window the request is gone: its folder staged,
dismissed, expired, developer mode turned off, or the last host gone (a page
saying its host stopped, or its window closing: the shell drops a closed
window's label itself, and a label whose window is gone never counts).

`plugin_dev_request` answers the waiting `{id, path}` (or `null`) without
taking it. `plugin_pick_dir({useRequest: id})` opens the native folder
picker for request `id` only while it is the one waiting, and only looks
at it; otherwise it is refused `request_gone` and shows nothing. The
request is used up by the folder it led to staging (`plugin_stage_dir`
for a pick made from that request), a Dismiss, expiry, or developer mode
going off. A cancel, or a folder that is refused, leaves it waiting with
the same id and expiry, so the card stays and the person can choose again.
The picker opens at the requested folder itself (the person asked for it
from their own terminal in the ordinary case; the consent sheet and the
native confirmation that follow show the picked folder on its own line),
and only when the path came through cleaning
unchanged, is written plainly (absolute, no `.`, `..` or empty segment),
is not a network place as far as the text tells (on Unix `//`, `/net`,
`/Network`, `/afs`, `/run/user/<n>/gvfs` and `/run/user/<n>/doc`; on
Windows anything but a drive letter on a fixed disk), and is not inside
the plugin store or the app's data or config folders; otherwise it
opens with no start folder. A share mounted elsewhere (`/Volumes/<name>`,
`/mnt/<name>`) cannot be told from the text. What gets staged is what
the person picks. The consent sheet shows that folder's path as data (up
to 512 characters, cut in the middle when longer), the pane row up to
120, and the native confirmation the cleaned path on its own line under
"Developer folder:" (up to 160, cut in the middle). The picker opens at
the hinted folder only after it is resolved to where it really is (a link into the plugin store or the app's folders is
refused by its target), and with no start folder when it cannot be
resolved. A folder whose path holds a control character, a line break
or a character that changes text direction is refused at the pick
(`folder_path_invalid`). Every value from outside Hew in a native
confirmation (a folder path, a file name, an earlier name, another
plugin's name) is on a line of its own after a fixed label line,
cleaned and cut in the middle, never inside a sentence or quotes. `plugin_dev_request_dismiss({id})`
forgets the request in every window and starts the 60-second quiet. The
pickers (`plugin_pick_dir`, `plugin_pick_package`) count as native
dialogs: while one is open every other dialog command is refused
`confirmation_open`, and they are refused while another is showing. A
folder whose path is not valid UTF-8 is refused at the pick
(`folder_path_invalid`), since its record keeps the path as text. In a
production build neither bus command exists in the shell. A plugin's
realm cannot reach either: its calls go to the kernel of its page over a
connection opened with its grant, which refuses them, and the shell's
router serves only the live API socket.

**The command line asks the same way.** `hew-cli plugin dev <folder>
--live` (a preview build of `hew-cli` only; a production binary does not
list it and answers `plugin` as an unknown word) resolves the
folder to its canonical absolute path (it must exist and be a folder; no
`.` or `..` segments, no trailing slash), sends `hew.plugin.dev` with
that path, and prints the path it sent and "Waiting for you in Hew: choose
the folder and read what the plugin asks for." Every string that came from
the app (names, ids, versions, states, refusal text) is printed with
control and direction-changing characters removed and cut to a bounded
length, so a plugin's name cannot drive the terminal. It then looks at `hew.plugin.list` about once a second and
prints a line when a folder plugin appears or changes after the request:
"<name> <version> is installed from a developer folder and turned on." (installed, turned on, nothing waiting), "Waiting
for you to review a change in Hew (<name> <version>).", "Turn on developer
mode in Hew: Settings > Plugins." or a note that the app cannot read a
plugin's record. A refusal is said in plain words and exits 1 (developer
mode off, a dialog open, another request already waiting, a request the
person dismissed a moment ago, Plugins turned off, plugins not
available, or a production app that answers `preview_only`); a refusal
name the CLI does not know is printed as "Hew answered: <name>". The app
closing before it answered the request also exits 1; closing during the
later watching ends the command with 0. `--once`
sends the request, prints the waiting line and exits 0, for scripts.
Ctrl-C ends it. `hew-cli plugin list --live` prints developer mode and one
line per installed plugin.

What it does not do. Nothing completes from the
command line: there is no flag that picks the folder, allows what it asks
for, or turns developer mode on, and the folder it names is only where the
person's picker starts. The list has no folder path and no running state,
so the command cannot know which listed plugin came from the folder it
was given, cannot tell that a plugin is running, and cannot see a reload
that leaves the version unchanged; it reports every folder plugin that is
new or different since it began, and says once that the list does not
show whether a plugin is running. Settings > Plugins in Hew shows that.
A `hew-cli` in `--live` mode that is a production build refuses
`hew.plugin.*` itself and never sends it, and the MCP server refuses
`hew.plugin.dev` to an agent in every build; neither is a boundary, since
a program that can reach the socket can send the frames itself. The
boundary is that the request completes nothing without the person.

**What the person sees.** In each document window, while the Plugins
switch is on, the plugin host shows the request as a small notice in the
bottom right corner, over the model and under any sheet: the line "A
program on this computer asked to load a plugin from this folder:", then
the path on its own in a fixed-width block (cleaned again in the page,
cut in the middle when long, never inside the sentence), with "Choose
Folder…" and "Dismiss". The sentence is ordinary text; the path under it
is labeled "Folder" and cut in the middle past 120 characters. Settings ▸
Plugins shows the same, including a request that comes while it is open;
where Settings is a page inside the document window (Windows) the corner
notice stands aside while the pane shows it, so it is shown once. It is shown when the event comes,
and also when the window starts (asked with `plugin_dev_request`).
"Choose Folder…" calls `plugin_pick_dir({useRequest: id})`, then the
ordinary staging, consent sheet and native confirmation; whatever the
person does in the picker uses the request up, and every window's notice
goes. A request that came while a pick or sheet was open is shown when
it ends. "Dismiss" calls `plugin_dev_request_dismiss`: the request goes
in every window, and the socket is turned away for a minute. If Hew
could not dismiss it, the card stays and says so.

**The pane.** On the desktop Settings ▸ Plugins has a "Developer mode"
switch, read with `plugin_dev_mode` whenever the pane lists plugins and
changed with `plugin_set_dev_mode` (a no in the native dialog is quiet;
`confirmation_open` is a plain notice); what the switch shows is what the
shell last said. "Developer folder…" is under Install… only while it is
on. A folder plugin's row reads "Developer folder: <path>" and, below
it, its state: "Needs developer mode" while it is off; "This plugin's
folder now asks for different permissions." with Review and Turn Off
while the record is marked (with a line for `startup_added`,
`manifest_unreadable`, `id_changed` and `requirements_unmet`); and a
`plugins-dev-changed` problem in plain words ("Hew can't find this
plugin's folder." for `folder_missing`), until a later notice for it
carries none. Review stages the review and shows the consent sheet in
review mode; a refusal of the review (`no_review_pending`,
`folder_id_changed`, a manifest refusal) is a plain notice; Cancel
leaves the marker.

**What a row says, and the detail page.** Under its name an enabled
plugin's row says what it is doing: "Running", "Not running yet" (on, and
nothing has started it), or, for a plugin that stopped itself, "Stopped:
<plain reason>. Turn it off and on to start it again." The reason is one
of the host's machine words (`unresponsive`, `timeout`, `violation`,
`error`, `start`), each mapped to a sentence; a word the pane does not know
gets "Stopped." alone, and no reason, no console line and no plugin text is
ever shown inside one of Hew's sentences. On the desktop with several
windows open it reads "Stopped in 1 of 2 windows". The words come from the
shell's `plugin_list` rows (`running_in`, `stopped_in`, `blocked_in`,
`last_stop`, listed again on each `plugins-state` hint) and, on the web,
from the host in the page (`app/src/plugins/hostState.ts`); a folder
plugin's own blocked lines above replace the line. The On/Off switch is
still the person's choice and does not move when a plugin stops.

A "Details" button on each row opens a page with three tabs, reached by
keyboard; Escape closes it and focus goes back to the row's button.
Overview shows the name, version and source, what the plugin is doing, what
it can do, the optional permissions as switches (required ones are listed,
never switchable) and, in plain words, where its saved data is kept ("stays
on this device, does not travel with a model file, is not part of Undo";
what is saved per model is kept under the model's id, so two copies of one
file share it and a SketchUp file does not bring it back). Console lists
the plugin's lines newest first (`plugin_console_read` and the
`plugins-console` hint on the desktop; the host's last 500 on the web),
each cleaned, cut to 1,000 characters and shown as wrapped text in its own
block, with "Clear view", which only empties what the page shows. Activity
lists the plugin's changes of state newest first in plain words
(`plugin_activity_read`; on the web the changes the page saw while the
pane was open). The pane's data is read through `app/src/plugins/paneData.ts`.

### Where it is kept

Desktop: `plugins/` in the app's data folder (`com.hew3d.Hew`), which a
preview build can move with `HEW_PLUGIN_DIR`. Web: the same layout under
`plugins/` in the browser's origin-private file system.

```text
plugins/
  <id>/
    hew-plugin.json      the manifest the person consented to, as written
    installed.json       the record
    storage.json         the plugin's saved data
    files/...            the package's files (none for a folder install)
    previous/            the version before, with the same layout
  .staging/<id>-<token>/ an install being written
```

An install is written whole into `.staging/` and moved into place; a
failure part way leaves a staging entry, which listing ignores and the
next install of the same id clears. Every path the store builds comes
from a validated id and a validated package path, so nothing it reads or
writes is outside `plugins/<id>/` (or, for a folder install, the folder).

`storage.json` is one JSON object of string keys (1 to 256 bytes) to
string values (the runtime's JSON text), at most 5 MiB as written; a
write past that is refused `storage_quota` and writes nothing. It belongs
to the plugin, not one version of it: an update and a rollback carry the
current copy to the version that takes over, and uninstalling removes
it. Only an installed plugin has one, and a key is only ever a key in
that object, never part of a path. On the desktop the four commands
`plugin_storage_get`, `plugin_storage_set`, `plugin_storage_delete` and
`plugin_storage_list` read and write it, each write through a temporary
file and a rename, and a change is announced to every window with the
event `plugins-storage-changed` (`{id, keys}`, no `scope`).

### Plugin data

Everything else a plugin keeps lives beside the store, not in it:
`plugin-data/` next to `plugins/` (in the app's data folder on the
desktop; with `HEW_PLUGIN_DIR` set to a folder of another name, beside it
as `<name>.plugin-data`; at the top of the origin-private file system on
the web).

```text
plugin-data/
  <id>/
    documents/<document-id>.json   per-document storage
    console.log, console.log.1     the plugin's console (desktop)
    activity.json                  what the windows reported (desktop)
    usage.json                     the running total for the quota
```

No install, update, rollback or developer-folder path looks at this
tree, so no version swap can lose or carry it. Uninstalling removes it
first, before the plugin's record, so an uninstall cut short leaves the
plugin installed (and removable again), never data kept for a plugin
that is gone; the web store's recovery of an interrupted change restores
only `plugins/`, never this tree. A different package installed later
under the same id starts with nothing. Every write is refused
`not_installed` for an id with no record, so nothing is ever made for a
plugin that is not there; the plugin id and any document id are checked
before a path is built from them. On the desktop a link, or anything
that is not a regular file or folder, where the tree expects one is
refused `storage_unreadable` and is never followed or rewritten.

**One quota.** Everything under `plugin-data/<id>/` counts toward 50 MiB
per plugin, each file at least 4 KiB; past it a write that would grow the
tree is refused `storage_quota` and writes nothing, while one that does
not grow it never is. The total is kept in `usage.json` rather than
measured on every write: it is raised before a write to cover both the
old and the new copy and lowered after, so an interruption leaves it too
high, never too low. A missing or damaged total is measured again by
walking the tree (names and sizes only), and a total that refuses a
write is measured again before the refusal stands, so one left too high
corrects itself.

**Per-document storage.** One JSON object of string keys to string
values per (plugin, document), `documents/<document-id>.json`, at most 5
MiB as written, with the same key rules as plugin storage (1 to 256
UTF-8 bytes, well-formed text). The document is named by the `.hew`
manifest's `document_id` ("The document id" above), checked as exactly
the 36-character lowercase UUID form (`document_id_invalid` otherwise).
Deleting the last key removes the file. On the desktop the commands are
`plugin_doc_storage_get`, `plugin_doc_storage_set`,
`plugin_doc_storage_delete` and `plugin_doc_storage_list`, taking `{id,
docId, key?, value?}` with the refusals of the plugin-storage commands
plus `document_id_invalid` right after the plugin id; a change is
announced with `plugins-storage-changed` as `{id, scope: "document",
document_id, keys}`. The web store has the same four calls and posts the
same fields on its channel.

What per-document storage is not, plainly:

- It stays on this device. It does not travel with the file: a copy of
  the `.hew` sent to someone else arrives without it.
- It is not part of Undo. Undoing a change in the model leaves the data
  as the plugin last wrote it.
- A `.skp` file has no place for the document id, so data kept for a
  document opened from a `.skp` does not survive closing and reopening
  it.
- Two copies of one `.hew` (a file duplicated in the Finder, say) carry
  the same document id, so they share the data.

**What the windows report.** Each window's plugin host tells the desktop
shell what a plugin is doing there with `plugin_report_state {id, state,
reason?, seq}`. The window is the one the shell saw call, never one the
page names. `seq` grows with every report a page sends (the clock in
milliseconds, or one more than the last report's when the clock is not
past it), so it also grows across a reload of the page; two reports can
reach the shell out of order, and the shell ignores one older than the
one it holds for that window. `state` is one of `enabled`, `activating`, `activated`, `crashed`,
`blocked`, `idle` or `unloaded` (`invalid_state` otherwise); `reason` is
cleaned and cut to 160 characters. One window may report about one
plugin 30 times in 10 seconds (`report_too_often` past that). A report
that changes the window's latest state or reason is kept in memory,
announced with the event `plugins-state` (`{id}`, a hint to list again),
and added to `activity.json`, which keeps the last 500 entries (`{at,
window, state, reason}`) and is written at most once a second per
plugin, in batches, by the shell alone. A window that closes takes its
reports with it. `plugin_list` rows carry the sum over open windows:
`running_in` (activating or activated), `stopped_in` (crashed),
`blocked_in`, `idle_in` (unloaded for sitting idle; it starts again when
needed), and `last_stop` (`{reason, at}` of the newest crash still
reported, or `null`), so Settings can show a crashed plugin as stopped.
`plugin_activity_read {id}` answers the entries, newest first. On the
web the pane reads the host's state directly.

**Console.** Each window's host sends a plugin's console lines to the
shell about once a second with `plugin_console_append {id, lines:
[{level, text, at}]}`: at most 64 lines a call (`too_many_lines`),
levels `log`, `info`, `warn` or `error` (`invalid_level`), each text
cleaned and cut to 2,000 characters again by the shell, each time
clamped to the shell's clock. One plugin may write about 256 KiB a
minute across windows, each line counted as the log stores it (its JSON
and the newline); lines past that are refused, not queued (the
answer `{accepted, dropped}` counts them; a call none of whose lines fits
is refused `console_rate_limited`). The shell alone writes
`console.log`, one JSON line each, and renames it to `console.log.1`
once it would pass 1 MiB, replacing the one before. The event
`plugins-console` (`{id}`) says there is more to read; it carries no
line. `plugin_console_read {id, limit?}` answers the newest lines,
newest first (200 by default, at most 1,000). On the web the console
stays in memory only.

**The live API.** `hew.plugin.activity {id}` (read-only) answers
`{entries}`, as `plugin_activity_read` does; refused `invalid_params`,
`not_installed`, `plugins_disabled`, `no_plugin_dir`.
`hew.plugin.reload {id}` asks every window to reload a developer-folder
plugin, through the same `plugins-dev-changed` event (with
`manifest_changed: false`) a change in its folder sends, and answers
`{status: "reloading"}`. It is refused, changing nothing, unless the
plugin was installed from a folder (`not_a_developer_folder`), developer
mode is on (`developer_mode_off`), a window runs a plugin host
(`plugins_off`) and it is installed (`not_installed`). It is refused for a
plugin a window reports crashed (`plugin_stopped`), or blocked or waiting
for a review (`plugin_blocked`), and for one the person has turned off
(`plugin_turned_off`): a plugin that stopped stays stopped until a person
turns it off and on, and a reload is not a way around that or around a
review. At most one reload per plugin every 2 seconds
(`reload_too_soon`); inside a rehearsal (`dry_run`) it is refused
`dry_run_host_command`. Nothing else about plugins is reachable from the
socket.

`installed.json`:

| Field | Meaning |
|---|---|
| `schema` | record format, first in the file; `1` |
| `id`, `version` | the plugin and the installed version |
| `source` | `{"kind":"file","name"}` or `{"kind":"dir","path"}` |
| `installed_at_ms` | when it was installed |
| `grant` | the permissions the person accepted (the manifest's `permissions`) |
| `optional_granted` | optional permissions granted later |
| `enabled` | on or off; a fresh install is on |
| `startup_allowed` | false at install; set by the first time the person turns the plugin on by hand, so a package cannot start itself just by being installed |
| `data_version` | the plugin's data format; an update keeps the installed value |
| `signer` | null until signing exists |
| `needs_consent` | present and `true` while a folder plugin's manifest change waits for review; anything there but `false` counts as set |
| `needs_consent_reason` | why: `permissions_changed`, `startup_added`, `manifest_unreadable`, `id_changed`, `requirements_unmet`, `consent_text_changed`, `authority_changed` or `command_name_taken` |

The last two ride as unknown fields, so they need no new record format.
Fields Hew does not know are kept and written back. A record with a
higher `schema`, or one that does not read, is listed as unreadable with
only a removal offered, and is never rewritten.

### Updating, rolling back, turning off, removing

Installing a package over an installed id is an update. The installed
copy moves to `previous/` (the copy before that is dropped), the new
record keeps `enabled`, `data_version`, unknown fields, and the optional
grants the new manifest still declares, and the sheet shows what is new
since the accepted grant. "Roll back to <version>" appears when
`previous/` exists and swaps the two, so a second rollback goes forward
again. The version coming back follows the install rules, checked
against what is installed now before anything moves (on the desktop
before the person is asked): its `hew` range (`hew_version_unsupported`),
what it requires (`dependency_missing`, `dependency_cycle`), and no
command name another installed plugin has (`command_name_taken`). Turning a plugin off also turns off every enabled plugin that
requires it, and the pane names them. Uninstall asks first, is refused
while an enabled plugin requires the one being removed, and removes
`plugin-data/<id>/` first, then the folder (its saved data included), any
staging entry, and the plugin's `hew.plugin.<id>.` settings keys.

On the desktop each of these that widens what a plugin may do, or
removes it, is confirmed in a native dialog the shell builds from what
it holds: installing, widening a grant, rolling back, turning a plugin
on, and uninstalling. Turning one off and narrowing a grant ask nothing.
Only one such dialog shows at a time; another command that needs one is
refused `confirmation_open` meanwhile. A no is `enable_declined`,
`uninstall_declined` or `rollback_declined` (an install answers `null`,
a grant change the record unchanged). Turning a plugin on and rolling
back are refused `plugin_changed`, on both tiers, when the record (or,
for a rollback, the previous version) is no longer the one the person
was shown: someone changed it in another window meanwhile. Nothing
changes, and the pane shows a plain notice, not an error: "This plugin
changed while you were deciding. Check it, then try again." A preview
desktop build whose content security policy lacks the realm boot
script's hash (the Tauri CLI run directly instead of through
`shells/tauri`'s scripts) refuses every plugin command
`plugins_disabled`, since no realm could start; the pane shows "This
build of Hew was not given the policy plugins need, so plugins are
turned off." as an error, and the shell logs how to build it properly.
In that dialog Cancel is the
default button: Enter on a fresh dialog, Escape, and closing it all
answer no (Linux and Windows; macOS still shows the dialog plugin's box,
where Enter accepts).

### What the consent sheet shows

The sheet appears before anything is written, on every tier, from the
payload `plugin-host` builds (`consent`); on the desktop the shell then
shows a native confirmation built from the same payload it holds, so what
is confirmed is what Rust holds, not what the page sent. The sheet shows,
in order:

- the plugin's name and version, and where it came from ("From file", or
  the folder path with a line that its files can change after install);
- "Unsigned. Hew cannot tell who made this." for every package, until
  signing exists;
- for an update, "Replaces <name> v<version>. Hew cannot check it is the
  same author."; for the installed version again, "Replaces the installed
  copy of v<version>." under the title "Install this plugin again?";
- Hew's warnings, in a highlighted box: "Can send what it reads from your
  model to <hosts>." when `model.read` and any `network:` permission are
  both declared (required or optional), and "Can send files you choose to
  <hosts>." for `files` with `network:`;
- the permission lines, then those the plugin "May ask for later";
- for an update, "New since <version>" with the added lines highlighted,
  and "No longer asks for" with the removed ones (`update.removed`,
  `update.removed_optional`);
- for a review of a folder plugin's change (`review: true`), "This
  plugin's folder now asks for different permissions." in place of the
  replacement line;
- "The plugin says", a separate block holding the plugin's own
  `activationReason` as plain text.

The plugin's name, reason, and file name pass through one cleaning
function before they reach the payload (control, bidirectional, and
zero-width characters removed, whitespace collapsed, length capped), and
the app never renders package text as markup. An icon is shown only as an
`<img>` with a blob URL. Escape cancels, Enter does nothing, and focus
starts on Cancel.

The permission lines, exactly:

| Permission | Line |
|---|---|
| `model.read` | Read the model: every object, component, material, tag, and attribute |
| `model.write` | Change the model: draw, move, delete, and edit geometry |
| `attributes.write` | Store its own data on parts, materials, and the document |
| `attributes.write:<ns>` | Store data under the "<ns>" namespace |
| `view` | Change the selection, hide and show parts, and move the camera |
| `documents.save` | Save, export, and import files |
| `documents.discard` | Close or revert documents |
| `documents.other` | Act on every open document, not only the current one |
| `files` | Ask you to pick files, and save files you choose |
| `clipboard` | Read and write the clipboard |
| `library` | Add to and change your Library |
| `network:<host>` | Connect to <host> (checked by Hew) |

The app keeps the same words in `app/src/plugins/permissionCopy.ts`; a
test fails if the two tables differ.

## How a plugin runs

A Preview feature, started only in a preview build with the Plugins
switch on, by `App.tsx` through `app/src/preview/loaderRoots.ts`
(`loadPluginHost`). Each document window has its own host
(`app/src/plugins/host.ts`, `hostCore.ts`). Shop Mode has one too, with
`platform: 'shop'` (below); it has no plugin management.

**Shop Mode.** `ShopApp` starts the host through the same loader root
(`useShopPluginHost` in `app/src/shop/usePluginHost.ts`) once its Scene
exists, only in a preview build with the Plugins switch on; Shop names no
plugin module and has no words of its own about plugins. Its bridge is
small: the Scene, the viewport, `reconcile` (the renderer's hidden set
pushed again), `reseedViewState` and `reconcileVisibility` (the hidden
nodes and tags read again from the document's registries and pushed, as an
open does). It has no selection, isolate, entering or Scenes target, so
those view directives do nothing there. Plugins get at most `model.read`
and `view` (the kernel narrows the grant). Shop loads each document into the
same Scene in place and never pumped the Scene's events, so the hook notes
`document_opened` after every open and `document_closed` when the page goes
away, and pumps; with no host running it makes no kernel call. Shop's
harness is `window.__hew_shop_test`, and the host attaches its `plugins`
test hooks there.

**Registration and start.** For every enabled plugin the host registers
the commands of the manifest the store holds (never a folder's live
copy). A plugin starts the first time one of its commands is called, or
with Hew when its manifest declares `on_startup` and the user turned it
on by hand at least once (`startup_allowed`), or with the first event
its manifest lists (below). On start the host opens
the plugin's own bus connection with the grant the kernel builds from
the stored record (`plugin_grant_json` from `grant` and
`optional_granted`), sends `hew.meta.hello` and `hew.doc.attach` on it,
and creates the realm. `hello.activation` says why: `command`,
`startup` or `event`. A realm lives until the plugin is turned off,
crashes, changes (an update, or a grant other than an optional
permission added: see "Optional permissions"), sits idle, or the window
closes.

**Idle.** A realm that has no command open, nothing queued either way,
no storage, file or permission request open, and has exchanged no
message with the host (pings and their answers aside) for 10 minutes is
unloaded: `unload` with reason `idle`. Every message counts, events
delivered to it included, so a plugin that answered its command and
keeps reporting, logging or calling stays; one silent for 10 minutes
does not. For up to 2 seconds after `unload` the host still takes the
realm's storage writes and its `unloaded`, and drops anything else it
sends then (not a reason to stop it); the SDK runs the
`hew.lifecycle.on('unload', fn)` handlers with the reason, waits for a
promise one returns and for the storage writes they made (at most
1.5 seconds), then says `unloaded`. Then the realm is stopped as on any
unload: pending writes land, the connection closes and its event
subscriptions go with it, `plugin_state` `unloaded` (detail `idle`) is
reported, and the plugin is `enabled` again with its commands still
registered. Storage writes that reached the host before the stop are
made before it, not dropped, even those it had not handled yet: when the
realm says `unloaded`, that is every write it sent before (messages
arrive in order). When the 2 seconds run out first, a write still on its
way to the host then is lost with the realm. The SDK's
1.5 seconds are inside the host's 2 on purpose: a realm that keeps to
its limit is heard out. The next call to one of them starts it again; a
call that comes during those 2 seconds waits for them and then starts it
(one start for every call that waited). An event the plugin listens for
that fires during those 2 seconds, or after the realm is gone and
before its listener is subscribed again, can be missed: the leaving
realm may get it but can no longer act on it, and nothing else is
listening yet. A plugin that starts with Hew is never unloaded for
idleness.

**Starting on an event.** For an enabled plugin that is not running,
not stopped and not blocked, whose stored manifest has `on_event` and
at least one name in `contributes.events`, the host keeps a listener: a
connection of the plugin's own, with the grant from the stored record,
that says hello, attaches and subscribes to those names. A name the
grant cannot receive is denied and not listened for (no listener at all
when none is left). The first event starts the plugin (`activation`
`event`). The handover is one step: when the starting plugin's
connection has said hello and attached, the host subscribes it to the
listener's names and closes the listener before any event is drained
again, so every event drained before went to the listener and every one
after comes on the plugin's connection. Both kinds are held while the
plugin starts, at most 64 (the oldest go first; how many were dropped is
logged once), and handed to the realm, in the order they came, right
after it says hello: none is lost and none arrives twice. A command
that starts the plugin does the same handover. At least 10 seconds pass
between two starts of one plugin by an event (events that come sooner
wait, held, and start it when the time is up); after a start that
failed the wait doubles, up to 5 minutes, and a start that works brings
it back to 10 seconds. The listener closes when the plugin starts, is
turned off, stops, is blocked or removed, and when the window closes;
it opens again when the plugin is idle-unloaded or a start fails
without stopping it. Events it held for a start that then never comes
(the plugin blocked, turned off or stopped while it waited out the 10
seconds) are dropped with it, never handed to a later start. A grant
widened while the listener is still opening applies once it is open: it
takes the wider grant and subscribes to the names the old one refused.
On the desktop, while the document has no id yet, the host waits for
one with a single wait per plugin, which a stop ends. On Shop the
listener's grant is the viewer one.

**The realm.** A sandboxed `srcdoc` frame (`allow-scripts` only, so an
opaque origin) whose one inline script is the boot script
`app/src/plugins/boot/boot-1.js`, pinned by hash in both the page's
preview policy and the frame's own. The boot script answers a nonce
handshake, takes a control port, and starts one classic Worker from a
`blob:` whose text is the SDK runtime followed by the plugin's `main`,
then hands the Worker the realm port. `main` is therefore one classic
script, run in strict mode after the runtime: bundle to a plain script
or an IIFE; `import` and `export` at the top level are not allowed. The
page's policy has no `blob:` in `script-src`, so inside the Worker
`importScripts` and `import()` of a blob are refused; nested Workers
from `blob:` work (see "The Worker's scope" below). The realm has no network (`connect-src 'none'`), no
storage of the page's origin, no DOM of Hew's, and no way to call the
desktop shell.

**The Worker's scope.** Before `main` runs, the runtime removes from the
Worker's global scope (from the global, every object on its prototype
chain, and `navigator` and its chain, each hole filled with a fixed
`undefined`) every API a plugin has no sanctioned use for that is storage
of its own, a channel, or ambient authority: `caches` and the
CacheStorage interfaces, `indexedDB` and the IDB interfaces, the file
system entry points, `navigator.storage`, `navigator.locks`,
`navigator.serviceWorker` and the service worker interfaces,
`BroadcastChannel`, `SharedWorker`, `fetch`, `XMLHttpRequest`,
`WebSocket`, `EventSource`, `WebTransport`, `importScripts`,
`navigator.sendBeacon`, `FontFace` and `fonts`, `Notification` and push,
background sync and fetch, and `cookieStore`. What stays is what a
plugin needs: `Worker`, `Blob`, blob URLs, `WebAssembly`, message
channels, `crypto`, timers, text coding, `structuredClone`,
`performance`, typed arrays, canvases and image bitmaps for computation,
and the console. `Worker` is replaced: a nested Worker can be started
only from a `blob:` URL the plugin made in that scope, only as a classic
Worker (`type: 'module'` is refused), and runs the same lockdown before
the plugin's own code, at every depth. A Worker from a `data:` or any
other URL is refused `SecurityError`. This is defense in depth, not a
boundary Hew relies on: the frame's policy and the host's checks on the
port are. It matters most on WebKit (the Linux and macOS desktop), where
a Worker of an opaque-origin frame has a working CacheStorage that
every plugin shares, kept on disk under the app's data folder, outside
the plugin's 5 MiB, and kept after an uninstall; in Chromium `caches`
is absent there anyway. The residual: a plugin that found a way around
the lockdown could keep data in that shared cache, which Hew cannot see
or delete, and read what another plugin put there. On WebKit
`navigator.locks` refused an opaque origin (`SecurityError`) and a
`BroadcastChannel` reached no other realm even before the lockdown, and
the realm frame can create a nested `about:blank` frame despite
`frame-src 'none'`, in which nothing runs.

**The port.** `sdk/plugin/src/protocol.ts` lists every message. Every
field whose size a plugin chooses is text (params, results and stored
values cross as JSON text), so the host measures a message by adding
lengths. The host checks every message (`realm/validate.ts`): a known
type, exactly its fields, each of the right type and size, at most
1 MiB of text per message, at most 1,000 messages and 16 Mi characters
in any rolling second, and forward and request ids that the host itself
issued and that are still open. Anything else stops the plugin.
Messages are checked and counted when they arrive, then handled in
arrival order a slice at a time (about 8 ms of the page's time, then the
page runs before the next slice), so a burst of costly calls cannot hold
Hew. A plugin may have at most 1,000 bus calls waiting in the host;
more stops it.

The SDK keeps a plugin that uses it as documented inside every one of
these limits: it cuts console arguments (16 of 2,000 characters each,
50 calls a second; calls past that reach neither the log nor the
Worker's own console; the host then keeps fewer, below), error messages and stacks, progress labels and
refusal explanations to size; it refuses a bus request over 1,000,000
characters (`RangeError`), a storage key over 256 UTF-8 bytes
(`storage_key_invalid`), a stored value whose JSON is over 1,000,000
characters (`storage_value_too_large`) and a package path over 1,024
characters (`file_missing`) without sending them; and it keeps at most
32 storage requests and 8 file requests open, holding the rest until
answers come. Bus calls are not held back: a plugin that sends more than
1,000 in a second is stopped.

**Console.** What a plugin writes to its console reaches Hew's log (the
level kept, the line prefixed with the plugin's id) within the host's
own budget, which is tighter than the SDK's 50 a second: at most 20
lines a second, and at most 200 lines in one run of sustained output.
Past 200 the log says once that further output is not shown, and
nothing more from that plugin is logged until it has been quiet for
10 seconds. Lines over these budgets are dropped, not a reason to stop
the plugin.

**Limits.** A command must answer within its declared `timeout_ms`,
clamped to 1 to 50 seconds (50 when it declares none; the same ceiling
as "Compounds"), counted from the moment its plugin is handed the call. Starting
a plugin has limits of its own: on the desktop the host waits for the
window's document id (it never guesses one), and for the document to let
the plugin's connection say hello, together at most 10 seconds, and a
plugin that does not start for one of those reasons is not stopped: the
call is answered `unavailable` and the next one tries again. The frame
then has 5 seconds to answer, and `main` 10 seconds to finish its
top-level code. A plugin's Worker must answer the host within a few seconds,
always, not only while it serves a command: the host pings it every
2 seconds from the moment it has said hello, and a realm that misses two
pings in a row (silent for roughly 4 to 6 seconds) is stopped. The
runtime answers a ping only between tasks, so a handler (or a timer, or
anything else) that computes without yielding for more than about
4 seconds is stopped. Yield to the event loop at least every 2 seconds:
`await new Promise((r) => setTimeout(r, 0))` (awaiting an already
resolved promise is not enough; it does not let the ping through), or
move heavy work to a nested Worker, which has its own thread and does
not block the realm's. Before hello, while `main` itself runs, there are
no pings: the plugin has 10 seconds to finish its top-level code.

There is no memory cap. The realm shares the browser tab's process, and
the web platform offers no limit per Worker. A plugin that allocates in
the JavaScript heap is stopped by the missed pings, and its Worker ends
about 2 seconds after that; a plugin that allocates very fast can still
exhaust the memory the page shares and take the browser tab (or the
desktop window) down first. Memory held in buffers outside the
JavaScript heap (`ArrayBuffer`s) is not stopped at all: a plugin that
keeps gigabytes of them while answering its pings keeps running.
Stopping a plugin (a crash) removes the frame, which ends
the Worker, closes its connection, unregisters its commands, answers
every open call `failed: crashed`, reports `plugin_state` `crashed`,
logs an error line, and shows a notice. The `plugin_state` detail is the
kind of stop as one word (such as `error`, `unresponsive` or
`violation`), the same word the shell's report carries:
every subscriber gets that event, other plugins and socket clients
included, so what the plugin wrote never rides along. Its own text goes
to its console and the log only. It stays stopped in that window until
it is turned off and on again (or, for a folder plugin, its folder
changes: see "Reloading" below). The notice says so: "<name> stopped
working. To start it again, turn it off and on in Settings > Plugins."
Shop Mode has no Settings > Plugins, so there it says "<name> stopped
working. Reload the page to start it again.": a reload starts a new host,
where the plugin is no longer stopped. Granting or taking back one of its
optional permissions in Settings > Plugins also ends the stop: the plugin
is built again, and it starts with Hew if it starts with Hew, otherwise
with its next command.

**Not startable.** A plugin that the store says may not run now is
`blocked`, never stopped: no crash record, no notice, no `crashed`
state. Its commands stay registered, so a caller is answered
`unavailable` (`plugin_unavailable`) rather than told the command does
not exist; a realm it had is stopped as when it is turned off (`unload`
`disable`, its open calls answered `unavailable`). The reasons, which
`__hew_test.plugins.reasons()` reports beside `states()`:

| Reason | When | Lifted by |
|---|---|---|
| `needs_developer_mode` | a folder plugin while developer mode is off (the host asks the shell, `plugin_dev_mode`, at every sync; off when it cannot say), or a read refused `developer_mode_off` | the store saying otherwise at the next sync (developer mode turned on) |
| `needs_review` | a record whose `needs_consent` is anything but absent or `false`, or a read refused `dev_consent_pending` | the next sync after the marker is cleared (a review accepted) |
| `folder_missing` | a folder plugin's start whose read of `main` is refused `folder_missing` | the next start: a call to one of its commands, or a reload |
| `main_missing` | a folder plugin's start whose read of `main` is refused `package_path_invalid`, or whose manifest names no `main` | the same |

Lifted by the store, it starts again with Hew if it starts with Hew;
otherwise its next command starts it. Nobody turns it off and on. Only
the shell's refusal names count: text a plugin wrote never blocks it,
and a packaged plugin whose code cannot be read is stopped as before.

**Reloading.** `plugins-dev-changed` is a hint; the host reads the store
again whatever it says. With `manifest_changed: false` and no `problem`
it reloads that plugin: the realm is stopped (`unload` `reload`, open
calls answered `unavailable` as in any reload, compounds closed), the
commands of the stored manifest (refreshed by the shell) are registered
again, and the plugin starts again if it was running, starting, or
blocked by its last start, or if it starts with Hew. Storage writes on
their way are kept and land, and the reloaded plugin reads them. Two
notices before the host gets to them are one reload; a notice for a
plugin that is off or not installed starts nothing. A reload also
brings back a folder plugin that crashed, since the person saved its
folder. A marked change (`manifest_changed: true`) blocks it
(`needs_review`) through the store; a `problem` changes nothing in the
host (a running realm has its code already; the next start reads the
folder again). So a running folder plugin whose folder disappears keeps
running. A reload asked for meanwhile (`hew.plugin.reload`, which still
answers `{status: "reloading"}`) stops it, and its start is then refused
`folder_missing`: it is blocked with that reason (its commands answer
`plugin_unavailable`) until the folder is back and a call to one of its
commands starts it.

**Who owns a call.** A call from a socket client is the live bridge's: it
opens the compound for a mutating command, shows the busy notice,
completes the call, and asks the host only for the plugin's answer (the
forward router, `app/src/api/forwardRouter.ts`, read only where the
registry's `plugin-forward` coupling is on). A call from a realm, or from
the window itself, is the host's, which does the same work itself. A
plugin's own bus calls made while it serves a mutating command run
inside that command's compound.

**Storage.** `hew.storage` is one JSON object per plugin,
`plugins/<id>/storage.json`, 5 MiB in all; a write over that is refused
`storage_quota`. A key is 1 to 256 UTF-8 bytes (`storage_key_invalid`
past that; an empty key is a `TypeError`), and one value is at most
1,000,000 characters of JSON (`storage_value_too_large`), so a key and
a value always fit one 1 MiB message. A key that is not well-formed
text (a surrogate without its pair) is refused too: the SDK refuses it
`storage_key_invalid` before sending, and a raw message carrying one
stops the plugin, as one with an over-long key does (the host's message
check refuses both). A value that is not well-formed text cannot come
from the SDK (its JSON escapes a lone surrogate), but a raw message can
carry one: the host refuses it `storage_value_invalid`, in either scope,
before any store sees it, and the plugin runs on.
Writes are gathered for a quarter second and then made together, each
batch after the one before, so writes land in the order the plugin made
them; a read sees every earlier write, made or not, and a write that
fails rejects its own `set` or `delete`.

**Storage for the document.** `hew.storage.document` has the same calls
(`get`, `set`, `delete`, `list`, `onChange`), key and value rules and
refusal names as `hew.storage`, kept per document (see "Per-document
storage" for where and what it is not). The host follows the document,
not the window: `hew.plugin.documentId` is the document's own id, or
`null` while it has none, and `hew.lifecycle.on('document', fn)` runs
with the new id when it changes. Opening or making another document
switches every running plugin to it: writes on their way land under the
document they were made for, then the plugin is told. A document never
saved has no id: its storage is kept by the window's host, in memory, at
most 5 MiB per plugin (`storage_quota` past that); the first save that
gives it an id writes all of it under that id, key by key (overwriting
keys already there), and only then tells the plugin the id. A write that
fails there is kept, read as written, and tried again with the plugin's
next write. A document opened from a file with no place for an id (a
`.skp`) starts empty every time. A document closed with none open yet
(a `.skp` import closes the old document and opens the new one only
when the import ends) is the same kind of change: writes on their way
land under the closed document, the plugin is told `null`, and until
the next document opens its document storage is a throwaway in memory,
never the closed document's; the next open drops it. A close followed
by an open in the same moment (an ordinary open) tells the plugin once,
of the new document. A call the plugin made for one
document that reaches the host after another document was opened is
refused `document_changed`, with nothing read or written, and the SDK
rejects it as any storage refusal: the host never puts it in the new
document. The first save of a document is not such a change (its data
moves to the id the save gives), so a call made just before it lands
under the new id. A change another window makes reaches only a plugin
whose document it is (`storage_changed` with `scope: 'document'`), and
never the plugin's own storage listeners. On the wire a `storage`
message carries `scope` (`plugin` when absent), and `storage_reply` and
`storage_changed` carry it too; `hello` has `document_id` beside
`document` (the window's name for the document), and `{t: 'document',
document_id}` says it changed. The host counts document changes, the
document epoch: it goes up when a document is closed, when another is
opened or made, or when a saved document is saved under another id, and
stays at a first save. `hello` carries it as `document_epoch` and
`{t: 'document'}` as `epoch`. Every document `storage` message carries `epoch`, the one the
SDK had last heard when the plugin made the call (a non-negative
integer; a message without one, or a plugin-scope message with one,
breaks the protocol and stops the plugin), and the host refuses one
that is not its current epoch `document_changed`.

**What the host reports.** Each window's host reports every state change
of a plugin once (`enabled`, `activating`, `activated`, `crashed` with
its reason, `blocked` with its reason, `idle` after an idle unload,
`unloaded` with the reason for any other stop), and hands on its console
lines (those that reached the log) in one batch a second, at most 64 a
call, and the rest when the realm stops. On the desktop they go to
`plugin_report_state` and `plugin_console_append` (see "What the windows
report" and "Console"), fire and forget: a refusal is logged once and
never reaches the plugin. A refused state report does not count as
sent: the next report of that state goes out again, and a refused
`crashed` or `blocked` is tried once more by itself after 10 seconds
(once, not in a loop) if nothing was reported since. In every window they also go to
`app/src/plugins/hostState.ts`, which the Plugins pane reads on the web
(`hostStates()`, `subscribeHostStates`, `activity(id)`, the state
changes the host saw, oldest first, the last 500, `consoleLines(id)`, the
last 500, and `subscribeConsole`).

**Package files.** `hew.assets` reads files of the plugin's own package.
The host serves one file request of a plugin at a time, reads a path
named twice in one request once, puts at most 64 MiB in one answer, and
reads at most about 128 MiB a second for one plugin; a file over any of
these is answered missing.

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
run uses the `desktop.probe` policy of `shells/csp/policies.json` unmodified.
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
| ipc-surface | recorded: not the desktop shell | not run | not run | not run | not run | recorded: no bridge globals, the parent page is unreadable, both bridge URLs blocked by `connect-src`; `webkit.messageHandlers.ipc` takes a message, and the shell does not act on one without its key (window title unchanged; the host page can change it) |
| ipc-background | recorded: not the desktop shell | not run | not run | not run | not run | recorded: the same as ipc-surface |
| ipc-worker | recorded: not the desktop shell | not run | not run | not run | not run | recorded: no bridge, no raw channel; both bridge URLs fail, with no policy violation reported inside a Worker on this engine |
| worker-under-coep | recorded: with the host isolated, the realm Worker and surface still start; frame and Worker are not isolated, no SharedArrayBuffer | not run | not run | not run | not run | not run (browser-only row) |
| surface-spin | recorded: HOST FROZEN: the page stopped answering while the spinning surface was alive (see fallbacks) | recorded: HOST FROZEN: the page stopped answering while the spinning surface was alive (see fallbacks) | not run | not run | not run | not run (explicit-only) |

### Fallbacks in force

- All engines, worker-module-type: the boot script starts a classic Worker, never a module Worker. A module Worker from a blob URL in an opaque-origin frame does not run on Chromium.
- Chromium, worker-denials: the Cache API does not exist in the blob worker, so the two required `Cache.add` entries could not be shown blocked; the API being absent is accepted as the denial (nothing can be called), and the cell stays "recorded", not "pass".
- WebKit (Playwright) and Linux WebKitGTK, worker-denials: no `securitypolicyviolation` was reported for any entry, so the policy is not shown to be what blocked them (several blocks are fetch load failures, an invalid-URL error for relative URLs, or SecurityError). Accepted as denials in effect: `Cache.add` of a same-origin and of a remote URL, and every fetch/socket/IndexedDB entry, failed. `caches.open("x")` itself succeeds (an ephemeral per-realm Cache API); it is not a network or cross-plugin path.
- WebKit (Playwright) and Linux WebKitGTK, surface-navigate-remote: Linux only has the load count (1) and cannot show whether a request left; the WebKit run showed zero requests and a frame URL of `about:srcdoc`. The Linux cell therefore stays "recorded" rather than "pass".
- All engines, surface-navigate-self: the surface's own navigation to the host origin is not blocked by the sandbox (Playwright saw the `/?probe-escape=1` request). The realm must treat a surface navigation to a same-origin URL as reachable and rely on the host's own rules for that URL.
- All engines, surface-spin: a spinning surface script freezes the host page on both Playwright engines (the driver's evaluate gets no answer and the row never finishes). A surface cannot be assumed to run off the host's thread; the realm design must not rely on a frozen surface leaving the host responsive (kill by removing the frame is only possible when the host itself still runs).
- WebKit (Playwright), surface-click-activation: `navigator.clipboard.readText()` is refused (no permission API on this engine), so the read half is recorded, not passed.
- Linux WebKitGTK, ipc-surface and ipc-background: the engine's raw message channel (`window.webkit.messageHandlers.ipc`) exists inside a sandboxed realm frame and accepts a message. The frame has none of the shell's bridge (`__TAURI_INTERNALS__`, the invoke key) and cannot read the host page, and the shell dropped an invoke-shaped message that lacked its key. What keeps a realm frame from the shell's commands on this engine is therefore the key alone, so every desktop command a plugin feature adds checks the calling window's label and the plugin id on the host side, and consent is confirmed outside the webview. A realm Worker has no channel at all. macOS WKWebView and Windows WebView2 are not measured.
- Cross-origin isolation is not used on any engine: the host sends no COOP/COEP, `hew.supports("wasm.threads")` is false everywhere, and a Worker is required on every engine (no `background.worker` fallback).

### Rows that failed with no fallback

- WebKit (Playwright), offline-boot: the stage's procedure (Playwright `setOffline(true)`) breaks the surface handshake on this engine (`surface-boot` timed out; `boot-hash` passed). The stage specifies no fallback for this row. A substitute run that aborts every http(s) request instead completed both handshakes with zero requests, and that outcome is kept as evidence, but the row is recorded as failed as specified and needs the maintainer's call (accept the substitute procedure, or find a different offline emulation for this engine).
- Linux WebKitGTK, offline-boot: no network cut or request counting is available in the desktop run, so the zero-request criterion was not tested there.
