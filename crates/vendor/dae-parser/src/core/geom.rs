use crate::*;

/// Describes the visual shape and appearance of an object in a scene.
#[derive(Clone, Debug)]
pub struct Geometry {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// The element that describes geometric data.
    pub element: GeometryElement,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Geometry {
    /// Construct a new `Geometry` given an `id` and the underlying element data.
    pub fn new(id: impl Into<String>, element: GeometryElement) -> Self {
        Self {
            id: Some(id.into()),
            name: None,
            asset: None,
            element,
            extra: vec![],
        }
    }

    /// Construct a new `Geometry` containing a [`Mesh`].
    pub fn new_mesh(
        id: impl Into<String>,
        sources: Vec<Source>,
        vertices: Vertices,
        elements: Vec<Primitive>,
    ) -> Self {
        Self::new(id, Mesh::new(sources, vertices, elements).into())
    }
}

impl XNode for Geometry {
    const NAME: &'static str = "geometry";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Geometry {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            element: parse_one_many(&mut it, GeometryElement::parse)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Geometry {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.element.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl CollectLocalMaps for Geometry {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        maps.insert(self);
        self.element.collect_local_maps(maps);
    }
}

/// Extra data associated to [`Instance`]<[`Geometry`]>.
#[derive(Clone, Debug, Default)]
pub struct InstanceGeometryData {
    /// Binds material symbols to material instances. This allows a
    /// single geometry to be instantiated into a scene multiple times
    /// each with a different appearance.
    pub bind_material: Option<BindMaterial>,
}

impl XNodeWrite for InstanceGeometryData {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.bind_material.write_to(w)
    }
}

impl Instantiate for Geometry {
    const INSTANCE: &'static str = "instance_geometry";
    type Data = InstanceGeometryData;
    fn parse_data(_: &Element, it: &mut ElementIter<'_>) -> Result<Self::Data> {
        Ok(InstanceGeometryData {
            bind_material: BindMaterial::parse_opt(it)?,
        })
    }
    fn is_empty(data: &Self::Data) -> bool {
        data.bind_material.is_none()
    }
}

impl Instance<Geometry> {
    /// Get the list of [`InstanceMaterial`]s bound in this [`Instance<Geometry>`].
    pub fn instance_materials(&self) -> &[InstanceMaterial] {
        match self.data.bind_material {
            Some(ref m) => &m.instance_material,
            None => &[],
        }
    }

    /// Look up an [`InstanceMaterial`] by symbol name.
    pub fn get_instance_material(&self, symbol: &str) -> Option<&InstanceMaterial> {
        let bm = self.data.bind_material.as_ref()?;
        bm.instance_material.iter().find(|mat| mat.symbol == symbol)
    }
}

/// An element that describes geometric data.
#[derive(Clone, Debug)]
pub enum GeometryElement {
    /// The parameter is a URI string of a `Geometry`, and this element
    /// describes the convex hull of the specified mesh.
    ConvexHullOf(Url),
    /// A mesh or convex mesh.
    Mesh(Mesh),
    /// A multisegment spline.
    Spline(Spline),
}

impl From<Spline> for GeometryElement {
    fn from(v: Spline) -> Self {
        Self::Spline(v)
    }
}

impl From<Mesh> for GeometryElement {
    fn from(v: Mesh) -> Self {
        Self::Mesh(v)
    }
}

impl CollectLocalMaps for GeometryElement {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        match self {
            GeometryElement::ConvexHullOf(_) => {}
            GeometryElement::Mesh(mesh) => mesh.sources.collect_local_maps(maps),
            GeometryElement::Spline(spline) => spline.sources.collect_local_maps(maps),
        }
    }
}

impl GeometryElement {
    /// Parse a [`GeometryElement`] from an XML element.
    pub fn parse(element: &Element) -> Result<Option<Self>> {
        Ok(Some(match element.name() {
            Mesh::CONVEX => Mesh::parse_convex(element)?,
            Mesh::NAME => GeometryElement::Mesh(Mesh::parse(false, element)?),
            Spline::NAME => GeometryElement::Spline(Spline::parse(element)?),
            _ => return Ok(None),
        }))
    }

