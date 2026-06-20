pub mod geom;
pub mod local_map;
pub mod source;

use ref_cast::RefCast;
use std::{collections::HashMap, marker::PhantomData, ops::Index};

use super::*;
pub(crate) use local_map::{CollectLocalMaps, HasId, Traversable};
pub use local_map::{LocalMap, LocalMaps};

/// A wrapper around a base type `A` to indicate that the value is pointing to
/// a value of type `T`.
///
/// This is not a strict type safety barrier; it is possible to convert a `Ref`
/// to and from its raw version. However this can help in API documentation,
/// as well as to assist type inference in functions like [`LocalMap::get`].
#[derive(Copy, Clone, RefCast)]
#[repr(transparent)]
pub struct Ref<A, T: ?Sized> {
    /// The underlying storage or "raw" value.
    pub val: A,
    _marker: PhantomData<T>,
}

impl<A: Display, T: ?Sized> Display for Ref<A, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.val.fmt(f)
    }
}

impl<A: Debug, T: ?Sized> Debug for Ref<A, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.val.fmt(f)
    }
}

/// A strongly typed URL referencing a `T`.
pub type UrlRef<T> = Ref<Url, T>;

/// A strongly typed string referencing a `T`.
pub type NameRef<T> = Ref<String, T>;

impl<A, T: ?Sized> Deref for Ref<A, T> {
    type Target = A;

    fn deref(&self) -> &Self::Target {
        &self.val
    }
}

impl<A, T: ?Sized> Ref<A, T> {
    /// Construct a new `Ref` by wrapping a raw value.
    pub fn new(val: A) -> Self {
        Self {
            val,
            _marker: PhantomData,
        }
    }
}

impl<A: FromStr, T: ?Sized> FromStr for Ref<A, T> {
    type Err = A::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        A::from_str(s).map(Ref::new)
    }
}

impl Document {
    /// Returns an iterator over `Library<T>` elements.
    pub fn library_iter<T>(&self) -> LibraryIter<'_, T> {
        LibraryIter {
            iter: self.library.iter(),
            _marker: PhantomData,
        }
    }
    /// Returns an iterator over all elements of type `T`.
    pub fn iter<T>(&self) -> ItemIter<'_, T> {
        ItemIter {
            iter: self.library_iter(),
            item: [].iter(),
        }
    }
}

/// An iterator over all `Library<T>` elements for a particular `T`,
/// returned by [`Document::library_iter`].
#[derive(Debug)]
pub struct LibraryIter<'a, T> {
    iter: std::slice::Iter<'a, LibraryElement>,
    _marker: PhantomData<T>,
}

impl<'a, T: ParseLibrary + 'a> Iterator for LibraryIter<'a, T> {
    type Item = &'a Library<T>;

    fn next(&mut self) -> Option<Self::Item> {
        for lib in &mut self.iter {
            if let Some(lib) = T::extract_element(lib) {
                return Some(lib);
            }
        }
        None
    }
}

/// An iterator over all `T` elements for a `ParseLibrary` type `T`,
/// returned by [`Document::iter`].
#[derive(Debug)]
pub struct ItemIter<'a, T> {
    iter: LibraryIter<'a, T>,
    item: std::slice::Iter<'a, T>,
}

impl<'a, T: ParseLibrary + 'a> Iterator for ItemIter<'a, T> {
    type Item = &'a T;
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(e) = self.item.next() {
                return Some(e);
            }
            if let Some(lib) = self.iter.next() {
                self.item = lib.items.iter();
            } else {
                return None;
            }
        }
    }
}
