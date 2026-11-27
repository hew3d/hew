/**
 * The Preview wall test (docs/design/preview-channel.md section 3, point 2).
 *
 * It walks `preview-features.json` and names no feature: adding an entry puts
 * it under every check here, and graduating one (deleting its entry) removes
 * it, so the diff of this run is the record of what became public.
 *
 *  - Flag OFF (a production build): nothing an entry names is reachable from
 *    any single source: the tool registry, the viewport registrations, the
 *    menus, the palette, the keymap (defaults and a saved binding), the native
 *    command set, the dock, the tool command table.
 *  - Flag ON: every id an entry names exists in the app (an entry can never
 *    gate nothing), and the same surfaces contain it.
 *
 * The build flag is read in one tiny module (`./build.ts`). To get the app as
 * a production build sees it inside one vitest run, each case replaces that
 * module with a mock and imports the app fresh. There is no run-time switch.
 */

import { execFileSync, spawnSync } from 'node:child_process'
import { mkdirSync, readFileSync, readdirSync, rmSync, statSync, symlinkSync } from 'node:fs'
import { dirname, join, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { afterAll, beforeAll, describe, expect, it, vi } from 'vitest'
import * as THREE from 'three'
import type { PreviewFeature } from './registry'
import { createWall } from './wall'
import { chainItems, chainText, dispatchMap, parseSubmenu, pruneChain } from './nativeChains'

/** The scans below filter and report source files by path, so those paths are
 *  written with `/` on every platform; with Windows' `\` the folder filters
 *  would match nothing. */
const slashed = (path: string): string => path.split(sep).join('/')
const HERE = slashed(dirname(fileURLToPath(import.meta.url)))
const SRC = slashed(resolve(HERE, '..'))
const ROOT = resolve(HERE, '../../..')
const FEATURES = (JSON.parse(readFileSync(join(ROOT, 'preview-features.json'), 'utf8')) as { features: PreviewFeature[] })
  .features

/**
 * What `loadApp` may take in a `beforeAll`. It waits for nothing but the
 * dynamic imports of the app's surfaces (the tool registry, the menus, the
 * keymap, the palette and what they pull in), which the first call in a file
 * has to transform and evaluate cold: about 1.5 s on an idle 16-core machine,
 * 3.6 to 4.7 s with twice as many busy processes as cores, and a fraction of
 * that for a call after it (the transforms are cached). Vitest's default hook
 * limit is 10 s, which a machine running a build or a browser suite beside
 * this one exceeded; there is no race in it, only a cold import under load.
 * The limit is large, not tight: a hung import still fails, just not a slow one.
 */
const COLD_IMPORT_MS = 60_000

/** The app's surfaces, loaded in a fresh module graph for one configuration. */
async function loadApp(build: boolean) {
  vi.resetModules()
  vi.doMock('./build', () => ({ previewBuild: build }))
  const [
    preview,
    toolRegistry,
    toolActions,
    registrations,
    toolPanels,
    menuModel,
    palette,
    defaults,
    keymap,
    nativeMenu,
    dock,
    curveActions,
    savePanel,
    kernelErrors,
    toastActions,
    healthReport,
    eraser,
    eraserCopy,
    creaseErrors,
  ] = await Promise.all([
    import('./index'),
    import('../tools/toolRegistry'),
    import('../tools/toolActions'),
    import('../tools/registrations'),
    import('../tools/toolPanels'),
    import('../panels/toolsMenuModel'),
    import('../palette/registry'),
    import('../keymap/defaults'),
    import('../keymap/keymap'),
    import('../keymap/nativeMenu'),
    import('../panels/dockLogic'),
    import('../curveActions'),
    import('../io/savePanel'),
    import('../kernelErrors'),
    import('../toastActions'),
    import('../health/healthReport'),
    import('../tools/EraserTool'),
    import('../tools/eraserCreaseCopy'),
    import('../kernelErrorCopy/crease'),
  ])
  return {
    preview, toolRegistry, toolActions, registrations, toolPanels, menuModel, palette, defaults, keymap,
    nativeMenu, dock, curveActions, savePanel, kernelErrors, toastActions, healthReport, eraser, eraserCopy, creaseErrors,
  }
}
type App = Awaited<ReturnType<typeof loadApp>>

/** An Eraser over a fake scene whose one solid edge is a hard edge, for the crease gate. */
function eraserOver(app: App) {
  const scene = {
    edge_smoothness: () => 0,
    logical_edge: () => ({ edges: () => new BigUint64Array([5n]), free: () => undefined }),
    edge_is_creased: () => false,
    edge_endpoints: () => new Float64Array([0, 0, 0, 1, 0, 0]),
    set_crease: vi.fn(),
    history_bracket_open: vi.fn(),
    history_bracket_commit: vi.fn(),
    history_bracket_abort: vi.fn(),
  }
  const toast = vi.fn()
  const tool = new app.eraser.EraserTool(
    scene as never,
    new THREE.Group(),
    { resolveRef: () => null, runDelete: vi.fn(), onCreaseCommit: vi.fn() },
    toast,
  )
  const ray = { origin: [0, 0, 5], direction: [0, 0, -1] } as never
  const snap = { x: 0, y: 0, z: 0, kind: 'edge', object: 7n, element: 5n, elementKind: 'edge' } as never
  return { scene, toast, tool, ray, snap }
}

const CONTEXTS = ['empty', 'object', 'group', 'instance', 'multi', 'sketch'] as const
const ALL_GATES = {
  canGroup: true, canMakeComponent: true, canBoolean: true, canSaveToLibrary: true, canUnlink: true, canUnlinkAll: true,
}
const PLATFORMS = [
  { isMac: true, isDesktop: true },
  { isMac: true, isDesktop: false },
  { isMac: false, isDesktop: true },
  { isMac: false, isDesktop: false },
]

/** Every dock verb id the app offers, across contexts and with and without gates. */
function dockIds(app: App): Set<string> {
  const ids = new Set<string>()
  for (const ctx of CONTEXTS) {
    for (const gates of [undefined, ALL_GATES, { ...ALL_GATES, canUnlink: false }]) {
      for (const v of app.dock.dockVerbsFor(ctx, gates)) ids.add(v.id)
    }
  }
  return ids
}

function menuActionIds(app: App): string[] {
  const walk = (entries: readonly import('../panels/toolsMenuModel').MenuEntry[]): string[] =>
    entries.flatMap((e) =>
      e.kind === 'submenu' ? walk(e.items) : [app.menuModel.menuEntryActionId(e)].filter((x): x is string => x !== undefined),
    )
  return [...walk(app.menuModel.TOOLS_MENU), ...walk(app.menuModel.DRAW_MENU)]
}

function menuActionIdsOf(entries: readonly import('../panels/toolsMenuModel').MenuEntry[], app: App): string[] {
  return entries.flatMap((e) =>
    e.kind === 'submenu' ? menuActionIdsOf(e.items, app) : [app.menuModel.menuEntryActionId(e)].filter((x): x is string => x !== undefined),
  )
}

function paletteIds(app: App): Set<string> {
  return new Set(app.palette.paletteEntries().map((e) => e.id))
}

function sourceFiles(dir: string): string[] {
  const out: string[] = []
  for (const name of readdirSync(dir)) {
    const p = slashed(join(dir, name))
    if (name === 'wasm' || name === 'locales' || name === 'node_modules') continue
    if (statSync(p).isDirectory()) out.push(...sourceFiles(p))
    else if (/\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) && !p.includes(`${SRC}/preview/`)) out.push(p)
  }
  return out
}

