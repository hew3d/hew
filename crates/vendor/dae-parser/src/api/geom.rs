//! Contains functions for reading vertex and other data out of the geometry types, like [`Mesh`].

use super::*;
use crate::source::{InputKind, SourceReader, ST, XYZ};

/// This trait abstracts over types that can be used to resolve URL references.
/// It is implemented for [`LocalMap`] and [`LocalMaps`], and user types can also implement
/// this trait to provide external URL resolution.
pub trait UrlResolver<'a, T> {
    /// The error type of the resolver.
    type Error;

    /// Resolve an individual URL.
    fn resolve(&self, url: &UrlRef<T>) -> Result<&'a T, Self::Error>;
}

impl<'a, T> UrlResolver<'a, T> for LocalMap<'a, T> {
    type Error = ();
    fn resolve(&self, url: &UrlRef<T>) -> Result<&'a T, Self::Error> {
        self.get(url).ok_or(())
    }
}

impl<'a, T: HasId> UrlResolver<'a, T> for LocalMaps<'a> {
    type Error = ();
    fn resolve(&self, url: &UrlRef<T>) -> Result<&'a T, Self::Error> {
        self.get(url).ok_or(())
    }
}

/// The result of reading a [`Vertices`] and resolving the references in it,
/// to produce an object that can be used to translate primitive lists (see [`Importer`]).
#[derive(Clone, Debug, Default)]
pub struct VertexImporter<'a> {
    position: Option<SourceReader<'a, XYZ>>,
    normal: Option<SourceReader<'a, XYZ>>,
    texcoord: Option<SourceReader<'a, ST>>,
}

impl<'a> VertexImporter<'a> {
    /// Get the importer for position data.
    pub fn position_importer(&self) -> Option<&SourceReader<'a, XYZ>> {
        self.position.as_ref()
    }

    /// Get the importer for normal data.
    pub fn normal_importer(&self) -> Option<&SourceReader<'a, XYZ>> {
        self.normal.as_ref()
    }

    /// Get the importer for texture coordinate data.
    pub fn texcoord_importer(&self) -> Option<&SourceReader<'a, ST>> {
        self.texcoord.as_ref()
    }
}

fn load<'a, K: InputKind + Default, R: UrlResolver<'a, Source>>(
    res: &R,
    input: &Input,
) -> Result<SourceReader<'a, K>, R::Error> {
    let src = res.resolve(input.source_as_source())?;
    Ok(src.reader(K::default()).unwrap())
}

impl Vertices {
    /// Construct a [`VertexImporter`] from the [`Vertices`] data. It requires a [`UrlResolver`]
    /// to find the source arrays (which are usually but not always stored in the `Mesh` itself).
    pub fn importer<'a, R: UrlResolver<'a, Source>>(
        &'a self,
        res: &R,
    ) -> Result<VertexImporter<'a>, R::Error> {
        let mut imp = VertexImporter::default();
        for input in &self.inputs {
            match input.semantic {
                Semantic::Position => imp.position = Some(load(res, input)?),
                Semantic::Normal => imp.normal = Some(load(res, input)?),
                Semantic::TexCoord => imp.texcoord = Some(load(res, input)?),
                _ => unimplemented!(),
            }
        }
        Ok(imp)
    }
}

