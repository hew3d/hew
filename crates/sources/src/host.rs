//! The public-host check for listed sources (6.4, `is_public_host`).

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// True when a connection to `addr` is allowed for a listed source: refuses
/// loopback, private, link-local, unique-local, unspecified, multicast and
/// broadcast addresses, the other ranges that are not globally routable
/// (carrier-grade NAT, benchmarking, reserved, documentation, 6to4 relay
/// anycast, site-local, IETF protocol assignments, SRv6), and IPv6 forms
/// that embed an IPv4 address (mapped, translated, NAT64, 6to4, compatible)
/// when that address is itself not public.
pub fn is_public_host(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => v4_public(v4),
        IpAddr::V6(v6) => v6_public(v6),
    }
}

fn v4_public(a: Ipv4Addr) -> bool {
    let [o0, o1, o2, _] = a.octets();
    let refused = a.is_loopback()
        || a.is_private()
        || a.is_link_local()
        || a.is_unspecified()
        || a.is_multicast()
        || a.is_broadcast()
        || o0 == 0 // 0.0.0.0/8 "this network"
        || (o0 == 100 && (64..=127).contains(&o1)) // 100.64.0.0/10 shared space
        || (o0 == 192 && o1 == 0 && o2 == 0) // 192.0.0.0/24 protocol assignments
        || (o0 == 192 && o1 == 0 && o2 == 2) // documentation
        || (o0 == 192 && o1 == 88 && o2 == 99) // 192.88.99.0/24 6to4 relay anycast
        || (o0 == 198 && (o1 == 18 || o1 == 19)) // 198.18.0.0/15 benchmarking
        || (o0 == 198 && o1 == 51 && o2 == 100) // documentation
        || (o0 == 203 && o1 == 0 && o2 == 113) // documentation
        || o0 >= 240; // 240.0.0.0/4 reserved
    !refused
}

fn v6_public(a: Ipv6Addr) -> bool {
    if let Some(v4) = a.to_ipv4_mapped() {
        return v4_public(v4);
    }
    let s = a.segments();
    let embedded = |hi: u16, lo: u16| {
        let [a0, a1] = hi.to_be_bytes();
        let [b0, b1] = lo.to_be_bytes();
        Ipv4Addr::new(a0, a1, b0, b1)
    };
    // 64:ff9b::/96 (NAT64) and ::ffff:0:0/96 (IPv4-translated) embed the
    // IPv4 address in the last 32 bits.
    if s[..6] == [0x64, 0xff9b, 0, 0, 0, 0] || s[..6] == [0, 0, 0, 0, 0xffff, 0] {
        return v4_public(embedded(s[6], s[7]));
    }
    // 2002::/16 (6to4) embeds it in bits 16 to 47.
    if s[0] == 0x2002 {
        return v4_public(embedded(s[1], s[2]));
    }
    // ::a.b.c.d (deprecated IPv4-compatible), apart from :: and ::1.
    if s[..6] == [0; 6] && !a.is_unspecified() && !a.is_loopback() {
        return v4_public(embedded(s[6], s[7]));
    }
    let refused = a.is_loopback()
        || a.is_unspecified()
        || a.is_multicast()
        || (s[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
        || (s[0] & 0xfe00) == 0xfc00 // fc00::/7 unique-local
        || (s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1) // 64:ff9b:1::/48 local NAT64
        || (s[0] & 0xffc0) == 0xfec0 // fec0::/10 site-local (deprecated)
        || (s[0] == 0x2001 && s[1] < 0x0200) // 2001::/23 IETF protocol assignments (Teredo, ORCHID, ...)
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        || (s[0] == 0x3fff && s[1] < 0x1000) // 3fff::/20 documentation
        || s[0] == 0x5f00 // 5f00::/16 SRv6 segment identifiers
        || (s[0] == 0x0100 && s[1] == 0 && s[2] == 0 && s[3] == 0); // 100::/64 discard
    !refused
}
