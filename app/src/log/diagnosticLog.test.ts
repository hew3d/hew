import { describe, it, expect, beforeEach, vi } from 'vitest'
import * as LogStore from './LogStore'
import * as Diag from './diagnosticLog'

beforeEach(() => {
  LogStore.clear()
  Diag.unbridgeLogStore()
  Diag.clear()
  Diag.setFileLogging(false)
})

describe('diagnosticLog', () => {
  describe('ingestKernel', () => {
    it('parses a kernel LogRecord JSON string into a DiagRecord', () => {
      const json = JSON.stringify({
        seq: 41,
        corr: 7,
        level: 'INFO',
        target: 'kernel::op',
        fields: { op: 'extrude_region', distance: 2.0 },
      })
      Diag.ingestKernel(json)
      const recs = Diag.getRecords()
      expect(recs).toHaveLength(1)
      expect(recs[0].source).toBe('kernel')
      expect(recs[0].seq).toBe(41)
      expect(recs[0].corr).toBe(7)
      expect(recs[0].level).toBe('INFO')
      expect(recs[0].target).toBe('kernel::op')
      expect(recs[0].fields).toEqual({ op: 'extrude_region', distance: 2.0 })
      expect(typeof recs[0].id).toBe('number')
      expect(typeof recs[0].t).toBe('number')
    })

    it('handles a null corr (outside any gesture)', () => {
      Diag.ingestKernel(
        JSON.stringify({ seq: 1, corr: null, level: 'TRACE', target: 'inference::resolve', fields: {} }),
      )
      expect(Diag.getRecords()[0].corr).toBeNull()
    })

    it('drops malformed JSON rather than throwing', () => {
      expect(() => Diag.ingestKernel('not json')).not.toThrow()
      expect(Diag.getRecords()).toHaveLength(0)
    })

    it('assigns monotonically increasing sink ids across ingests', () => {
      Diag.ingestKernel(JSON.stringify({ seq: 1, corr: null, level: 'INFO', target: 't', fields: {} }))
      Diag.ingestKernel(JSON.stringify({ seq: 2, corr: null, level: 'INFO', target: 't', fields: {} }))
      const recs = Diag.getRecords()
      expect(recs[1].id).toBeGreaterThan(recs[0].id)
    })
  })

  describe('ring buffer bounding', () => {
    it('drops oldest records once over the cap', () => {
      // Use logUi (cheap, no JSON parse) to push many records quickly.
      const CAP = 50_000
      for (let i = 0; i < CAP + 10; i++) {
        Diag.logUi('test', 'INFO', { i })
      }
      const recs = Diag.getRecords()
      expect(recs).toHaveLength(CAP)
      // The oldest 10 were dropped; the first remaining record is i === 10.
      expect(recs[0].fields.i).toBe(10)
      expect(recs[recs.length - 1].fields.i).toBe(CAP + 9)
    })
  })

  describe('bridgeLogStore', () => {
    it('ingests a LogStore append as a ui record', () => {
      Diag.bridgeLogStore()
      LogStore.append('warn', 'tool', 'something happened')
      const recs = Diag.getRecords()
      expect(recs).toHaveLength(1)
      expect(recs[0].source).toBe('ui')
      expect(recs[0].seq).toBeNull()
      expect(recs[0].level).toBe('WARN')
      expect(recs[0].target).toBe('tool')
      expect(recs[0].fields).toEqual({ message: 'something happened' })
    })

    it('does not double-ingest on repeated notify (subscribe delivers full array)', () => {
      LogStore.append('info', 'src', 'pre-existing')
      Diag.bridgeLogStore() // subscribe() delivers the full current array immediately
      expect(Diag.getRecords()).toHaveLength(1)

      LogStore.append('info', 'src', 'second')
      // The second notify delivers [pre-existing, second]; only "second" is new.
      expect(Diag.getRecords()).toHaveLength(2)
      expect(Diag.getRecords()[1].fields.message).toBe('second')
    })

    it('is idempotent — calling it twice does not double-subscribe', () => {
      Diag.bridgeLogStore()
      Diag.bridgeLogStore()
      LogStore.append('info', 'src', 'once')
      expect(Diag.getRecords()).toHaveLength(1)
    })
  })

  describe('gesture correlation', () => {
    it('beginGesture sets the shared corr, picked up by subsequent ui records', () => {
      const fakeBegin = vi.fn(() => 99n)
      const id = Diag.beginGesture(fakeBegin)
      expect(id).toBe(99n)
      expect(Diag.getCurrentCorr()).toBe(99)

      Diag.logUi('tool', 'INFO', { message: 'drag start' })
      const recs = Diag.getRecords()
      expect(recs[0].corr).toBe(99)
    })

    it('endGesture clears the shared corr', () => {
      const fakeBegin = vi.fn(() => 5n)
      const fakeEnd = vi.fn()
      Diag.beginGesture(fakeBegin)
      Diag.endGesture(fakeEnd)
      expect(fakeEnd).toHaveBeenCalledOnce()
      expect(Diag.getCurrentCorr()).toBeNull()

      Diag.logUi('tool', 'INFO', { message: 'after gesture' })
      expect(Diag.getRecords()[0].corr).toBeNull()
    })
  })

  describe('toNDJSON', () => {
    it('round-trips: each line parses back to an equivalent record', () => {
      Diag.ingestKernel(
        JSON.stringify({ seq: 1, corr: 2, level: 'INFO', target: 'kernel::op', fields: { op: 'push_pull' } }),
      )
      Diag.logUi('app', 'INFO', { message: 'hello' })

      const ndjson = Diag.toNDJSON()
      const lines = ndjson.split('\n')
      expect(lines).toHaveLength(2)

      const parsed = lines.map((l) => JSON.parse(l))
      expect(parsed[0].source).toBe('kernel')
      expect(parsed[0].fields.op).toBe('push_pull')
      expect(parsed[1].source).toBe('ui')
      expect(parsed[1].fields.message).toBe('hello')
    })

    it('serialises an empty ring as an empty string', () => {
      expect(Diag.toNDJSON()).toBe('')
    })
  })

  describe('installKernelDrain', () => {
    it('calls init_logging then registers the drain callback', () => {
      const initLogging = vi.fn()
      let registered: ((json: string) => void) | null = null
      const setLogDrain = vi.fn((cb: (json: string) => void) => {
        registered = cb
      })

      Diag.installKernelDrain(initLogging, setLogDrain)

      expect(initLogging).toHaveBeenCalledWith('info')
      expect(setLogDrain).toHaveBeenCalledOnce()
      expect(registered).not.toBeNull()

      // Simulate the kernel invoking the registered drain callback.
      registered!(JSON.stringify({ seq: 1, corr: null, level: 'INFO', target: 'kernel::op', fields: {} }))
      expect(Diag.getRecords()).toHaveLength(1)
      expect(Diag.getRecords()[0].source).toBe('kernel')
    })
  })

  describe('clear', () => {
    it('empties the ring buffer', () => {
      Diag.logUi('a', 'INFO', {})
      Diag.clear()
      expect(Diag.getRecords()).toHaveLength(0)
    })
  })
})
