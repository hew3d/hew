//! # Collada parser
//!
//! This is a parser for the Collada (`.dae`) format, used for interchange between 3D renderers
//! and games. Compared to the [`collada`](https://crates.io/crates/collada) crate,
//! this crate attempts to more directly represent the Collada data model, and it is also
//! significantly more complete. It supports both reading and writing.
//!
//! ## Usage
//!
//! The main entry point is the [`Document`] type, which has a [`FromStr`] implementation to convert
//! literal strings / slices, or [`Document::from_file`] to read from a `.dae` file on disk.
//!
//! Collada documents are parsed eagerly, validating everything according to the
//! [COLLADA schema](https://www.khronos.org/files/collada_spec_1_4.pdf).
//! Once parsed, the data structures (structs and enums) can be navigated directly,
//! as all the data structures are public, and reflect the XML schema closely.
//!
//! This library implements only version 1.4.1 of the Collada spec, although it may be expanded
//! in the future. (Please open an issue or PR if you find anything missing from the spec,
//! or if you have a use case for a later version.)
//!
//! ```
//! use std::str::FromStr;
//! use dae_parser::*;
//!
//! let dae_file = r##"<?xml version="1.0" encoding="utf-8"?>
//! <COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
//!   <asset>
//!     <created>1970-01-01T00:00:00Z</created>
//!     <modified>1970-01-01T00:00:00Z</modified>
//!   </asset>
//!   <library_geometries>
//!     <geometry id="Cube-mesh" name="Cube">
//!       <mesh>
//!         <source id="Cube-mesh-positions">
//!           <float_array id="Cube-mesh-positions-array" count="18">
//!             1 1 1 1 -1 1 1 -1 -1 -1 1 1 -1 -1 1 -1 -1 -1
//!           </float_array>
//!           <technique_common>
//!             <accessor source="#Cube-mesh-positions-array" count="6" stride="3">
//!               <param name="X" type="float"/>
//!               <param name="Y" type="float"/>
//!               <param name="Z" type="float"/>
//!             </accessor>
//!           </technique_common>
//!         </source>
//!         <vertices id="Cube-mesh-vertices">
//!           <input semantic="POSITION" source="#Cube-mesh-positions"/>
//!         </vertices>
//!         <triangles material="Material-material" count="4">
//!           <input semantic="VERTEX" source="#Cube-mesh-vertices" offset="0"/>
//!           <p>3 1 0 1 5 2 3 4 1 1 4 5</p>
//!         </triangles>
//!       </mesh>
//!     </geometry>
//!   </library_geometries>
//! </COLLADA>"##;
//!
//! let document = Document::from_str(dae_file).unwrap();
//! let cube = document.local_map::<Geometry>().unwrap().get_str("Cube-mesh").unwrap();
//! let sources_map = document.local_map::<Source>().unwrap();
//! let vertices_map = document.local_map::<Vertices>().unwrap();
//! // sources.get("Cube-mesh-positions").unwrap();
//! assert_eq!(cube.id.as_ref().unwrap(), "Cube-mesh");
//! let mesh = cube.element.as_mesh().unwrap();
//! let tris = mesh.elements[0].as_triangles().unwrap();
//! assert_eq!(
//!     tris.data.as_deref().unwrap(),
//!     &[3, 1, 0, 1, 5, 2, 3, 4, 1, 1, 4, 5]
//! );
//! assert_eq!(tris.inputs[0].semantic, Semantic::Vertex);
//! let vertices = vertices_map.get_raw(&tris.inputs[0].source).unwrap();
//! assert_eq!(vertices.id, "Cube-mesh-vertices");
//! let source = sources_map
//!     .get_raw(&vertices.position_input().source)
//!     .unwrap();
//! assert_eq!(source.id.as_deref(), Some("Cube-mesh-positions"));
//! ```
//! ## License
//!
//! Licensed under either of
//!
//!  * Apache License, Version 2.0
//!    ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
//!  * MIT license
//!    ([LICENSE-MIT](LICENSE-MIT) or <http://opensource.org/licenses/MIT>)
//!
//! at your option.
//!
//! ## Contribution
//!
//! Unless you explicitly state otherwise, any contribution intentionally submitted
//! for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
//! dual licensed as above, without any additional terms or conditions.