#[derive(Clone, Debug)]
enum Instruction<'a> {
    Normal(SourceReader<'a, XYZ>),
    TexCoord(SourceReader<'a, ST>, Option<u32>),
    // TODO: implement more kinds of data
}

/// A completed impporter, which is used to translate primitive lists.
/// Created with [`Geom::importer`],
#[derive(Clone, Debug, Default)]
pub struct Importer<'a> {
    vimp: VertexImporter<'a>,
    vtx_offset: usize,
    stride: usize,
    insts: Vec<(usize, Instruction<'a>)>,
}

impl<T> Geom<T> {
    /// Construct an [`Importer`] from a [`VertexImporter`] and a [`UrlResolver`].
    pub fn importer<'a, R: UrlResolver<'a, Source>>(
        &'a self,
        res: &R,
        vimp: VertexImporter<'a>,
    ) -> Result<Importer<'a>, R::Error> {
        let mut insts = vec![];
        let vtx_offset = self
            .inputs
            .iter()
            .find(|p| p.semantic == Semantic::Vertex)
            .expect("no VERTEX input found")
            .offset as usize;
        for i in &*self.inputs {
            match i.semantic {
                Semantic::Normal => {
                    insts.push((i.offset as usize, Instruction::Normal(load(res, i)?)))
                }
                Semantic::TexCoord => insts.push((
                    i.offset as usize,
                    Instruction::TexCoord(load(res, i)?, i.set),
                )),
                _ => {}
            }
        }
        Ok(Importer {
            vimp,
            vtx_offset,
            stride: self.inputs.stride,
            insts,
        })
    }
}

/// A trait, to be implemented by user types, to describe how to construct a vertex object
/// from the stored data.
/// The context `C` is an arbitrary type that can be used for getting
/// (or modifying, with interior mutability) additional data to create the object.
pub trait VertexLoad<'a, C: ?Sized = ()>: Clone {
    /// Construct a new vertex using only position data.
    ///
    /// The position `index` is provided as is, but the intended use is to call `reader.get(i)`
    /// and use the resulting `[f32; 3]` as the position.
    fn position(ctx: &C, reader: &SourceReader<'a, XYZ>, index: u32) -> Self;

    /// Add normal data to a vertex.
    ///
    /// The position `index` is provided as is, but the intended use is to call `reader.get(i)`
    /// and use the resulting `[f32; 3]` as the normal.
    fn add_normal(&mut self, ctx: &C, reader: &SourceReader<'a, XYZ>, index: u32);

    /// Add texture coordinate data to a vertex.
    ///
    /// The position `index` is provided as is, but the intended use is to call `reader.get(i)`
    /// and use the resulting `[f32; 2]` as the u-v data.
    fn add_texcoord(
        &mut self,
        ctx: &C,
        reader: &SourceReader<'a, ST>,
        index: u32,
        set: Option<u32>,
    );
}

impl<'a> Importer<'a> {
    /// Construct a new vertex from a slice of the index data.
    /// This is a low level function; prefer [`read`](Self::read)
    /// and the functions on the [`ArrayIter`] type.
    pub fn build_vertex<C: ?Sized, V: VertexLoad<'a, C>>(&self, ctx: &C, data: &[u32]) -> V {
        let vtx_data = data[self.vtx_offset];
        let mut vtx = V::position(ctx, self.vimp.position.as_ref().unwrap(), vtx_data);
        if let Some(ref reader) = self.vimp.normal {
            vtx.add_normal(ctx, reader, vtx_data)
        }
        if let Some(ref reader) = self.vimp.texcoord {
            vtx.add_texcoord(ctx, reader, vtx_data, None)
        }
        for inst in &self.insts {
            match *inst {
                (off, Instruction::Normal(ref reader)) => vtx.add_normal(ctx, reader, data[off]),
                (off, Instruction::TexCoord(ref reader, set)) => {
                    vtx.add_texcoord(ctx, reader, data[off], set)
                }
            }
        }
        vtx
    }

    /// Get the importer for normal data.
    pub fn normal_importer(&self) -> Option<&SourceReader<'a, XYZ>> {
        let mut importer = self.vimp.normal_importer();
        for inst in &self.insts {
            if let Instruction::Normal(imp) = &inst.1 {
                importer = Some(imp)
            }
        }
        importer
    }

    /// Get the importer for texture coordinate data.
    pub fn texcoord_importer(&self, set: u32) -> Option<&SourceReader<'a, ST>> {
        let mut importer = self.vimp.texcoord_importer();
        for inst in &self.insts {
            if let Instruction::TexCoord(imp, i) = &inst.1 {
                if i.map_or(true, |i| i == set) {
                    importer = Some(imp)
                }
            }
        }
        importer
    }

    /// Construct a new vertex iterator from some user context `C` (can be `()`),
    /// and an array coming from one of the `prim` fields:
    /// * [`LineGeom::prim`]
    /// * the elements of [`LineStripGeom::prim`]
    /// * [`PolygonHole::verts`] and the elements of [`PolygonHole::hole`]
    /// * [`PolyListGeom::prim`]
    /// * [`TriangleGeom::prim`]
    /// * the elements of [`TriFanGeom::prim`]
    /// * the elements of [`TriStripGeom::prim`]
    pub fn read<'b, C: ?Sized, V: VertexLoad<'a, C>>(
        &'b self,
        ctx: &'b C,
        array: &'a [u32],
    ) -> ArrayIter<'a, 'b, C, V> {
        assert!(self.stride != 0);
        let len = array.len() / self.stride;
        assert!(len * self.stride == array.len());
        ArrayIter {
            imp: self,
            ctx,
            len,
            array,
            _mark: PhantomData,
        }
    }
}

