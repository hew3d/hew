// Copyright (c) 2020 Maxime “pep” Buquet <pep@bouah.net>
// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

//! Provides a `Parser` type, which takes bytes and returns Elements. It also keeps a hold of
//! ascendant elements to be able to handle namespaces properly.

use crate::element::Element;
use crate::error::{Error, ParserError, Result};

use bytes::BytesMut;
use quick_xml::Reader as EventReader;
use std::cell::RefCell;
use std::str;

/// Parser
#[derive(Debug)]
pub struct Parser {
    buffer: RefCell<BytesMut>,
    state: ParserState,
}

/// Describes the state of the parser.
///
/// This parser will only accept one-level documents. The root element is kept for convenience, to
/// be able to pass namespaces down to children who are themselves children.
#[derive(Debug)]
pub enum ParserState {
    /// Not enough data has been processed to find the first element.
    Empty,

    /// The normal state. the root element has been identified and children are processed.
    Root {
        /// Root element. Kept for future reference
        root: Element,

        /// Child element
        child: Option<Element>,

        /// XXX: Weird flag to say if we've already sent what we could send or if there's more to
        /// send. This Variant needs to be changed.
        sent: bool,
    },

    /// Something was passed in the buffer that made the parser get into an error state.
    Error,

    /// The root element has been closed. No feed-ing can happen past this point.
    Closed,
}

/// Result of polling the parser
#[derive(Debug)]
pub enum ParserResult {
    /// Buffer is not empty but needs more data
    Partial,

    /// An Element has been generated from the buffer.
    Single(Element),
}

/*
/// Split <stream:stream> and parse it.
fn split_stream_stream_stream_features(string: String) -> (Element, Element) {
    let mut stuff = string.splitn(2, '>');
    let stream_opening_str = stuff.next().unwrap().to_string() + "/>";
    let rest = stuff.next().unwrap().to_string();
    let stream_opening: Element = stream_opening_str.parse().unwrap();
    let rest: Element = rest.parse().unwrap();
    println!("opening: {}", String::from(&stream_opening));
    println!("features: {}", String::from(&rest));
    (stream_opening, rest)
}
*/

fn maybe_split_prolog(string: &str) -> &str {
    if string.starts_with("<?xml") {
        let mut stuff = string.splitn(2, '>');
        stuff.next();
        stuff.next().unwrap()
    } else {
        string
    }
}

impl Parser {
    /// Creates a new Parser
    pub fn new() -> Parser {
        Parser {
            buffer: RefCell::new(BytesMut::new()),
            state: ParserState::Empty,
        }
    }

    /// Feed bytes to the parser.
    pub fn feed(&mut self, bytes: BytesMut) -> Result<()> {
        self.buffer.borrow_mut().unsplit(bytes);
        let state = match self.state {
            ParserState::Empty => {
                // TODO: Try splitting xml prolog and stream header
                let foo = self.buffer.borrow();
                let header = maybe_split_prolog(str::from_utf8(foo.as_ref())?);
                println!("FOO: header: {:?}", header);
                let mut reader = EventReader::from_str(header);
                let root = Element::from_reader(&mut reader);
                match root {
                    Ok(root) => {
                        println!("FOO: elem: {:?}", root);
                        ParserState::Root {
                            root,
                            child: None,
                            sent: false,
                        }
                    }
                    Err(e) => {
                        println!("FOO: err: {:?}", e);
                        ParserState::Empty
                    }
                }
            }
            ParserState::Closed => return Err(Error::ParserError(ParserError::Closed)),
            _ => ParserState::Empty,
        };

        self.state = state;
        Ok(())
    }

    /// Returns Elements to the application.
    pub fn poll(&mut self) -> Result<Option<ParserResult>> {
        match &self.state {
            ParserState::Empty if self.buffer.borrow().len() != 0 => {
                Ok(Some(ParserResult::Partial))
            }
            ParserState::Empty | ParserState::Closed | ParserState::Error => Ok(None),
            ParserState::Root {
                root, child: None, ..
            } => Ok(Some(ParserResult::Single(root.clone()))),
            ParserState::Root {
                child: Some(child), ..
            } => Ok(Some(ParserResult::Single(child.clone()))),
        }
    }

    /// Resets the parser
    pub fn reset(&mut self) {
        *self = Parser::new();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{BufMut, BytesMut};

    #[test]
    fn test_prolog() {
        let mut parser = Parser::new();
        let mut buf = BytesMut::new();
        buf.put(&b"<?xml version='1.0'?>"[..]);
        buf.put(&b"<stream:stream xmlns='jabber:client' xml:lang='en' xmlns:stream='http://etherx.jabber.org/streams' version='1.0' to='foo.bar'>"[..]);
        match parser.feed(buf) {
            Ok(_) => (),
            _ => panic!(),
        }

        let elem = Element::builder("stream:stream", "http://etherx.jabber.org/streams")
            .prefix_ns(None, "jabber:client")
            .attr("xml:lang", "en")
            .attr("version", "1.0")
            .attr("to", "foo.bar")
            .build();

        println!("BAR: elem: {:?}", elem);

        match parser.poll() {
            Ok(Some(ParserResult::Single(e))) => assert_eq!(e, elem),
            _ => panic!(),
        }
    }
}
