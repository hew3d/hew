/**
 * Plain-language error messages for every operation Hew refuses.
 *
 * The wasm boundary throws stringly `"CODE: message"` errors
 * (docs/dev/DEVELOPMENT.md B3), where CODE is the innermost typed kernel error's
 * variant name. This module owns the CODE → user-facing copy table: what
 * happened in plain words, then a suggested next step — never a raw
 * technical error (docs/agents/ROADMAP.md: plain-language error messages).
 *
 * Copy ground rules:
 *  - First sentence: what was refused, in the user's vocabulary. Hew's
 *    audience knows "watertight solid" (it's the product's core promise);
 *    they don't know "manifold", "coplanar", or "topology".
 *  - Second sentence: the most likely next step, phrased as an action.
 *  - Refusals are the kernel keeping its no-silent-repair promise (rule 4);
 *    the copy says what to change, never apologizes for refusing.
 *
 * The table is exhaustively checked against the kernel's error inventory by
 * `kernelErrors.test.ts` — adding a kernel error variant without copy here
 * fails that test, not a user.
 */

import { i18n } from '@lingui/core'
import { msg, t } from '@lingui/core/macro'
import type { MessageDescriptor } from '@lingui/core'

// Every entry below is a whole sentence pair written out in full, including
// the shared tail ("To make it solid, ..." / "If this keeps happening, ..."),
// so a translator sees and can reorder the complete message. The table is
// built once at import, so it holds descriptors; `kernelErrorMessage`
// resolves them against the active language at call time.

/**
 * Human copy for every kernel error code that can cross the wasm boundary,
 * keyed by the `"CODE: message"` prefix.
 */
