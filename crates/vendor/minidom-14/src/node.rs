// Copyright (c) 2020 lumi <lumi@pew.im>
// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
// Copyright (c) 2020 Maxime “pep” Buquet <pep@bouah.net>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Provides the `Node` struct, which represents a node in the DOM.

use crate::element::{Element, ElementBuilder};
use crate::error::Result;

use std::collections::BTreeMap;
use std::io::Write;

use quick_xml::events::{BytesText, Event};
use quick_xml::Writer as EventWriter;

/// A node in an element tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// An `Element`.
    Element(Element),
    /// A text node.
    Text(String),
}

impl Node {
    /// Turns this into a reference to an `Element` if this is an element node.
    /// Else this returns `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use minidom::Node;
    ///
    /// let elm = Node::Element("<meow xmlns=\"ns1\"/>".parse().unwrap());
    /// let txt = Node::Text("meow".to_owned());
    ///
    /// assert_eq!(elm.as_element().unwrap().name(), "meow");
    /// assert_eq!(txt.as_element(), None);
    /// ```
    pub fn as_element(&self) -> Option<&Element> {
        match *self {
            Node::Element(ref e) => Some(e),
            Node::Text(_) => None,
        }
    }

    /// Turns this into a mutable reference of an `Element` if this is an element node.
    /// Else this returns `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use minidom::Node;
    ///
    /// let mut elm = Node::Element("<meow xmlns=\"ns1\"/>".parse().unwrap());
    /// let mut txt = Node::Text("meow".to_owned());
    ///
    /// assert_eq!(elm.as_element_mut().unwrap().name(), "meow");
    /// assert_eq!(txt.as_element_mut(), None);
    /// ```
    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match *self {
            Node::Element(ref mut e) => Some(e),
            Node::Text(_) => None,
        }
    }

    /// Turns this into an `Element`, consuming self, if this is an element node.
    /// Else this returns `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use minidom::Node;
    ///
    /// let elm = Node::Element("<meow xmlns=\"ns1\"/>".parse().unwrap());
    /// let txt = Node::Text("meow".to_owned());
    ///
    /// assert_eq!(elm.into_element().unwrap().name(), "meow");
    /// assert_eq!(txt.into_element(), None);
    /// ```
    pub fn into_element(self) -> Option<Element> {
        match self {
            Node::Element(e) => Some(e),
            Node::Text(_) => None,
        }
    }

    /// Turns this into an `&str` if this is a text node.
    /// Else this returns `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use minidom::Node;
    ///
    /// let elm = Node::Element("<meow xmlns=\"ns1\"/>".parse().unwrap());
    /// let txt = Node::Text("meow".to_owned());
    ///
    /// assert_eq!(elm.as_text(), None);
    /// assert_eq!(txt.as_text().unwrap(), "meow");
    /// ```
    pub fn as_text(&self) -> Option<&str> {
        match *self {
            Node::Element(_) => None,
            Node::Text(ref s) => Some(s),
        }
    }

    /// Turns this into an `&mut String` if this is a text node.
    /// Else this returns `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use minidom::Node;
    ///
    /// let mut elm = Node::Element("<meow xmlns=\"ns1\"/>".parse().unwrap());
    /// let mut txt = Node::Text("meow".to_owned());
    ///
    /// assert_eq!(elm.as_text_mut(), None);
    /// {
    ///     let text_mut = txt.as_text_mut().unwrap();
    ///     assert_eq!(text_mut, "meow");
    ///     text_mut.push_str("zies");
    ///     assert_eq!(text_mut, "meowzies");
    /// }
    /// assert_eq!(txt.as_text().unwrap(), "meowzies");
    /// ```
    pub fn as_text_mut(&mut self) -> Option<&mut String> {
        match *self {
            Node::Element(_) => None,
            Node::Text(ref mut s) => Some(s),
        }
    }

    /// Turns this into an `String`, consuming self, if this is a text node.
    /// Else this returns `None`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use minidom::Node;
    ///
    /// let elm = Node::Element("<meow xmlns=\"ns1\"/>".parse().unwrap());
    /// let txt = Node::Text("meow".to_owned());
    ///
    /// assert_eq!(elm.into_text(), None);
    /// assert_eq!(txt.into_text().unwrap(), "meow");
    /// ```
    pub fn into_text(self) -> Option<String> {
        match self {
            Node::Element(_) => None,
            Node::Text(s) => Some(s),
        }
    }

    #[doc(hidden)]
    pub(crate) fn write_to_inner<W: Write>(
        &self,
        writer: &mut EventWriter<W>,
        prefixes: &mut BTreeMap<Option<String>, String>,
    ) -> Result<()> {
        match *self {
            Node::Element(ref elmt) => elmt.write_to_inner(writer, prefixes)?,
            Node::Text(ref s) => {
                writer.write_event(Event::Text(BytesText::new(s)))?;
            }
        }

        Ok(())
    }
}

impl<I> From<I> for Node
where
    I: Into<Element>,
{
    fn from(elm: I) -> Node {
        Node::Element(elm.into())
    }
}

impl From<String> for Node {
    fn from(s: String) -> Node {
        Node::Text(s)
    }
}

impl<'a> From<&'a str> for Node {
    fn from(s: &'a str) -> Node {
        Node::Text(s.to_owned())
    }
}

impl From<ElementBuilder> for Node {
    fn from(builder: ElementBuilder) -> Node {
        Node::Element(builder.build())
    }
}