/// An iterator / accessor for retrieving vertex objects from a stored `u32` array.
#[derive(Clone, Debug)]
pub struct ArrayIter<'a, 'b, C: ?Sized, V> {
    imp: &'b Importer<'a>,
    ctx: &'b C,
    len: usize,
    array: &'a [u32],
    _mark: PhantomData<V>,
}

impl<'a, 'b, C: ?Sized, V: VertexLoad<'a, C>> ArrayIter<'a, 'b, C, V> {
    /// Get the index slice at a vertex index.
    /// This is a low level function, prefer [`get`](Self::get), but this can be used as input to
    /// [`Importer::build_vertex`].
    pub fn slice(&self, i: usize) -> &'a [u32] {
        &self.array[i * self.imp.stride..][..self.imp.stride]
    }

    /// Return a vertex object by index.
    pub fn get(&self, i: usize) -> V {
        self.imp.build_vertex(self.ctx, self.slice(i))
    }

    /// Advances the iterator by `n` elements. See [`Iterator::advance_by`].
    pub fn advance_by(&mut self, n: usize) -> Result<(), usize> {
        if n <= self.len {
            self.array = &self.array[n * self.imp.stride..];
            Ok(())
        } else {
            Err(self.len)
        }
    }

    /// Advances the iterator from the back by `n` elements.
    /// See [`DoubleEndedIterator::advance_back_by`].
    pub fn advance_back_by(&mut self, n: usize) -> Result<(), usize> {
        if n <= self.len {
            self.array = &self.array[..self.array.len() - n * self.imp.stride];
            Ok(())
        } else {
            Err(self.len)
        }
    }
}

impl<'a, 'b, C: ?Sized, V: VertexLoad<'a, C>> ExactSizeIterator for ArrayIter<'a, 'b, C, V> {
    fn len(&self) -> usize {
        self.len
    }
}

impl<'a, 'b, C: ?Sized, V: VertexLoad<'a, C>> DoubleEndedIterator for ArrayIter<'a, 'b, C, V> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.len = self.len.checked_sub(1)?;
        let (left, right) = self.array.split_at(self.len);
        self.array = left;
        Some(self.imp.build_vertex(self.ctx, right))
    }

    fn nth_back(&mut self, n: usize) -> Option<Self::Item> {
        self.advance_back_by(n).ok()?;
        self.next_back()
    }
}

impl<'a, 'b, C: ?Sized, V: VertexLoad<'a, C>> Iterator for ArrayIter<'a, 'b, C, V> {
    type Item = V;

    fn next(&mut self) -> Option<Self::Item> {
        self.len = self.len.checked_sub(1)?;
        let (left, right) = self.array.split_at(self.imp.stride);
        self.array = right;
        Some(self.imp.build_vertex(self.ctx, left))
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