const DESCRIPTIONS: Record<string, MessageDescriptor> = {
  // ------------------------------------------------------- stale handles
  UnknownObject: msg`That object is no longer there — the model changed since it was picked. Click it again.`,
  UnknownFace: msg`That face is no longer there — the model changed since it was picked. Click it again.`,
  UnknownEdge: msg`That edge is no longer there — the model changed since it was picked. Click it again.`,
  UnknownVertex: msg`That point is no longer there — the model changed since it was picked. Click it again.`,
  UnknownSketch: msg`That sketch is no longer there — the model changed since it was picked. Click it again.`,
  UnknownRegion:
    msg`That profile is no longer there — the model changed since it was picked. Click it again.`,
  UnknownIsland: msg`That shape is no longer there — the model changed since it was picked. Click it again.`,
  UnknownGroup: msg`That group is no longer there — the model changed since it was picked. Click it again.`,
  UnknownComponent:
    msg`That component is no longer there — the model changed since it was picked. Click it again.`,
  UnknownInstance:
    msg`That component instance is no longer there — the model changed since it was picked. Click it again.`,
  UnknownGuide: msg`That guide is no longer there — the model changed since it was picked. Click it again.`,
  UnknownMaterial: msg`That material is no longer in the palette. Pick another swatch.`,
  DegenerateFace: msg`That face has no usable area. Pick a different face.`,
  ReplaceMaterialReplayStale:
    msg`Some of the faces that material swap touched have since been reshaped, so it can no longer be undone or redone on its own. Nothing was changed.`,

  // ---------------------------------------------------------- drawing
  PointOffPlane:
    msg`That point isn't on the drawing surface. Draw on the highlighted face or the ground plane.`,
  DegenerateSegment: msg`The line's two ends are the same point. Click a second, different point.`,
  WouldRetopologize:
    msg`Moving this point would make lines cross or merge. Move it somewhere clear, or delete and redraw the lines instead.`,
  MalformedRegion:
    msg`This profile's outline couldn't be traced. Redraw the shape; if it keeps failing, use Report Bug.`,
  RegionTouchesHole:
    msg`This shape touches another shape at a point, so it can't be pulled as one piece. Pull the shape inside it instead, or move them a hair apart.`,
  RegionHasNoFace:
    msg`This loop of lines has no face to push or pull. Draw over one of its lines with the Line tool to give it a face.`,
  DegenerateCurve: msg`That circle or arc is too small to keep. Drag a larger radius, or type an exact one.`,
  UnknownCurve: msg`That curve is no longer there — the model changed since it was picked. Click it again.`,
  CurveNotAnalytic:
    msg`This shape has no circle behind it, so there's nothing to re-build. Only a circle drawn with the Circle tool has a segment count.`,
  CurveNotRefacetable:
    msg`Only a whole, untouched circle can have its segment count changed — not a part-erased one, and not one other lines run into. Redraw the circle at the count you want.`,
  SegmentsBelowFloor:
    msg`A circle needs at least 24 segments — below that it stops being a circle and becomes a polygon. Use the Polygon tool for a coarser shape.`,
  SegmentsAboveCap: msg`That is more segments than a circle can hold. Enter a smaller count.`,
  RestoreConflicts:
    msg`Undo needs to put the original outline back, but newer drawing is in its way. Erase the overlapping lines and undo again.`,
  OffsetTooSmall:
    msg`That offset distance is too small to make a new boundary. Drag further, or type an exact distance.`,
  OffsetCollapsed:
    msg`The shape can't absorb that offset — its boundary would collapse, cross itself, or spike out of a sharp corner. Try a smaller distance, or soften the sharpest corner.`,
  SketchGestureAlreadyOpen: msg`The drawing tools got out of step. Press Escape and try again.`,
  SketchOwnerMismatch:
    msg`That drawing can't belong to this object: it doesn't lie on any of the object's faces.`,
  DraftAlreadyOpen: msg`Another drawing step is still in progress. Finish or cancel it first.`,
  DraftNotOpen: msg`There is no drawing step in progress to finish.`,
  DraftBoundaryEdgeTouched:
    msg`A solid's own edges can't be moved or erased by drawing. Push/Pull or a boolean changes the solid itself.`,
  RegionNotPushable:
    msg`That shape can't be pushed or pulled: part of it is drawn on the solid's face and part is loose, so the face can't hold it. Move the drawing clear of the edge, or push/pull the neighboring pieces instead.`,
  RegionSliver: msg`That shape is too thin to push or pull — the face can't hold a sliver that small.`,
  SketchGestureNotOpen: msg`The drawing tools got out of step. Press Escape and try again.`,
  DegenerateGuide: msg`The guide needs a definite direction. Drag a little further before dropping it.`,
  DuplicateTag: msg`A tag with that name already exists. Choose a different name.`,
  InvalidTagPath:
    msg`That isn't a valid tag name — it can't be empty, and a tag can't be moved inside itself.`,
  // ------------------------------------------------ Scenes (scenes.md)
  UnknownScene: msg`That Scene no longer exists — it may have been deleted.`,
  DuplicateSceneName: msg`A Scene with that name already exists. Choose a different name.`,
  EmptySceneName: msg`A Scene needs a name.`,
  InvalidSectionPlane: msg`That section plane isn't valid. Place it again on a face or the ground.`,
  DegenerateUvFrame: msg`That texture position isn't valid. Drag the handle further before releasing it.`,
  BadUvFrame: msg`That texture position couldn't be read. Press Escape and start the positioning again.`,

  // ------------------------------------------------- extrude / push-pull
  DistanceTooSmall: msg`That distance is too small to build anything. Drag further, or type an exact length.`,
  DegenerateGeometry:
    msg`This profile can't be extruded into a valid solid. Simplify the shape and try again.`,
  ObjectNotSolid:
    msg`This object isn't solid, so it can't be pushed or pulled. To make it solid, select it and choose Object ▸ Make Solid…`,
  WouldVanish:
    msg`Pushing that far would remove the whole object. Push a shorter distance, or delete the object instead.`,
  NonManifoldResult:
    msg`The walls this would create run into the object's other geometry. Try a different distance, or reshape the surrounding faces first.`,
  NotASubFace: msg`Push/Pull here needs a shape drawn on the face. Draw a closed outline on the face first.`,
  RadiusVanishes: msg`Pushing that far would shrink the curved wall to nothing. Push a shorter distance.`,
  WallNeighborNonPlanar:
    msg`Offsetting this curved wall would bend a neighboring face out of flat, so it was refused. Adjust or simplify the touching faces first.`,

  // -------------------------------------------------------- follow me
  EmptyPath: msg`Follow Me needs a path. Select or click the line, curve, or face edge to follow.`,
  UnknownPathEdge:
    msg`That path line is no longer there — the model changed since it was picked. Click it again.`,
  PathBranches:
    msg`The path forks — Follow Me needs one continuous run of lines. Select a single chain with no branches.`,
  PathDisconnected: msg`The path is in separate pieces. Select one connected run of lines.`,
  PathSegmentTooShort: msg`Part of the path is too short to follow. Redraw the path without tiny segments.`,
  ProfileNotPerpendicular:
    msg`Hew tries to stand the profile up automatically, but couldn't square it to the path here. Move the profile closer to where the sweep should start — on a drawn circle, near the rim, in line with the circle's center.`,
  PathDetachedFromProfile:
    msg`The path doesn't start on the profile's surface. Start the path at the profile — or, on a loop, cross the profile partway along a straight run, not at a corner.`,
  PathReverses:
    msg`The path doubles back on itself — or turns nearly all the way around — so there is no clean corner to turn. Remove the reversal, or soften the corner.`,
  PathTooTight:
    msg`The path turns tighter than the profile is wide, so the sweep would fold into itself. Use a smaller profile or gentler turns.`,
  ProfileCrossesAxis:
    msg`This profile crosses the circle's center axis, so revolving it would lose the geometry on one side. Draw the profile entirely on one side of the axis instead.`,
  PartialSweepOnPole:
    msg`A partial sweep can't cut open this shape — its profile touches the turning axis, so the pole only exists in the full revolution. Sweep the whole path, or move the profile off the axis.`,
  SweepSelfIntersects:
    msg`Following this path would make the shape run into itself. Shorten the path or use a smaller profile.`,
  SweepDegenerate:
    msg`This profile can't be swept along that path into a valid solid. Simplify the profile or the path and try again.`,

  // ------------------------------------------- drawing on faces / merging
  PathTooShort: msg`The cut needs at least two points. Click a start and an end.`,
  EndpointNotOnBoundary:
    msg`A splitting line must start and end on the face's edges. Snap both ends to the face boundary.`,
  PointNotOnFace: msg`Part of the line leaves the face. Keep every point on the face being split.`,
  PathNotSimple:
    msg`The line crosses itself or touches the face's edge partway along. Draw a simple path from edge to edge.`,
  FacesNotCoplanar:
    msg`These two faces aren't in the same plane, so they can't be merged. Pick an edge whose two faces lie flat in one plane.`,
  BoundaryEdge: msg`This edge has a face on only one side — there's nothing to merge it with.`,
  SameFaceOnBothSides:
    msg`The same face is on both sides of this edge, so dissolving it would puncture the surface.`,
  SharedChainDisconnected:
    msg`These faces touch along more than one separate edge run, which merge can't dissolve yet. Merge along one shared run at a time.`,
  SharedChainCoversBoundary:
    msg`One face's entire boundary lies on the other, so this isn't an edge merge. Select the inner face itself and remove it instead.`,
  CurveClaimOffLoop:
    msg`The drawn outline and its circle disagree, so the imprint was refused. Redraw the circle; if it keeps failing, use Report Bug.`,
  LoopNotStrictlyInside:
    msg`The shape can't go there: it would run off the face, touch an edge or another shape at just one point, or leave a piece too small to keep. A shape being moved can't overlap another one either. Keep it on the face, and draw across or along edges instead of touching them at one point.`,
  LoopSelfIntersects: msg`The shape's outline crosses itself. Draw a simple, non-crossing outline.`,
  NotAnInnerFace:
    msg`Only a shape drawn fully inside a face can be removed this way. Select the imprinted inner face itself.`,
  NotInPlane:
    msg`A shape on a face can only move within the face's plane. Slide, turn, or scale it on the face, or delete it and draw it again where you want it.`,
  NestedNotFlat:
    msg`Something inside this shape has already been pushed or pulled, so the shape cannot move. Undo that push/pull or flatten it first.`,
  NotAChord:
    msg`Only a shape drawn on a face can be moved or deleted this way — and one that other shapes cut into pieces (or a line whose two sides also meet along another line) can be deleted but not moved. Delete it and draw it again where you want it.`,
  WouldCorrupt:
    msg`That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again.`,

  // -------------------------------------------------------- booleans
  OperandNotSolid:
    msg`Combining needs a solid on both sides. To make it solid, select it and choose Object ▸ Make Solid…`,
  EmptyResult:
    msg`The result would be empty — the objects don't overlap that way. Check that the solids actually intersect.`,
  SingularTransform: msg`One object's placement is scaled down to nothing, so the operation can't run.`,
  DegenerateContact:
    msg`The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect.`,
  NothingRemoved:
    msg`Nothing was cut away: the two solids only touch or sit apart. Check that the cutter is the shape you expect, then move it so it passes through the other solid.`,

  // ----------------------------------------------------------- slice
  NotSolid: msg`Only a solid can be sliced. To make it solid, select it and choose Object ▸ Make Solid…`,
  PlaneMissesSolid:
    msg`The slicing plane doesn't pass through the object. Position the cut so it goes through the solid.`,
  Degenerate:
    msg`The cut lines up exactly with an existing face or edge, so it wouldn't create two pieces. Move the cut slightly.`,

  // ------------------------------------------------------- transforms
  Singular: msg`That transform would scale the object down to nothing, so it was refused.`,
  DegenerateAxis:
    msg`The rotation axis needs two distinct points. Pick a second point further from the first.`,
  DegenerateTextureFrame:
    msg`That would stretch a picture or texture on the object so far it couldn't stay on its face, so it was refused. Try a smaller stretch.`,
  InvalidRescaleFactor:
    msg`That resize factor isn't valid. Pick two points that aren't on top of each other and type a positive distance.`,

  // ------------------------------------------- groups & components
  // App-side refusal (not a kernel code): a selection containing sketch
  // geometry reached a node-only structural command — see treeModel.ts's
  // structuralSelection. NOTE for the rebase onto the centralized
  // kernelErrors.isErrorLevelCode: classify InvalidSelection as
  // error-level there (App.tsx carries it inline until then).
  InvalidSelection:
    msg`Sketches can't be part of a group or component. Select only objects, groups, or components.`,
  EmptyGroup: msg`Select at least one object to group.`,
  EmptySelection:
    msg`The selection has nothing visible to transform — everything in it is hidden or empty. Unhide its contents, or select something visible.`,
  EmptyComponent: msg`Select at least one object to turn into a component.`,
  ComponentCycle:
    msg`Placing that instance here would make a component contain itself, directly or through another component. Pick a different definition, or explode/make unique the instance that closes the loop.`,
  ComponentDepthExceeded:
    msg`That would nest components deeper than Hew supports. Flatten one of the levels — explode an instance, or make it unique — before placing this one.`,
  ComponentExpansionExceeded:
    msg`That would multiply past a million rendered component parts. Reduce how many copies the nested components repeat — explode or thin out a level — and try again.`,
  CannotExplodeNonUniformScale:
    msg`This instance has an unfinished sketch inside it, and its scale isn't even across all three axes, so the sketch's circles and arcs can't be baked exactly. Even out the instance's scale first, or finish (extrude) the sketch before exploding.`,
  DuplicateMember: msg`The same object is in the selection twice. Reselect and try again.`,
  MixedParents:
    msg`Only siblings can be grouped — everything selected must be top-level, or all inside the same group. Move them to one level first.`,
  GroupCycle:
    msg`A group can't be moved into itself or into one of its own members. Pick a different group, or move it out to the top level.`,
  TooManyOccurrences: msg`This model has more placed parts than Hew can list at once.`,
  InvalidFrame:
    msg`That frame is not a valid right-handed set of axes. Give three unit-length, mutually perpendicular directions.`,
  RebaseDuringSession: msg`Finish editing the component before changing its axes.`,
  NotPlaced: msg`That part is no longer in the model. Pick it again.`,
  NoPartFrame: msg`That part has no faces to measure.`,
  GroupedOperand:
    msg`This operation can't target an object inside a group. Ungroup it, or leave the group context, first.`,
  BooleanOperandHasInstance:
    msg`Combining can't consume a component instance — its geometry is shared with every copy. Explode the instance (or Make Unique, then Explode) first.`,
  AmbiguousHost:
    msg`This copy is made of more than one part, so there's no single part to cut. Pick the part you want to cut, then try again.`,
  NotWrappable:
    msg`Only an object or a group can start a set of linked copies. This is already a component, so copy it as it is.`,
  NotLinkable: msg`A component can't be turned back into linked copies. Copy one of its parts to start a new set.`,
  NotLinked: msg`This is already a component, not a set of linked copies, so there's nothing to change.`,
  BooleanOperandNotSolid:
    msg`Something in that selection isn't solid, so it can't be combined. To make it solid, select it and choose Object ▸ Make Solid…`,
  BooleanOperandEmpty:
    msg`That selection has no solids to combine. Pick a solid object or a group of solids.`,

  // ------------------------------------------- component-edit-parity.md phase K1/K2
  LastDefinitionMember:
    msg`This is the component's only member — deleting it would leave every instance empty. Delete the instances instead, or add another member first.`,
  DefinitionNestedInDefinition:
    msg`This component is used inside another component's definition. Make that component's instances unique or explode them first, or delete the outer component.`,
  AmbiguousInstanceScale:
    msg`This instance is scaled unevenly across its axes, so a single typed distance can't map onto it without ambiguity. Drag to the exact size instead of typing a length.`,

  // ------------------------------------------------ Model Health repairs
  ObjectIsSolid: msg`This is already solid, so there's nothing to close. Use Push/Pull to change its shape.`,
  UnknownRim: msg`That opening is already closed. Check the list again for what is left.`,
  RimNotPlanar: msg`This opening isn't flat, so one face can't close it. Draw faces across it by hand.`,
  RimSelfIntersecting:
    msg`The edge of this opening crosses itself, so one face can't close it. Draw faces across it by hand.`,
  CapWouldBeCoplanar:
    msg`This is a flat panel, not an open shape. Keep it as a panel, or add thickness to make it solid.`,
  NonManifoldRim:
    msg`The edges around this opening branch off in more than one direction, so Hew can't tell where a face goes. Fix those edges by hand first.`,
  CapsOverlap:
    msg`A face here would overlap another part of this object. Close all of the object's openings together, or draw faces across this one by hand.`,
  RemovesEverything: msg`That would delete every face. Delete the object instead.`,
  Invalid:
    msg`Hew couldn't make this change without damaging the object, so nothing changed. Please send the file with Help ▸ Report Bug.`,
  NotAFlatSheet:
    msg`Only flat panels can get a thickness this way. Close the openings of this shape instead.`,
  ThicknessTooSmall: msg`That thickness is too small. Enter at least 0.001 mm.`,
  UnjoinedSeam:
    msg`Two faces in this panel meet without sharing an edge, so a thickness would leave a crack inside. Draw a line where they meet, then try again.`,

  // ---------------------------------------------------------- explode session
  ExplodeSessionOpen:
    msg`This component is already open for editing further out. Step back out to it (Escape) instead of opening it again — and close the editor before saving.`,
  ExplodeSessionNotOpen: msg`No component is currently open for editing.`,
  ExplodeSessionPoseUnsupported:
    msg`This instance is scaled unevenly, so it can't be opened for direct editing. Even out its scale first.`,
  ExplodeSessionScope:
    msg`That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again.`,
  ExplodeSessionGroupedInstance:
    msg`This instance sits inside a group, so it opens in the in-context editing mode instead.`,
  ExplodeSessionNestedGroup:
    msg`This group is nested inside another one, so it can't be opened for editing directly. Enter its enclosing group first, then drill down to this one.`,
  // App-side refusal (not a kernel code, like InvalidSelection above): a
  // double-click on a component instance whose definition has nested
  // members (member groups/instances) — the viewport gates BOTH the explode
  // session and its in-context-edit (K1/K2) fallback on this, before ever
  // asking the kernel, since a pose/grouped-instance refusal on the SAME
  // instance would otherwise fall back to K1 silently.
  NestedComponentInContext:
    msg`This nested component can't open for editing here — its placement is unevenly scaled or inside a group. Explode the instance, or make it unique, to edit its parts.`,

  // ---------------------------------------------------------- history
  NothingToUndo: msg`Nothing to undo.`,
  NothingToRedo: msg`Nothing to redo.`,
  // Both are raised by redo as well as undo (the same proof-carrying replay
  // runs in either direction), so the copy must not name one.
  InverseFailed:
    msg`This step couldn't be undone or redone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.`,
  InverseDiverged:
    msg`Undo or redo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session.`,
  // Raised only on the FORWARD commit of a 3D-text placement, where
  // `place_text` folds the just-closed drawing gesture into one undo step
  // and does not find it at the top of the history. Never raised by an
  // actual undo or redo, so the copy must not blame one.
  UnexpectedGestureState:
    msg`Finishing this step needed the drawing it just closed and couldn't find it, so nothing was placed and the model was left unchanged. If this keeps happening, use Report Bug to capture the session.`,
  // Raised only through the API's transaction bracket (never by the UI's
  // own tools): a connected client's batch tripped a commit-time guard and
  // was rolled back whole.
  TransactionHistoryDisturbed:
    msg`A connected tool undid history in the middle of its own batch, so the whole batch was discarded and the model was left unchanged.`,
  TransactionSessionUnbalanced:
    msg`A connected tool changed which group or component was open for editing without putting it back, so its whole batch was discarded and the model was left unchanged.`,
  TransactionGestureUnbalanced:
    msg`A connected tool left a drawing step unfinished at the end of its batch, so the whole batch was discarded and the model was left unchanged.`,
  // A plugin command held the document and its compound bookkeeping did not
  // line up (docs/agents/PLUGINS.md "Compounds"): a host bug, never the
  // person's doing.
  CompoundTokenMismatch: msg`Hew lost track of a plugin's work in progress. Nothing was changed; if this keeps happening, use Report Bug.`,
  // Not a kernel variant: the wasm boundary's busy guard, while a plugin
  // command holds the document.
  DocumentBusy: msg`A plugin is still working on this model. Wait for it to finish, or cancel it from the notice at the top of the window.`,

  // ------------------------------------------- attribute dictionaries
  // Reachable only through the API surface (the UI has no attribute
  // editor yet); phrased for the connected-tool framing regardless.
  UnknownTag: msg`That tag doesn't exist in this model. Check the tag name and try again.`,
  InvalidAttrName:
    msg`Attribute names need both a namespace and a key. Give the attribute a non-empty name and try again.`,
  NonFiniteAttrValue: msg`Attribute numbers can't be NaN or infinity. Use a finite number and try again.`,
  AttrValueTooDeep: msg`This attribute value is nested too deeply to store. Flatten it and try again.`,
  UnknownAttr: msg`That attribute doesn't exist on this item, so there was nothing to remove.`,

  // ------------------------------------------------------ file loading
  NotAContainer: msg`This file isn't a Hew document. Pick a .hew file saved by Hew.`,
  Incomplete:
    msg`This Hew document looks incomplete or damaged. It may not have finished downloading or copying, so get a fresh copy and open that.`,
  UnsupportedVersion: msg`This file needs a newer version of Hew than this one. Update Hew and try again.`,
  MalformedManifest: msg`This file is damaged and can't be opened.`,
  DanglingReference: msg`This file is damaged and can't be opened.`,
  MissingAsset: msg`This file is missing some of its data and can't be opened.`,
  Geometry: msg`This file's geometry data is damaged and can't be opened.`,

  // ----------------------------------------------------- math helpers
  DegenerateVector: msg`That direction is too short to work with. Pick points further apart.`,
  DegeneratePlane: msg`Those points don't define a plane. Pick three points that aren't in a line.`,

  // -------------------------------------------------------- annotations
  UnknownAnnotation:
    msg`That dimension or label is no longer there — the model changed since it was picked. Click it again.`,
  AnnotationHasNoAttrs:
    msg`A dimension or label can't carry attributes. Put them on the object it measures instead.`,
  DegenerateAnnotation:
    msg`That dimension or label doesn't have a usable placement — its two points are on top of each other, or the circle it measures has no size. Drag to points with more room between them, or a bigger circle, and try again.`,
  MismatchedAnnotationKind:
    msg`Hew tried to turn this dimension or label into a different kind of entity, which isn't allowed. If this keeps happening, use Report Bug.`,

  // --------------------------------- malformed request (internal — a bug
  // in Hew itself, never something the user's model or click caused).
  BadNodeKind:
    msg`Hew's own request to the model kernel named an object kind it doesn't recognize. If this keeps happening, use Report Bug.`,
  BadPoint:
    msg`Hew's own request to the model kernel carried a malformed point. If this keeps happening, use Report Bug.`,
  BadVec:
    msg`Hew's own request to the model kernel carried a malformed direction. If this keeps happening, use Report Bug.`,
  // `triple()` checks a raw xyz array for the camera state's eye, target
  // AND up — two positions and one direction — so this one cannot claim
  // either shape, unlike BadPoint and BadVec above, whose typed decoders
  // each carry exactly one.
  BadVector:
    msg`Hew's own request to the model kernel carried a malformed xyz value. If this keeps happening, use Report Bug.`,
  BadProjection:
    msg`Hew's own request to the model kernel named a camera projection it doesn't recognize. If this keeps happening, use Report Bug.`,
  BadCamera:
    msg`Hew's own request to the model kernel carried a malformed camera. If this keeps happening, use Report Bug.`,
  BadPose:
    msg`Hew's own request to the model kernel carried a malformed pose. If this keeps happening, use Report Bug.`,
  BadViewStyle:
    msg`Hew's own request to the model kernel carried a malformed view style. If this keeps happening, use Report Bug.`,
  BadDisplay:
    msg`Hew's own request to the model kernel carried malformed display settings. If this keeps happening, use Report Bug.`,
  BadPlane:
    msg`Hew's own request to the model kernel carried a malformed plane. If this keeps happening, use Report Bug.`,
  BadRadialKind:
    msg`Hew's own request to the model kernel asked for a dimension style it doesn't recognize. If this keeps happening, use Report Bug.`,
  BadAffine:
    msg`Hew's own request to the model kernel carried a malformed move, rotate, or scale. If this keeps happening, use Report Bug.`,
  BadOp:
    msg`Hew's own request to the model kernel asked for a combine style it doesn't recognize. If this keeps happening, use Report Bug.`,
  BadNodeList:
    msg`Hew's own request to the model kernel carried a malformed selection list. If this keeps happening, use Report Bug.`,
  BadLibraryMeta:
    msg`Hew's own request to the model kernel carried malformed library-item metadata. If this keeps happening, use Report Bug.`,
  BadSelection:
    msg`Hew's own request to the model kernel carried a malformed selection. If this keeps happening, use Report Bug.`,
  BadCount:
    msg`Hew's own request to the model kernel asked for an invalid copy count. If this keeps happening, use Report Bug.`,
  BadAnchor:
    msg`Hew's own request to the model kernel carried a malformed snap point. If this keeps happening, use Report Bug.`,
  BadAxis:
    msg`Hew's own request to the model kernel named an axis it doesn't recognize. If this keeps happening, use Report Bug.`,
  BadFormat:
    msg`Hew's own request to the model kernel named an image format it doesn't recognize. If this keeps happening, use Report Bug.`,
  BadImagePlacement:
    msg`Hew's own request to the model kernel placed a picture on a plane it can't hold. If this keeps happening, use Report Bug.`,
  // ------------------------------------------------------- Place Image
  ImageUnreadable: msg`Hew can't read that picture. Save it as a PNG or JPEG and place it again.`,
  ImageTooLarge: msg`That picture is larger than 64 megapixels. Save a smaller copy of it and place that instead.`,
  ImageTooSmall: msg`That picture would be too small to place: each side needs to be at least 0.001 mm. Drag it out larger or type a bigger width.`,
  ImageTooBig: msg`That picture would be too big to place: each side can be at most 1,000,000 m. Drag it out smaller or type a smaller width.`,
  GRANT: msg`A plugin could not connect because its permissions were unreadable. Reinstall the plugin, or use Report Bug if this keeps happening.`,
  REGISTER: msg`A plugin's commands could not be added: its manifest declares a command Hew cannot accept. Update or reinstall the plugin.`,
  FORWARD: msg`A plugin command's answer could not be delivered. If this keeps happening, use Report Bug.`,
  SELECTION: msg`The selection could not be shared with a connected program. If this keeps happening, use Report Bug.`,
  ISOLATION: msg`What is isolated could not be shared with a connected program. If this keeps happening, use Report Bug.`,
  CONTEXT: msg`What is open for editing could not be shared with a connected program. If this keeps happening, use Report Bug.`,
  BadRenderMode:
    msg`The view mode must be shaded, xray or wireframe. If this keeps happening, use Report Bug.`,
  REPLAY: msg`This recorded session couldn't be replayed. If this keeps happening, use Report Bug.`,
}

