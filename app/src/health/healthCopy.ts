/**
 * Every Model Health and Make Solid string, in one place
 * (docs/design/import-health.md, Revisions 2 and 3, "Words"). One name per
 * thing: an object is solid or not; one that isn't has openings, is a flat
 * panel or a curved sheet, is part of a split shape, or can't be closed.
 * Not being solid is normal for a SketchUp model, so nothing here calls it a
 * problem. Kernel terms (rim, planar, manifold, seam, verdict, leaky) never
 * reach the screen.
 */

import { msg, plural, t } from '@lingui/core/macro'

/*
 * How this file stays translatable. The tables below are read as plain
 * strings by many components (`ACTION.close`, `MENU.makeSolid`), and some
 * are read once at module load by other tables. So their string members are
 * getters: each read resolves against the language active at that moment,
 * and no consumer had to change. Members that take a count or a name are
 * functions for the same reason. Counts use `plural` so each language
 * supplies its own forms.
 */

// ---------------------------------------------------------------- kinds

/** Group names, in the order the wizard and the panel show them. */
export const KIND_NAME = {
  get openings() {
    return t`Has openings`
  },
  get split() {
    return t`Split shapes`
  },
  get flatPanels() {
    return t`Flat panels`
  },
  get curvedSheets() {
    return t`Curved sheets`
  },
  get closerLook() {
    return t`Can't be closed`
  },
}

export type KindKey = keyof typeof KIND_NAME

/** Why Hew can't close an object (`openReason` in healthReport.ts). */
export type OpenReason = 'branches' | 'notFlat' | 'lip'

/** The short reason under a "can't be closed" item. */
export const REASON_SHORT: Record<OpenReason, string> = {
  get branches() {
    return t`Open edges branch`
  },
  get notFlat() {
    return t`Open edges don't lie flat`
  },
  get lip() {
    return t`Open edges run along a lip`
  },
}

/** The full reason, shown for the selected item while its open edges are highlighted. */
export const REASON_SENTENCE: Record<OpenReason, string> = {
  get branches() {
    return t`The highlighted edges are open. They branch or stop short, so Hew can't tell where each opening ends.`
  },
  get notFlat() {
    return t`The highlighted edges are open. They don't lie flat, so one flat face can't close them.`
  },
  get lip() {
    return t`The highlighted edges are open. They run along a flat lip, so a face there would cover the lip.`
  },
}

/** Follows the reason: what the person can conclude. */
export const REASON_ON_PURPOSE = msg`Shapes like this are usually open on purpose, like a pipe fitting or an electrical box, and work as they are.`

/** A row's detail text in the panel and the wizard lists. */
export function rowDetail(
  kind: KindKey,
  openings: number,
  kept: boolean,
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- OpenReason identifier, not copy
  reason: OpenReason = 'notFlat',
): string {
  switch (kind) {
    case 'openings':
      return plural(openings, { one: '# opening', other: '# openings' })
    case 'split':
      return t`Piece of a split shape`
    case 'flatPanels':
      return kept ? t`Flat panel, kept` : t`Flat panel`
    case 'curvedSheets':
      return kept ? t`Curved sheet, kept` : t`Curved sheet`
    case 'closerLook':
      return REASON_SHORT[reason]
  }
}

/** Object Info's Geometry line for an object that isn't solid. */
// oxlint-disable-next-line lingui/no-unlocalized-strings -- OpenReason identifier default, not copy
export function geometryLine(kind: KindKey, openings: number, kept: boolean, reason: OpenReason = 'notFlat'): string {
  switch (kind) {
    case 'openings':
      return plural(openings, { one: 'Open: # opening', other: 'Open: # openings' })
    case 'split':
      return t`Open: piece of a split shape`
    case 'flatPanels':
      return kept ? t`Flat panel (kept as is)` : t`Flat panel`
    case 'curvedSheets':
      return kept ? t`Curved sheet (kept as is)` : t`Curved sheet`
    case 'closerLook':
      // "Open edges branch": the reason already says the object is open.
      return REASON_SHORT[reason]
  }
}

/**
 * List labels that stay apart when several items share an imported name:
 * "Group#9", "Group#9 (2)", "Group#9 (3)".
 */
