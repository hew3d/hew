// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
// Copyright (c) 2020 Astro <astro@spaceboyz.net>
// Copyright (c) 2020 Maxime “pep” Buquet <pep@bouah.net>
// Copyright (c) 2020 Xidorn Quan <me@upsuper.org>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use std::collections::BTreeMap;
use std::fmt;

pub type Prefix = Option<String>;
pub type Namespace = String;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Prefixes {
    prefixes: BTreeMap<Prefix, Namespace>,
}

impl fmt::Debug for Prefixes {
    // TODO: Fix end character
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Prefixes(")?;
        for (prefix, namespace) in &self.prefixes {
            write!(
                f,
                "xmlns{}={:?} ",
                match prefix {
                    None => String::new(),
                    Some(prefix) => format!(":{}", prefix),
                },
                namespace
            )?;
        }
        write!(f, ")")
    }
}

impl Prefixes {
    pub fn declared_prefixes(&self) -> &BTreeMap<Prefix, Namespace> {
        &self.prefixes
    }

    pub fn get(&self, prefix: &Prefix) -> Option<&Namespace> {
        self.prefixes.get(prefix)
    }

    pub(crate) fn insert<S: Into<Namespace>>(&mut self, prefix: Prefix, namespace: S) {
        self.prefixes.insert(prefix, namespace.into());
    }
}

impl From<BTreeMap<Prefix, Namespace>> for Prefixes {
    fn from(prefixes: BTreeMap<Prefix, Namespace>) -> Self {
        Prefixes { prefixes }
    }
}

impl From<Option<String>> for Prefixes {
    fn from(namespace: Option<String>) -> Self {
        match namespace {
            None => Self::default(),
            Some(namespace) => Self::from(namespace),
        }
    }
}

impl From<Namespace> for Prefixes {
    fn from(namespace: Namespace) -> Self {
        let mut prefixes = BTreeMap::new();
        prefixes.insert(None, namespace);

        Prefixes { prefixes }
    }
}

impl From<(Prefix, Namespace)> for Prefixes {
    fn from(prefix_namespace: (Prefix, Namespace)) -> Self {
        let (prefix, namespace) = prefix_namespace;
        let mut prefixes = BTreeMap::new();
        prefixes.insert(prefix, namespace);

        Prefixes { prefixes }
    }
}

impl From<(String, String)> for Prefixes {
    fn from(prefix_namespace: (String, String)) -> Self {
        let (prefix, namespace) = prefix_namespace;
        Self::from((Some(prefix), namespace))
    }
}
