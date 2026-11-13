import { describe, expect, it } from 'vitest'
import type { NodeRef } from '../panels/treeModel'
import { OP_INTERSECT, OP_SUBTRACT, OP_UNION, routeBoolean, routeBooleanMany } from './placementBoolean'

const obj = (id: bigint): NodeRef => ({ kind: 'object', id })
const grp = (id: bigint): NodeRef => ({ kind: 'group', id })
const ins = (id: bigint): NodeRef => ({ kind: 'instance', id })
/** Instances 10 and 11 are copies of a linked set; 20 is an ordinary component. */
const isLinked = (i: bigint): boolean => i === 10n || i === 11n

describe('routeBooleanMany', () => {
  it('answers_two_operands_as_the_two_operand_route', () => {
    const pairs: [NodeRef, NodeRef][] = [
      [obj(1n), obj(2n)],
      [ins(10n), obj(2n)],
      [ins(10n), grp(3n)],
      [obj(1n), ins(10n)],
      [ins(20n), obj(2n)],
    ]
    for (const op of [OP_UNION, OP_SUBTRACT, OP_INTERSECT]) {
      for (const [a, b] of pairs) {
        expect(routeBooleanMany(op, [a, b], isLinked)).toBe(routeBoolean(op, a, b, isLinked))
      }
    }
  })

  it('plain_solids_and_ordinary_components_take_the_plain_route', () => {
    for (const op of [OP_UNION, OP_SUBTRACT, OP_INTERSECT]) {
      expect(routeBooleanMany(op, [obj(1n), obj(2n), obj(3n)], isLinked)).toBe('plain')
      expect(routeBooleanMany(op, [obj(1n), ins(20n), grp(3n)], isLinked)).toBe('plain')
    }
  })

  it('a_linked_host_is_cut_through_its_shape_by_several_loose_solids', () => {
    expect(routeBooleanMany(OP_SUBTRACT, [ins(10n), obj(1n), obj(2n)], isLinked)).toBe('through-placement')
  })

  it('a_linked_host_refuses_a_cutter_that_is_not_a_loose_solid', () => {
    expect(routeBooleanMany(OP_SUBTRACT, [ins(10n), obj(1n), grp(2n)], isLinked)).toBe('refuse-cutter')
    expect(routeBooleanMany(OP_SUBTRACT, [ins(10n), obj(1n), ins(11n)], isLinked)).toBe('refuse-cutter')
  })

  it('a_linked_copy_anywhere_else_is_unlinked_and_joined', () => {
    expect(routeBooleanMany(OP_UNION, [obj(1n), obj(2n), ins(10n)], isLinked)).toBe('unlink-and-join')
    expect(routeBooleanMany(OP_INTERSECT, [ins(10n), obj(1n), obj(2n)], isLinked)).toBe('unlink-and-join')
    expect(routeBooleanMany(OP_SUBTRACT, [obj(1n), obj(2n), ins(11n)], isLinked)).toBe('unlink-and-join')
  })
})
