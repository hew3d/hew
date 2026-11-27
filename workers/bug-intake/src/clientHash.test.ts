import { test, describe } from 'node:test'
import assert from 'node:assert/strict'

import { createHash } from 'node:crypto'

import { clientNetwork, hashClient } from './clientHash.ts'

const DAY_MS = 24 * 60 * 60 * 1000
const day1 = Date.parse('2026-09-13T12:00:00.000Z')
const day2 = day1 + DAY_MS

describe('hashClient', () => {
  test('is deterministic for the same secret/ip/day', async () => {
    const a = await hashClient('secret', '203.0.113.7', day1)
    const b = await hashClient('secret', '203.0.113.7', day1 + 1000) // same UTC day
    assert.equal(a, b)
  })

  test('changes across a UTC day boundary', async () => {
    const a = await hashClient('secret', '203.0.113.7', day1)
    const b = await hashClient('secret', '203.0.113.7', day2)
    assert.notEqual(a, b)
  })

  test('changes with the secret', async () => {
    const a = await hashClient('secret-a', '203.0.113.7', day1)
    const b = await hashClient('secret-b', '203.0.113.7', day1)
    assert.notEqual(a, b)
  })

  test('changes with the IP', async () => {
    const a = await hashClient('secret', '203.0.113.7', day1)
    const b = await hashClient('secret', '203.0.113.8', day1)
    assert.notEqual(a, b)
  })

  test('never contains the raw IP as a substring', async () => {
    const ip = '203.0.113.7'
    const hash = await hashClient('secret', ip, day1)
    assert.ok(!hash.includes(ip))
    assert.match(hash, /^[0-9a-f]{64}$/) // hex SHA-256 digest, nothing else
  })
})

describe('clientNetwork: what counts as one client', () => {
  const sameClient = async (a: string, b: string) => (await hashClient('secret', a, day1)) === (await hashClient('secret', b, day1))

  test('addresses in one IPv6 /64 are one client', async () => {
    assert.ok(await sameClient('2001:db8:1:2::1', '2001:db8:1:2:ffff:ffff:ffff:ffff'))
    assert.ok(await sameClient('2001:db8:1:2:aaaa:bbbb:cccc:dddd', '2001:db8:1:2::'))
  })

  test('neighbouring /64s are different clients', async () => {
    assert.ok(!(await sameClient('2001:db8:1:2::1', '2001:db8:1:3::1')))
    assert.ok(!(await sameClient('2001:db8:1:2::1', '2001:db9:1:2::1')))
  })

  test('compressed, expanded, uppercase and zone-suffixed forms agree', () => {
    const expected = '2001:db8:0:0::/64'
    for (const form of [
      '2001:db8::1',
      '2001:0db8:0000:0000:0000:0000:0000:0001',
      '2001:DB8::ABCD',
      '2001:db8:0:0:1::',
      '2001:db8::192.0.2.7',
      '2001:db8::1%eth0',
    ]) {
      assert.equal(clientNetwork(form), expected, form)
    }
    assert.equal(clientNetwork('fe80::1%en0'), 'fe80:0:0:0::/64')
    assert.equal(clientNetwork('::1'), '0:0:0:0::/64')
    assert.equal(clientNetwork('::'), '0:0:0:0::/64')
    // `::` early in the address changes which groups fall in the prefix.
    assert.equal(clientNetwork('2001::2:3:4:5:6'), '2001:0:0:2::/64')
    assert.equal(clientNetwork('1:2:3:4:5:6:7::'), '1:2:3:4::/64')
  })

  test('an IPv4-mapped address is that IPv4 client, not part of one shared /64', async () => {
    for (const form of ['::ffff:203.0.113.7', '::FFFF:203.0.113.7', '0:0:0:0:0:ffff:203.0.113.7', '::ffff:cb00:7107', '::ffff:203.0.113.7%1']) {
      assert.equal(clientNetwork(form), '203.0.113.7', form)
    }
    assert.ok(await sameClient('::ffff:203.0.113.7', '203.0.113.7'))
    assert.ok(!(await sameClient('::ffff:203.0.113.7', '::ffff:203.0.113.8')))
  })

  test('IPv4 is hashed exactly as before', async () => {
    assert.equal(clientNetwork('203.0.113.7'), '203.0.113.7')
    const expected = createHash('sha256').update('secret|2026-09-13|203.0.113.7').digest('hex')
    assert.equal(await hashClient('secret', '203.0.113.7', day1), expected)
  })

  test('unparseable input is used as given and never throws', async () => {
    for (const junk of [
      '', 'unknown', ':', ':::', '1::2::3', '2001:db8', '1:2:3:4:5:6:7:8:9', '1:2:3:4::5:6:7:8',
      '2001:db8::g', '2001:db8::12345', '::ffff:256.0.0.1', '::ffff:1.2.3', '203.0.113.7:443', '[2001:db8::1]', '%',
    ]) {
      assert.equal(clientNetwork(junk), junk, JSON.stringify(junk))
      assert.match(await hashClient('secret', junk, day1), /^[0-9a-f]{64}$/)
    }
  })
})
