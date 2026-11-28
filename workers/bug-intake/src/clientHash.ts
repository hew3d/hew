/**
 * The per-client key rate limiting is counted against: SHA-256 of
 * `IP_HASH_SECRET + UTC date + client network` (`clientNetwork`: the IPv4
 * address, or the IPv6 /64), never the raw IP
 * (docs/design/report-bug.md §4 "Abuse", §8's `IP_HASH_SECRET`). Salting
 * with the secret makes the hash unreversible without it; including the
 * UTC date makes a client's hash change every day, which is also what lets
 * `indexStore.ts` treat "one row per client per day" as the natural key for
 * the daily counter instead of tracking a separate date column.
 */

function utcDateString(now: number): string {
  return new Date(now).toISOString().slice(0, 10) // "YYYY-MM-DD"
}

/** The eight 16-bit groups of an IPv6 address written in any of its text
 *  forms (either case, `::` compression, a dotted IPv4 tail), or null if
 *  `text` is not one. */
function parseIpv6(text: string): number[] | null {
  let hex = text
  const dotted = /^(.*:)(\d{1,3})\.(\d{1,3})\.(\d{1,3})\.(\d{1,3})$/.exec(text)
  if (dotted !== null) {
    const [a, b, c, d] = dotted.slice(2).map(Number)
    if (a > 255 || b > 255 || c > 255 || d > 255) return null
    hex = `${dotted[1]}${((a << 8) | b).toString(16)}:${((c << 8) | d).toString(16)}`
  }
  const halves = hex.split('::')
  if (halves.length > 2) return null
  const [head, tail] = halves.map((half) => (half === '' ? [] : half.split(':')))
  if (![...head, ...(tail ?? [])].every((group) => /^[0-9a-f]{1,4}$/i.test(group))) return null
  let groups = head
  if (tail !== undefined) {
    // `::` stands for at least one zero group.
    const zeros = 8 - head.length - tail.length
    if (zeros < 1) return null
    groups = [...head, ...Array<string>(zeros).fill('0'), ...tail]
  }
  return groups.length === 8 ? groups.map((group) => parseInt(group, 16)) : null
}

/** What counts as one client for the limits. An IPv6 subscriber is usually
 *  handed a whole /64 and can use a new address from it for every request,
 *  so an IPv6 address counts as its /64. An IPv4-mapped address
 *  (`::ffff:a.b.c.d`) counts as that IPv4 address — they all share one /64.
 *  IPv4, and anything that does not parse, is used exactly as given. */
export function clientNetwork(ip: string): string {
  if (!ip.includes(':')) return ip
  // A zone suffix (`%eth0`) names a local interface, not a different client.
  const groups = parseIpv6(ip.split('%')[0])
  if (groups === null) return ip
  if (groups.slice(0, 5).every((g) => g === 0) && groups[5] === 0xffff) {
    return [groups[6] >> 8, groups[6] & 0xff, groups[7] >> 8, groups[7] & 0xff].join('.')
  }
  return `${groups.slice(0, 4).map((g) => g.toString(16)).join(':')}::/64`
}

/** `ip` is whatever `CF-Connecting-IP` carried — never logged or stored by
 *  the caller, only ever passed straight in here. Hashes the client's
 *  `clientNetwork`. Returns a hex digest. */
export async function hashClient(secret: string, ip: string, now: number): Promise<string> {
  const data = new TextEncoder().encode(`${secret}|${utcDateString(now)}|${clientNetwork(ip)}`)
  const digest = await crypto.subtle.digest('SHA-256', data)
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}
