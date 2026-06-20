//! Contains functions for reading data out of a [`Source`].

use super::*;

/// A trait implemented on marker types like [`XYZ`] to allow for strict typing of input kinds.
pub trait InputKind {
    /// The array type that this reader is expecting. Must be one of the types in [`ArrayElement`].
    type Array: ArrayKind;
    /// The associated reader type.
    type Reader: SourceRead<Self>;

    /// Constructs a new reader object from the given [`Accessor`].
    fn new_reader(&self, acc: &Accessor) -> Option<Self::Reader>;
}

/// A trait for [`Source`] readers, which are used via the [`SourceReader`]
/// type returned from [`Source::reader`]. Users can implement this trait
/// on their own type to customize reading, or use [`XYZ`] and [`ST`]
/// for the common cases.
pub trait SourceRead<K: InputKind + ?Sized>: Sized {
    /// The output value. This must be `Clone` because outputs are reused in some mesh kinds,
    /// like [`TriStrips`].
    type Output: Clone;

    /// Given the array data for a single vertex, extract the values into [`Output`](Self::Output).
    /// The length of `data` will be the [`stride`](Accessor::stride) of the accessor.
    ///
    /// Readers should be stateless, because the output of a `load` call may be cached and reused
    /// instead of calling `load` again.
    fn load(&self, data: &[<K::Array as ArrayKind>::Elem]) -> Self::Output;
}

/// The output of [`SourceReader::map`].
#[derive(Copy, Clone, Debug)]
pub struct Map<R, F> {
    reader: R,
    f: F,
}

impl<K: InputKind, R: SourceRead<K>, O: Clone, F: Fn(R::Output) -> O> SourceRead<K> for Map<R, F> {
    type Output = O;

    fn load(&self, data: &[<K::Array as ArrayKind>::Elem]) -> Self::Output {
        (self.f)(self.reader.load(data))
    }
}

impl Accessor {
    /// Get the offset for a parameter with the given name and type.
    pub fn param_offset(&self, name: &str, ty: &str) -> Option<usize> {
        let pos = self
            .param
            .iter()
            .position(|p| p.name.as_deref() == Some(name) && p.ty == ty)?;
        Some(self.offset + pos)
    }
}

/// A `SourceReader` is responsible for preparing a [`Source`] for reading, by doing some up front
/// calulation in `Source::reader` to determine how the data will be sliced. Once it is ready,
/// [`SourceReader::get`] is used to access values from the array.
/// [`SourceReader`] also implements [`Iterator`], so it can be used in for loops.
#[derive(Clone, Debug)]
pub struct SourceReader<'a, K: InputKind, R = <K as InputKind>::Reader> {
    kind: K,
    array: &'a [<K::Array as ArrayKind>::Elem],
    stride: usize,
    len: usize,
    reader: R,
}

impl Source {
    /// Construct a new [`SourceReader`] for this source, which can be used to access elements
    /// of the stored array. The input reader can be a user struct, or one of the pre-built readers
    /// [`XYZReader`] and [`STReader`] by passing the marker types
    /// [`XYZ`] or [`ST`] to [`Source::reader`].
    pub fn reader<K: InputKind>(&self, kind: K) -> Option<SourceReader<'_, K>> {
        let arr = self.array.as_ref()?;
        if matches!((arr.id(), &self.accessor.source), (Some(id), Url::Fragment(s)) if s == id) {
            let array = K::Array::from_array_element(arr)?;
            debug_assert!(self.accessor.count * self.accessor.stride == array.len());
            Some(SourceReader {
                reader: kind.new_reader(&self.accessor)?,
                kind,
                array,
                stride: self.accessor.stride,
                len: self.accessor.count,
            })
        } else {
            None // Reading external arrays is not supported
        }
    }
}