#![deny(unsafe_code)]
#![warn(
    missing_docs,
    missing_debug_implementations,
    missing_copy_implementations,
    trivial_casts,
    trivial_numeric_casts,
    unused_import_braces,
    unused_qualifications
)]

#[macro_use]
mod macros;
mod api;
mod core;
mod fx;
mod physics;
mod url;

use std::convert::{TryFrom, TryInto};
use std::fmt::{Debug, Display};
use std::io::{BufRead, BufReader, Write};
use std::ops::Deref;
use std::path::Path;
use std::str::FromStr;

pub use crate::{api::*, core::*, fx::*, physics::*};
use minidom::quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
pub use minidom::Element;
pub use url::Url;

type XReader<R> = minidom::quick_xml::Reader<R>;
type XWriter<R> = minidom::quick_xml::Writer<R>;

/// The main error type used by this library.
#[derive(Debug)]
pub enum Error {
    /// An error during XML parsing.
    Minidom(minidom::Error),
    /// A generic error given by a string.
    Other(&'static str),
    /// A generic error given by a string.
    Str(String),
}

impl From<std::io::Error> for Error {
    fn from(v: std::io::Error) -> Self {
        Self::Minidom(v.into())
    }
}

impl From<minidom::Error> for Error {
    fn from(v: minidom::Error) -> Self {
        Self::Minidom(v)
    }
}

impl From<minidom::quick_xml::Error> for Error {
    fn from(v: minidom::quick_xml::Error) -> Self {
        Self::Minidom(v.into())
    }
}

impl From<&'static str> for Error {
    fn from(v: &'static str) -> Self {
        Self::Other(v)
    }
}

impl From<String> for Error {
    fn from(v: String) -> Self {
        Self::Str(v)
    }
}

type Result<T, E = Error> = std::result::Result<T, E>;

type ElementIter<'a> = std::iter::Peekable<minidom::Children<'a>>;

fn get_text(element: &Element) -> Option<&str> {
    let mut it = element.nodes();
    let text = match it.next() {
        None => "",
        Some(s) => s.as_text()?,
    };
    if it.next().is_some() {
        return None;
    }
    Some(text)
}

fn parse_text(element: &Element) -> Result<String> {
    Ok(get_text(element).ok_or("expecting a text node")?.to_owned())
}

fn parse_array<T: FromStr>(e: &Element) -> Result<Box<[T]>> {
    get_text(e)
        .ok_or("expected text node")?
        .split_ascii_whitespace()
        .map(|s| s.parse())
        .collect::<Result<_, _>>()
        .map_err(|_| "parse error".into())
}

fn parse_array_n<T: FromStr, const N: usize>(element: &Element) -> Result<Box<[T; N]>> {
    Ok(parse_array(element)?
        .try_into()
        .map_err(|_| "unexpected number of elements")?)
}

fn parse_elem<T: FromStr>(e: &Element) -> Result<T> {
    get_text(e)
        .ok_or("expected text node")?
        .parse()
        .map_err(|_| "parse error".into())
}

fn parse_attr<T: FromStr>(attr: Option<&str>) -> Result<Option<T>> {
    Ok(match attr {
        None => None,
        Some(s) => Some(s.parse().map_err(|_| "parse failure")?),
    })
}

fn parse_one<'a, T>(
    name: &str,
    it: &mut impl Iterator<Item = &'a Element>,
    f: impl FnOnce(&'a Element) -> Result<T>,
) -> Result<T> {
    let e = it.next().ok_or_else(|| format!("expected <{}>", name))?;
    if e.name() != name {
        return Err(format!("expected <{}>", name).into());
    }
    f(e)
}

fn parse_one_many<'a, T>(
    it: &mut impl Iterator<Item = &'a Element>,
    f: impl FnOnce(&'a Element) -> Result<Option<T>>,
) -> Result<T> {
    let e = it.next().ok_or("expected element")?;
    Ok(f(e)?.ok_or("expected element")?)
}

fn parse_opt<'a, T>(
    name: &str,
    it: &mut ElementIter<'a>,
    f: impl FnOnce(&'a Element) -> Result<T>,
) -> Result<Option<T>> {
    let mut res = None;
    if let Some(&e) = it.peek() {
        if e.name() == name {
            res = Some(f(e)?);
            it.next();
        }
    }
    Ok(res)
}

