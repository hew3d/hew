/**
 * The desktop save panel with the format choice in it (the shell's
 * `pick_save_target`, `shells/tauri/src-tauri/src/save_panel.rs`): macOS's
 * "Format:" popup, Windows' "Save as type" list, GTK's file-type filter,
 * with the chosen format's options beside it — the convention every
 * desktop app follows, SketchUp's own Save As and Export included.
 *
 * The browser has no such panel (Chrome's picker never says which type
 * was picked, and Safari/Firefox only download), so there the in-app
 * Save As format step and Export dialog stay. Both read their formats and
 * options from here, so the two offer exactly the same things.
 */

import { t } from '@lingui/core/macro'
import { isTauri, type DocFormat } from './fileHost'
import {
  SAVE_FORMAT_CHOICES,
  SKP_OLDER_PICKER_ID,
  SKP_STREAM_FORMATS,
  compatLabel,
  extensionFor,
  isSkpStreamFormat,
  normalizeDocFormat,
  olderGroupLabel,
  olderReleaseFor,
  olderReleaseFrom,
  skpStreamYear,
  takesCompat,
  type SkpStreamFormat,
} from './docFormat'
import { VIEW_LABEL, type PrintViewKind } from '../print/printJob'
import { scaleDisplay, scaleHint, scalePresetsFor, type PrintScale } from '../print/scale'
import type { LengthFormat } from '../settings/units'

export interface PanelChoice {
  value: string
  label: string
}

/** One option control the panel shows for a format. */
export type PanelOption =
  | { kind: 'select'; id: string; label: string; choices: PanelChoice[]; value: string }
  | { kind: 'check'; id: string; label: string; value: boolean }

/** One entry of the panel's format list. */
export interface PanelFormat {
  id: string
  label: string
  /** Without the dot. */
  extension: string
  options?: PanelOption[]
}

/** What the user confirmed in the panel. */
export interface PanelResult {
  path: string
  format: string
  options: Record<string, string | boolean>
}

/** Whether this host shows the native panel (every desktop build). */
export function hasNativeSavePanel(): boolean {
  return isTauri
}

/** Show the native panel; `null` when the user canceled. */
export async function pickSaveTarget(
  title: string,
  defaultName: string,
  formats: PanelFormat[],
  initial: string,
): Promise<PanelResult | null> {
  const { invoke } = await import('@tauri-apps/api/core')
  return invoke<PanelResult | null>('pick_save_target', { title, defaultName, formats, initial })
}

// ------------------------------------------------------------------ Save As

/** The picker entry that stands for the older releases: one list entry whose
 * `release` option carries which, preselected to `release`. */
function olderPanelFormat(release: SkpStreamFormat): PanelFormat {
  return {
    id: SKP_OLDER_PICKER_ID,
    label: olderGroupLabel(),
    extension: 'skp',
    options: [
      {
        kind: 'select',
        id: 'release',
        label: t`Release`,
        choices: SKP_STREAM_FORMATS.map((f) => {
          const year = skpStreamYear(f)
          return { value: f, label: t`SketchUp ${year}` }
        }),
        value: release,
      },
    ],
  }
}

/** The compat checkbox of the 2021-and-later container. */
function compatOption(value: boolean): PanelOption {
  return { kind: 'check', id: 'compat', label: compatLabel(), value }
}

/** The panel's entry id for a document format: the older releases share one. */
export function panelIdFor(format: DocFormat): string {
  return isSkpStreamFormat(format) ? SKP_OLDER_PICKER_ID : format
}

/**
 * Save As's formats, as the in-app step lists them: Hew, the 2021-and-later
 * container (with its compat checkbox), then one "Older SketchUp versions"
 * entry that lists the releases — preselected to the file's own, `current`.
 */
export function savePanelFormats(current?: DocFormat, compat = false): PanelFormat[] {
  return [
    ...SAVE_FORMAT_CHOICES.filter((c) => c.group === undefined).map(
      (c): PanelFormat => ({
        id: c.format,
        label: c.label,
        extension: extensionFor(c.format).slice(1),
        options: takesCompat(c.format) ? [compatOption(compat)] : [],
      }),
    ),
    olderPanelFormat(olderReleaseFor(current)),
  ]
}

