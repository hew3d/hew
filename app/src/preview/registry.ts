/**
 * The Preview registry as the app sees it: `preview-features.json` at the
 * repository root, shared with the Rust side (docs/design/preview-channel.md
 * section 3). This is the only app-side file that imports it.
 *
 * A production build strips each entry down to the ids the wall needs
 * (vite.config.ts), so the feature names and state lines are not in its
 * bundle; only preview-only code reads `name` and `state`.
 */

import registry from '../../../preview-features.json'

export interface PreviewFeature {
  readonly id: string
  /** English only: the Settings section is exempt from localization. */
  readonly name: string
  /** English only, one line. */
  readonly state: string
  readonly owner: string
  /** `ToolName`s the feature gates. */
  readonly tools: readonly string[]
  /** Command ids (menu items, palette entries, keymap defaults, dispatch). */
  readonly actions: readonly string[]
  /** Selection-dock chip ids. */
  readonly dockChips: readonly string[]
  /** API command names (the Rust side enforces these). */
  readonly apiCommands: readonly string[]
  /** Non-list couplings, named in code as `behaviorEnabled('<id>')`. */
  readonly behaviors: readonly string[]
  /** The switch's state before the user touches it; absent means on. */
  readonly defaultOn?: boolean
  /** Entry points of a feature that loads and runs code. A production build
   *  strips this whole object: it has none of them to ask about. */
  readonly loader?: PreviewLoader
}

/** What a code-loading feature gates beyond the lookup kinds. The app asks
 *  about `roots` and `panes`; the rest are enforced by the Rust build and by
 *  `scripts/check-production-artifacts.mjs`. */
export interface PreviewLoader {
  /** Window roots selected by location hash (`main.tsx`). */
  readonly roots: readonly string[]
  /** Settings panes. */
  readonly panes: readonly string[]
  readonly shellCommands: readonly string[]
  readonly kernelExports: readonly string[]
  readonly cliCommands: readonly string[]
  readonly storage: readonly string[]
  readonly markers: readonly string[]
}

export const PREVIEW_FEATURES: readonly PreviewFeature[] = registry.features as readonly PreviewFeature[]
