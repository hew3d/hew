# Vendored: minidom-14 0.18.0 (fork of crates.io 0.17.0)

Source: https://github.com/digama0/minidom-14 (crates.io `minidom-14` v0.17.0),
licensed MPL-2.0 (see `LICENSE`).

Why vendored: the published `minidom-14` 0.17.0 pins `quick-xml = "^0.36"`, and
quick-xml < 0.41 carries two high-severity (7.5) DoS advisories that are
reachable through Hew's untrusted `.dae` XML import path:

- **RUSTSEC-2026-0194** — quadratic run time checking a start tag for duplicate
  attribute names (CPU exhaustion).
- **RUSTSEC-2026-0195** — unbounded namespace-declaration allocation (memory
  exhaustion).

quick-xml fixed both in 0.41.0. `minidom-14` is the only consumer holding us on
the vulnerable line, and its upstream repo is dormant (last release 2024-10-04),
so we vendor a copy bumped to quick-xml 0.41 rather than wait. This is a
**temporary** patch: once an upstream release carries the bump, drop this
directory and restore the crates.io dependency.

`dae-parser` (the sibling vendored crate) depends on this copy via a path dep
(`minidom-14 = { path = "../minidom-14" }`) instead of the registry version.

## Local patches (Hew) — the quick-xml 0.36 → 0.41 port

All changes are the mechanical consequence of the quick-xml 0.41 API. See
`CHANGELOG.md` (0.18.0) for the summary. Files touched:

- `Cargo.toml` — `quick-xml = "0.41"`; version `0.18.0`.
- `src/element.rs`
  - `Element::from_reader` `Event::Text`: quick-xml 0.41 no longer folds entity
    references into text, so a `Text` event is now literal character data —
    decode only (the old `BytesText::unescape`, which was removed, is gone).
  - New `Event::GeneralRef` arm: 0.41 emits each `&entity;` / `&#char;` as its
    own event. We resolve numeric character references (`resolve_char_ref`) and
    the five XML predefined entities (`escape::resolve_predefined_entity`), and
    error on any unknown entity — matching the pre-0.41 unescape behaviour for
    DTD-less documents. The pre-root event loop ignores `GeneralRef` (as it
    already ignores stray text).
  - Attribute values: deprecated `decode_and_unescape_value(decoder)` replaced
    with `decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)`.
- `src/error.rs` — added `From<quick_xml::encoding::EncodingError>` and
  `From<quick_xml::escape::EscapeError>` (both route through
  `quick_xml::Error`), since the new decode/resolve steps surface those types.
- `src/tests.rs` — added `reader_resolves_entities_and_char_refs` and
  `reader_rejects_unknown_entity` covering the rewritten reference path (upstream
  had read-side entity coverage only via `test_real_data`).

## Upstream PR

Prepared but **not yet opened** (Kurt will submit it manually):
`UPSTREAM_PR/` holds `0001-update-quick-xml-to-0.41.patch` and `PR.md` (title +
body). The patch applies to a clean checkout of `digama0/minidom-14` @ 0.17.0.