export function distinctLabels(names: string[]): string[] {
  const seen = new Map<string, number>()
  return names.map((name) => {
    const copy = (seen.get(name) ?? 0) + 1
    seen.set(name, copy)
    return copy === 1
      ? name
      : t({ comment: 'A list label made unique: the imported name, then which one of several with that name this is', message: `${name} (${copy})` })
  })
}

// --------------------------------------------------------------- actions

export const ACTION = {
  get close() {
    return t`Close`
  },
  get closeThis() {
    return t`Close this opening`
  },
  get closeAll() {
    return t`Close all`
  },
  get keep() {
    return t`Keep as is`
  },
  get keepAll() {
    return t`Keep all`
  },
  get unkeep() {
    return t`Don't keep`
  },
  get addThickness() {
    return t`Add thickness…`
  },
  get rejoin() {
    return t`Rejoin`
  },
  get rejoinSolid() {
    return t`Rejoin as one solid`
  },
  get keepInsideWalls() {
    return t`Keep the inside walls`
  },
  get previousOpening() {
    return t`Previous opening`
  },
  get nextOpening() {
    return t`Next opening`
  },
  get showInList() {
    return t`Show in list`
  },
  get makeSolid() {
    return t`Make Solid…`
  },
  get ok() {
    return t`OK`
  },
}

/** `placements` is how many copies of a component a repair reaches. */
export const TOOLTIP = {
  close: (placements: number) =>
    placements > 1
      ? plural(placements, {
          one: 'Close every opening with a flat face in all # copy',
          other: 'Close every opening with a flat face in all # copies',
        })
      : t`Close every opening with a flat face`,
  closeThis: (placements: number) =>
    placements > 1
      ? plural(placements, {
          one: 'Close the opening shown in the model in all # copy',
          other: 'Close the opening shown in the model in all # copies',
        })
      : t`Close the opening shown in the model`,
  keep: (placements: number) =>
    placements > 1
      ? plural(placements, {
          one: 'Leave this as it is and hide it from this list in all # copy',
          other: 'Leave this as it is and hide it from this list in all # copies',
        })
      : t`Leave this as it is and hide it from this list`,
  unkeep: (placements: number) =>
    placements > 1
      ? plural(placements, { one: 'List this again in all # copy', other: 'List this again in all # copies' })
      : t`List this again`,
  get addThickness() {
    return t`Give this panel a thickness so it becomes solid`
  },
  closeAll: (objects: number) =>
    plural(objects, {
      one: 'Close the openings of # object in one step',
      other: 'Close the openings of # objects in one step',
    }),
  keepAll: (objects: number) =>
    plural(objects, {
      one: 'Keep # object as it is, in one step',
      other: 'Keep # objects as they are, in one step',
    }),
  get addThicknessAll() {
    return t`Give these panels a thickness so they become solid`
  },
  get noCurvedThickness() {
    return t`Hew can't add thickness to curved sheets yet`
  },
  get rejoin() {
    return t`Close the pieces where they meet and remove the inside walls between them`
  },
  get rejoinSolid() {
    return t`Close the pieces where they meet, remove the inside walls, and join the pieces into one solid`
  },
  get rejoinKeepingWalls() {
    return t`Close the pieces where they meet and keep the inside walls`
  },
  get previousOpening() {
    return t`Previous opening (Shift+Tab)`
  },
  get nextOpening() {
    return t`Next opening (Tab)`
  },
}

// ----------------------------------------------------------------- panel

export const PANEL = {
  get title() {
    return t`Model Health`
  },
  get scopeLastImport() {
    return t`Last import`
  },
  get scopeSelection() {
    return t`Selection`
  },
  get scopeModel() {
    return t`Whole model`
  },
  get useSelection() {
    return t`Use current selection`
  },
  get search() {
    return t`Search`
  },
  get showKept() {
    return t`Show kept panels and sheets`
  },
  get allSolid() {
    return t`Every object in the model is solid.`
  },
  get nothingSelected() {
    return t`Select something in the model first.`
  },
  get nothingImported() {
    return t`Nothing has been imported yet.`
  },
  get isSolid() {
    return t`This object is solid.`
  },
  get noMatches() {
    return t`Nothing matches your search.`
  },
  get showingOne() {
    return t`Showing one object`
  },
  get showAllObjects() {
    return t`Show all objects`
  },
  get useSelectionTitle() {
    return t`List the objects that are selected now`
  },
  get working() {
    return t`Working…`
  },
}