    /// The `sources` field, which gives the list of [`Source`] elements on this element.
    pub fn sources(&self) -> &[Source] {
        match self {
            GeometryElement::ConvexHullOf(_) => &[],
            GeometryElement::Mesh(mesh) => &mesh.sources,
            GeometryElement::Spline(spline) => &spline.sources,
        }
    }

    /// Returns this element if it is a `<mesh>`.
    pub fn as_mesh(&self) -> Option<&Mesh> {
        match self {
            GeometryElement::Mesh(mesh) if !mesh.convex => Some(mesh),
            _ => None,
        }
    }
}

impl XNodeWrite for GeometryElement {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            GeometryElement::ConvexHullOf(s) => {
                let mut e = ElemBuilder::new(Mesh::CONVEX);
                e.print_attr("convex_hull_of", s);
                e.end(w)
            }
            GeometryElement::Mesh(e) => e.write_to(w),
            GeometryElement::Spline(e) => e.write_to(w),
        }
    }
}

/// Describes basic geometric meshes using vertex and primitive information.
#[derive(Clone, Debug)]
pub struct Mesh {
    /// If true, this is a `<convex_mesh>` element.
    /// Both elements have the same structure otherwise.
    pub convex: bool,
    /// Provides the bulk of the mesh’s vertex data.
    pub sources: Vec<Source>,
    /// Describes the mesh-vertex attributes and establishes their topological identity.
    pub vertices: Option<Vertices>,
    /// Geometric primitives, which assemble values from the
    /// inputs into vertex attribute data.
    pub elements: Vec<Primitive>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Mesh {
    /// Construct a new `Mesh` from vertices and elements.
    pub fn new(sources: Vec<Source>, vertices: Vertices, elements: Vec<Primitive>) -> Self {
        assert!(!sources.is_empty());
        Self {
            convex: false,
            sources,
            vertices: Some(vertices),
            elements,
            extra: vec![],
        }
    }

    /// Construct a new convex `Mesh` from vertices and elements.
    pub fn new_convex(sources: Vec<Source>, vertices: Vertices, elements: Vec<Primitive>) -> Self {
        assert!(!sources.is_empty());
        Self {
            convex: true,
            sources,
            vertices: Some(vertices),
            elements,
            extra: vec![],
        }
    }

    /// The name of the XML node: `convex_mesh`
    pub const CONVEX: &'static str = "convex_mesh";

    /// Parse a `<convex_mesh>` XML element.
    pub fn parse_convex(element: &Element) -> Result<GeometryElement> {
        debug_assert_eq!(element.name(), Self::CONVEX);
        if let Some(s) = parse_attr(element.attr("convex_hull_of"))? {
            return Ok(GeometryElement::ConvexHullOf(s));
        }
        Ok(GeometryElement::Mesh(Mesh::parse(true, element)?))
    }

    /// Parse a [`Mesh`] from an XML element of type `<convex_mesh>` or `<mesh>`.
    pub fn parse(convex: bool, element: &Element) -> Result<Self> {
        debug_assert_eq!(
            element.name(),
            if convex { Self::CONVEX } else { Self::NAME }
        );
        let mut it = element.children().peekable();
        Ok(Mesh {
            convex,
            sources: Source::parse_list_n::<1>(&mut it)?,
            vertices: Vertices::parse_opt(&mut it)?,
            elements: parse_list_many(&mut it, Primitive::parse)?,
            extra: Extra::parse_many(it)?,
        })
    }

    fn write_inner<W: Write>(&self, e: ElemBuilder, w: &mut XWriter<W>) -> Result<()> {
        let e = e.start(w)?;
        self.sources.write_to(w)?;
        self.vertices.write_to(w)?;
        self.elements.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl XNode for Mesh {
    const NAME: &'static str = "mesh";
    fn parse(element: &Element) -> Result<Self> {
        Self::parse(false, element)
    }
}

impl XNodeWrite for Mesh {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.write_inner(Self::elem(), w)
    }
}

/// Declares the attributes and identity of mesh-vertices.
#[derive(Clone, Debug)]
pub struct Vertices {
    /// A text string containing the unique identifier of the element.
    /// This value must be unique within the document.
    pub id: String,
    /// The text string name of this element.
    pub name: Option<String>,
    /// The list of inputs.
    pub inputs: Vec<Input>,
    /// The index into `inputs` for the [`Semantic::Position`] input (which must exist).
    pub position: usize,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for Vertices {
    const NAME: &'static str = "vertices";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let inputs = Input::parse_list(&mut it)?;
        Ok(Vertices {
            id: element.attr("id").ok_or("missing 'id' attr")?.into(),
            name: element.attr("name").map(Into::into),
            position: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Position)
                .ok_or("vertices: missing POSITION input")?,
            inputs,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Vertices {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.inputs.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl Traversable for Vertices {
    fn traverse<'a, E>(
        doc: &'a Document,
        mut f: impl FnMut(&'a Self) -> Result<(), E>,
    ) -> Result<(), E>
    where
        Self: 'a,
    {
        for lib in doc.iter::<Geometry>() {
            if let GeometryElement::Mesh(Mesh {
                vertices: Some(v), ..
            }) = &lib.element
            {
                f(v)?
            }
        }
        Ok(())
    }
}

impl Vertices {
    /// Construct a new `Vertices` object with the given inputs.
    /// * One of the inputs must have [`Semantic::Position`].
    pub fn new(id: impl Into<String>, inputs: Vec<Input>) -> Self {
        Self {
            id: id.into(),
            name: None,
            position: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Position)
                .expect("vertices: missing POSITION input"),
            inputs,
            extra: vec![],
        }
    }

    /// The input with [`Semantic::Position`].
    pub fn position_input(&self) -> &Input {
        &self.inputs[self.position]
    }
}

/// The common data for the geometry types:
///
/// * [`Lines`]` = Geom<`[`LineGeom`]`>`
/// * [`LineStrips`]` = Geom<`[`LineStripGeom`]`>`
/// * [`Polygons`]` = Geom<`[`PolygonGeom`]`>`
/// * [`PolyList`]` = Geom<`[`PolyListGeom`]`>`
/// * [`Triangles`]` = Geom<`[`TriangleGeom`]`>`
/// * [`TriFans`]` = Geom<`[`TriFanGeom`]`>`
/// * [`TriStrips`]` = Geom<`[`TriStripGeom`]`>`
#[derive(Clone, Default, Debug)]
pub struct Geom<T> {
    /// The text string name of this element.
    pub name: Option<String>,
    /// Declares a symbol for a material.
    /// This symbol is bound to a material at the time of instantiation;
    /// see [`Instance<Geometry>`] and [`BindMaterial`].
    /// If not specified then the lighting and shading results are application defined.
    pub material: Option<String>,
    /// The number of line/triangle/polygon primitives.
    pub count: usize,
    /// The vertex attribute access information.
    pub inputs: InputList,
    /// The specific data for the geometry element.
    pub data: T,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl<T: ParseGeom> Geom<T> {
    fn new_geom(material: Option<String>, inputs: InputList, count: usize, data: T) -> Self {
        Self {
            name: None,
            material,
            count,
            inputs,
            data,
            extra: vec![],
        }
    }
}

pub(crate) use private::ParseGeom;
pub(crate) mod private {
    use super::*;
    /// The trait for types that can appear in a [`Geom<T>`].
    pub trait ParseGeom: XNodeWrite + Default {
        /// The name of the element for the enclosing `Geom`, for example
        /// `"lines"` for [`LineGeom`].
        const NAME: &'static str;

        /// Parse the data from an element iterator.
        fn parse(it: &mut ElementIter<'_>) -> Result<Self>;

        /// Perform custom validation on the resulting [`Geom<T>`] before yielding it.
        fn validate(_: &Geom<Self>) -> Result<()>;
    }
}

impl<T: ParseGeom> XNode for Geom<T> {
    const NAME: &'static str = T::NAME;

    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Geom {
            name: element.attr("name").map(Into::into),
            material: element.attr("material").map(Into::into),
            count: parse_attr(element.attr("count"))?.ok_or("expected 'count' attr")?,
            inputs: InputList::parse::<0>(&mut it)?,
            data: T::parse(&mut it)?,
            extra: Extra::parse_many(it)?,
        };
        T::validate(&res)?;
        Ok(res)
    }
}

impl<T: ParseGeom> XNodeWrite for Geom<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("name", &self.name);
        e.opt_attr("material", &self.material);
        e.print_attr("count", self.count);
        let e = e.start(w)?;
        self.inputs.write_to(w)?;
        self.data.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

macro_rules! mk_primitive {
    ($($(#[$doc:meta])* $name:ident($as:ident),)*) => {
        /// A collection of primitive elements.
        #[derive(Clone, Debug)]
        pub enum Primitive {
            $($(#[$doc])* $name($name),)*
        }

        $(
            impl From<$name> for Primitive {
                fn from(val: $name) -> Self {
                    Self::$name(val)
                }
            }
        )*

        impl Primitive {
            /// Parse a [`Primitive`] from an XML element.
            pub fn parse(e: &Element) -> Result<Option<Self>> {
                Ok(Some(match e.name() {
                    $($name::NAME => Self::$name(Geom::parse(e)?),)*
                    _ => return Ok(None),
                }))
            }

            $(
                /// An accessor for the variant.
                pub fn $as(&self) -> Option<&$name> {
                    if let Self::$name(x) = self {
                        Some(x)
                    } else {
                        None
                    }
                }
            )*
        }

        impl XNodeWrite for Primitive {
            fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
                match self {
                    $(Self::$name(e) => e.write_to(w),)*
                }
            }
        }
    }
}

mk_primitive! {
    /// Line primitives.
    Lines(as_lines),
    /// Line-strip primitives.
    LineStrips(as_line_strips),
    /// Polygon primitives which may contain holes.
    Polygons(as_polygons),
    /// Polygon primitives that cannot contain holes.
    PolyList(as_polylist),
    /// Triangle primitives.
    Triangles(as_triangles),
    /// Triangle-fan primitives.
    TriFans(as_trifans),
    /// Triangle-strip primitives.
    TriStrips(as_tristrips),
}

/// The data for a [`Lines`] element.
///
/// Each line described by the mesh has two vertices.
/// The first line is formed from the first and second vertices.
/// The second line is formed from the third and fourth vertices, and so on.
#[derive(Clone, Default, Debug)]
pub struct LineGeom {
    /// Contains indices that describe the vertex attributes for an
    /// arbitrary number of individual lines.
    pub prim: Option<Box<[u32]>>,
}

/// Provides the information needed for a mesh to bind vertex attributes
/// together and then organize those vertices into individual lines.
pub type Lines = Geom<LineGeom>;

impl Lines {
    /// Construct a new `Lines` object from a data buffer.
    /// The data buffer `prim` contains exactly `count` lines, consisting of 2 vertices,
    /// each consisting of `inputs.len()` indices,
    /// for a total of `inputs.len() * 2 * count` values.
    pub fn new(
        material: Option<String>,
        inputs: Vec<InputS>,
        count: usize,
        prim: Box<[u32]>,
    ) -> Self {
        let inputs = InputList::new(inputs);
        assert!(inputs.len() * 2 * count == prim.len());
        Self::new_geom(material, inputs, count, LineGeom { prim: Some(prim) })
    }
}

impl Deref for LineGeom {
    type Target = Option<Box<[u32]>>;

    fn deref(&self) -> &Self::Target {
        &self.prim
    }
}

impl ParseGeom for LineGeom {
    const NAME: &'static str = "lines";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(LineGeom {
            prim: parse_opt("p", it, parse_array)?,
        })
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        if let Some(ref data) = *res.data {
            if res.inputs.stride * 2 * res.count != data.len() {
                return Err("line count does not match <p> field".into());
            }
        }
        Ok(())
    }
}

impl XNodeWrite for LineGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        opt(&self.prim, |e| ElemBuilder::print_arr("p", e, w))
    }
}

/// The data for a [`LineStrips`] element.
///
/// Each line-strip described by the mesh has an arbitrary number of vertices.
/// Each line segment within the line-strip is formed from the current vertex and the preceding vertex.
#[derive(Clone, Default, Debug)]
pub struct LineStripGeom {
    /// Contains indices that describe the vertex attributes for an
    /// arbitrary number of connected line segments.
    pub prim: Vec<Box<[u32]>>,
}

/// Provides the information needed to bind vertex attributes together and then organize those vertices into
/// connected line-strips.
pub type LineStrips = Geom<LineStripGeom>;

impl LineStrips {
    /// Construct a new `LineStrips` object from a data buffer.
    /// Each buffer in `data` contains 2 or more vertices,
    /// each consisting of `inputs.len()` indices,
    /// for a total of `inputs.len() * verts` values.
    pub fn new(material: Option<String>, inputs: Vec<InputS>, prim: Vec<Box<[u32]>>) -> Self {
        let inputs = InputList::new(inputs);
        debug_assert!(prim.iter().all(|p| inputs.check_prim::<2>(p)));
        Self::new_geom(material, inputs, prim.len(), LineStripGeom { prim })
    }
}

impl Deref for LineStripGeom {
    type Target = Vec<Box<[u32]>>;

    fn deref(&self) -> &Self::Target {
        &self.prim
    }
}

impl ParseGeom for LineStripGeom {
    const NAME: &'static str = "line_strips";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(LineStripGeom {
            prim: parse_list("p", it, parse_array)?,
        })
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        if res.count != res.data.len() {
            return Err("line strip count does not match <p> fields".into());
        }
        if !res.data.iter().all(|p| res.inputs.check_prim::<2>(p)) {
            return Err("incorrect <p> field in line strips".into());
        }
        Ok(())
    }
}

impl XNodeWrite for LineStripGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.prim
            .iter()
            .try_for_each(|e| ElemBuilder::print_arr("p", e, w))
    }
}

