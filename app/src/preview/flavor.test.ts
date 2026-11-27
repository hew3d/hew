import { describe, expect, it } from 'vitest'
import { kernelFlavorProblem } from './flavor'

describe('kernelFlavorProblem', () => {
  it('accepts a module of the same flavor', () => {
    expect(kernelFlavorProblem(true, true)).toBeNull()
    expect(kernelFlavorProblem(false, false)).toBeNull()
  })
  it('refuses a production page on a preview module (the live API would reach Preview commands)', () => {
    expect(kernelFlavorProblem(false, true)).toMatch(/production build.*preview kernel/)
  })
  it('refuses a preview page on a production module', () => {
    expect(kernelFlavorProblem(true, false)).toMatch(/preview build.*production kernel/)
  })
  it('has nothing to compare against a module built before the export existed', () => {
    expect(kernelFlavorProblem(false, undefined)).toBeNull()
    expect(kernelFlavorProblem(true, undefined)).toBeNull()
  })
})
