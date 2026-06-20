use std::{
    fmt::{Debug, Display},
    str::FromStr,
};

use percent_encoding::{percent_decode_str, percent_encode, AsciiSet, CONTROLS};

/// https://url.spec.whatwg.org/#fragment-percent-encode-set
const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

/// A (really) basic URL parser.
///
/// It is optimized for the case of COLLADA documents where most "URLs"
/// are really just names of other entities in the document, prefixed by `#`.
/// Unfortunately the `url` crate does not like these fragments,
/// and we don't have a base URL to work from since the parser does not do URL resolution.
/// So we parse fragments and leave everything else to be parsed by a proper URL crate
/// during resolution.
#[derive(Clone, PartialEq, Eq)]
pub enum Url {
    /// A fragment `#foo`. The string is the percent-decoded payload `"foo"`.
    Fragment(String),
    /// A maybe-URL which is not a fragment. These are unparsed and unvalidated.
    Other(String),
}

impl FromStr for Url {
    type Err = std::str::Utf8Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut it = s.chars();
        Ok(if it.next() == Some('#') {
            Url::Fragment(percent_decode_str(it.as_str()).decode_utf8()?.into())
        } else {
            Url::Other(s.into())
        })
    }
}

impl Display for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Fragment(s) => write!(f, "#{}", percent_encode(s.as_bytes(), FRAGMENT)),
            Self::Other(s) => write!(f, "{}", s),
        }
    }
}

impl Debug for Url {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}
