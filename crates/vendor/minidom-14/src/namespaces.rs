// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
// Copyright (c) 2020 Astro <astro@spaceboyz.net>
// Copyright (c) 2020 Maxime “pep” Buquet <pep@bouah.net>
// Copyright (c) 2020 Xidorn Quan <me@upsuper.org>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
/// Use to compare namespaces
pub enum NSChoice<'a> {
    /// The element must have no namespace
    None,
    /// The element's namespace must match the specified namespace
    OneOf(&'a str),
    /// The element's namespace must be in the specified vector
    AnyOf(&'a [&'a str]),
    /// The element can have any namespace, or no namespace
    Any,
}

impl<'a> From<&'a str> for NSChoice<'a> {
    fn from(ns: &'a str) -> NSChoice<'a> {
        NSChoice::OneOf(ns)
    }
}

impl<'a> NSChoice<'a> {
    pub(crate) fn compare(&self, ns: &str) -> bool {
        match (ns, &self) {
            (_, NSChoice::None) => false,
            (_, NSChoice::Any) => true,
            (ns, NSChoice::OneOf(wanted_ns)) => &ns == wanted_ns,
            (ns, NSChoice::AnyOf(wanted_nss)) => wanted_nss.iter().any(|w| &ns == w),
        }
    }
}
