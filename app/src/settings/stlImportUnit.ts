/**
 * STL import unit-scale setting — module-level singleton, session-scoped only.
 *
 * STL files carry no unit information: the
 * near-universal maker convention is millimeters, but a file could just as
 * easily be centimeters, inches, or meters. The units-chooser modal
 * (`StlUnitsDialog`) asks on every `.stl` import; this module remembers the
 * user's last answer so re-importing several STLs in one session doesn't
 * repeat the same click, WITHOUT persisting across app restarts — unlike the
 * display length format in `units.ts`, there is no single "correct" default
 * per user, only per file, so carrying a stale choice across sessions would
 * be more likely to silently mis-scale a later import than to help.
 */

/** One STL import unit choice: label + meters-per-STL-unit conversion. */
export interface StlUnitOption {
  value: 'mm' | 'cm' | 'in' | 'm'
  label: string
  /** Meters per one STL file unit. */
  unitScale: number
}

/** Offered in this order — millimeters first and marked default (DESIGN §5). */
export const STL_UNIT_OPTIONS: StlUnitOption[] = [
  { value: 'mm', label: 'Millimeters', unitScale: 0.001 },
  { value: 'cm', label: 'Centimeters', unitScale: 0.01 },
  { value: 'in', label: 'Inches', unitScale: 0.0254 },
  { value: 'm', label: 'Meters', unitScale: 1.0 },
]

const DEFAULT_UNIT: StlUnitOption['value'] = 'mm'

let lastChoice: StlUnitOption['value'] = DEFAULT_UNIT

/** The unit to preselect in the chooser (the last choice made this session). */
export function getLastStlImportUnit(): StlUnitOption['value'] {
  return lastChoice
}

/** Record the user's choice for the rest of this session. */
export function setLastStlImportUnit(value: StlUnitOption['value']): void {
  lastChoice = value
}

/** Reset to the default — exposed for tests only. */
export function resetStlImportUnitForTest(): void {
  lastChoice = DEFAULT_UNIT
}