/// The data for an individual polygon-with-hole.
#[derive(Clone, Debug)]
pub struct PolygonHole {
    /// The vertex data for the polygon.
    pub verts: Box<[u32]>,
    /// A list of 0 or more holes, each of which describes a polygonal hole
    /// in the main polygon.
    pub hole: Vec<Box<[u32]>>,
}

impl PolygonHole {
    /// Construct a new `PolygonHole` from a list of vertices and 0 or more holes.
    pub fn new(verts: Box<[u32]>, hole: Vec<Box<[u32]>>) -> Self {
        Self { verts, hole }
    }
}

impl XNodeWrite for PolygonHole {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = ElemBuilder::new("ph").start(w)?;
        ElemBuilder::print_arr("p", &self.verts, w)?;
        many(&self.hole, |h| ElemBuilder::print_arr("h", h, w))?;
        e.end(w)
    }
}

/// The data for a [`Polygons`] element.
#[derive(Clone, Default, Debug)]
pub struct PolygonGeom(
    /// The list of polygons, each of which may contain 0 or more holes.
    pub Vec<PolygonHole>,
);

/// Provides the information needed for a mesh to bind vertex attributes
/// together and then organize those vertices into individual polygons.
pub type Polygons = Geom<PolygonGeom>;

impl Polygons {
    /// Construct a new `Polygons` object from raw data.
    pub fn new(material: Option<String>, inputs: Vec<InputS>, prim: Vec<PolygonHole>) -> Self {
        let inputs = InputList::new(inputs);
        debug_assert!(prim.iter().all(|ph| {
            inputs.check_prim::<3>(&ph.verts) && ph.hole.iter().all(|h| inputs.check_prim::<3>(h))
        }));
        Self::new_geom(material, inputs, prim.len(), PolygonGeom(prim))
    }
}

