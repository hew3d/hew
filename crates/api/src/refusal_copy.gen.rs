// GENERATED from app/src/kernelErrors.ts — do not edit; regenerate with:
//   REGENERATE_REFUSAL_COPY=1 pnpm --dir app exec vitest run src/kernelErrorsDump.test.ts

/// UI copy for a kernel error CODE, or `None` if the UI table (app/src/
/// kernelErrors.ts) has none — the caller falls back to the kernel's own
/// `Display` text for those.
pub fn ui_copy(code: &str) -> Option<&'static str> {
    Some(match code {
        "UnknownObject" => {
            "That object is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownFace" => {
            "That face is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownEdge" => {
            "That edge is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownVertex" => {
            "That point is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownSketch" => {
            "That sketch is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownRegion" => {
            "That profile is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownIsland" => {
            "That shape is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownGroup" => {
            "That group is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownComponent" => {
            "That component is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownInstance" => {
            "That component instance is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownGuide" => {
            "That guide is no longer there — the model changed since it was picked. Click it again."
        }
        "UnknownMaterial" => "That material is no longer in the palette. Pick another swatch.",
        "DegenerateFace" => "That face has no usable area. Pick a different face.",
        "ReplaceMaterialReplayStale" => {
            "Some of the faces that material swap touched have since been reshaped, so it can no longer be undone or redone on its own. Nothing was changed."
        }
        "PointOffPlane" => {
            "That point isn't on the drawing surface. Draw on the highlighted face or the ground plane."
        }
        "DegenerateSegment" => {
            "The line's two ends are the same point. Click a second, different point."
        }
        "WouldRetopologize" => {
            "Moving this point would make lines cross or merge. Move it somewhere clear, or delete and redraw the lines instead."
        }
        "MalformedRegion" => {
            "This profile's outline couldn't be traced. Redraw the shape; if it keeps failing, use Report Bug."
        }
        "DegenerateCurve" => {
            "That circle or arc is too small to keep. Drag a larger radius, or type an exact one."
        }
        "UnknownCurve" => {
            "That curve is no longer there — the model changed since it was picked. Click it again."
        }
        "CurveNotAnalytic" => {
            "This shape has no circle behind it, so there's nothing to re-build. Only a circle drawn with the Circle tool has a segment count."
        }
        "CurveNotRefacetable" => {
            "Only a whole, untouched circle can have its segment count changed — not a part-erased one, and not one other lines run into. Redraw the circle at the count you want."
        }
        "SegmentsBelowFloor" => {
            "A circle needs at least 24 segments — below that it stops being a circle and becomes a polygon. Use the Polygon tool for a coarser shape."
        }
        "SegmentsAboveCap" => {
            "That is more segments than a circle can hold. Enter a smaller count."
        }
        "RestoreConflicts" => {
            "Undo needs to put the original outline back, but newer drawing is in its way. Erase the overlapping lines and undo again."
        }
        "OffsetTooSmall" => {
            "That offset distance is too small to make a new boundary. Drag further, or type an exact distance."
        }
        "OffsetCollapsed" => {
            "The shape can't absorb that offset — its boundary would collapse, cross itself, or spike out of a sharp corner. Try a smaller distance, or soften the sharpest corner."
        }
        "SketchGestureAlreadyOpen" => {
            "The drawing tools got out of step. Press Escape and try again."
        }
        "SketchGestureNotOpen" => "The drawing tools got out of step. Press Escape and try again.",
        "DegenerateGuide" => {
            "The guide needs a definite direction. Drag a little further before dropping it."
        }
        "DegenerateUvFrame" => {
            "That texture position isn't valid. Drag the handle further before releasing it."
        }
        "BadUvFrame" => {
            "That texture position couldn't be read. Press Escape and start the positioning again."
        }
        "DistanceTooSmall" => {
            "That distance is too small to build anything. Drag further, or type an exact length."
        }
        "DegenerateGeometry" => {
            "This profile can't be extruded into a valid solid. Simplify the shape and try again."
        }
        "ObjectNotSolid" => {
            "This object isn't a watertight solid, so it can't be pushed or pulled. Check its solid status in the Object Info panel."
        }
        "WouldVanish" => {
            "Pushing that far would remove the whole object. Push a shorter distance, or delete the object instead."
        }
        "NonManifoldResult" => {
            "The walls this would create run into the object's other geometry. Try a different distance, or reshape the surrounding faces first."
        }
        "NotASubFace" => {
            "Push/Pull here needs a shape drawn on the face. Draw a closed outline on the face first."
        }
        "RadiusVanishes" => {
            "Pushing that far would shrink the curved wall to nothing. Push a shorter distance."
        }
        "WallNeighborNonPlanar" => {
            "Offsetting this curved wall would bend a neighboring face out of flat, so it was refused. Adjust or simplify the touching faces first."
        }
        "EmptyPath" => {
            "Follow Me needs a path. Select or click the line, curve, or face edge to follow."
        }
        "UnknownPathEdge" => {
            "That path line is no longer there — the model changed since it was picked. Click it again."
        }
        "PathBranches" => {
            "The path forks — Follow Me needs one continuous run of lines. Select a single chain with no branches."
        }
        "PathDisconnected" => "The path is in separate pieces. Select one connected run of lines.",
        "PathSegmentTooShort" => {
            "Part of the path is too short to follow. Redraw the path without tiny segments."
        }
        "ProfileNotPerpendicular" => {
            "Hew tries to stand the profile up automatically, but couldn't square it to the path here. Move the profile closer to where the sweep should start — on a drawn circle, near the rim, in line with the circle's center."
        }
        "PathDetachedFromProfile" => {
            "The path doesn't start on the profile's surface. Start the path at the profile — or, on a loop, cross the profile partway along a straight run, not at a corner."
        }
        "PathReverses" => {
            "The path doubles back on itself — or turns nearly all the way around — so there is no clean corner to turn. Remove the reversal, or soften the corner."
        }
        "PathTooTight" => {
            "The path turns tighter than the profile is wide, so the sweep would fold into itself. Use a smaller profile or gentler turns."
        }
        "ProfileCrossesAxis" => {
            "This profile crosses the circle's centre axis, so revolving it would lose the geometry on one side. Draw the profile entirely on one side of the axis instead."
        }
        "PartialSweepOnPole" => {
            "A partial sweep can't cut open this shape — its profile touches the turning axis, so the pole only exists in the full revolution. Sweep the whole path, or move the profile off the axis."
        }
        "SweepSelfIntersects" => {
            "Following this path would make the shape run into itself. Shorten the path or use a smaller profile."
        }
        "SweepDegenerate" => {
            "This profile can't be swept along that path into a valid solid. Simplify the profile or the path and try again."
        }
        "PathTooShort" => "The cut needs at least two points. Click a start and an end.",
        "EndpointNotOnBoundary" => {
            "A splitting line must start and end on the face's edges. Snap both ends to the face boundary."
        }
        "PointNotOnFace" => {
            "Part of the line leaves the face. Keep every point on the face being split."
        }
        "PathNotSimple" => {
            "The line crosses itself or touches the face's edge partway along. Draw a simple path from edge to edge."
        }
        "FacesNotCoplanar" => {
            "These two faces aren't in the same plane, so they can't be merged. Pick an edge whose two faces lie flat in one plane."
        }
        "BoundaryEdge" => {
            "This edge has a face on only one side — there's nothing to merge it with."
        }
        "SameFaceOnBothSides" => {
            "The same face is on both sides of this edge, so dissolving it would puncture the surface."
        }
        "SharedChainDisconnected" => {
            "These faces touch along more than one separate edge run, which merge can't dissolve yet. Merge along one shared run at a time."
        }
        "SharedChainCoversBoundary" => {
            "One face's entire boundary lies on the other, so this isn't an edge merge. Select the inner face itself and remove it instead."
        }
        "CurveClaimOffLoop" => {
            "The drawn outline and its circle disagree, so the imprint was refused. Redraw the circle; if it keeps failing, use Report Bug."
        }
        "LoopNotStrictlyInside" => {
            "The shape must sit fully inside the face, clear of its edges. Draw it a little smaller or further from the boundary."
        }
        "LoopSelfIntersects" => {
            "The shape's outline crosses itself. Draw a simple, non-crossing outline."
        }
        "NotAnInnerFace" => {
            "Only a shape drawn fully inside a face can be removed this way. Select the imprinted inner face itself."
        }
        "WouldCorrupt" => {
            "That edit would damage the surrounding geometry, so it was refused. Adjust the shape slightly and try again."
        }
        "BadLoop" => "The outline needs at least three points.",
        "OperandNotSolid" => {
            "Combining needs watertight solids on both sides. Check each object's solid status in the Object Info panel."
        }
        "EmptyResult" => {
            "The result would be empty — the objects don't overlap that way. Check that the solids actually intersect."
        }
        "SingularTransform" => {
            "One object's placement is scaled down to nothing, so the operation can't run."
        }
        "DegenerateContact" => {
            "The objects only touch along a face, edge, or corner — combining needs real overlap. Nudge one object so their volumes intersect."
        }
        "NotSolid" => {
            "Only a watertight solid can be sliced. Check its solid status in the Object Info panel."
        }
        "PlaneMissesSolid" => {
            "The slicing plane doesn't pass through the object. Position the cut so it goes through the solid."
        }
        "Degenerate" => {
            "The cut lines up exactly with an existing face or edge, so it wouldn't create two pieces. Move the cut slightly."
        }
        "Singular" => "That transform would scale the object down to nothing, so it was refused.",
        "DegenerateAxis" => {
            "The rotation axis needs two distinct points. Pick a second point further from the first."
        }
        "Reflection" => {
            "This would turn the object inside out (a mirror), which can't be baked into a solid. Mirror a component instance instead."
        }
        "InvalidRescaleFactor" => {
            "That resize factor isn't valid. Pick two points that aren't on top of each other and type a positive distance."
        }
        "InvalidSelection" => {
            "Sketches can't be part of a group or component. Select only objects, groups, or components."
        }
        "EmptyGroup" => "Select at least one object to group.",
        "EmptySelection" => {
            "The selection has nothing visible to transform — everything in it is hidden or empty. Unhide its contents, or select something visible."
        }
        "EmptyComponent" => "Select at least one object to turn into a component.",
        "ComponentCycle" => {
            "Placing that instance here would make a component contain itself, directly or through another component. Pick a different definition, or explode/make unique the instance that closes the loop."
        }
        "ComponentDepthExceeded" => {
            "That would nest components deeper than Hew supports. Flatten one of the levels — explode an instance, or make it unique — before placing this one."
        }
        "ComponentExpansionExceeded" => {
            "That would multiply past a million rendered component parts. Reduce how many copies the nested components repeat — explode or thin out a level — and try again."
        }
        "CannotExplodeReflected" => {
            "A mirrored instance can't be exploded — baking the mirror would turn the solid inside out. Use Make Unique instead."
        }
        "CannotExplodeNonUniformScale" => {
            "This instance has an unfinished sketch inside it, and its scale isn't even across all three axes, so the sketch's circles and arcs can't be baked exactly. Even out the instance's scale first, or finish (extrude) the sketch before exploding."
        }
        "DuplicateMember" => "The same object is in the selection twice. Reselect and try again.",
        "MixedParents" => {
            "Only siblings can be grouped — everything selected must be top-level, or all inside the same group. Move them to one level first."
        }
        "GroupedOperand" => {
            "This operation can't target an object inside a group. Ungroup it, or leave the group context, first."
        }
        "BooleanOperandHasInstance" => {
            "Combining can't consume a component instance — its geometry is shared with every copy. Explode the instance (or Make Unique, then Explode) first."
        }
        "BooleanOperandNotSolid" => {
            "Something in that selection is not a watertight solid, so it can't be combined. Check each object's solid badge in Object Info and fix or remove the leaky one."
        }
        "BooleanOperandEmpty" => {
            "That selection has no solids to combine. Pick a solid object or a group of solids."
        }
        "LastDefinitionMember" => {
            "This is the component's only member — deleting it would leave every instance empty. Delete the instances instead, or add another member first."
        }
        "AmbiguousInstanceScale" => {
            "This instance is scaled unevenly across its axes, so a single typed distance can't map onto it without ambiguity. Drag to the exact size instead of typing a length."
        }
        "ExplodeSessionOpen" => {
            "This component is already open for editing further out. Step back out to it (Escape) instead of opening it again — and close the editor before saving."
        }
        "ExplodeSessionNotOpen" => "No component is currently open for editing.",
        "ExplodeSessionPoseUnsupported" => {
            "This instance's pose is scaled unevenly or mirrored, so it can't be opened for direct editing. Even out its scale and unmirror it first."
        }
        "ExplodeSessionScope" => {
            "That isn't available while a group or component is open for editing. Close it first (Escape, or double-click outside), then try again."
        }
        "ExplodeSessionGroupedInstance" => {
            "A placement of this component sits inside a group, so it opens in the in-context editing mode instead."
        }
        "ExplodeSessionNestedGroup" => {
            "This group is nested inside another one, so it can't be opened for editing directly. Enter its enclosing group first, then drill down to this one."
        }
        "NestedComponentInContext" => {
            "This nested component can't open for editing here — its placement is mirrored, unevenly scaled, or inside a group. Explode the instance, or make it unique, to edit its parts."
        }
        "NothingToUndo" => "Nothing to undo.",
        "NothingToRedo" => "Nothing to redo.",
        "InverseFailed" => {
            "This step couldn't be undone safely, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session."
        }
        "InverseDiverged" => {
            "Undo produced a different result than expected, so the model was left unchanged. If this keeps happening, use Report Bug to capture the session."
        }
        "UnexpectedGestureState" => {
            "Finishing this step needed the drawing it just closed and couldn't find it, so nothing was placed and the model was left unchanged. If this keeps happening, use Report Bug to capture the session."
        }
        "TransactionHistoryDisturbed" => {
            "A connected tool undid history in the middle of its own batch, so the whole batch was discarded and the model was left unchanged."
        }
        "TransactionSessionUnbalanced" => {
            "A connected tool changed which group or component was open for editing without putting it back, so its whole batch was discarded and the model was left unchanged."
        }
        "TransactionGestureUnbalanced" => {
            "A connected tool left a drawing step unfinished at the end of its batch, so the whole batch was discarded and the model was left unchanged."
        }
        "UnknownTag" => "That tag doesn't exist in this model. Check the tag name and try again.",
        "InvalidAttrName" => {
            "Attribute names need both a namespace and a key. Give the attribute a non-empty name and try again."
        }
        "NonFiniteAttrValue" => {
            "Attribute numbers can't be NaN or infinity. Use a finite number and try again."
        }
        "AttrValueTooDeep" => {
            "This attribute value is nested too deeply to store. Flatten it and try again."
        }
        "UnknownAttr" => {
            "That attribute doesn't exist on this item, so there was nothing to remove."
        }
        "NotAContainer" => "This file isn't a Hew document. Pick a .hew file saved by Hew.",
        "UnsupportedVersion" => {
            "This file needs a newer version of Hew than this one. Update Hew and try again."
        }
        "MalformedManifest" => "This file is damaged and can't be opened.",
        "DanglingReference" => "This file is damaged and can't be opened.",
        "MissingAsset" => "This file is missing some of its data and can't be opened.",
        "Geometry" => "This file's geometry data is damaged and can't be opened.",
        "DegenerateVector" => {
            "That direction is too short to work with. Pick points further apart."
        }
        "DegeneratePlane" => {
            "Those points don't define a plane. Pick three points that aren't in a line."
        }
        "UnknownAnnotation" => {
            "That dimension or label is no longer there — the model changed since it was picked. Click it again."
        }
        "DegenerateAnnotation" => {
            "That dimension or label doesn't have a usable placement — its two points are on top of each other, or the circle it measures has no size. Drag to points with more room between them, or a bigger circle, and try again."
        }
        "MismatchedAnnotationKind" => {
            "Hew tried to turn this dimension or label into a different kind of entity, which isn't allowed. If this keeps happening, use Report Bug."
        }
        "BadNodeKind" => {
            "Hew's own request to the model kernel named an object kind it doesn't recognize. If this keeps happening, use Report Bug."
        }
        "BadPoint" => {
            "Hew's own request to the model kernel carried a malformed point. If this keeps happening, use Report Bug."
        }
        "BadVec" => {
            "Hew's own request to the model kernel carried a malformed direction. If this keeps happening, use Report Bug."
        }
        "BadVector" => {
            "Hew's own request to the model kernel carried a malformed xyz value. If this keeps happening, use Report Bug."
        }
        "BadProjection" => {
            "Hew's own request to the model kernel named a camera projection it doesn't recognize. If this keeps happening, use Report Bug."
        }
        "BadPlane" => {
            "Hew's own request to the model kernel carried a malformed plane. If this keeps happening, use Report Bug."
        }
        "BadRadialKind" => {
            "Hew's own request to the model kernel asked for a dimension style it doesn't recognize. If this keeps happening, use Report Bug."
        }
        "BadAffine" => {
            "Hew's own request to the model kernel carried a malformed move, rotate, or scale. If this keeps happening, use Report Bug."
        }
        "BadOp" => {
            "Hew's own request to the model kernel asked for a combine style it doesn't recognize. If this keeps happening, use Report Bug."
        }
        "BadNodeList" => {
            "Hew's own request to the model kernel carried a malformed selection list. If this keeps happening, use Report Bug."
        }
        "BadLibraryMeta" => {
            "Hew's own request to the model kernel carried malformed library-item metadata. If this keeps happening, use Report Bug."
        }
        "BadSelection" => {
            "Hew's own request to the model kernel carried a malformed selection. If this keeps happening, use Report Bug."
        }
        "BadCount" => {
            "Hew's own request to the model kernel asked for an invalid copy count. If this keeps happening, use Report Bug."
        }
        "BadCurve" => {
            "Hew's own request to the model kernel carried a malformed circle. If this keeps happening, use Report Bug."
        }
        "BadPath" => {
            "Hew's own request to the model kernel carried a malformed path. If this keeps happening, use Report Bug."
        }
        "BadAnchor" => {
            "Hew's own request to the model kernel carried a malformed snap point. If this keeps happening, use Report Bug."
        }
        "BadAxis" => {
            "Hew's own request to the model kernel named an axis it doesn't recognize. If this keeps happening, use Report Bug."
        }
        "BadFormat" => {
            "Hew's own request to the model kernel named an image format it doesn't recognize. If this keeps happening, use Report Bug."
        }
        "REPLAY" => {
            "This recorded session couldn't be replayed. If this keeps happening, use Report Bug."
        }
        _ => return None,
    })
}