const NO_SWITCHES = { off: new Set<string>(), on: new Set<string>() }
const switches = (off: string[], on: string[] = []) => ({ off: new Set(off), on: new Set(on) })
/** The ids a preview build has on before anyone touches a switch. */
const ON_BY_DEFAULT = FEATURES.filter((f) => f.defaultOn !== false).map((f) => f.id)
const LOADERS = FEATURES.filter((f) => f.loader !== undefined)

describe('the registry file', () => {
  it('has unique feature ids, and no id claimed by two features', () => {
    expect(new Set(FEATURES.map((f) => f.id)).size).toBe(FEATURES.length)
    for (const kind of ['tools', 'actions', 'dockChips', 'behaviors', 'apiCommands'] as const) {
      const all = FEATURES.flatMap((f) => f[kind])
      expect(all.filter((id, i) => all.indexOf(id) !== i), `${kind} named twice`).toEqual([])
    }
  })
  it('names every loader entry point once, and only on a feature that is off until asked for', () => {
    expect(LOADERS.length).toBeGreaterThan(0)
    for (const kind of ['roots', 'panes', 'shellCommands', 'kernelExports', 'cliCommands', 'storage'] as const) {
      const all = LOADERS.flatMap((f) => f.loader![kind])
      expect(all.filter((id, i) => all.indexOf(id) !== i), `${kind} named twice`).toEqual([])
    }
    for (const f of LOADERS) {
      expect(f.defaultOn, `${f.id} loads code, so its switch starts off`).toBe(false)
      expect(f.loader!.markers.length, `${f.id} has nothing a production artifact can be scanned for`).toBeGreaterThan(0)
    }
  })
  it('every loader root is selected in main.tsx through the wall, and main.tsx selects no other', () => {
    const main = readFileSync(join(SRC, 'main.tsx'), 'utf8')
    const asked = [...main.matchAll(/rootAllowed\('([^']+)'\)/g)].map((m) => m[1])
    expect([...new Set(asked)].sort()).toEqual(LOADERS.flatMap((f) => f.loader!.roots).sort())
    // The build literal comes first, so a production bundle drops the root's
    // name and its import; and the hash is compared with the same id.
    for (const id of asked) {
      expect(main, id).toContain(`previewBuild && rootAllowed('${id}') && window.location.hash.startsWith('#${id}')`)
    }
  })
  it('preview-only code is imported only in loaderRoots.ts, each import behind the build literal', () => {
    // A dynamic import behind a constant from another module still ships its
    // chunk, so the literal has to be in the importing module itself.
    const importers = [...sourceFiles(SRC), ...readdirSync(HERE).filter((n) => /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n)).map((n) => `${HERE}/${n}`)]
      .filter((f) => !f.includes(`${SRC}/plugins/`))
      .filter((f) => /(from|import\()\s*['"`][./]+\/plugins(\/|['"`])/.test(readFileSync(f, 'utf8')))
      .map((f) => f.slice(SRC.length + 1))
    expect(importers).toEqual(['preview/loaderRoots.ts'])
    const roots = readFileSync(join(HERE, 'loaderRoots.ts'), 'utf8')
    expect(roots).not.toMatch(/^(import|export) [^\n]*\/plugins(\/|['"`])/m)
    const imports = [...roots.matchAll(/import\(\s*['"`][^'"`]+['"`]\s*\)/g)]
    expect(imports.length).toBeGreaterThan(0)
    for (const m of imports) {
      const line = roots.slice(roots.lastIndexOf('\nexport const ', m.index), m.index)
      expect(line, m[0]).toMatch(/=\s*__HEW_PREVIEW__\s*\?\s/)
    }
  })
  it('every-build code takes only types from the plugin SDK', () => {
    // The SDK runtime is the realm's code: a value import would put it in
    // every build. `import type` (and `export type`) is erased.
    const valueImport = (text: string): string[] =>
      [
        ...text.matchAll(/^\s*(import|export)\b(?!\s+type\b)[^;]*?from\s*['"`][^'"`]*sdk\/plugin[^'"`]*['"`]/gm),
        ...text.matchAll(/^\s*import\s*['"`][^'"`]*sdk\/plugin[^'"`]*['"`]/gm),
        ...text.matchAll(/import\(\s*['"`][^'"`]*sdk\/plugin[^'"`]*['"`]\s*\)/g),
      ].map((m) => m[0].trim())
    // The rule itself, on what it must catch and let through.
    expect(valueImport("import { createRuntime } from '../../../sdk/plugin/src/core'")).toHaveLength(1)
    expect(valueImport("import '../../sdk/plugin/src/runtime'")).toHaveLength(1)
    expect(valueImport("const m = await import('../../sdk/plugin/src/core')")).toHaveLength(1)
    expect(valueImport("export { RUNTIME_LIMITS } from '../../sdk/plugin/src/protocol'")).toHaveLength(1)
    expect(valueImport("import type { HostToRealm } from '../../../sdk/plugin/src/protocol'")).toEqual([])
    expect(valueImport("export type { HostToRealm } from '../../../sdk/plugin/src/protocol'")).toEqual([])
    const offenders = sourceFiles(SRC)
      .filter((f) => !f.includes(`${SRC}/plugins/`))
      .flatMap((f) => valueImport(readFileSync(f, 'utf8')).map((m) => `${f.slice(SRC.length + 1)}: ${m}`))
    expect(offenders).toEqual([])
  })
  it('every loader pane is a Settings pane asked for through the wall', () => {
    const settings = readFileSync(join(SRC, 'settings/SettingsWindow.tsx'), 'utf8')
    const asked = [...settings.matchAll(/paneAllowed\('([^']+)'\)/g)].map((m) => m[1])
    expect([...new Set(asked)].sort()).toEqual(LOADERS.flatMap((f) => f.loader!.panes).sort())
  })
  it('gives every feature an English name and a one-line state', () => {
    for (const f of FEATURES) {
      expect(f.name.trim(), f.id).not.toBe('')
      expect(f.state.trim(), f.id).not.toBe('')
      expect(f.state.includes('\n'), f.id).toBe(false)
    }
  })
})

describe('createWall', () => {
  const some = FEATURES[0]
  it('withholds everything it lists when the build flag is off', () => {
    const wall = createWall(false, FEATURES, NO_SWITCHES)
    expect(wall.featureEnabled(some.id)).toBe(false)
    expect(wall.features()).toEqual([])
    for (const t of some.tools) expect(wall.toolAllowed(t)).toBe(false)
    expect(wall.toolAllowed('a tool nobody lists')).toBe(true)
  })
  it('honors a switch only in a preview build', () => {
    const preview = createWall(true, FEATURES, switches([some.id]))
    expect(preview.featureEnabled(some.id)).toBe(false)
    for (const t of some.tools) expect(preview.toolAllowed(t)).toBe(false)
    const other = FEATURES.find((f) => f.id !== some.id && f.defaultOn !== false)
    if (other !== undefined) expect(preview.featureEnabled(other.id)).toBe(true)
    // A stored switch means nothing in production, and nothing is switched on there.
    expect(createWall(false, FEATURES, NO_SWITCHES).featureEnabled(some.id)).toBe(false)
  })
  it('hangs the Eraser\'s crease modifier on Smooth, and leaves the Eraser itself public', () => {
    expect(FEATURES.find((f) => f.id === 'smooth')?.behaviors).toContain('eraser-crease')
    expect(FEATURES.find((f) => f.id === 'smooth')?.apiCommands).toContain('hew.object.crease')
    expect(FEATURES.some((f) => f.tools.includes('Eraser') || f.actions.includes('tool-eraser'))).toBe(false)
    const on = createWall(true, FEATURES, NO_SWITCHES)
    const smoothOff = createWall(true, FEATURES, switches(['smooth']))
    const production = createWall(false, FEATURES, NO_SWITCHES)
    expect(on.behaviorEnabled('eraser-crease')).toBe(true)
    for (const w of [on, smoothOff, production]) {
      expect(w.toolAllowed('Eraser')).toBe(true)
      expect(w.actionAllowed('tool-eraser')).toBe(true)
    }
    expect(smoothOff.behaviorEnabled('eraser-crease')).toBe(false)
    expect(production.behaviorEnabled('eraser-crease')).toBe(false)
  })
  it('keeps a default-off feature off until its switch is turned on, in a preview build only', () => {
    for (const f of FEATURES.filter((x) => x.defaultOn === false)) {
      expect(createWall(true, FEATURES, NO_SWITCHES).featureEnabled(f.id), f.id).toBe(false)
      expect(createWall(true, FEATURES, switches([], [f.id])).featureEnabled(f.id), f.id).toBe(true)
      // Off wins over on, and a stored switch means nothing in production.
      expect(createWall(true, FEATURES, switches([f.id], [f.id])).featureEnabled(f.id), f.id).toBe(false)
      expect(createWall(false, FEATURES, switches([], [f.id])).featureEnabled(f.id), f.id).toBe(false)
    }
    // An "on" entry for an ordinary feature changes nothing about it.
    expect(createWall(true, FEATURES, switches([some.id], [some.id])).featureEnabled(some.id)).toBe(false)
  })
  it('answers the loader questions only for a listed id, a preview build and a switch that is on', () => {
    for (const f of LOADERS) {
      const walls = {
        production: createWall(false, FEATURES, switches([], [f.id])),
        switchOff: createWall(true, FEATURES, NO_SWITCHES),
        switchedOffAgain: createWall(true, FEATURES, switches([f.id], [f.id])),
        on: createWall(true, FEATURES, switches([], [f.id])),
      }
      for (const [name, wall] of Object.entries(walls)) {
        const want = name === 'on'
        expect(wall.loaderEnabled(f.id), `${f.id} ${name}`).toBe(want)
        for (const id of f.loader!.roots) expect(wall.rootAllowed(id), `${id} ${name}`).toBe(want)
        for (const id of f.loader!.panes) expect(wall.paneAllowed(id), `${id} ${name}`).toBe(want)
      }
    }
    // Unlike a tool or a command, an unlisted id is refused: nothing these
    // guard is public, so a typo or a graduated entry cannot open one.
    const everythingOn = createWall(true, FEATURES, switches([], FEATURES.map((f) => f.id)))
    expect(everythingOn.rootAllowed('a root nobody lists')).toBe(false)
    expect(everythingOn.paneAllowed('a pane nobody lists')).toBe(false)
    expect(everythingOn.loaderEnabled('a feature nobody lists')).toBe(false)
    // A feature with no loader has no loader to enable.
    expect(everythingOn.loaderEnabled(some.id)).toBe(some.loader !== undefined)
    expect(createWall(true, [], NO_SWITCHES).rootAllowed('plugin-probe')).toBe(false)
  })
  it('treats an id the registry does not list as public (a graduated feature)', () => {
    const wall = createWall(false, [], NO_SWITCHES)
    expect(wall.featureEnabled('anything')).toBe(true)
    expect(wall.toolAllowed('Fillet')).toBe(true)
  })
})

describe('flag OFF: a production build', () => {
  let app: App
  beforeAll(async () => {
    app = await loadApp(false)
  }, COLD_IMPORT_MS)
  afterAll(() => {
    vi.doUnmock('./build')
    vi.resetModules()
  })

  it('reports itself as a production build with every feature off', () => {
    expect(app.preview.previewBuild).toBe(false)
    expect(app.preview.previewFeatures()).toEqual([])
    expect(app.preview.enabledFeatureIds()).toEqual([])
    for (const f of FEATURES) expect(app.preview.featureEnabled(f.id), f.id).toBe(false)
  })

  for (const f of FEATURES) {
    describe(`entry "${f.id}"`, () => {
      it('has no tool in the registry, the rail, the registrations or the panels', () => {
        const names = app.toolRegistry.TOOL_REGISTRY.map((t) => t.name as string)
        const railed = app.toolRegistry.RAIL_GROUPS.flatMap((g) => app.toolRegistry.toolsInGroup(g)).map((t) => t.name as string)
        for (const tool of f.tools) {
          expect(app.preview.toolAllowed(tool), tool).toBe(false)
          expect(names, tool).not.toContain(tool)
          expect(app.toolRegistry.TOOLS as readonly string[], tool).not.toContain(tool)
          expect(railed, tool).not.toContain(tool)
          expect(app.registrations.TOOL_REGISTRATIONS.map((r) => r.name as string), tool).not.toContain(tool)
          expect(app.registrations.toolRegistration(tool), tool).toBeUndefined()
          expect(app.toolPanels.TOOL_PANELS.map((p) => p.tool as string), tool).not.toContain(tool)
          expect(app.toolPanels.panelsForTool(tool), tool).toEqual([])
          // A stale id (persisted, restored, scripted) falls back to Select.
          expect(app.toolRegistry.usableTool(tool), tool).toBe('Select')
        }
      })

      it('has no action in the menus, the palette, the keymap, the native set or the dispatch table', () => {
        const menus = menuActionIds(app)
        const palette = paletteIds(app)
        const dispatched = app.toolActions.TOOL_ACTIONS.map((a) => a.id)
        for (const id of f.actions) {
          expect(app.preview.actionAllowed(id), id).toBe(false)
          expect(app.curveActions.actionOffered(id), id).toBe(false)
          expect(menus, id).not.toContain(id)
          expect(palette, id).not.toContain(id)
          expect(dispatched, id).not.toContain(id)
          expect(app.toolActions.toolActionFor(id), id).toBeUndefined()
          const opened: string[] = []
          expect(app.toolActions.runToolAction(id, (t) => opened.push(t)), id).toBe(false)
          expect(opened, id).toEqual([])
          expect(app.nativeMenu.NATIVE_COMMANDS.has(id), id).toBe(false)
          for (const platform of PLATFORMS) expect(Object.keys(app.defaults.defaultBindings(platform)), id).not.toContain(id)
        }
        // No palette entry opens a withheld tool either, whatever its id.
        for (const e of app.palette.paletteEntries()) {
          if (e.toolName !== undefined) expect(f.tools, e.id).not.toContain(e.toolName)
        }
      })

      it('ignores a saved key binding for each of its actions', () => {
        const base = app.defaults.defaultBindings({ isMac: false, isDesktop: false })
        for (const id of f.actions) {
          const km = app.keymap.buildKeymap(base, { [id]: ['Mod+Alt+9'] }, false)
          expect(km.bindings[id], id).toBeUndefined()
          expect([...km.byChord.values()], id).not.toContain(id)
          expect(km.byChord.has('Ctrl+Alt+9'), id).toBe(false)
        }
      })

      it('has no dock chip', () => {
        const chips = dockIds(app)
        for (const id of f.dockChips) {
          expect(app.preview.chipAllowed(id), id).toBe(false)
          expect(chips, id).not.toContain(id)
        }
        for (const id of f.actions) expect(chips, id).not.toContain(id)
      })

      it('has every behavior off', () => {
        for (const b of f.behaviors) expect(app.preview.behaviorEnabled(b), b).toBe(false)
      })
    })
  }

  it('prunes the menus without leaving an empty submenu or a stray separator', () => {
    const check = (entries: readonly import('../panels/toolsMenuModel').MenuEntry[]): void => {
      expect(entries[0]?.kind).not.toBe('separator')
      expect(entries[entries.length - 1]?.kind).not.toBe('separator')
      entries.forEach((e, i) => {
        if (e.kind === 'separator') expect(entries[i - 1]?.kind).not.toBe('separator')
        if (e.kind === 'submenu') {
          expect(e.items.length).toBeGreaterThan(0)
          check(e.items)
        }
      })
    }
    check(app.menuModel.TOOLS_MENU)
    check(app.menuModel.DRAW_MENU)
  })

  it('leaves the ungated tools alone', () => {
    const gated = new Set(FEATURES.flatMap((f) => f.tools))
    const names = app.toolRegistry.TOOL_REGISTRY.map((t) => t.name as string)
    expect(names.length).toBeGreaterThan(10)
    expect(names.filter((n) => gated.has(n))).toEqual([])
    for (const n of ['Select', 'Line', 'Push/Pull', 'Follow Me', 'Move', 'Eraser']) expect(names).toContain(n)
  })

  describe('behaviors that are not lists', () => {
    it('keeps the Eraser: its tool, action, palette entry and shortcut are all offered', () => {
      expect(app.toolRegistry.TOOL_REGISTRY.map((t) => t.name as string)).toContain('Eraser')
      expect(app.preview.toolAllowed('Eraser')).toBe(true)
      expect(app.preview.actionAllowed('tool-eraser')).toBe(true)
      expect(menuActionIds(app)).toContain('tool-eraser')
      expect(paletteIds(app).has('tool-eraser')).toBe(true)
      expect(app.nativeMenu.NATIVE_COMMANDS.has('tool-eraser')).toBe(true)
      for (const platform of PLATFORMS) expect(app.defaults.defaultBindings(platform)['tool-eraser']).toEqual(['E'])
    })
    it('ignores the Eraser\'s crease modifier: a Ctrl-click on a solid edge changes nothing and says nothing of creasing', () => {
      expect(app.preview.behaviorEnabled('eraser-crease')).toBe(false)
      const { scene, toast, tool, ray, snap } = eraserOver(app)
      tool.setCreaseModifier(true)
      tool.onPointerMove(snap, ray)
      expect(tool.statusHint()).toBe("Edges of a solid can't be erased.")
      tool.onPointerRawDown(0, 0)
      tool.onPointerDown(snap, ray)
      tool.onPointerUp(snap, ray)
      expect(scene.set_crease).not.toHaveBeenCalled()
      expect(toast).not.toHaveBeenCalled()
    })
    it('compiles the crease words out: the copy objects are empty, so no crease text is in the bundle', () => {
      expect(app.eraserCopy.creaseCopy).toBeNull()
      expect(app.creaseErrors.CREASE_ERRORS).toEqual({})
    })
    it('hides the cage export option from the Export dialog and the native panel', () => {
      expect(app.savePanel.CAGE_FORMATS.size).toBe(0)
      expect(app.savePanel.exportChoiceFrom({ format: 'glb', options: { cages: true } } as never, 'mm' as never)?.cages).toBeUndefined()
    })
    it('words the zero-area refusal without pointing at Remove degenerate faces, and offers no button', () => {
      expect(app.kernelErrors.kernelErrorMessage('DegenerateInput', '')).not.toMatch(/Model Health|Remove degenerate/)
      expect(app.toastActions.toastActionFor('DegenerateInput', () => undefined)).toBeUndefined()
    })
    it('leaves zero-area objects out of the Model Health report', () => {
      const json = JSON.stringify({
        rows: [
          { object: '1', verdict: { kind: 'solid' }, degenerate_faces: 3 },
          { object: '2', verdict: { kind: 'flat_sheet' }, degenerate_faces: 2 },
        ],
      })
      const report = app.healthReport.parseHealthReport(json)
      expect(report.rows.map((r) => r.object)).toEqual([2n])
      expect(report.rows[0].degenerateFaces).toBe(0)
    })
    it('keeps Smooth > Off working but offers it only from Object Info', () => {
      expect(app.preview.actionAllowed('smooth-off')).toBe(true)
      expect(app.curveActions.actionOffered('smooth-off')).toBe(false)
      expect(paletteIds(app).has('smooth-off')).toBe(false)
      expect(app.nativeMenu.NATIVE_COMMANDS.has('smooth-off')).toBe(false)
      for (const platform of PLATFORMS) expect(Object.keys(app.defaults.defaultBindings(platform))).not.toContain('smooth-off')
    })
  })
})

describe('flag ON: a preview build', () => {
  let app: App
  beforeAll(async () => {
    app = await loadApp(true)
  }, COLD_IMPORT_MS)
  afterAll(() => {
    vi.doUnmock('./build')
    vi.resetModules()
  })

  const allToolNames = (): Set<string> => new Set(app.toolRegistry.ALL_TOOL_SPECS.map((t) => t.name as string))

  it('is a preview build with every feature at its default: on, except one that loads code', () => {
    expect(app.preview.previewBuild).toBe(true)
    expect(app.preview.previewFeatures().map((f) => f.id)).toEqual(FEATURES.map((f) => f.id))
    expect(app.preview.enabledFeatureIds()).toEqual(ON_BY_DEFAULT)
    for (const f of LOADERS) {
      expect(app.preview.loaderEnabled(f.id), f.id).toBe(false)
      for (const id of f.loader!.roots) expect(app.preview.rootAllowed(id), id).toBe(false)
    }
  })

  it('offers the Eraser\'s crease modifier (the eraser-crease behavior, part of Smooth)', () => {
    expect(app.preview.behaviorEnabled('eraser-crease')).toBe(true)
    const { scene, tool, ray, snap } = eraserOver(app)
    tool.setCreaseModifier(true)
    tool.onPointerMove(snap, ray)
    expect(tool.statusHint()).toBe('Crease: keeps this edge sharp under Smooth')
    tool.onPointerRawDown(0, 0)
    tool.onPointerDown(snap, ray)
    tool.onPointerUp(snap, ray)
    expect(scene.set_crease).toHaveBeenCalledOnce()
  })

  it('carries the crease words in a preview build', () => {
    expect(app.eraserCopy.creaseCopy?.creaseOn).toBe('Crease: keeps this edge sharp under Smooth')
    expect(Object.keys(app.creaseErrors.CREASE_ERRORS).sort()).toEqual(['CreaseOnBoundary', 'CreaseOnSoftEdge'])
  })

  it('has the whole tool registry (nothing is withheld)', () => {
    expect(app.toolRegistry.TOOL_REGISTRY.map((t) => t.name)).toEqual(app.toolRegistry.ALL_TOOL_SPECS.map((t) => t.name))
  })

  for (const f of FEATURES) {
    describe(`entry "${f.id}"`, () => {
      it('names tools that exist, each declaring its command in the same entry', () => {
        const specs = new Map(app.toolRegistry.ALL_TOOL_SPECS.map((t) => [t.name as string, t]))
        for (const tool of f.tools) {
          const spec = specs.get(tool)
          expect(spec, `"${tool}" is not a tool in toolRegistry.ts`).toBeDefined()
          expect(f.actions, `${tool}'s command ${spec?.actionId} is not in ${f.id}.actions`).toContain(spec?.actionId)
        }
        expect(allToolNames().size).toBeGreaterThan(0)
      })

      it('has each tool in the registry, the rail (when it has a slot), the registrations and the dispatch table', () => {
        for (const tool of f.tools) {
          const spec = app.toolRegistry.ALL_TOOL_SPECS.find((t) => t.name === tool)!
          expect(app.toolRegistry.TOOL_REGISTRY.map((t) => t.name as string)).toContain(tool)
          if (spec.group !== undefined) expect(app.toolRegistry.toolsInGroup(spec.group).map((t) => t.name as string)).toContain(tool)
          expect(app.toolActions.toolActionFor(spec.actionId)?.tool).toBe(tool)
          expect(app.toolRegistry.usableTool(tool)).toBe(tool)
        }
      })

      it('names actions the app dispatches, lists and offers', () => {
        const known = new Set<string>([
          ...app.toolActions.TOOL_ACTIONS.map((a) => a.id),
          ...paletteIds(app),
          ...app.nativeMenu.ALL_NATIVE_COMMANDS,
          ...menuActionIds(app),
        ])
        const palette = paletteIds(app)
        for (const id of f.actions) {
          expect(known.has(id), `action "${id}" is not a command anywhere in the app`).toBe(true)
          expect(app.preview.actionAllowed(id), id).toBe(true)
          expect(palette.has(id), `${id} is missing from the palette`).toBe(true)
          // A command the native menu has (not every one does: Skin is palette-only) is in its set.
          expect(app.nativeMenu.NATIVE_COMMANDS.has(id), `${id} is missing from the native command set`).toBe(
            app.nativeMenu.ALL_NATIVE_COMMANDS.includes(id),
          )
        }
      })

      it('names dock chips the dock offers', () => {
        const chips = dockIds(app)
        for (const id of f.dockChips) expect(chips.has(id), `dock chip "${id}" is not in the dock`).toBe(true)
      })

      it('names API commands that exist in the generated client', () => {
        const client = readFileSync(join(SRC, 'api/hewApi.gen.ts'), 'utf8')
        for (const cmd of f.apiCommands) expect(client.includes(`'${cmd}'`), `${cmd} is not in hewApi.gen.ts`).toBe(true)
      })
    })
  }

  it('lets a saved key binding through for an action that is on', () => {
    const base = app.defaults.defaultBindings({ isMac: false, isDesktop: false })
    const id = FEATURES.flatMap((f) => f.actions)[0]
    if (id === undefined) return
    expect(app.keymap.buildKeymap(base, { [id]: ['Mod+Alt+9'] }, false).bindings[id]).toEqual(['Ctrl+Alt+9'])
  })
})

describe('behavior ids in the code and the registry', () => {
  const referenced = new Map<string, string[]>()
  for (const file of sourceFiles(SRC)) {
    const text = readFileSync(file, 'utf8')
    for (const m of text.matchAll(/\bbehaviorEnabled\(\s*'([^']+)'\s*\)/g)) {
      referenced.set(m[1], [...(referenced.get(m[1]) ?? []), file.slice(SRC.length + 1)])
    }
  }
  const declared = FEATURES.flatMap((f) => f.behaviors)

  it('every behavior the registry names is read somewhere (an entry cannot gate nothing)', () => {
    expect(declared.filter((b) => !referenced.has(b))).toEqual([])
  })
  it('every behavior the code reads is in the registry (a typo would leave it open)', () => {
    expect([...referenced.keys()].filter((b) => !declared.includes(b))).toEqual([])
  })
})

describe('the wall at its entry points', () => {
  const app = readFileSync(join(SRC, 'App.tsx'), 'utf8')
  const viewport = readFileSync(join(SRC, 'viewport/Viewport.tsx'), 'utf8')

  it("drops a withheld command at the top of App's menuActionRef dispatch", () => {
    const at = app.indexOf('menuActionRef.current = (payload: string) => {')
    expect(at).toBeGreaterThan(0)
    expect(app.slice(at, at + 900)).toContain('if (!actionAllowed(payload)) return')
  })
  it("sends every write to App's activeTool through the wall", () => {
    expect(app).toContain('usableTool(typeof next ===')
    expect(app.match(/setActiveToolState\(/g)?.length).toBe(1)
  })
  it("resolves the Viewport's tool switch through the wall before anything else runs", () => {
    const at = viewport.indexOf('switchToolRef.current = (requestedTool: string) => {')
    expect(at).toBeGreaterThan(0)
    expect(viewport.slice(at, at + 700)).toContain('const toolName = usableTool(requestedTool)')
  })
  it('starts the plugin host only through loaderRoots, in a preview build, while the Plugins switch is on', () => {
    // App.tsx asks the lazy export (null in a production build) and the
    // switch before it loads anything; nothing else names the host's entry.
    expect(app).toContain("loadPluginHost === null || !loaderEnabled('plugins')) return")
    const starters = sourceFiles(SRC)
      .filter((f) => !f.includes(`${SRC}/plugins/`))
      .filter((f) => /startPluginHost|plugins\/host/.test(readFileSync(f, 'utf8')))
      .map((f) => f.slice(SRC.length + 1))
    expect(starters).toEqual([])
    const roots = readFileSync(join(HERE, 'loaderRoots.ts'), 'utf8')
    expect(roots).toMatch(/export const loadPluginHost:[^\n]*= __HEW_PREVIEW__\n\s*\? \(\) => import\('\.\.\/plugins\/host\.ts'\)/)
    // The test harness reaches the host only through what the host attaches.
    expect(readFileSync(join(SRC, 'test/harness.ts'), 'utf8')).not.toMatch(/from\s+['"][./]+\/plugins\//)
  })
  it('starts the Shop Mode plugin host only through loaderRoots, in a preview build, while the Plugins switch is on', () => {
    // Shop reaches plugin code through the one loader root and nothing else:
    // no file of `shop/` names a plugin module or the host's entry.
    const shop = sourceFiles(join(SRC, 'shop'))
    expect(shop.length).toBeGreaterThan(0)
    for (const f of shop) {
      const text = readFileSync(f, 'utf8')
      expect(text, f).not.toMatch(/(from|import\()\s*['"`][./]+\/plugins(\/|['"`])/)
      expect(text, f).not.toMatch(/startPluginHost|plugins\/host/)
    }
    const hook = readFileSync(join(SRC, 'shop/usePluginHost.ts'), 'utf8')
    expect(hook).toContain("import { loadPluginHost } from '../preview/loaderRoots'")
    expect(hook).toContain("loadPluginHost === null || !loaderEnabled('plugins')")
    expect(hook).toContain("platform: 'shop'")
    // The Shop window asks the hook; nothing else in Shop starts a host.
    const users = shop.filter((f) => /loadPluginHost/.test(readFileSync(f, 'utf8'))).map((f) => f.slice(SRC.length + 1))
    expect(users).toEqual(['shop/usePluginHost.ts'])
    expect(readFileSync(join(SRC, 'shop/ShopApp.tsx'), 'utf8')).toContain('useShopPluginHost({')
  })
  it('has no plugin host to load in a production build', async () => {
    const roots = await import('./loaderRoots')
    expect(roots.loadPluginHost === null).toBe(!(globalThis as { __HEW_PREVIEW__?: boolean }).__HEW_PREVIEW__)
  })
  it('lets the live bridge ask the forward router only where the registry turns plugin-forward on', () => {
    const bridge = readFileSync(join(SRC, 'api/liveBridge.ts'), 'utf8')
    const reads = [...bridge.matchAll(/forwardRouter\(\)/g)]
    expect(reads).toHaveLength(1)
    expect(bridge).toContain("behaviorEnabled('plugin-forward') ? forwardRouter() : null")
    // Only the plugin host fills the slot.
    const fillers = sourceFiles(SRC)
      .filter((f) => /setForwardRouter\(/.test(readFileSync(f, 'utf8')))
      .map((f) => f.slice(SRC.length + 1))
    expect(fillers.filter((f) => f !== 'api/forwardRouter.ts')).toEqual(['plugins/host.ts'])
  })
  it('does not read the build flag anywhere but the one tiny module', () => {
    const offenders = sourceFiles(SRC)
      .filter((f) => !f.endsWith('src/test/previewFlag.ts') && !f.endsWith('src/vite-env.d.ts'))
      .filter((f) => /__HEW_PREVIEW__/.test(readFileSync(f, 'utf8')))
      .map((f) => f.slice(SRC.length + 1))
    expect(offenders).toEqual([])
    expect(readFileSync(join(HERE, 'build.ts'), 'utf8').match(/__HEW_PREVIEW__/g)?.length).toBeGreaterThan(0)
    // Inside the wall's own folder, only build.ts and the lazy-import table read it.
    const inWall = readdirSync(HERE).filter((n) => /\.tsx?$/.test(n) && !/\.test\.tsx?$/.test(n) && /__HEW_PREVIEW__/.test(readFileSync(join(HERE, n), 'utf8')))
    expect(inWall.sort()).toEqual(['build.ts', 'loaderRoots.ts'])
  })
  it('has no run-time way to set the flag (no URL parameter, storage key or global setter)', () => {
    const wallSources = readdirSync(HERE).filter((n) => /\.tsx?$/.test(n) && !n.endsWith('.test.ts'))
    for (const name of wallSources) {
      const text = readFileSync(join(HERE, name), 'utf8')
      expect(text, name).not.toMatch(/location\.(search|hash)|URLSearchParams|window\.__|globalThis\.__/)
    }
    // The only storage the wall touches is the per-feature switch list.
    const keys = wallSources.flatMap((n) => [...readFileSync(join(HERE, n), 'utf8').matchAll(/'(hew\.[A-Za-z.]+)'/g)].map((m) => m[1]))
    expect([...new Set(keys)]).toEqual(['hew.settings.preview'])
  })
})

describe('native menu chains in a production build (derived from the registry)', () => {
  const main = readFileSync(join(ROOT, 'shells/tauri/src-tauri/src/main.rs'), 'utf8')
  const gatedActions = new Set(FEATURES.flatMap((f) => f.actions))
  // Smooth > Off is not in the registry but is not offered either (see actionOffered).
  const offered = (action: string): boolean => !gatedActions.has(action) && action !== 'smooth-off'
  const SUBMENUS = ['submenu:tools', 'submenu:draw', 'submenu:object', 'submenu:view']

  for (const id of SUBMENUS) {
    it(`${id}: no withheld command, no empty submenu, no stray separator`, () => {
      const full = parseSubmenu(main, id)
      const pruned = pruneChain(full, offered)
      const inPruned = chainItems(pruned).map((i) => i.action)
      for (const a of gatedActions) expect(inPruned, a).not.toContain(a)
      expect(inPruned).not.toContain('smooth-off')
      // Everything not withheld survives, in order.
      expect(inPruned).toEqual(chainItems(full).map((i) => i.action).filter(offered))
      const text = chainText(pruned)
      expect(text).not.toMatch(/\[submenu:[a-z-]+: \]/)
      expect(text).not.toMatch(/-, -/)
      if (process.env.HEW_WALL_PRINT === '1') console.log(`${id}\n  on : ${chainText(full)}\n  off: ${text}`)
    })
  }

  it('the Tools chain matches the web Tools menu once the wall is up (same commands, same order)', async () => {
    const app = await loadApp(false)
    const web = menuActionIdsOf(app.menuModel.TOOLS_MENU, app)
    const native = chainItems(pruneChain(parseSubmenu(main, 'submenu:tools'), offered)).map((i) => i.action)
    expect(native).toEqual(web)
    vi.doUnmock('./build')
    vi.resetModules()
  })

  it('every id the dispatch forwards for a withheld command is gone from the derived chains', () => {
    const forwarded = [...dispatchMap(main)].filter(([, action]) => gatedActions.has(action)).map(([native]) => native)
    expect(forwarded.length).toBeGreaterThan(0)
    const all = SUBMENUS.flatMap((id) => chainItems(pruneChain(parseSubmenu(main, id), offered)).map((i) => i.native))
    for (const native of forwarded) expect(all, native).not.toContain(native)
  })
})

describe('the desktop wrapper resolves one flavor for the shell, the page and the policy', () => {
  const wrapper = join(ROOT, 'shells/tauri/scripts/tauri-policy.mjs')
  const dry = (command: string, hewPreview: string | undefined) => {
    const env = { ...process.env }
    delete env.HEW_PREVIEW
    if (hewPreview !== undefined) env.HEW_PREVIEW = hewPreview
    return spawnSync(process.execPath, [wrapper, '--print-args', command, '--debug'], { encoding: 'utf8', env })
  }
  const table: [string | undefined, 'dev' | 'build', boolean][] = [
    [undefined, 'dev', true],
    ['', 'dev', true],
    ['1', 'dev', true],
    ['0', 'dev', false],
    [undefined, 'build', false],
    ['', 'build', false],
    ['1', 'build', true],
    ['0', 'build', false],
  ]
  for (const [value, command, preview] of table) {
    it(`${command} with HEW_PREVIEW=${value === undefined ? '(unset)' : JSON.stringify(value)} is a ${preview ? 'preview' : 'production'} build`, () => {
      const r = dry(command, value)
      expect(r.status, r.stderr).toBe(0)
      const plan = JSON.parse(r.stdout) as { preview: boolean; hewPreview: string; overlay: string | null; tauriArgs: string[] }
      expect(plan.preview).toBe(preview)
      // The Rust shell and the page get the same answer the overlay was chosen by.
      expect(plan.hewPreview).toBe(preview ? '1' : '0')
      expect(plan.tauriArgs.slice(0, 2)).toEqual([command, '--debug'])
      const configs = plan.tauriArgs.filter((a) => a === '--config')
      expect(configs.length).toBe(preview ? 1 : 0)
      expect(plan.overlay === null).toBe(!preview)
    })
  }
  it('puts the overlay before a `--`, among the CLI\'s own options', () => {
    const env = { ...process.env, HEW_PREVIEW: '1' }
    const r = spawnSync(process.execPath, [wrapper, '--print-args', 'build', '--debug', '--', '--features', 'x'], { encoding: 'utf8', env })
    expect(r.status, r.stderr).toBe(0)
    const plan = JSON.parse(r.stdout) as { tauriArgs: string[] }
    expect(plan.tauriArgs.slice(0, 4)).toEqual(['build', '--debug', '--config', plan.tauriArgs[3]])
    expect(plan.tauriArgs.slice(4)).toEqual(['--', '--features', 'x'])
  })
  it('skips links, dangling ones included, when it looks for a desktop binary', () => {
    const dir = join(ROOT, 'target', `wall-test-binary-${process.pid}`)
    try {
      mkdirSync(join(dir, 'release'), { recursive: true })
      symlinkSync(join(dir, 'nowhere'), join(dir, 'release', 'dangling'))
      const r = spawnSync(process.execPath, [join(ROOT, 'scripts/check-desktop-binary.mjs'), '--want', 'preview', '--target', dir], { encoding: 'utf8' })
      // Nothing to check is a failure of its own, not a crash on the link.
      expect(r.status, r.stderr).toBe(1)
      expect(r.stderr).toContain('no hew binary')
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
  it('refuses a value that is neither 1, 0 nor empty', () => {
    for (const command of ['dev', 'build']) {
      const r = dry(command, 'garbage')
      expect(r.status).toBe(2)
      expect(r.stderr).toContain('HEW_PREVIEW must be 1')
    }
  })
  it('writes an overlay whose policy allows the realm boot script, and production allows none', () => {
    const hash = readFileSync(join(ROOT, 'app/src/plugins/boot/boot-1.hash'), 'utf8').trim()
    const out = join(ROOT, 'target', `wall-test-overlay-${process.pid}.json`)
    try {
      execFileSync(process.execPath, [join(ROOT, 'shells/csp/tauri-overlay.mjs'), 'preview', out], { stdio: 'ignore' })
      const csp = (JSON.parse(readFileSync(out, 'utf8')) as { app: { security: { csp: string } } }).app.security.csp
      expect(csp).toContain(`'sha256-${hash}'`)
    } finally {
      rmSync(out, { force: true })
    }
    const conf = readFileSync(join(ROOT, 'shells/tauri/src-tauri/tauri.conf.json'), 'utf8')
    expect(conf).not.toContain(hash)
  })
})