impl Deref for PolygonGeom {
    type Target = Vec<PolygonHole>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ParseGeom for PolygonGeom {
    // HEW PATCH: upstream had "polygon" (singular), which never matches the real
    // <polygons> element — every <polygons> fell through to the <extra> parser and
    // aborted the document. See VENDORED.md.
    const NAME: &'static str = "polygons";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        let mut polys = parse_list("p", it, |e| {
            Ok(PolygonHole {
                verts: parse_array(e)?,
                hole: vec![],
            })
        })?;
        let more_polys = parse_list("ph", it, |e| {
            let mut it = e.children().peekable();
            let verts = parse_one("p", &mut it, parse_array)?;
            let hole = parse_list("h", &mut it, parse_array)?;
            if hole.is_empty() {
                return Err(
                    "<ph> element can only be used when at least one hole is present".into(),
                );
            }
            finish(PolygonHole { verts, hole }, it)
        })?;
        polys.extend(more_polys);
        Ok(PolygonGeom(polys))
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        if res.count != res.data.len() {
            return Err("polygon count does not match <p> fields".into());
        }
        if !res.data.iter().all(|ph| {
            res.inputs.check_prim::<3>(&ph.verts)
                && ph.hole.iter().all(|h| res.inputs.check_prim::<3>(h))
        }) {
            return Err("incorrect <p> field in polygon".into());
        }
        Ok(())
    }
}

