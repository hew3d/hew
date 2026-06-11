import { describe, it, expect } from 'vitest'

function add(a: number, b: number): number {
  return a + b
}

describe('smoke test', () => {
  it('should add two numbers', () => {
    expect(add(2, 3)).toBe(5)
  })
})
