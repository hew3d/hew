// Copyright (c) 2020 lumi <lumi@pew.im>
// Copyright (c) 2020 Emmanuel Gil Peyrot <linkmauve@linkmauve.fr>
// Copyright (c) 2020 Bastien Orivel <eijebong+minidom@bananium.fr>
// Copyright (c) 2020 Astro <astro@spaceboyz.net>
// Copyright (c) 2020 Maxime “pep” Buquet <pep@bouah.net>
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

#![deny(missing_docs)]

//! A minimal DOM crate built on top of quick-xml, targeting exclusively the subset of XML useful
//! for XMPP.
//!
//! This library exports an `Element` struct which represents a DOM tree.
//!
//! # Example
//!
//! Run with `cargo run --example articles`. Located in `examples/articles.rs`.
//!
//! ```rust,ignore
//! extern crate minidom;
//!
//! use minidom::Element;
//!
//! const DATA: &'static str = r#"<articles xmlns="article">
//!     <article>
//!         <title>10 Terrible Bugs You Would NEVER Believe Happened</title>
//!         <body>
//!             Rust fixed them all. &lt;3
//!         </body>
//!     </article>
//!     <article>
//!         <title>BREAKING NEWS: Physical Bug Jumps Out Of Programmer's Screen</title>
//!         <body>
//!             Just kidding!
//!         </body>
//!     </article>
//! </articles>"#;
//!
//! const ARTICLE_NS: &'static str = "article";
//!
//! #[derive(Debug)]
//! pub struct Article {
//!     title: String,
//!     body: String,
//! }
//!
//! fn main() {
//!     let root: Element = DATA.parse().unwrap();
//!
//!     let mut articles: Vec<Article> = Vec::new();
//!
//!     for child in root.children() {
//!         if child.is("article", ARTICLE_NS) {
//!             let title = child.get_child("title", ARTICLE_NS).unwrap().text();
//!             let body = child.get_child("body", ARTICLE_NS).unwrap().text();
//!             articles.push(Article {
//!                 title: title,
//!                 body: body.trim().to_owned(),
//!             });
//!         }
//!     }
//!
//!     println!("{:?}", articles);
//! }
//! ```
//!
//! # Usage
//!
//! To use `minidom`, add this to your `Cargo.toml` under `dependencies`:
//!
//! ```toml,ignore
//! minidom = "*"
//! ```

pub use quick_xml;

pub mod convert;
pub mod element;
pub mod error;
mod namespaces;
pub mod node;
mod prefixes;

#[cfg(test)]
mod tests;

pub use convert::IntoAttributeValue;
pub use element::{Children, ChildrenMut, Element, ElementBuilder};
pub use error::{Error, Result};
pub use namespaces::NSChoice;
pub use node::Node;
