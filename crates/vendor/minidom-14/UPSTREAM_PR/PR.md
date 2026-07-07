# Upstream PR materials for digama0/minidom-14

**Status:** prepared, not yet submitted upstream.

**Target repo:** https://github.com/digama0/minidom-14 (branch off `master` /
the 0.17.0 tag).

**Patch:** `0001-update-quick-xml-to-0.41.patch` in this directory. Apply with:

```
git clone https://github.com/digama0/minidom-14
cd minidom-14
git checkout -b update-quick-xml-0.41
git am /path/to/0001-update-quick-xml-to-0.41.patch
cargo test
```

(If `git am` complains about the mailbox format, use `git apply` then commit.)

---

## PR title

Update quick-xml to 0.41 (fixes RUSTSEC-2026-0194 / -0195)

## PR body

`minidom-14` 0.17.0 pins `quick-xml = "^0.36"`. quick-xml `<0.41` is affected by
two high-severity (7.5) advisories:

- **RUSTSEC-2026-0194** — quadratic run time when checking a start tag for
  duplicate attribute names.
- **RUSTSEC-2026-0195** — unbounded namespace-declaration allocation in
  `NsReader`.

Both were fixed in quick-xml 0.41.0, but every downstream that pulls
`minidom-14` is currently forced onto the vulnerable line and fails
`cargo audit`. This bumps the dependency and ports to the 0.41 API.

### API changes handled

- `BytesText::unescape` was removed. quick-xml 0.41 emits entity/character
  references as their own `Event::GeneralRef` events instead of folding them
  into `Text`. `Element::from_reader` now:
  - treats `Event::Text` as literal character data (decode only), and
  - resolves each `Event::GeneralRef` explicitly — numeric character references
    via `BytesRef::resolve_char_ref`, the five XML predefined entities via
    `escape::resolve_predefined_entity`, erroring on any unknown entity (same
    behaviour as the old `unescape` for these DTD-less documents).
- `Attribute::decode_and_unescape_value` is deprecated; replaced with
  `decoded_and_normalized_value(XmlVersion::Implicit1_0, decoder)`.
- Added `From<EncodingError>` and `From<EscapeError>` for `minidom::Error`
  (both route through `quick_xml::Error`), since the split decode/resolve steps
  surface those error types.

### Tests

Existing tests pass unchanged. Added two read-side tests
(`reader_resolves_entities_and_char_refs`, `reader_rejects_unknown_entity`)
covering the rewritten reference-resolution path, which previously had coverage
only through `test_real_data`.

Bumps the crate to 0.18.0 (re-exported `quick_xml` version is part of the public
API, so this is a breaking change — same rationale as the 0.15.0 bump).