// ------------------------------------------------------------------- Export

/** `skp` is the SketchUp 2021-and-later container; the stream releases (`SKP_STREAM_FORMATS`) are those releases' own formats. */
export type ExportFormat = 'glb' | 'stl' | '3mf' | 'usdz' | 'svg' | 'skp' | SkpStreamFormat

/** The SketchUp export formats. */
export type SkpExportFormat = Extract<ExportFormat, `skp${string}`>

/** Whether an export format writes a `.skp`. */
export function isSkpExportFormat(format: ExportFormat): format is SkpExportFormat {
  return format.startsWith('skp')
}

/** SVG line-drawing options (docs/design/printing.md §7b). */
export interface SvgExportOptions {
  view: PrintViewKind
  scale: PrintScale
  hiddenDashed: boolean
  includeDimensions: boolean
}

/** Export's formats: the panel's labels (the in-app dialog adds a hint). */
export const EXPORT_FORMATS: {
  format: ExportFormat
  label: string
  extension: string
  hint: string
  /** `older`: a stream release, listed under "Older SketchUp versions". */
  group?: 'older'
}[] = [
  {
    format: 'glb',
    get label() {
      return t`glTF binary (.glb)`
    },
    extension: 'glb',
    get hint() {
      return t`Y-up, meters`
    },
  },
  {
    format: 'stl',
    get label() {
      return t`STL binary (.stl)`
    },
    extension: 'stl',
    get hint() {
      return t`Z-up, millimeters, for 3D printing`
    },
  },
  {
    format: '3mf',
    get label() {
      return t`3MF (.3mf)`
    },
    extension: '3mf',
    get hint() {
      return t`Z-up, millimeters, keeps part names and colors`
    },
  },
  {
    format: 'usdz',
    get label() {
      return t`USDZ (.usdz)`
    },
    extension: 'usdz',
    get hint() {
      return t`Y-up, meters, for AR Quick Look and USD pipelines`
    },
  },
  {
    format: 'svg',
    get label() {
      return t`SVG line drawing (.svg)`
    },
    extension: 'svg',
    get hint() {
      return t`hidden lines removed, true size at a drawing scale`
    },
  },
  {
    format: 'skp',
    get label() {
      return t`SketchUp 2021 and later (.skp)`
    },
    extension: 'skp',
    hint: '',
  },
  // Every stream release, newest to oldest, after the 2021-and-later container.
  ...SKP_STREAM_FORMATS.map((format) => ({
    format,
    group: 'older' as const,
    get label() {
      const release = skpStreamYear(format)
      return t`SketchUp ${release} (.skp)`
    },
    extension: 'skp',
    get hint() {
      const release = skpStreamYear(format)
      return t`opens in SketchUp ${release} and later`
    },
  })),
]

/** STL curve-resolution choices: segments per full turn (0 = stored facets). */
export const STL_RESOLUTIONS: { value: number; label: string }[] = [

  {
    value: 0,
    get label() {
      return t`As modeled (stored facets)`
    },
  },
  {
    value: 24,
    get label() {
      return t`Draft (24 segments per turn)`
    },
  },
  {
    value: 48,
    get label() {
      return t`Standard (48 segments per turn)`
    },
  },
  {
    value: 96,
    get label() {
      return t`Fine (96 segments per turn)`
    },
  },
  {
    value: 192,
    get label() {
      return t`Ultra (192 segments per turn)`
    },
  },
]

/** The STL resolution an export starts on. */
export const DEFAULT_STL_SEGMENTS = 48

/** The SVG scale an export starts on: 1:1 (laser/CNC read true size). */
export function defaultSvgScaleIndex(presets: PrintScale[]): number {
  return Math.max(0, presets.findIndex((p) => Math.abs(p.paperMeters / p.modelMeters - 1) < 1e-9))
}

/** A scale preset's label, with its metric hint when it has one. */
export function svgScaleLabel(p: PrintScale, unit: LengthFormat): string {
  const hint = scaleHint(p, unit)
  return hint === null ? scaleDisplay(p) : `${p.label}  (${hint})`
}