impl XNodeWrite for PolygonGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.0.write_to(w)
    }
}

/// The data for a [`PolyList`] element.
#[derive(Clone, Default, Debug)]
pub struct PolyListGeom {
    /// Contains a list of integers, each specifying the number of
    /// vertices for one polygon described by the [`PolyList`] element.
    pub vcount: Box<[u32]>,
    /// Contains a list of integers that specify the vertex attributes
    /// (indices) for an individual polylist.
    /// The winding order of vertices produced is counter-clockwise
    /// and describes the front side of each polygon.
    pub prim: Box<[u32]>,
}

/// Provides the information needed for a mesh to bind vertex attributes
/// together and then organize those vertices into individual polygons.
pub type PolyList = Geom<PolyListGeom>;

impl PolyList {
    /// Construct a new `PolyList` object from a data buffer.
    /// Each value `n` in `vcount` corresponds to a polygon with `n` vertices,
    /// to which `inputs.len() * n` index values are associated in `prim`.
    pub fn new(
        material: Option<String>,
        inputs: Vec<InputS>,
        vcount: Box<[u32]>,
        prim: Box<[u32]>,
    ) -> Self {
        let inputs = InputList::new(inputs);
        debug_assert!(inputs.len() * vcount.iter().sum::<u32>() as usize == prim.len());
        Self::new_geom(
            material,
            inputs,
            vcount.len(),
            PolyListGeom { vcount, prim },
        )
    }
}

