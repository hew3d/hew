Version 0.18.0, released 2026-07-06:
  * Changes
    * Update quick-xml to 0.41 (fixes RUSTSEC-2026-0194 / RUSTSEC-2026-0195,
      two high-severity DoS advisories against quick-xml <0.41)
    * Port to the quick-xml 0.41 API: `BytesText::unescape` was removed and
      references are now emitted as `Event::GeneralRef`; text is decoded and
      character/entity references are resolved explicitly. Deprecated
      `Attribute::decode_and_unescape_value` replaced with
      `decoded_and_normalized_value`.

Version 0.17.0, released 2024-10-04:
  * Changes
    * Update quick-xml to 0.36
    * Allow comments
  * Fixes
    * Fix PartialEq implementation for Element

Version 0.16.0, released 2023-03-22:
  * Changes
    * Update quick-xml to 0.28
    * use edition 2021

Version 0.15.0, released 2023-03-22:
  * Changes
    * Bump quick-xml dependency (breaking change only because it is re-exported)
  * Fixes
    * Fix warnings

**minidom-14** version 0.14.0, released 2023-03-22:
  * No changes (identical to minidom 0.14.0)

---

**minidom** version 0.14.0, released 2022-03-07:
  * Changes
    * Bump quick-xml dependency (thanks eijebong!)
  * Fixes
    * Handle identical namespaces of sibling elements correctly (thanks Jasper!)
    * Fix support for newer rustc (see https://github.com/rust-lang/rust/issues/90199)

Version 0.13.0, released 2021-01-13:
  * Changes
    * Force namespaces on Element, which was a breaking change.

Version 0.12.1, released 2021-01-13, yanked:
  * Changes
    * Bump quick-xml dependency.

Version 0.12, released 2020-02-15:
  * Breaking
    * `Element.write_to` doesn't prepand xml prelude anymore. Use `write_to_decl` when necessary.
    * PartialEq implementation for Element and Node have been changed to
      ensure namespaces match even if the objects are not structurally
      equivalent in Rust.
  * Changes
    * Explicitely focus on XMPP. Some features will eventually be removed from
      the project to comply with this.
    * Update edition to 2018
    * Add NSChoice enum to allow comparing NSs differently
    * Add impl for From<Into<Element>> for Node
  * Fixes
    * Update old CI configuration with newer Rust images

Version 0.11.1, released 2019-09-06:
  * Changes
    * Update to quick-xml 0.16
    * Add a default "comments" feature to transform comments into errors when unset.

Version 0.11.0, released 2019-06-14:
  * Breaking
    * Get rid of IntoElements, replace with `Into<Node>` and `<T: Into<Node> IntoIterator<Item = T>>`
  * Fixes
    * Remote unused `mut` attribute on variable
  * Changes
    * Update quick-xml to 0.14
    * Split Node into its own module
    * Nicer Debug implementation for NamespaceSet

Version 0.10.0, released 2018-10-21:
  * Changes
    * Update quick-xml to 0.13
    * Update doc to reflect switch from xml-rs to quick-xml.

Version 0.9.1, released 2018-05-29:
  * Fixes
    * Lumi fixed CDATA handling, minidom will not unescape CDATA bodies anymore.
  * Small changes
    - Link Mauve implemented IntoAttributeValue on std::net::IpAddr.

Version 0.9.0, released 2018-04-10:
  * Small changes
    - Upgrade quick_xml to 0.12.1

Version 0.8.0, released 2018-02-18:
  * Additions
    - Link Mauve replaced error\_chain with failure ( https://gitlab.com/lumi/minidom-rs/merge_requests/27 )
    - Yue Liu added support for writing comments and made the writing methods use quick-xml's EventWriter ( https://gitlab.com/lumi/minidom-rs/merge_requests/26 )

Version 0.6.2, released 2017-08-27:
  * Additions
    - Link Mauve added an implementation of IntoElements for all Into<Element> ( https://gitlab.com/lumi/minidom-rs/merge_requests/19 )

Version 0.6.1, released 2017-08-20:
  * Additions
    - Astro added Element::has_ns, which checks whether an element's namespace matches the passed argument. ( https://gitlab.com/lumi/minidom-rs/merge_requests/16 )
    - Link Mauve updated the quick-xml dependency to the latest version.
  * Fixes
    - Because break value is now stable, Link Mauve rewrote some code marked FIXME to use it.

Version 0.6.0, released 2017-08-13:
  * Big changes
    - Astro added proper support for namespace prefixes. ( https://gitlab.com/lumi/minidom-rs/merge_requests/14 )
  * Fixes
    - Astro fixed a regression that caused the writer not to escape its xml output properly. ( https://gitlab.com/lumi/minidom-rs/merge_requests/15 )

Version 0.5.0, released 2017-06-10:
  * Big changes
    - Eijebong made parsing a lot faster by switching the crate from xml-rs to quick_xml. ( https://gitlab.com/lumi/minidom-rs/merge_requests/11 )