/** Export's formats for the native panel, each with its options. */
export function exportPanelFormats(unit: LengthFormat, current?: ExportFormat, compat = false): PanelFormat[] {
  const presets = scalePresetsFor(unit)
  const options: Partial<Record<ExportFormat, PanelOption[]>> = {
    skp: [compatOption(compat)],
    stl: [
      {
        kind: 'select',
        id: 'segments',
        label: t`Curve resolution`,
        choices: STL_RESOLUTIONS.map((r) => ({ value: String(r.value), label: r.label })),
        value: String(DEFAULT_STL_SEGMENTS),
      },
    ],
    svg: [
      {
        kind: 'select',
        id: 'view',
        label: t`View`,
        choices: (Object.keys(VIEW_LABEL) as PrintViewKind[]).map((v) => ({ value: v, label: VIEW_LABEL[v] })),
        value: 'current',
      },
      {
        kind: 'select',
        id: 'scale',
        label: t`Scale`,
        choices: presets.map((p, i) => ({ value: String(i), label: svgScaleLabel(p, unit) })),
        value: String(defaultSvgScaleIndex(presets)),
      },
      { kind: 'check', id: 'hidden', label: t`Hidden lines dashed`, value: false },
      { kind: 'check', id: 'dimensions', label: t`Dimensions & text`, value: true },
    ],
  }
  return [
    ...EXPORT_FORMATS.filter((f) => f.group === undefined).map((f) => ({
      id: f.format as string,
      label: f.label,
      extension: f.extension,
      options: options[f.format] ?? [],
    })),
    olderPanelFormat(olderReleaseFor(current !== undefined && isSkpStreamFormat(current) ? current : undefined)),
  ]
}

/** The panel entry an export format is listed under. */
export function exportPanelIdFor(format: ExportFormat): string {
  return isSkpStreamFormat(format) ? SKP_OLDER_PICKER_ID : format
}

/** What an export does, read back from the panel's answer. */
export interface ExportChoice {
  format: ExportFormat
  /** `skp` only: write revision 17 of the container. */
  skpCompat: boolean
  stlSegmentsPerTurn: number
  svg?: SvgExportOptions
}

/** Turn the panel's answer into an export; `null` for an unknown format. */
export function exportChoiceFrom(result: PanelResult, unit: LengthFormat): ExportChoice | null {
  const o = result.options
  const format =
    result.format === SKP_OLDER_PICKER_ID
      ? olderReleaseFrom(o.release)
      : EXPORT_FORMATS.find((f) => f.format === result.format && f.group === undefined)?.format
  if (format === undefined) return null
  const segments = Number(o.segments ?? DEFAULT_STL_SEGMENTS)
  const choice: ExportChoice = {
    format,
    skpCompat: format === 'skp' && o.compat === true,
    stlSegmentsPerTurn: Number.isFinite(segments) ? segments : DEFAULT_STL_SEGMENTS,
  }
  if (format === 'svg') {
    const presets = scalePresetsFor(unit)
    const view = (typeof o.view === 'string' && o.view in VIEW_LABEL ? o.view : 'current') as PrintViewKind
    const at = Number(o.scale)
    const scale = presets[Number.isInteger(at) && at >= 0 && at < presets.length ? at : defaultSvgScaleIndex(presets)]
    choice.svg = {
      view,
      scale: scale as PrintScale,
      hiddenDashed: o.hidden === true,
      includeDimensions: o.dimensions !== false,
    }
  }
  return choice
}

/** Save As's format from the panel's answer (anything else is `.hew`). */
export function saveFormatFrom(result: PanelResult): DocFormat {
  if (result.format === SKP_OLDER_PICKER_ID) return olderReleaseFrom(result.options.release)
  return normalizeDocFormat(result.format)
}

/** Whether the panel's answer asked for revision 17 of the 2021-and-later container. */
export function saveCompatFrom(result: PanelResult): boolean {
  return takesCompat(saveFormatFrom(result)) && result.options.compat === true
}