impl<'a, K: InputKind, R: SourceRead<K>> SourceReader<'a, K, R> {
    /// Map a function on a source reader.
    pub fn map_reader<S: SourceRead<K>>(self, f: impl FnOnce(R) -> S) -> SourceReader<'a, K, S> {
        SourceReader {
            kind: self.kind,
            array: self.array,
            stride: self.stride,
            len: self.len,
            reader: f(self.reader),
        }
    }
    /// Map a function on a source reader.
    pub fn map<O: Clone, F: Fn(R::Output) -> O>(self, f: F) -> SourceReader<'a, K, Map<R, F>> {
        self.map_reader(|reader| Map { reader, f })
    }

    /// Return a specified element of the array.
    pub fn get(&self, i: usize) -> R::Output {
        let elems = &self.array[i * self.stride..][..self.stride];
        self.reader.load(elems)
    }

    /// Advances the iterator by `n` elements. See [`Iterator::advance_by`].
    pub fn advance_by(&mut self, n: usize) -> Result<(), usize> {
        if n <= self.len {
            self.array = &self.array[n * self.stride..];
            Ok(())
        } else {
            Err(self.len)
        }
    }

    /// Advances the iterator from the back by `n` elements.
    /// See [`DoubleEndedIterator::advance_back_by`].
    pub fn advance_back_by(&mut self, n: usize) -> Result<(), usize> {
        if n <= self.len {
            self.array = &self.array[..self.array.len() - n * self.stride];
            Ok(())
        } else {
            Err(self.len)
        }
    }
}

impl<'a, K: InputKind, R: SourceRead<K>> ExactSizeIterator for SourceReader<'a, K, R> {
    fn len(&self) -> usize {
        self.len
    }
}

impl<'a, K: InputKind, R: SourceRead<K>> DoubleEndedIterator for SourceReader<'a, K, R> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.len = self.len.checked_sub(1)?;
        let (left, right) = self.array.split_at(self.len);
        self.array = left;
        Some(self.reader.load(right))
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.advance_back_by(n).ok()?;
        self.next_back()
    }
}

impl<'a, K: InputKind, R: SourceRead<K>> Iterator for SourceReader<'a, K, R> {
    type Item = R::Output;

    fn next(&mut self) -> Option<Self::Item> {
        self.len = self.len.checked_sub(1)?;
        let (left, right) = self.array.split_at(self.stride);
        self.array = right;
        Some(self.reader.load(left))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.len, Some(self.len))
    }

    fn count(self) -> usize {
        self.len
    }

    fn last(mut self) -> Option<Self::Item> {
        self.next_back()
    }

    fn nth(&mut self, n: usize) -> Option<Self::Item> {
        self.advance_by(n).ok()?;
        self.next()
    }
}

/// A marker type for inputs with `name="X"`, `"Y"`, `"Z"`.
/// This is used in Collada to store vertex position data, and normal data.
#[derive(Copy, Clone, Debug, Default)]
pub struct XYZ;

impl InputKind for XYZ {
    type Array = FloatArray;
    type Reader = XYZReader;
    fn new_reader(&self, acc: &Accessor) -> Option<Self::Reader> {
        Some(XYZReader {
            x: acc.param_offset("X", "float")?,
            y: acc.param_offset("Y", "float")?,
            z: acc.param_offset("Z", "float")?,
        })
    }
}

/// A reader which accesses fields with `name="X"`, `"Y"`, `"Z"` in the source array.
/// This is used in Collada to store vertex position data, and normal data.
#[derive(Copy, Clone, Debug)]
pub struct XYZReader {
    x: usize,
    y: usize,
    z: usize,
}

impl SourceRead<XYZ> for XYZReader {
    type Output = [f32; 3];
    fn load(&self, data: &[f32]) -> Self::Output {
        [data[self.x], data[self.y], data[self.z]]
    }
}

/// A marker type for inputs with `name="S"`, `"T"`.
/// Collada uses `S` and `T` to denote texture coordinates (usually called "UV"),
/// because `"U"` and `"V"` are used for generic parameters.
#[derive(Copy, Clone, Debug, Default)]
pub struct ST;

impl InputKind for ST {
    type Array = FloatArray;
    type Reader = STReader;
    fn new_reader(&self, acc: &Accessor) -> Option<Self::Reader> {
        Some(STReader {
            s: acc.param_offset("S", "float")?,
            t: acc.param_offset("T", "float")?,
        })
    }
}

/// A reader which accesses fields with `name="S"`, `"T"` in the source array.
/// Collada uses `S` and `T` to denote texture coordinates (usually called "UV"),
/// because `"U"` and `"V"` are used for generic parameters.
#[derive(Copy, Clone, Debug)]
pub struct STReader {
    s: usize,
    t: usize,
}

impl SourceRead<ST> for STReader {
    type Output = [f32; 2];
    fn load(&self, data: &[f32]) -> Self::Output {
        [data[self.s], data[self.t]]
    }
}
