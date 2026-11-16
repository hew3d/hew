/**
 * The window roots of code-loading Preview features (`loader.roots` in
 * `preview-features.json`), as lazy components, or `null` in a production
 * build.
 *
 * This is the second file that reads the build literal itself, and the
 * reason is the bundler: a dynamic `import()` behind a constant imported
 * from another module (`previewBuild`) still gets its chunk emitted, so the
 * code would ship in a production build even though nothing could reach it.
 * Behind the literal in the same module, the import is removed before
 * chunks are made, and `scripts/check-production-artifacts.mjs` fails the
 * gate if it ever is not. Every dynamic import of preview-only code goes
 * here, each on its own `__HEW_PREVIEW__ ?` line.
 *
 * Whether a root may RENDER is still the wall's answer (`rootAllowed`),
 * asked in `main.tsx`.
 */

import { lazy, type ComponentType, type LazyExoticComponent } from 'react'

type Root = LazyExoticComponent<ComponentType> | null

export const ProbeRoot: Root = __HEW_PREVIEW__
  ? lazy(() => import('../plugins/probe/ProbeRoot.tsx').then((m) => ({ default: m.ProbeRoot })))
  : null

export const PluginsPane: Root = __HEW_PREVIEW__
  ? lazy(() => import('../plugins/PluginsPane.tsx').then((m) => ({ default: m.PluginsPane })))
  : null
