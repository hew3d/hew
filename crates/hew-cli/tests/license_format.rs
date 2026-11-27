//! `hew-cli catalog check` failure code `license_format`
//! (docs/design/shared-library-sources.md 7.6): a license value, on the
//! index or on an item, is either an SPDX-expression-shaped string or a
//! `LicenseRef-…` with an https `licenseUrl` on the index. Syntax only.

mod common;

use common::Fixture;
use hew_cli::catalog::{FetchPolicy, HttpFetcher, check_source, code};
use std::time::Duration;

fn codes(f: &Fixture) -> Vec<&'static str> {
    let p = FetchPolicy::new(Duration::from_millis(0), Duration::from_secs(10)).with_loopback();
    check_source(&f.reference(), &HttpFetcher::new(&p, None), &p)
        .iter()
        .map(|x| x.code)
        .collect()
}

fn with_license(license: &str, url: Option<&str>) -> Fixture {
    let f = Fixture::new("Fixture", Some("CC0-1.0"));
    f.edit_index(|v| {
        v["license"] = license.into();
        match url {
            Some(u) => v["licenseUrl"] = u.into(),
            None => {
                v.as_object_mut().unwrap().remove("licenseUrl");
            }
        }
    });
    f
}

#[test]
fn a_license_with_spaces_fails() {
    assert_eq!(
        codes(&with_license("CC BY 4.0", None)),
        vec![code::LICENSE_FORMAT]
    );
}

#[test]
fn a_plain_spdx_id_passes() {
    assert_eq!(codes(&with_license("CC-BY-4.0", None)), Vec::<&str>::new());
}

#[test]
fn an_spdx_expression_passes() {
    assert_eq!(
        codes(&with_license("MIT OR Apache-2.0", None)),
        Vec::<&str>::new()
    );
    assert_eq!(
        codes(&with_license("(MIT AND CC0-1.0) WITH Exc-1.0", None)),
        Vec::<&str>::new()
    );
}

#[test]
fn unbalanced_parentheses_fail() {
    assert_eq!(
        codes(&with_license("(MIT OR Apache-2.0", None)),
        vec![code::LICENSE_FORMAT]
    );
}

#[test]
fn a_license_ref_without_a_license_url_fails() {
    assert_eq!(
        codes(&with_license("LicenseRef-Shop", None)),
        vec![code::LICENSE_FORMAT]
    );
}

#[test]
fn a_license_ref_with_an_https_license_url_passes() {
    assert_eq!(
        codes(&with_license(
            "LicenseRef-Shop",
            Some("https://example.com/license")
        )),
        Vec::<&str>::new()
    );
}

#[test]
fn an_item_level_license_that_is_not_a_license_fails() {
    let f = Fixture::new("Fixture", Some("CC0-1.0"));
    f.edit_index(|v| v["items"][0]["license"] = "Not A License".into());
    assert_eq!(codes(&f), vec![code::LICENSE_FORMAT]);
}

#[test]
fn an_item_level_license_ref_uses_the_index_license_url() {
    let f = Fixture::new("Fixture", Some("CC0-1.0"));
    f.edit_index(|v| v["items"][0]["license"] = "LicenseRef-Shop".into());
    assert_eq!(codes(&f), vec![code::LICENSE_FORMAT]);
    let f = Fixture::new("Fixture", Some("CC0-1.0"));
    f.edit_index(|v| {
        v["licenseUrl"] = "https://example.com/license".into();
        v["items"][0]["license"] = "LicenseRef-Shop".into();
    });
    assert_eq!(codes(&f), Vec::<&str>::new());
}

#[test]
fn a_missing_license_reports_only_license_missing() {
    let f = Fixture::new("Fixture", None);
    assert_eq!(codes(&f), vec![code::LICENSE_MISSING]);
}
