/**
 * docFormat — pure helpers for the document formats a file can be bound to
 * (`DocFormat`, fileHost.ts): file extensions, names, and the labels the
 * Save As format dialog shows. UI-free and wasm-free so it unit-tests
 * without a DOM.
 */

import { t } from '@lingui/core/macro'
import type { DocFormat } from './fileHost'

/**
 * The SketchUp stream releases a `.skp` is written in, newest first: the
 * wire names (`skp2020` is SketchUp 2020, `skp8` SketchUp 8). The one table
 * every picker, label and check reads; a new release is one line here (and
 * its row in `crates/api`'s `SKP_STREAM_RELEASES`).
 */
export const SKP_STREAM_FORMATS = [
  'skp2020',
  'skp2019',
  'skp2018',
  'skp2017',
  'skp2016',
  'skp2015',
  'skp2014',
  'skp2013',
  'skp8',
  'skp7',
  'skp6',
  'skp5',
  'skp4',
  'skp3',
] as const

/** One of the stream releases of `SKP_STREAM_FORMATS`. */
export type SkpStreamFormat = (typeof SKP_STREAM_FORMATS)[number]

/** Every extension Hew opens or imports; stripped from a name before a new
 * one is appended (`Model.skp` suggests `Model`, never `Model.skp.hew`). */
const KNOWN_EXTENSION = /\.(hew|skp|dae|stl|glb|gltf)$/i

/** `Model.skp` -> `Model`. Only one trailing known extension is removed. */
export function stripKnownExtension(name: string): string {
  return name.replace(KNOWN_EXTENSION, '')
}

/** True for the two SketchUp document formats. */
export function isSkpFormat(format: DocFormat): boolean {
  return format !== 'hew'
}

/** File extension (with the dot) a format is written with. */
export function extensionFor(format: DocFormat): '.hew' | '.skp' {
  return format === 'hew' ? '.hew' : '.skp'
}

/** `name` with its known extension replaced by the format's. */
export function nameForFormat(name: string, format: DocFormat): string {
  return stripKnownExtension(name) + extensionFor(format)
}

/** The `Scene.save_skp` / `export_skp` target for a SketchUp format. */
export type SkpTarget = 'skp2026' | SkpStreamFormat

/** Whether `value` names one of the stream releases (`skp2017`, `skp8`). */
export function isSkpStreamFormat(value: unknown): value is SkpStreamFormat {
  return (SKP_STREAM_FORMATS as readonly unknown[]).includes(value)
}

/** The release a stream format names: `2017` for `skp2017`, `8` for `skp8`. */
export function skpStreamYear(format: SkpStreamFormat): string {
  return format.slice('skp'.length)
}

export function skpTarget(format: DocFormat): SkpTarget {
  return isSkpStreamFormat(format) ? format : 'skp2026'
}

/** Normalize a persisted or externally supplied format value; anything
 * unrecognized (including a missing field from an older record) is `hew`. */
export function normalizeDocFormat(value: unknown): DocFormat {
  return value === 'skp2026' || isSkpStreamFormat(value) ? value : 'hew'
}

/** Picker filter description for a format. */
/**
 * The format a save to `path` writes: the API's `format` when it names one
 * (`"hew"`, `"skp"` for the 2026 container, or a stream release: `"skp3"` to
 * `"skp8"`, `"skp2013"` to `"skp2020"`), else the path's
 * extension — a `.skp` keeps the document's own SketchUp container (the
 * 2026 one for a document that did not come from a `.skp`), anything else
 * is `.hew`. `null` when a named format contradicts the extension: one
 * format's bytes never go under the other's.
 */
export function saveFormatForPath(path: string, apiFormat: unknown, current: DocFormat | undefined): DocFormat | null {
  const skpPath = /\.skp$/i.test(path)
  if (apiFormat === undefined || apiFormat === null) {
    if (!skpPath) return 'hew'
    return current !== undefined && isSkpFormat(current) ? current : 'skp2026'
  }
  const named: DocFormat | null =
    apiFormat === 'hew'
      ? 'hew'
      : apiFormat === 'skp'
        ? 'skp2026'
        : isSkpStreamFormat(apiFormat)
          ? apiFormat
          : null
  if (named === null || isSkpFormat(named) !== skpPath) return null
  return named
}

export function formatDescription(format: DocFormat): string {
  if (format === 'skp2026') return t`SketchUp model`
  if (isSkpStreamFormat(format)) {
    const release = skpStreamYear(format)
    return t`SketchUp ${release} model`
  }
  return t`Hew model`
}

export interface SaveFormatChoice {
  format: DocFormat
  label: string
  hint: string
  /** `older`: one of the stream releases, which the pickers gather under one
   * "Older SketchUp versions" entry instead of listing each beside the rest. */
  group?: 'older'
}

/** The id of the single picker entry that stands for the older releases in the
 * desktop panel (its `release` option carries which). Never a `DocFormat`. */
export const SKP_OLDER_PICKER_ID = 'skp-older'

/** The picker entry that gathers the stream releases. */
export function olderGroupLabel(): string {
  return t`Older SketchUp versions (.skp)`
}

/** The group heading of the stream releases in a list. */
export function olderGroupHeading(): string {
  return t`Older SketchUp versions`
}

/** The checkbox that writes the 2021-and-later container at its widest
 * compatibility (revision 17). Shown for that container only. */
export function compatLabel(): string {
  return t`Avoid features older SketchUp versions cannot open`
}

/** Whether a compat choice applies to `format` (the 2021-and-later
 * container only). */
export function takesCompat(format: DocFormat): boolean {
  return format === 'skp2026'
}

/** The stream release a picker answer names (`release` option of
 * {@link SKP_OLDER_PICKER_ID}); the newest release for anything unknown. */
export function olderReleaseFrom(value: unknown): SkpStreamFormat {
  return isSkpStreamFormat(value) ? value : SKP_STREAM_FORMATS[0]
}

/** The older release a picker preselects: the file's own when it is one. */
export function olderReleaseFor(current: DocFormat | undefined): SkpStreamFormat {
  return current !== undefined && isSkpStreamFormat(current) ? current : SKP_STREAM_FORMATS[0]
}

/** The options of the Save As format dialog, in display order. */
export const SAVE_FORMAT_CHOICES: readonly SaveFormatChoice[] = [
  // `label` and `hint` are getters so the text resolves in the active locale
  // at read time, not at module load.
  {
    format: 'hew',
    get label() {
      return t`Hew document (.hew)`
    },
    get hint() {
      return t`Keeps everything Hew stores.`
    },
  },
  {
    format: 'skp2026',
    get label() {
      return t`SketchUp 2021 and later (.skp)`
    },
    get hint() {
      return t`The current SketchUp file format.`
    },
  },
  // The 2021-and-later format first, then the older group: every stream
  // release newest to oldest.
  ...SKP_STREAM_FORMATS.map(
    (format): SaveFormatChoice => ({
      format,
      group: 'older',
      get label() {
        const release = skpStreamYear(format)
        return t`SketchUp ${release} (.skp)`
      },
      get hint() {
        const release = skpStreamYear(format)
        return t`Opens in SketchUp ${release} and later.`
      },
    }),
  ),
]