fn parse_opt_many<'a, T>(
    it: &mut ElementIter<'a>,
    f: impl FnOnce(&'a Element) -> Result<Option<T>>,
) -> Result<Option<T>> {
    let res = match it.peek() {
        None => None,
        Some(&e) => f(e)?,
    };
    if res.is_some() {
        it.next();
    }
    Ok(res)
}

fn parse_list<'a, T>(
    name: &str,
    it: &mut ElementIter<'a>,
    mut f: impl FnMut(&'a Element) -> Result<T>,
) -> Result<Vec<T>> {
    parse_list_many(it, |e| {
        Ok(if e.name() == name { Some(f(e)?) } else { None })
    })
}

fn finish<'a, T>(t: T, mut it: impl Iterator<Item = &'a Element>) -> Result<T> {
    if let Some(e) = it.next() {
        return Err(format!("unexpected node <{}>", e.name()).into());
    }
    Ok(t)
}

fn parse_list_many<'a, T>(
    it: &mut ElementIter<'a>,
    mut f: impl FnMut(&'a Element) -> Result<Option<T>>,
) -> Result<Vec<T>> {
    let mut res = vec![];
    while let Some(&e) = it.peek() {
        match f(e)? {
            Some(t) => res.push(t),
            None => break,
        }
        it.next();
    }
    Ok(res)
}

fn print_str(s: &str, w: &mut XWriter<impl Write>) -> Result<()> {
    Ok(w.write_event(Event::Text(BytesText::new(s)))?)
}

fn print_elem<T: Display>(elem: &T, w: &mut XWriter<impl Write>) -> Result<()> {
    print_str(&format!("{}", elem), w)
}

#[inline]
fn opt<'a, T, E>(elem: &'a Option<T>, f: impl FnOnce(&'a T) -> Result<(), E>) -> Result<(), E> {
    if let Some(elem) = elem {
        f(elem)?
    }
    Ok(())
}

#[inline]
fn many<'a, T, E>(elem: &'a [T], f: impl FnMut(&'a T) -> Result<(), E>) -> Result<(), E> {
    elem.iter().try_for_each(f)
}

fn arr_to_string<T: Display>(elem: &[T]) -> Option<String> {
    let (e1, rest) = elem.split_first()?;
    let mut s = format!("{}", e1);
    for e in rest {
        use std::fmt::Write;
        write!(s, " {}", e).expect("can't fail")
    }
    Some(s)
}
fn print_arr<T: Display>(elem: &[T], w: &mut XWriter<impl Write>) -> Result<()> {
    opt(&arr_to_string(elem), |s| print_str(s, w))
}

#[doc(hidden)]
#[derive(Debug)]
pub struct ElemBuilder<'a>(BytesStart<'a>);

struct ElemEnd<'a>(BytesEnd<'a>);

impl<'a> ElemBuilder<'a> {
    #[inline]
    fn new(name: &'a str) -> Self {
        Self(BytesStart::new(name))
    }

    fn print_str(name: &'a str, elem: &str, w: &mut XWriter<impl Write>) -> Result<()> {
        let e = Self::new(name).start(w)?;
        print_str(elem, w)?;
        e.end(w)
    }

    fn print<T: Display>(name: &'a str, elem: &T, w: &mut XWriter<impl Write>) -> Result<()> {
        let e = Self::new(name).start(w)?;
        print_elem(elem, w)?;
        e.end(w)
    }

    fn opt_print<T: Display>(
        name: &'a str,
        elem: &Option<T>,
        w: &mut XWriter<impl Write>,
    ) -> Result<()> {
        opt(elem, |e| Self::print(name, e, w))
    }

    fn def_print<T: Display + PartialEq>(
        name: &'a str,
        value: T,
        def: T,
        w: &mut XWriter<impl Write>,
    ) -> Result<()> {
        if value != def {
            Self::print(name, &value, w)?
        }
        Ok(())
    }

    fn print_arr<T: Display>(name: &'a str, elem: &[T], w: &mut XWriter<impl Write>) -> Result<()> {
        let e = Self::new(name).start(w)?;
        print_arr(elem, w)?;
        e.end(w)
    }

    #[inline]
    fn raw_attr(&mut self, key: &str, value: &[u8]) {
        self.0.push_attribute((key.as_bytes(), value));
    }

    #[inline]
    fn attr(&mut self, key: &str, value: &str) {
        self.0.push_attribute((key, value));
    }