/** The notice row for objects that are the same part (Link identical copies). */
export const LOOK_ALIKE = {
  /** "N look-alike objects can be linked". */
  notice(count: number): string {
    return plural(count, {
      one: '# look-alike object can be linked',
      other: '# look-alike objects can be linked',
    })
  },
  get action() {
    return t`Link identical copies`
  },
  get actionTitle() {
    return t`Make these objects linked copies, so editing one changes them all`
  },
}

/** The line under a row an action refused. */
export function rowWarning(action: 'close' | 'thicken' | 'keep' | 'rejoin', reason: string): string {
  switch (action) {
    case 'close':
      return t`Couldn't close: ${reason}`
    case 'thicken':
      return t`Couldn't add thickness: ${reason}`
    case 'keep':
      return t`Couldn't keep: ${reason}`
    case 'rejoin':
      return t`Couldn't rejoin: ${reason}`
  }
}

/** Under a split-shape piece left alone because another piece refused. */
export const SPLIT_PIECE_LEFT = msg`Left as is, because another piece of this shape couldn't be closed.`

// --------------------------------------------------------- undo and toasts

/** Undo-menu labels for the app's batches. */
export const UNDO_LABEL = {
  close: (objects: number) => plural(objects, { one: 'Close # object', other: 'Close # objects' }),
  keep: (objects: number) => plural(objects, { one: 'Keep # object as is', other: 'Keep # objects as is' }),
  thicken: (panels: number) => plural(panels, { one: 'Add thickness to # panel', other: 'Add thickness to # panels' }),
  rejoin: (shapes: number) => plural(shapes, { one: 'Rejoin # split shape', other: 'Rejoin # split shapes' }),
  get rejoinSolid() {
    return t`Rejoin as one solid`
  },
  get makeSolid() {
    return t`Make Solid`
  },
}

/** What a batch did, for its toast. */
export type ToastKind = 'close' | 'mark' | 'thicken' | 'repair' | 'rejoinSolid'

/**
 * A batch's toast: "Closed 11 objects." / "Closed 11 objects. 1 couldn't be
 * closed; see the list." / "Nothing was closed. 2 couldn't be closed; see
 * the list."
 */
export function batchToast(kind: ToastKind, ok: number, failed: number): string {
  let head: string
  let tail: string
  switch (kind) {
    case 'close':
      head =
        ok > 0
          ? plural(ok, { one: 'Closed # object.', other: 'Closed # objects.' })
          : t`Nothing was closed.`
      tail = plural(failed, { one: "# couldn't be closed; see the list.", other: "# couldn't be closed; see the list." })
      break
    case 'mark':
      head =
        ok > 0 ? plural(ok, { one: 'Kept # object.', other: 'Kept # objects.' }) : t`Nothing was kept.`
      tail = plural(failed, { one: "# couldn't be kept; see the list.", other: "# couldn't be kept; see the list." })
      break
    case 'thicken':
      head =
        ok > 0
          ? plural(ok, { one: 'Added thickness to # object.', other: 'Added thickness to # objects.' })
          : t`Nothing was given a thickness.`
      tail = plural(failed, {
        one: "# couldn't be given a thickness; see the list.",
        other: "# couldn't be given a thickness; see the list.",
      })
      break
    case 'rejoinSolid':
      head = ok > 0 ? t`Rejoined as one solid.` : t`Nothing was rejoined.`
      tail = plural(failed, { one: "# couldn't be rejoined; see the list.", other: "# couldn't be rejoined; see the list." })
      break
    case 'repair':
      head =
        ok > 0
          ? plural(ok, { one: 'Rejoined # object.', other: 'Rejoined # objects.' })
          : t`Nothing was rejoined.`
      tail = plural(failed, { one: "# couldn't be rejoined; see the list.", other: "# couldn't be rejoined; see the list." })
      break
  }
  return failed === 0 ? head : `${head} ${tail}`
}