pub(crate) fn validate_vcount<T>(
    count: usize,
    stride: usize,
    vcount: &[u32],
    prim: &[T],
) -> Result<()> {
    if count != vcount.len() {
        return Err("count does not match <vcount> field".into());
    }
    if stride * vcount.iter().sum::<u32>() as usize != prim.len() {
        return Err("vcount does not match <p>/<v> field".into());
    }
    Ok(())
}

impl ParseGeom for PolyListGeom {
    const NAME: &'static str = "polylist";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(PolyListGeom {
            vcount: parse_opt("vcount", it, parse_array)?.unwrap_or_default(),
            prim: parse_opt("p", it, parse_array)?.unwrap_or_default(),
        })
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        validate_vcount(
            res.count,
            res.inputs.stride,
            &res.data.vcount,
            &res.data.prim,
        )
    }
}

impl XNodeWrite for PolyListGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        if !self.vcount.is_empty() {
            ElemBuilder::print_arr("vcount", &self.vcount, w)?;
        }
        if !self.prim.is_empty() {
            ElemBuilder::print_arr("p", &self.prim, w)?;
        }
        Ok(())
    }
}

/// The data for a [`Triangles`] element.
///
/// Each triangle described by the mesh has three vertices.
/// The first triangle is formed from the first, second, and third vertices.
/// The second triangle is formed from the fourth, fifth, and sixth vertices, and so on.
#[derive(Clone, Default, Debug)]
pub struct TriangleGeom {
    /// Contains indices that describe the vertex attributes for a number of triangles.
    /// The indices reference into the parent’s [`Source`] elements that are
    /// referenced by the [`InputS`] elements.
    pub prim: Option<Box<[u32]>>,
}