/**
 * Parse a `"CODE: message"` thrown error string, returning the code prefix.
 * Returns null if the format doesn't match.
 */
export function parseKernelErrorCode(err: unknown): string | null {
  const text = err instanceof Error ? err.message : String(err)
  const match = /^([A-Za-z][A-Za-z0-9_]*):\s/.exec(text)
  return match ? match[1] : null
}

/**
 * Plain-language copy (what happened + suggested next step) for a kernel
 * error code. An unmapped code — a new kernel error the table hasn't caught
 * up with, kept from ever being a user's problem by the exhaustiveness
 * test — still gets a plain sentence, with the code kept for Report Bug.
 */
export function kernelErrorMessage(code: string, rawMsg: string): string {
  const described = DESCRIPTIONS[code]
  if (described !== undefined) return i18n._(described)
  return t`Hew couldn't complete that (${code}: ${rawMsg}). If this keeps happening, use Report Bug.`
}

/**
 * Importer wrapper prefixes: `import_dae`/`import_gltf`/`import_skp`/
 * `import_stl` (and, historically, `Scene::load`) tag errors with the
 * FORMAT, not a typed variant, and their payload text is already written
 * for people — e.g. the `.skp` version message carries exact re-save
 * guidance, and `.stl`'s carries "This file isn't a valid STL." / "This STL
 * is empty." Unwrap the tag and show the payload as-is; running it through
 * the table's fallback would bury real guidance inside boilerplate.
 *
 * Exported so `kernelErrors.test.ts`'s structural exhaustiveness check can
 * exclude these from "must have DESCRIPTIONS copy" — they're minted as
 * `ApiError(format!("DAE: ..."))`-style literals in wasm-api too, so the
 * source-derived code inventory sees them, but they deliberately bypass the
 * table above.
 */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- importer wrapper tags, matched against thrown error prefixes