// ---------------------------------------------------- import report, badge

/**
 * The import report's text. `verb` is "Imported" or "Opened". Objects that
 * aren't solid are stated as normal: SketchUp models are full of panels,
 * sheets and open shapes that work as they are.
 */
export function importSummary(verb: 'Imported' | 'Opened', objects: number, solid: number): string {
  const parts = [
    // `${objects}` rather than `#`: `#` adds a thousands separator ("1,491"), and
    // this line has always printed the bare number.
    verb === 'Imported'
      ? plural(objects, { one: `Imported ${objects} object.`, other: `Imported ${objects} objects.` })
      : plural(objects, { one: `Opened ${objects} object.`, other: `Opened ${objects} objects.` }),
  ]
  if (solid === objects) {
    parts.push(objects === 1 ? t`It is solid.` : t`All of them are solid.`)
    return parts.join(' ')
  }
  const rest = objects - solid
  if (solid > 0) parts.push(plural(solid, { one: '# is solid.', other: '# are solid.' }))
  parts.push(
    solid > 0
      ? plural(rest, {
          one: 'The other one is a panel, a sheet, or an open shape, like glass, terrain, or a pipe, which SketchUp models often have. It works as it is.',
          other:
            'The other # are panels, sheets, and open shapes, like glass, terrain, and pipes, which SketchUp models often have. They work as they are.',
        })
      : plural(rest, {
          one: 'It is a panel, a sheet, or an open shape, like glass, terrain, or a pipe, which SketchUp models often have. It works as it is.',
          other:
            'All # are panels, sheets, and open shapes, like glass, terrain, and pipes, which SketchUp models often have. They work as they are.',
        }),
  )
  parts.push(t`To push, pull, combine, or 3D print one, make it solid first with Make Solid.`)
  return parts.join(' ')
}

/** The import report's section labels. */
export const REPORT = {
  leftOut: (parts: number) => t`Parts left out (${parts})`,
  missingTextures: (textures: number) => t`Missing textures (${textures})`,
  details: (details: number) => t`Details (${details})`,
}

/** The log line for an import ("12 solid, 5 not solid"). */
export function importLogSplit(solid: number, notSolid: number): string {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- diagnostic log line for the maintainer, not UI copy
  return `${solid} solid, ${notSolid} not solid`
}

/** The status-bar badge's text: neutral either way. */
export function badgeText(objects: number, notSolid: number): string {
  if (notSolid > 0) {
    return plural(objects, {
      one: `# object · ${notSolid} not solid`,
      other: `# objects · ${notSolid} not solid`,
    })
  }
  return plural(objects, { one: '# object ✓ solid', other: '# objects ✓ solid' })
}

/** The badge's tooltip when some objects aren't solid (clicking opens Model Health). */
export const BADGE_TOOLTIP_LIST = msg`List the objects that aren't solid`

export const OUTLINER_TOOLTIP = {
  get open() {
    return t`Open shape: not solid`
  },
  get panel() {
    return t`Flat panel or curved sheet`
  },
}

export const MENU = {
  get makeSolid() {
    return t`Make Solid…`
  },
  get makeSolidHelp() {
    return t`Turn open shapes into solids so you can push, pull, combine, or 3D print them.`
  },
  get modelHealth() {
    return t`Model Health`
  },
  get modelHealthItem() {
    return t`Model Health…`
  },
  get modelHealthHelp() {
    return t`Open the list of every object that isn't solid.`
  },
}

/** The STL/3MF export warning (`format` is "STL" or "3MF"). */
export const EXPORT_NOT_SOLID = {
  title: (format: string) => t`Export ${format} Anyway?`,
  body: (format: string) => t`These objects aren't solid, so the ${format} may not print cleanly:`,
  get hint() {
    return t`Make Solid can close what can be closed.`
  },
  get makeSolid() {
    return t`Make Solid…`
  },
  get cancel() {
    return t`Cancel`
  },
  get exportAnyway() {
    return t`Export Anyway`
  },
}

// ------------------------------------------------------- add thickness dialog