/// Provides the information needed for a mesh to bind vertex attributes
/// together and then organize those vertices into individual triangles.
pub type Triangles = Geom<TriangleGeom>;

impl Triangles {
    /// Construct a new `Triangles` object from a data buffer.
    /// The data buffer `prim` contains exactly `count` triangles, consisting of 3 vertices,
    /// each consisting of `inputs.len()` indices,
    /// for a total of `inputs.len() * 3 * count` values.
    pub fn new(
        material: Option<String>,
        inputs: Vec<InputS>,
        count: usize,
        prim: Box<[u32]>,
    ) -> Self {
        let inputs = InputList::new(inputs);
        assert!(inputs.len() * 3 * count == prim.len());
        Self::new_geom(material, inputs, count, TriangleGeom { prim: Some(prim) })
    }
}

impl Deref for TriangleGeom {
    type Target = Option<Box<[u32]>>;

    fn deref(&self) -> &Self::Target {
        &self.prim
    }
}

impl ParseGeom for TriangleGeom {
    const NAME: &'static str = "triangles";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(TriangleGeom {
            prim: parse_opt("p", it, parse_array)?,
        })
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        if let Some(ref data) = *res.data {
            if res.inputs.stride * 3 * res.count != data.len() {
                return Err("triangle count does not match <p> field".into());
            }
        }
        Ok(())
    }
}

impl XNodeWrite for TriangleGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        opt(&self.prim, |e| ElemBuilder::print_arr("p", e, w))
    }
}

/// The data for a [`TriFans`] element.
///
/// Each triangle described by the mesh has three vertices.
/// The first triangle is formed from the first, second, and third vertices.
/// Each subsequent triangle is formed from the current vertex,
/// reusing the first and the previous vertices.
#[derive(Clone, Default, Debug)]
pub struct TriFanGeom {
    /// Contains indices that describe the vertex attributes for an
    /// arbitrary number of connected triangles.
    /// The indices reference into the parent’s [`Source`] elements that are
    /// referenced by the [`InputS`] elements.
    pub prim: Vec<Box<[u32]>>,
}

/// Provides the information needed for a mesh to bind vertex attributes
/// together and then organize those vertices into connected triangles.
pub type TriFans = Geom<TriFanGeom>;

impl TriFans {
    /// Construct a new `TriFans` object from a data buffer.
    /// Each buffer in `data` contains 3 or more vertices,
    /// each consisting of `inputs.len()` indices,
    /// for a total of `inputs.len() * verts` values.
    pub fn new(material: Option<String>, inputs: Vec<InputS>, prim: Vec<Box<[u32]>>) -> Self {
        let inputs = InputList::new(inputs);
        debug_assert!(prim.iter().all(|p| inputs.check_prim::<3>(p)));
        Self::new_geom(material, inputs, prim.len(), TriFanGeom { prim })
    }
}

impl Deref for TriFanGeom {
    type Target = Vec<Box<[u32]>>;

    fn deref(&self) -> &Self::Target {
        &self.prim
    }
}

impl ParseGeom for TriFanGeom {
    const NAME: &'static str = "trifans";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(TriFanGeom {
            prim: parse_list("p", it, parse_array)?,
        })
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        if res.count != res.data.len() {
            return Err("triangle fan count does not match <p> fields".into());
        }
        if !res.data.iter().all(|p| res.inputs.check_prim::<3>(p)) {
            return Err("incorrect <p> field in triangle fans".into());
        }
        Ok(())
    }
}

impl XNodeWrite for TriFanGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        many(&self.prim, |e| ElemBuilder::print_arr("p", e, w))
    }
}

/// The data for a [`TriStrips`] element.
///
/// Each triangle described by the mesh has three vertices.
/// The first triangle is formed from the first, second, and third vertices.
/// Each subsequent triangle is formed from the current vertex,
/// reusing the previous two vertices.
#[derive(Clone, Default, Debug)]
pub struct TriStripGeom {
    /// Contains indices that describe the vertex attributes for an
    /// arbitrary number of connected triangles.
    /// The indices reference into the parent’s [`Source`] elements that are
    /// referenced by the [`InputS`] elements.
    pub prim: Vec<Box<[u32]>>,
}