export const WRAPPER_CODES = new Set(['DAE', 'glTF', 'SKP', 'STL', 'LOAD'])

/**
 * One-step convenience for `catch` sites holding a raw thrown value: map a
 * `"CODE: message"` kernel error to its plain-language copy, and pass any
 * other error text (host file errors, etc. — usually already human-readable)
 * through unchanged.
 */
export function friendlyErrorText(err: unknown): string {
  const raw = err instanceof Error ? err.message : String(err)
  const code = parseKernelErrorCode(err)
  if (code === null) return raw
  const rest = raw.slice(code.length + 2)
  return WRAPPER_CODES.has(code) ? rest : kernelErrorMessage(code, rest)
}

/** Every kernel error code with copy — exported for the exhaustiveness test. */
export function describedErrorCodes(): string[] {
  return Object.keys(DESCRIPTIONS)
}

/**
 * Codes logged at ERROR level (a real refusal of the user's geometry) rather
 * than warn — the single source App.tsx's toast/log path reads, kept here
 * beside the copy table so a new code gets its severity where it gets its
 * message, instead of drifting in a second hand-maintained array.
 */
/* oxlint-disable lingui/no-unlocalized-strings -- kernel error code identifiers, not copy */
const ERROR_LEVEL_CODES: ReadonlySet<string> = new Set([
  'WouldVanish', 'NonManifoldResult', 'ObjectNotSolid', 'DegenerateGeometry',
  'OperandNotSolid', 'DegenerateContact', 'EmptyResult', 'SingularTransform',
  'NothingRemoved',
  'BooleanOperandHasInstance', 'BooleanOperandNotSolid', 'BooleanOperandEmpty',
  // A refusal to rebuild geometry the user asked to rebuild. Its sibling
  // segment-count codes stay at warn: those are an out-of-range number, not a
  // refusal of the shape.
  'CurveNotRefacetable',
  // App-emitted refusal (no kernel enum): structural verbs on a sketch selection.
  'InvalidSelection',
])
/* oxlint-enable lingui/no-unlocalized-strings */

/** Whether `code` should be logged/toasted as an error (vs a warning). */
export function isErrorLevelCode(code: string): boolean {
  return ERROR_LEVEL_CODES.has(code)
}