    fn opt_attr(&mut self, key: &str, value: &Option<String>) {
        if let Some(value) = value {
            self.attr(key, value)
        }
    }

    #[inline]
    fn print_attr(&mut self, key: &str, value: impl Display) {
        self.0.push_attribute((key, &*format!("{}", value)));
    }

    fn opt_print_attr(&mut self, key: &str, value: &Option<impl Display>) {
        if let Some(value) = value {
            self.0.push_attribute((key, &*format!("{}", value)));
        }
    }

    fn def_print_attr<T: Display + PartialEq>(&mut self, key: &str, value: T, def: T) {
        if value != def {
            self.print_attr(key, value)
        }
    }

    fn start(self, w: &mut XWriter<impl Write>) -> Result<ElemEnd<'static>> {
        let end = self.0.to_end().into_owned();
        w.write_event(Event::Start(self.0))?;
        Ok(ElemEnd(end))
    }

    fn end(self, w: &mut XWriter<impl Write>) -> Result<()> {
        Ok(w.write_event(Event::Empty(self.0))?)
    }
}

impl<'a> ElemEnd<'a> {
    fn end(self, w: &mut XWriter<impl Write>) -> Result<()> {
        Ok(w.write_event(Event::End(self.0))?)
    }
}

use private::{XNode, XNodeWrite};
pub(crate) mod private {
    use super::*;

    /// A common trait for all data structures that represent an XML element.
    pub trait XNode: XNodeWrite + Sized {
        /// The name of the XML element.
        const NAME: &'static str;

        /// Parse an XML element into this type. In most cases, the parser will require with a
        /// `debug_assert` that the element to parse has name [`Self::NAME`].
        fn parse(element: &Element) -> Result<Self>;

        /// Parse an XML element and return the data structure in a `Box`.
        /// This can be faster in some cases when the data structure is large.
        fn parse_box(element: &Element) -> Result<Box<Self>> {
            Self::parse(element).map(Box::new)
        }

        /// Parse a single required element from the given element iterator.
        fn parse_one<'a>(it: &mut impl Iterator<Item = &'a Element>) -> Result<Self> {
            parse_one(Self::NAME, it, Self::parse)
        }

        /// Parse an optional element from the given element iterator, using [`Self::NAME`] to
        /// determine if it is the correct type.
        fn parse_opt(it: &mut ElementIter<'_>) -> Result<Option<Self>> {
            parse_opt(Self::NAME, it, Self::parse)
        }

        /// Parse an optional boxed element from the given element iterator, using [`Self::NAME`] to
        /// determine if it is the correct type.
        fn parse_opt_box(it: &mut ElementIter<'_>) -> Result<Option<Box<Self>>> {
            parse_opt(Self::NAME, it, Self::parse_box)
        }

        /// Parse a list of elements from the given element iterator,
        /// as long as it continues yielding elements of name [`Self::NAME`].
        fn parse_list(it: &mut ElementIter<'_>) -> Result<Vec<Self>> {
            parse_list(Self::NAME, it, Self::parse)
        }

        /// Parse a list of elements from the given element iterator,
        /// as long as it continues yielding elements of name [`Self::NAME`],
        /// and assert that the resulting list has length at least `N`.
        fn parse_list_n<const N: usize>(it: &mut ElementIter<'_>) -> Result<Vec<Self>> {
            let arr = parse_list(Self::NAME, it, Self::parse)?;
            if arr.len() < N {
                return Err(format!("parse error: expected {} {} elements", N, Self::NAME).into());
            }
            Ok(arr)
        }

        /// Create a new element builder.
        #[doc(hidden)]
        fn elem<'a>() -> ElemBuilder<'a> {
            ElemBuilder::new(Self::NAME)
        }
    }

    pub trait XNodeWrite {
        /// Writes the node to the given [`quick_xml::Writer`](minidom::quick_xml::Writer).
        fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()>;
    }
}

impl<T: XNodeWrite> XNodeWrite for Box<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        (**self).write_to(w)
    }
}

impl<T: XNodeWrite> XNodeWrite for Option<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        opt(self, |e| e.write_to(w))
    }
}

impl<T: XNodeWrite> XNodeWrite for Vec<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        many(self, |e| e.write_to(w))
    }
}

