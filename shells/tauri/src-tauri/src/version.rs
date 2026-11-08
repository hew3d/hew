//! The version this app reports in its outbound identifiers (Report Bug
//! payloads, the relay and report User-Agent, the live discovery file): the
//! release tag's version when the release workflow stamped one at compile
//! time (`HEW_RELEASE_VERSION`, the same stamp `hew-cli` and `hew-relay`
//! read), else this crate's own. An empty stamp counts as unset.

pub const VERSION: &str = version_from(
    option_env!("HEW_RELEASE_VERSION"),
    env!("CARGO_PKG_VERSION"),
);

pub const fn version_from(stamp: Option<&'static str>, package: &'static str) -> &'static str {
    match stamp {
        Some(v) if !v.is_empty() => v,
        _ => package,
    }
}

#[cfg(test)]
mod tests {
    use super::version_from;

    #[test]
    fn the_stamp_wins_and_empty_or_missing_falls_back() {
        assert_eq!(version_from(Some("1.5.0-pre.x"), "1.3.1"), "1.5.0-pre.x");
        assert_eq!(version_from(Some(""), "1.3.1"), "1.3.1");
        assert_eq!(version_from(None, "1.3.1"), "1.3.1");
    }
}