export const THICKNESS = {
  title: (panels: number) =>
    panels === 1
      ? t`Add thickness`
      : plural(panels, { one: 'Add thickness to # panel', other: 'Add thickness to # panels' }),
  get field() {
    return t`Thickness`
  },
  get inFront() {
    return t`In front`
  },
  get behind() {
    return t`Behind`
  },
  get centered() {
    return t`Centered`
  },
  get confirm() {
    return t`Add thickness`
  },
  get cancel() {
    return t`Cancel`
  },
  get needValue() {
    return t`Enter a thickness, such as 3 mm.`
  },
  get needPositive() {
    return t`The thickness must be more than zero.`
  },
}

// --------------------------------------------------------------- isolation

export function isolationBar(objects: number): string {
  return plural(objects, { one: 'Showing # object', other: 'Showing # objects' })
}
export const SHOW_EVERYTHING = msg`Show everything`

// ------------------------------------------------------------------ wizard

/** Where Make Solid runs: the last import, the selection, or the whole model. */
export type WizardScope = 'import' | 'selection' | 'model'

export const WIZARD = {
  get title() {
    return t`Make Solid`
  },
  get back() {
    return t`Back`
  },
  get accept() {
    return t`Accept`
  },
  get showAll() {
    return t`Show all`
  },
  get chooseEach() {
    return t`Choose for each item`
  },
  get makeThisSolid() {
    return t`Make solid`
  },
  get showMore() {
    return t`Show 200 more`
  },
  get close() {
    return t`Close`
  },
  get doneButton() {
    return t`Done`
  },

  overviewTitle: (scope: WizardScope): string =>
    scope === 'selection'
      ? t`Make the selection solid`
      : scope === 'import'
        ? t`Make this import solid`
        : t`Make this model solid`,
  overview: (scope: WizardScope, objects: number, solid: number): string => {
    const checked =
      scope === 'import'
        ? plural(objects, { one: 'Hew checked the object in this import.', other: 'Hew checked the # objects in this import.' })
        : scope === 'selection'
          ? plural(objects, { one: 'Hew checked the object you selected.', other: 'Hew checked the # objects you selected.' })
          : plural(objects, { one: 'Hew checked the object in this model.', other: 'Hew checked the # objects in this model.' })
    const parts = [checked]
    parts.push(
      solid === objects
        ? objects === 1
          ? t`It is solid.`
          : t`All of them are solid.`
        : plural(solid, { one: '# is solid.', other: '# are solid.' }),
    )
    if (solid < objects) {
      parts.push(t`Open shapes work for most things; a solid can also be pushed, pulled, combined, and 3D printed.`)
    }
    return parts.join(' ')
  },
  get acceptRecommended() {
    return t`Use recommended choices`
  },
  get stepByStep() {
    return t`Go step by step`
  },
  get nothingToFix() {
    return t`Every object here is solid.`
  },

  // One overview line per page, with what Hew recommends.
  overviewLine: {
    openings: (shapes: number, close: boolean) =>
      close
        ? plural(shapes, {
            one: '# shape has openings: close them.',
            other: '# shapes have openings: close them.',
          })
        : plural(shapes, {
            one: '# shape has openings: leave them open.',
            other: '# shapes have openings: leave them open.',
          }),
    split: (shapes: number) =>
      plural(shapes, {
        one: '# shape was split into pieces: rejoin them.',
        other: '# shapes were split into pieces: rejoin them.',
      }),
    flatPanels: (panels: number) =>
      plural(panels, {
        one: '# flat panel: leave as it is.',
        other: '# flat panels: leave as they are.',
      }),
    curvedSheets: (sheets: number) =>
      plural(sheets, {
        one: "# curved sheet: Hew can't make it solid yet.",
        other: "# curved sheets: Hew can't make these solid yet.",
      }),
    closerLook: (shapes: number) =>
      plural(shapes, {
        one: "# shape can't be closed with flat faces.",
        other: "# shapes can't be closed with flat faces.",
      }),
  },

  get openingsTitle() {
    return t`Openings`
  },
  get openingsBody() {
    return t`These shapes have open sides or gaps. Most are open on purpose, like a duct, a pipe, or a box without a lid. Closing one adds a flat face across each opening, which makes it solid.`
  },
  get openingsClose() {
    return t`Close the openings`
  },
  get openingsLeave() {
    return t`Leave them open`
  },

  get splitTitle() {
    return t`Rejoin split shapes`
  },
  get splitBody() {
    return t`SketchUp drew each of these as one shape with a wall inside it, and they arrived as separate pieces. Hew can close the pieces where they meet and remove the wall between them. Openings the shape was drawn with stay open.`
  },
  get splitRejoin() {
    return t`Rejoin the pieces and remove the inside walls`
  },
  get splitRejoinSolid() {
    return t`Rejoin as one solid`
  },
  get splitRejoinSolidDetail() {
    return t`Close the pieces where they meet, remove the inside walls, and join the pieces into one solid`
  },
  get splitKeepWalls() {
    return t`Rejoin the pieces and keep the inside walls`
  },
  get splitLeave() {
    return t`Leave them as they are`
  },

  get panelsTitle() {
    return t`Flat panels`
  },
  get panelsBody() {
    return t`These are single flat faces, such as glass, signs, or a drawn floor. To make one solid, give it a thickness.`
  },
  get panelsKeep() {
    return t`Leave them as panels`
  },
  get panelsThicken() {
    return t`Add thickness`
  },

  get sheetsTitle() {
    return t`Curved sheets`
  },
  get sheetsBody() {
    return t`These are single curved sheets, such as terrain or fabric. Hew can't add thickness to curved sheets yet, so they stay as they are.`
  },
  get sheetsKeep() {
    return t`Leave them as sheets`
  },

  get closerTitle() {
    return t`Can't be closed`
  },
  get closerBody() {
    return t`Hew can't close these with flat faces. Most are open on purpose, like a pipe fitting or an electrical box, and work as they are. Click one to see its open edges and why.`
  },

  get recommended() {
    return t`Recommended`
  },
  copies: (copies: number) => (copies > 1 ? plural(copies, { one: '# copy', other: '# copies' }) : ''),

  get readyTitle() {
    return t`Ready`
  },
  ready: {
    close: (shapes: number) =>
      plural(shapes, { one: 'Close the openings in # shape.', other: 'Close the openings in # shapes.' }),
    rejoin: (shapes: number, keepWalls: boolean) =>
      keepWalls
        ? plural(shapes, {
            one: 'Rejoin # split shape and keep the inside walls.',
            other: 'Rejoin # split shapes and keep the inside walls.',
          })
        : plural(shapes, { one: 'Rejoin # split shape.', other: 'Rejoin # split shapes.' }),
    rejoinSolid: (shapes: number) =>
      plural(shapes, { one: 'Rejoin # split shape as one solid.', other: 'Rejoin # split shapes as one solid.' }),
    thicken: (panels: number, thickness: string) =>
      plural(panels, {
        one: `Give # flat panel a thickness of ${thickness}.`,
        other: `Give # flat panels a thickness of ${thickness}.`,
      }),
    leave: (objects: number) =>
      plural(objects, { one: 'Leave # object as it is.', other: 'Leave # objects as they are.' }),
    get nothing() {
      return t`Nothing will change. Go back to make a choice, or close this window.`
    },
  },
  get fix() {
    return t`Make Solid`
  },
  get working() {
    return t`Working…`
  },

  get doneTitle() {
    return t`Done`
  },
  done: (solidNow: number): string =>
    solidNow === 0
      ? t`No object became solid. To undo what did change, choose Edit ▸ Undo.`
      : plural(solidNow, {
          one: '# object is solid now. To undo all of it, choose Edit ▸ Undo.',
          other: '# objects are solid now. To undo all of it, choose Edit ▸ Undo.',
        }),
  stillOpen: (shapes: number) =>
    plural(shapes, { one: "# shape can't be closed:", other: "# shapes can't be closed:" }),
  couldntFix: (shapes: number) =>
    plural(shapes, { one: "Hew couldn't make this one solid:", other: "Hew couldn't make these # solid:" }),
  leftAsChosen: (objects: number) =>
    plural(objects, { one: '# object was left as you chose.', other: '# objects were left as you chose.' }),
  get nothingChangedTitle() {
    return t`Nothing changed`
  },
  get nothingChanged() {
    return t`Nothing was changed, so there is nothing to undo.`
  },
}
