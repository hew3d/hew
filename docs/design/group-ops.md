# Group-level booleans and group duplication

Status: implemented (this document is the contract the implementation makes
true — DEVELOPMENT.md §4).

New users make a Group within minutes of first launch, and then hit a wall:
the boolean commands refuse grouped operands outright, and until recently
Move+copy only had first-class spec coverage for plain Objects. This design
extends both to Groups, without weakening any of Hew's standing positions —
combination stays explicit, nothing is repaired silently, every mutation is
atomic and exactly undoable.

## 1. Group duplication

`Document::duplicate_node` already deep-clones any tree node — Object, Group,
or Instance — under the same parent, as one undoable step
(`DocAction::Duplicated`). This effort **hardens** that path rather than
re-building it. The contract, now pinned by specs:

- **Deep copy.** Duplicating a Group clones its entire subtree recursively:
  nested Groups become new Groups, baked leaf Objects get fresh, independent
  geometry (own handle, own empty per-Object history), and component
  Instances inside the subtree become **new Instances of the same
  definition** — geometry stays shared; a copy is never an implicit
  Make Unique.
- **Attributes carry over.** Names, tags, per-face materials, UV frames, and
  each Object's base material are preserved on every cloned node. (User-hide
  view state is deliberately *not* copied — the copy arrives visible.)
- **Placement.** The placement transform is baked into cloned Object leaves
  and composed into cloned Instance poses, exactly matching the corresponding
  transform ops. Reflecting placements are refused typed (they would invert
  baked winding).
- **Independence.** After the copy, editing either side never affects the
  other (shared component definitions excepted, by design).
- **Atomic undo.** One undo step removes the whole copy — every created
  object, group, and instance — and redo restores it, handle-stably. A
  mid-clone failure rolls the partial clone back; the document is untouched
  on `Err`.

Seam note: a sibling effort (`array-copy`) builds ×N array copies **on top
of** this single-copy path; this effort deliberately ships only the single
copy, solid, so arrays inherit it at merge time.

## 2. Group-level booleans

### 2.1 Entry point

A new document op:

```rust
pub fn boolean_nodes(
    &mut self,
    op: BooleanOp,
    a: NodeId,
    b: NodeId,
) -> Result<(NodeId, DocChange), DocumentError>
```

Either operand may be a plain solid Object or a Group (mixed operands
allowed). Subtract is `a − b`. The existing object-only
`Document::boolean` is kept unchanged (recorded sessions replay against it);
the UI routes all three boolean commands through `boolean_nodes`, so the
user-facing rule is uniform.

### 2.2 Operand eligibility (typed refusals, nothing repaired)

Checked before anything mutates; the document is untouched on every `Err`:

- **Instances are refused, not implicitly made unique.**
  `BooleanOperandHasInstance` — raised when an operand *is* a component
  instance or *contains* one anywhere in its subtree. SketchUp's Solid Tools
  silently make the instance unique and consume it; that is exactly the
  implicit magic Hew's data model exists to avoid (a boolean consumes its
  operand — consuming shared geometry would either mutate every sibling
  instance or hide a Make Unique inside another verb). The refusal names the
  explicit ways out: Explode the instance, or Make Unique and explode, then
  combine. `NodeId::Instance` operands and instances nested under a Group
  operand get the same error.
- **Every leaf must be a watertight solid.** `BooleanOperandNotSolid { which }`
  — booleans are volume algebra; a leaky leaf anywhere under an operand
  Group refuses the whole op, identifying the offending operand side. (The
  per-object `OperandNotSolid` from the object-level engine is subsumed by
  this document-level check for `boolean_nodes`.)
- **An operand with no solids.** `BooleanOperandEmpty` — a Group whose
  subtree flattens to zero leaf Objects (defensive; the tree normally cannot
  produce one).
- **Operands must be top-level.** `GroupedOperand`, exactly as the existing
  replacing ops (boolean/slice/push-through): a replacing op consumes its
  operands and emits fresh top-level results; consuming a node inside some
  other group would orphan that group's member list. The UI only offers
  booleans at the top level, so this is the same wall it has always been.
- `a == b`, stale, or hidden handles are refused as today
  (`DegenerateContact` via self-combine, `Unknown*`).

Hidden-tag / user-hidden **view** state is ignored: like `transform_group`,
the op acts on every live leaf under the operand, visible on screen or not.
View state never changes what geometry *is*.

### 2.3 Semantics

1. **Compose each operand** (on clones — the document is not touched):
   the operand's leaf Objects, in tree order, are folded with the existing
   boolean **Union**. Grouping never welded these solids; the user has now
   explicitly asked for volume algebra, so each operand is first made into a
   single composite volume — SketchUp Solid Tools behave the same way. All
   group transforms are already baked into world coordinates (groups carry
   no pose), so composition needs no frame mapping. Disjoint members simply
   yield a multi-shell composite (the engine supports that directly);
   overlapping and flush members weld exactly (coplanar contact is resolved,
   not refused). A genuine measure-zero tangency between members refuses
   typed (`DegenerateContact`), as everywhere else.
2. **Apply the requested op** between the two composites with the existing
   engine (`Object::boolean`), identity frame.
3. **Dissolve coplanar seams** the composition and the final op introduced,
   preserving imprints the original leaves already carried — the same
   preserve-list rule the object boolean uses, with segments collected from
   every original leaf of both operands. Differing face materials remain a
   hard stop for seam dissolution, as today.
