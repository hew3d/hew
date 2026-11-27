//! 6.4 `is_public_host` (S4's oracle names it a pure function) and the
//! catalog's "every final URL is https on a public address" check (7.6).
use std::net::IpAddr;

use sources::is_public_host;

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[track_caller]
fn refused(list: &[&str]) {
    for s in list {
        assert!(!is_public_host(ip(s)), "{s} must NOT be public");
    }
}

#[test]
fn loopback_is_refused() {
    refused(&["127.0.0.1", "127.0.0.2", "127.255.255.255", "::1"]);
}

#[test]
fn private_ipv4_is_refused() {
    refused(&[
        "10.0.0.0",
        "10.0.0.1",
        "10.255.255.255",
        "172.16.0.0",
        "172.16.0.1",
        "172.31.255.255",
        "192.168.0.0",
        "192.168.1.1",
        "192.168.255.255",
    ]);
}

#[test]
fn ranges_next_to_private_are_public() {
    for s in [
        "172.15.255.255",
        "172.32.0.0",
        "11.0.0.1",
        "192.167.255.255",
        "192.169.0.1",
        "9.255.255.255",
    ] {
        assert!(is_public_host(ip(s)), "{s} is public");
    }
}

#[test]
fn link_local_is_refused_including_the_cloud_metadata_address() {
    refused(&[
        "169.254.0.1",
        "169.254.169.254",
        "169.254.255.255",
        "fe80::1",
        "febf::1",
    ]);
}

#[test]
fn unique_local_ipv6_is_refused() {
    refused(&[
        "fc00::1",
        "fd00::1",
        "fd12:3456:789a::1",
        "fdff:ffff:ffff:ffff::1",
    ]);
}

#[test]
fn unspecified_is_refused() {
    refused(&["0.0.0.0", "::"]);
}

#[test]
fn multicast_is_refused() {
    refused(&["224.0.0.1", "239.255.255.255", "ff02::1", "ff0e::1"]);
}

#[test]
fn ipv4_mapped_ipv6_follows_the_embedded_ipv4() {
    refused(&[
        "::ffff:10.0.0.1",
        "::ffff:127.0.0.1",
        "::ffff:192.168.1.1",
        "::ffff:172.16.0.1",
        "::ffff:169.254.169.254",
        "::ffff:0.0.0.0",
        "::ffff:224.0.0.1",
    ]);
    for s in ["::ffff:8.8.8.8", "::ffff:140.82.112.3"] {
        assert!(is_public_host(ip(s)), "{s}");
    }
}

#[test]
fn broadcast_is_refused() {
    refused(&["255.255.255.255"]);
}

#[test]
fn public_addresses_are_accepted() {
    for s in [
        "8.8.8.8",
        "1.1.1.1",
        "140.82.112.3",
        "185.199.108.133",
        "93.184.216.34",
        "2606:4700:4700::1111",
        "2001:4860:4860::8888",
        "2a00:1450:4001:81b::200e",
    ] {
        assert!(is_public_host(ip(s)), "{s} is public");
    }
}