impl XNodeWrite for Element {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        use std::{cell::RefCell, collections::BTreeMap};
        thread_local! {
            static COLLADA_PREFIX: RefCell<BTreeMap<Option<String>, String>> =
                const { RefCell::new(BTreeMap::new()) };
        }
        COLLADA_PREFIX.with(|pfxs| {
            let mut pfxs = pfxs.borrow_mut();
            if pfxs.is_empty() {
                pfxs.insert(None, "http://www.collada.org/2005/11/COLLADASchema".into());
            }
            Ok(self.write_to_inner(w, &mut pfxs)?)
        })
    }
}

impl XNodeWrite for () {
    fn write_to<W: Write>(&self, _: &mut XWriter<W>) -> Result<()> {
        Ok(())
    }
}

/// A Collada document. Represents the `<COLLADA>` root node.
///
/// This is the main entry point for parsing.
#[derive(Clone, Debug)]
pub struct Document {
    /// Metadata about the origin of the document
    pub asset: Asset,
    /// The main content, organized into a list of "libraries".
    pub library: Vec<LibraryElement>,
    /// The actual scene being described, which references / instantiates
    /// objects in the libraries.
    pub scene: Option<Scene>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl FromStr for Document {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::try_from(s.as_bytes())
    }
}

impl TryFrom<&str> for Document {
    type Error = Error;
    fn try_from(s: &str) -> Result<Self> {
        Self::from_str(s)
    }
}

impl TryFrom<&[u8]> for Document {
    type Error = Error;
    fn try_from(s: &[u8]) -> Result<Self> {
        Self::from_reader(std::io::Cursor::new(s))
    }
}

impl Document {
    /// Constructs a new empty [`Document`].
    pub fn new(asset: Asset) -> Self {
        Self {
            asset,
            library: vec![],
            scene: None,
            extra: vec![],
        }
    }

    /// Constructs a new empty [`Document`] with creation date set to the current date/time.
    pub fn create_now() -> Self {
        Self::new(Asset::create_now())
    }

    /// Add a new library element with the given items.
    pub fn push_library<T: ParseLibrary>(&mut self, items: Vec<T>) {
        self.library.push(T::mk_element(Library::new(items)))
    }

    /// Constructs a [`Document`] from a file.
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::from_reader(BufReader::new(std::fs::File::open(path)?))
    }

    /// Constructs a [`Document`] from any [`BufRead`] reader.
    /// Use [`BufReader`] to construct a [`BufRead`] if you only have a
    /// [`Read`](std::io::Read) instance.
    pub fn from_reader<R: BufRead>(reader: R) -> Result<Self> {
        Self::from_xml_reader(&mut XReader::from_reader(reader))
    }

    /// Constructs a [`Document`] from a
    /// [`quick_xml::Reader`](minidom::quick_xml::Reader).
    pub fn from_xml_reader<R: BufRead>(reader: &mut XReader<R>) -> Result<Self> {
        let root = Element::from_reader(reader)?;
        Self::parse(&root)
    }

    /// Write the document to a writer.
    pub fn write_to<W: Write>(&self, w: W) -> Result<()> {
        XNodeWrite::write_to(self, &mut XWriter::new_with_indent(w, b' ', 2))
    }
}

impl XNode for Document {
    const NAME: &'static str = "COLLADA";
    /// Parse a [`Document`] from a `<COLLADA>` [`Element`].
    fn parse(element: &Element) -> Result<Self> {
        if element.name() != Self::NAME {
            return Err("Expected COLLADA root node".into());
        }
        if element.attr("version") != Some("1.4.1") {
            return Err("Unsupported COLLADA version".into());
        }
        let mut it = element.children().peekable();
        Ok(Document {
            asset: Asset::parse_one(&mut it)?,
            library: parse_list_many(&mut it, LibraryElement::parse)?,
            scene: Scene::parse_opt(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Document {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        w.write_event(Event::Decl(BytesDecl::new("1.0", Some("utf-8"), None)))?;
        let mut e = Self::elem();
        e.raw_attr("xmlns", b"http://www.collada.org/2005/11/COLLADASchema");
        e.raw_attr("version", b"1.4.1");
        e.raw_attr("xmlns:xsi", b"http://www.w3.org/2001/XMLSchema-instance");
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.library.write_to(w)?;
        self.scene.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl CollectLocalMaps for Document {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        self.library.collect_local_maps(maps);
    }
}