4. **Split the result into connected components** (`split_connected_
   components`) and insert:
   - exactly **one** component → a single plain top-level **Object** (like
     the object boolean today: unnamed, base material inherited from operand
     A's first leaf);
   - **several** components, every one a genuine solid (positive signed
     volume) → one new top-level **result Group** containing one Object per
     component, named from the operands (`"A − B"`, `"A ∪ B"`, `"A ∩ B"`;
     an unnamed operand falls back to its kind word, "Object"/"Group").
     Disjoint solids stay discrete Objects — a multi-shell single Object
     would hide real, independent volumes behind one handle;
   - a **cavity** component — a fully-enclosed subtract leaves the cavity
     walls as their own connected component with *negative* signed volume —
     is not a solid: `Object::split_solids` attaches each cavity to the
     smallest positive-volume component whose interior contains it (parity
     ray-cast containment, the boolean classifier's mechanism), so a
     hollowed host keeps its hollow while unrelated solids still split out
     discretely, multiple cavities land in their respective hosts, and a
     floating island inside a cavity is itself a positive component that
     splits out as a discrete solid. Splitting a cavity out on its own
     would mint an inside-out "solid"; fusing bystander solids into the
     hollow would break the discrete-Object rule — per-host assignment is
     the only honest reading. Degenerate cases (a cavity nothing contains,
     or no positive component at all) fall back conservatively to one
     multi-shell Object, never a wrong split.
5. **Consume the operands**: both operand subtrees (the nodes themselves and
   every descendant) are hidden — tombstoned, never erased, so every handle
   stays valid for undo/redo.

The result rule is one sentence: **a boolean yields one solid per connected
volume of the result; when there is more than one, they arrive together in a
group named after the operands.**

### 2.4 Failure atomicity (strong exception guarantee)

Every fallible step — composition, the final op, seam dissolution, component
splitting, validation — runs on owned clones *before* the document is
touched. Only after the full result set exists does the op mutate the
document, and those mutations (slotmap inserts, hidden flags, one undo push)
are infallible. This is the clone–compose–validate–swap shape
DEVELOPMENT.md §4 prescribes, applied across a multi-solid compound op: any
`Err` anywhere leaves the entire document byte-identical to before the call.

### 2.5 Undo

One new action:

```rust
DocAction::BooleanNodes {
    a: NodeId,
    b: NodeId,
    /// Every node hidden by consuming the operands (both subtrees, pre-order).
    hidden_operands: Vec<NodeId>,
    /// The result pieces (one per connected component).
    result_objects: Vec<ObjectId>,
    /// The result container group when there was more than one piece.
    result_group: Option<GroupId>,
}
```

Undo hides the result (objects + container group) and unhides exactly
`hidden_operands`; redo reverses. All ids are handle-stable
(hide-not-delete), matching every other replacing op. This is pure
visibility flipping — no geometry is recomputed on replay, so no rule-9
fingerprint machinery is needed (nothing can drift).

### 2.6 Validator coverage

The op establishes no new invariant class: results pass the full topology
validator before commit (always-on for user-reachable mutations), and the
result group participates in the existing debug tree validator
(member/parent agreement, no duplicates, live members, acyclic parents),
which runs after the op like after every document mutation. Specs assert
watertightness of every result piece and tree consistency across
undo/redo cycles.

### 2.7 Error taxonomy (new variants)

| Variant | Meaning | User-facing suggestion |
|---|---|---|
| `BooleanOperandHasInstance` | An operand is, or contains, a component instance | Explode the instance (or Make Unique, then Explode) first |
| `BooleanOperandNotSolid { which }` | A leaf under operand A/B is not watertight | Fix or remove the leaky object first |
| `BooleanOperandEmpty` | An operand contains no solids | — (defensive) |

Existing: `GroupedOperand`, `Unknown*`, and every `BooleanError` from the
engine (`OperandNotSolid`, `EmptyResult`, `DegenerateContact`, …) surface
unchanged. All new variants get plain-language copy in the UI error table
(enforced by its exhaustiveness test).

### 2.8 Surface changes (flagged per DEVELOPMENT.md rule 8)

- **wasm-api addition** (public surface): `Scene::boolean_nodes(op: u8,
  a_kind: u8, a: u64, b_kind: u8, b: u64) -> NodeJs` — the node-operand
  boolean, returning the result root (object or group), mirroring
  `duplicate_node`'s kind/id convention and reusing the existing `NodeJs`
  report class. The existing `Scene::boolean` is unchanged.
- **File format: no change.** The result group is an ordinary group; its
  name is an ordinary name. Nothing new is persisted.

## 3. Property tests

- **Composition ≡ sequential unions.** Union of a Group of disjoint boxes
  with a plain box produces the same set of connected-component geometries
  as unioning the same solids sequentially with the object-level boolean
  (compared canonically, per component).
- **Splits are solid.** A subtract that severs the target yields ≥2 result
  pieces, each watertight, housed in a result group.
- **Undo is exact.** For boolean_nodes and duplicate_node alike:
  `state_hash` after op + undo equals the hash before the op; after redo it
  equals the hash after the op.

## 4. Out of scope

- ×N array copies (the `array-copy` sibling effort builds on this).
- Booleans on operands nested inside groups (same `GroupedOperand` wall as
  every replacing op; the UI does not offer it).
- Implicit Make Unique of instance operands (refused typed instead; see
  2.2).
- N-ary booleans (more than two operands in one call) — compose by chaining,
  as today.