/// Provides the information needed for a mesh to bind vertex attributes
/// together and then organize those vertices into connected triangles.
pub type TriStrips = Geom<TriStripGeom>;

impl TriStrips {
    /// Construct a new `TriStrips` object from a data buffer.
    /// Each buffer in `data` contains 3 or more vertices,
    /// each consisting of `inputs.len()` indices,
    /// for a total of `inputs.len() * verts` values.
    pub fn new(material: Option<String>, inputs: Vec<InputS>, prim: Vec<Box<[u32]>>) -> Self {
        let inputs = InputList::new(inputs);
        debug_assert!(prim.iter().all(|p| inputs.check_prim::<3>(p)));
        Self::new_geom(material, inputs, prim.len(), TriStripGeom { prim })
    }
}

impl Deref for TriStripGeom {
    type Target = Vec<Box<[u32]>>;

    fn deref(&self) -> &Self::Target {
        &self.prim
    }
}

impl ParseGeom for TriStripGeom {
    const NAME: &'static str = "tristrips";

    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(TriStripGeom {
            prim: parse_list("p", it, parse_array)?,
        })
    }

    fn validate(res: &Geom<Self>) -> Result<()> {
        if res.count != res.data.len() {
            return Err("triangle strip count does not match <p> fields".into());
        }
        if !res.data.iter().all(|p| res.inputs.check_prim::<3>(p)) {
            return Err("incorrect <p> field in triangle strips".into());
        }
        Ok(())
    }
}

impl XNodeWrite for TriStripGeom {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        many(&self.prim, |e| ElemBuilder::print_arr("p", e, w))
    }
}

/// Describes a multisegment spline with control vertex (CV) and segment information.
#[derive(Clone, Debug)]
pub struct Spline {
    /// Whether there is a segment connecting the first and last control vertices.
    /// The default is "false", indicating that the spline is open.
    pub closed: bool,
    /// Provides the values for the CVs and segments of the spline.
    pub sources: Vec<Source>,
    /// Describes the CVs of the spline.
    pub controls: ControlVertices,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Spline {
    /// Construct a new `Spline` object with the given inputs.
    pub fn new(sources: Vec<Source>, controls: Vec<Input>) -> Self {
        Self {
            closed: false,
            sources,
            controls: ControlVertices::new(controls),
            extra: vec![],
        }
    }
}

impl XNode for Spline {
    const NAME: &'static str = "spline";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Spline {
            closed: parse_attr(element.attr("closed"))?.unwrap_or(false),
            sources: Source::parse_list_n::<1>(&mut it)?,
            controls: ControlVertices::parse_one(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Spline {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.def_print_attr("closed", self.closed, false);
        let e = e.start(w)?;
        self.sources.write_to(w)?;
        self.controls.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Describes the control vertices (CVs) of a spline.
#[derive(Clone, Debug)]
pub struct ControlVertices {
    /// The list of inputs.
    pub inputs: Vec<Input>,
    /// The index into `inputs` for the [`Semantic::Position`] input (which must exist).
    pub position: usize,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for ControlVertices {
    const NAME: &'static str = "control_vertices";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let inputs = Input::parse_list(&mut it)?;
        Ok(ControlVertices {
            position: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Position)
                .ok_or("control_vertices: missing POSITION input")?,
            inputs,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for ControlVertices {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.inputs.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl ControlVertices {
    /// Construct a new `Vertices` object with the given inputs.
    /// * One of the inputs must have [`Semantic::Position`].
    pub fn new(inputs: Vec<Input>) -> Self {
        Self {
            position: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Position)
                .expect("control_vertices: missing POSITION input"),
            inputs,
            extra: vec![],
        }
    }

    /// The input with [`Semantic::Position`].
    pub fn position_input(&self) -> &Input {
        &self.inputs[self.position]
    }
}
