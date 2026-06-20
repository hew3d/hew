use crate::*;

/// Describes a stream of values from an array data source.
#[derive(Clone, Debug)]
pub struct Accessor {
    /// The location of the array to access using a URI expression.
    /// This element may refer to a COLLADA array element or to an
    /// array data source outside the scope of the document;
    /// the source does not need to be a COLLADA document.
    pub source: Url,
    /// The number of times the array is accessed.
    pub count: usize,
    /// The index of the first value to be read from the array.
    pub offset: usize,
    /// The number of values that are to be considered a unit during each access to the array.
    /// The default is 1, indicating that a single value is accessed.
    pub stride: usize,
    /// The list of accesses.
    pub param: Vec<Param>,
}

impl Accessor {
    /// Construct a new `Accessor` from the mandatory data.
    pub fn new(source: Url, count: usize, param: Vec<Param>) -> Self {
        Self {
            source,
            count,
            offset: 0,
            stride: param.len(),
            param,
        }
    }
}

impl XNode for Accessor {
    const NAME: &'static str = "accessor";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Accessor {
            source: parse_attr(element.attr("source"))?.ok_or("missing source attr")?,
            count: parse_attr(element.attr("count"))?.ok_or("expected 'count' attr")?,
            offset: parse_attr(element.attr("offset"))?.unwrap_or(0),
            stride: parse_attr(element.attr("stride"))?.unwrap_or(1),
            param: Param::parse_list(&mut it)?,
        };
        if res.stride < res.param.len() {
            return Err("accessor stride does not match params".into());
        }
        finish(res, it)
    }
}

impl XNodeWrite for Accessor {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("source", &self.source);
        e.print_attr("count", self.count);
        e.def_print_attr("offset", self.offset, 0);
        e.def_print_attr("stride", self.stride, 1);
        let e = e.start(w)?;
        self.param.write_to(w)?;
        e.end(w)
    }
}

fn parse_array_count<T: FromStr>(e: &Element) -> Result<Box<[T]>> {
    let count: usize = parse_attr(e.attr("count"))?.ok_or("expected 'count' attr")?;
    let mut vec = Vec::with_capacity(count);
    for s in get_text(e)
        .ok_or("expected text node")?
        .split_ascii_whitespace()
    {
        vec.push(s.parse().map_err(|_| "parse error")?)
    }
    if vec.len() != count {
        return Err("'count' does not match array length".into());
    }
    Ok(vec.into())
}

/// A trait for the common functionality of the array types.
pub trait ArrayKind: HasId + XNode + Deref<Target = [Self::Elem]> + 'static {
    /// The stored element type.
    type Elem: Clone + Display + Debug + 'static;
    /// Extract a typed array from an [`ArrayElement`].
    fn from_array_element(elem: &ArrayElement) -> Option<&Self>;
}

macro_rules! mk_arrays {
    ($($(#[$doc:meta])* $name:ident($tyname:ident[$ty:ty]) = $s:literal,)*) => {
        $(
            $(#[$doc])*
            #[derive(Clone, Debug)]
            pub struct $tyname {
                /// A text string containing the unique identifier of the element.
                pub id: Option<String>,
                /// The stored array of values.
                pub val: Box<[$ty]>,
            }
            impl $tyname {
                /// Construct a new array element from an array.
                pub fn new(id: impl Into<String>, val: Box<[$ty]>) -> Self {
                    Self {
                        id: Some(id.into()),
                        val,
                    }
                }
            }
            impl From<$tyname> for ArrayElement {
                fn from(val: $tyname) -> Self {
                    Self::$name(val)
                }
            }
            impl Deref for $tyname {
                type Target = [$ty];

                fn deref(&self) -> &Self::Target {
                    &self.val
                }
            }
            impl XNode for $tyname {
                const NAME: &'static str = $s;
                fn parse(element: &Element) -> Result<Self> {
                    debug_assert_eq!(element.name(), Self::NAME);
                    Ok(Self {
                        id: element.attr("id").map(Into::into),
                        val: parse_array_count(element)?,
                    })
                }
            }
            impl XNodeWrite for $tyname {
                fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
                    let mut e = Self::elem();
                    e.opt_attr("id", &self.id);
                    e.print_attr("count", self.val.len());
                    let e = e.start(w)?;
                    print_arr(&self.val, w)?;
                    e.end(w)
                }
            }
            impl CollectLocalMaps for $tyname {
                fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
                    maps.insert(self)
                }
            }
            impl ArrayKind for $tyname {
                type Elem = $ty;
                fn from_array_element(elem: &ArrayElement) -> Option<&Self> {
                    match elem {
                        ArrayElement::$name(arr) => Some(arr),
                        _ => None,
                    }
                }
            }
        )*

        /// A data array element.
        #[derive(Clone, Debug)]
        pub enum ArrayElement {
            $($(#[$doc])* $name($tyname),)*
        }

        impl XNodeWrite for ArrayElement {
            fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
                match self {
                    $(Self::$name(e) => e.write_to(w),)*
                }
            }
        }

        impl CollectLocalMaps for ArrayElement {
            fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
                match self {
                    $(Self::$name(arr) => arr.collect_local_maps(maps),)*
                }
            }
        }

        impl ArrayElement {
            /// Parse an [`ArrayElement`] from an XML element.
            pub fn parse(e: &Element) -> Result<Option<Self>> {
                Ok(Some(match e.name() {
                    $($tyname::NAME => Self::$name($tyname::parse(e)?),)*
                    _ => return Ok(None),
                }))
            }

            /// Get the ID of this element.
            pub fn id(&self) -> Option<&str> {
                match self {
                    $(ArrayElement::$name(arr) => arr.id.as_deref(),)*
                }
            }

            /// Get the number of values in this array.
            pub fn len(&self) -> usize {
                match self {
                    $(ArrayElement::$name(arr) => arr.len(),)*
                }
            }

            /// Returns true if the array is empty.
            pub fn is_empty(&self) -> bool {
                self.len() == 0
            }
        }
    }
}

mk_arrays! {
    /// Stores a homogenous array of ID reference values.
    IdRef(IdRefArray[String]) = "IDREF_array",
    /// Stores a homogenous array of symbolic name values.
    Name(NameArray[String]) = "Name_array",
    /// Stores a homogenous array of Boolean values.
    Bool(BoolArray[bool]) = "bool_array",
    /// Stores a homogenous array of floating-point values.
    Float(FloatArray[f32]) = "float_array",
    /// Stores a homogenous array of integer values.
    Int(IntArray[u32]) = "int_array",
}

/// Declares parametric information for its parent element.
#[derive(Clone, Debug)]
pub struct Param {
    /// A text string value containing the subidentifier of this element.
    /// This value must be unique within the scope of the parent element.
    pub sid: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// The type of the value data. This text string must be understood by the application.
    pub ty: String,
    /// The user-defined meaning of the parameter.
    pub semantic: Option<Semantic>,
}

impl Param {
    /// Construct a new `Param` with given name and type.
    pub fn new(name: impl Into<String>, ty: impl Into<String>) -> Self {
        Self {
            sid: None,
            name: Some(name.into()),
            ty: ty.into(),
            semantic: None,
        }
    }

    /// Construct parameters corresponding to `X,Y,Z` accesses, used for vector values.
    pub fn new_xyz() -> Vec<Self> {
        vec![
            Self::new("X", "float"),
            Self::new("Y", "float"),
            Self::new("Z", "float"),
        ]
    }

    /// Construct parameters corresponding to `S,T` accesses, used for texture coordinates.
    pub fn new_st() -> Vec<Self> {
        vec![Self::new("S", "float"), Self::new("T", "float")]
    }
}

impl XNode for Param {
    const NAME: &'static str = "param";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Param {
            sid: element.attr("sid").map(Into::into),
            name: element.attr("name").map(Into::into),
            ty: element.attr("type").ok_or("expecting 'type' attr")?.into(),
            semantic: parse_attr(element.attr("semantic"))?,
        })
    }
}

impl XNodeWrite for Param {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("sid", &self.sid);
        e.opt_attr("name", &self.name);
        e.attr("type", &self.ty);
        e.opt_print_attr("semantic", &self.semantic);
        e.end(w)
    }
}

/// Declares a data repository that provides values
/// according to the semantics of an [`Input`] element that refers to it.
#[derive(Clone, Debug)]
pub struct Source {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// A data array element.
    pub array: Option<ArrayElement>,
    /// The access pattern into the data element.
    pub accessor: Accessor,
    /// Declares the information used to process some portion of the content. (optional)
    pub technique: Vec<Technique>,
}

impl Source {
    /// Construct a new `Source` given an inline array and access information.
    pub fn new_local(
        id: impl Into<String>,
        param: Vec<Param>,
        array: impl Into<ArrayElement>,
    ) -> Self {
        let id = id.into();
        let array = array.into();
        assert!(!param.is_empty());
        let count = array.len() / param.len();
        assert!(param.len() * count == array.len());
        Self {
            accessor: Accessor::new(
                Url::Fragment(array.id().expect("array requires id").into()),
                count,
                param,
            ),
            id: Some(id),
            name: None,
            asset: None,
            array: Some(array),
            technique: vec![],
        }
    }
}

impl XNode for Source {
    const NAME: &'static str = "source";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Source {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            array: parse_opt_many(&mut it, ArrayElement::parse)?,
            accessor: parse_one(Technique::COMMON, &mut it, |e| {
                let mut it = e.children().peekable();
                finish(Accessor::parse_one(&mut it)?, it)
            })?,
            technique: Technique::parse_list(&mut it)?,
        };
        if let Some(arr) = &res.array {
            if arr.len() < res.accessor.offset + res.accessor.stride * res.accessor.count {
                return Err("array is too short for accessor".into());
            }
        }
        finish(res, it)
    }
}

impl XNodeWrite for Source {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.array.write_to(w)?;
        let common = ElemBuilder::new(Technique::COMMON).start(w)?;
        self.accessor.write_to(w)?;
        common.end(w)?;
        self.technique.write_to(w)?;
        e.end(w)
    }
}

impl CollectLocalMaps for Source {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        maps.insert(self);
        self.array.collect_local_maps(maps);
    }
}

impl Traversable for Source {
    fn traverse<'a, E>(
        doc: &'a Document,
        mut f: impl FnMut(&'a Self) -> Result<(), E>,
    ) -> Result<(), E>
    where
        Self: 'a,
    {
        doc.library.iter().try_for_each(|elem| match elem {
            LibraryElement::Animations(lib) => lib
                .items
                .iter()
                .try_for_each(|anim| anim.source.iter().try_for_each(&mut f)),
            LibraryElement::Controllers(lib) => lib
                .items
                .iter()
                .try_for_each(|con| con.element.sources().iter().try_for_each(&mut f)),
            LibraryElement::Geometries(lib) => lib
                .items
                .iter()
                .try_for_each(|geom| geom.element.sources().iter().try_for_each(&mut f)),
            _ => Ok(()),
        })
    }
}

/// Declares the input semantics of a data source and connects a consumer to that source.
/// In the COLLADA spec this is called "`<input>` (unshared)".
#[derive(Clone, Debug)]
pub struct Input {
    /// The user-defined meaning of the input connection.
    pub semantic: Semantic,
    /// The location of the data source.
    /// The type referenced here depends on the `semantic`:
    /// * For [`Semantic::Vertex`] it references a [`Vertices`]
    /// * For most other semantics it references a [`Source`]
    pub source: Url,
}

impl XNode for Input {
    const NAME: &'static str = "input";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Input {
            semantic: parse_attr(element.attr("semantic"))?.ok_or("missing semantic attr")?,
            source: parse_attr(element.attr("source"))?.ok_or("missing source attr")?,
        })
    }
}

impl XNodeWrite for Input {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("semantic", &self.semantic);
        e.print_attr("source", &self.source);
        e.end(w)
    }
}

impl Input {
    /// Construct a new [`Input`].
    pub fn new(semantic: Semantic, source: Url) -> Self {
        Self { semantic, source }
    }

    /// Typecast `self.source` as a `UrlRef<Source>`.
    /// The `semantic` is checked in debug mode to ensure that it is compatible with a
    /// [`Source`] target.
    pub fn source_as_source(&self) -> &UrlRef<Source> {
        debug_assert!(!matches!(self.semantic, Semantic::Vertex));
        ref_cast::RefCast::ref_cast(&self.source)
    }

    /// Typecast `self.source` as a `UrlRef<Vertices>`.
    /// The `semantic` is checked in debug mode to ensure that it is compatible with a
    /// [`Vertices`] target.
    pub fn source_as_vertices(&self) -> &UrlRef<Vertices> {
        debug_assert!(matches!(self.semantic, Semantic::Vertex));
        ref_cast::RefCast::ref_cast(&self.source)
    }
}

/// Declares the input semantics of a data source and connects a consumer to that source.
/// In the COLLADA spec this is called "`<input>` (shared)".
#[derive(Clone, Debug)]
pub struct InputS {
    /// [`InputS`] inherits from [`Input`].
    pub input: Input,
    /// The offset into the list of indices defined by the parent element’s `prim` field.
    /// If two [`InputS`] elements share the same offset, they are indexed the same.
    /// This is a simple form of compression for the list of indices
    /// and also defines the order in which the inputs are used.
    pub offset: u32,
    /// Which inputs to group as a single set. This is helpful when multiple inputs
    /// share the same semantics.
    pub set: Option<u32>,
}

impl InputS {
    /// Construct a new [`InputS`].
    pub fn new(semantic: Semantic, source: Url, offset: u32, set: Option<u32>) -> Self {
        Self {
            input: Input { semantic, source },
            offset,
            set,
        }
    }
}

impl Deref for InputS {
    type Target = Input;
    fn deref(&self) -> &Self::Target {
        &self.input
    }
}

impl XNode for InputS {
    const NAME: &'static str = "input";
    fn parse(element: &Element) -> Result<Self> {
        Ok(InputS {
            input: Input::parse(element)?,
            offset: parse_attr(element.attr("offset"))?.ok_or("missing offset attr")?,
            set: parse_attr(element.attr("set"))?,
        })
    }
}

impl XNodeWrite for InputS {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("semantic", &self.semantic);
        e.print_attr("source", &self.source);
        e.print_attr("offset", self.offset);
        e.opt_print_attr("set", &self.set);
        e.end(w)
    }
}

/// Wraps a group of inputs and precalculates the `depth` field.
#[derive(Clone, Default, Debug)]
pub struct InputList {
    /// The list of inputs.
    pub inputs: Vec<InputS>,
    /// The stride of an input list is the largest offset plus one;
    /// this is the stride of accesses applied by these inputs.
    pub stride: usize,
}

impl Deref for InputList {
    type Target = Vec<InputS>;

    fn deref(&self) -> &Self::Target {
        &self.inputs
    }
}

impl From<Vec<InputS>> for InputList {
    fn from(inputs: Vec<InputS>) -> Self {
        Self::new(inputs)
    }
}

impl InputList {
    /// Construct a new `InputList` from a list of inputs.
    pub fn new(inputs: Vec<InputS>) -> Self {
        let stride = inputs.iter().map(|i| i.offset).max().map_or(0, |n| n + 1) as usize;
        Self { inputs, stride }
    }

    pub(crate) fn parse<const N: usize>(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(InputList::new(InputS::parse_list_n::<N>(it)?))
    }

    pub(crate) fn check_prim<const MIN: usize>(&self, data: &[u32]) -> bool {
        // HEW PATCH: upstream had `data.len() < self.stride * MIN`, which rejects
        // any primitive with *enough* vertices (a 4-gon at stride 2 has 8 indices,
        // but `8 < 2*3` is false). The intent is "at least MIN vertices", i.e.
        // `>=`. This broke <polygons>/<triangles>/fans/strips for real exports.
        // See VENDORED.md.
        self.stride != 0 && data.len() >= self.stride * MIN && data.len() % self.stride == 0
    }
}

mk_extensible_enum! {
  /// An [`Input`] semantic attribute. Common / defined values are pre-parsed,
  /// and the remainder are in the `Other` variant.
  pub enum Semantic {
      /// Geometric binormal (bitangent) vector
      Binormal = "BINORMAL",
      /// Color coordinate vector. Color inputs are RGB (float3)
      Color = "COLOR",
      /// Continuity constraint at the control vertex (CV)
      Continuity = "CONTINUITY",
      /// Raster or MIP-level input
      Image = "IMAGE",
      /// Sampler input
      Input = "INPUT",
      /// Tangent vector for preceding control point
      InTangent = "IN_TANGENT",
      /// Sampler interpolation type
      Interpolation = "INTERPOLATION",
      /// Inverse of local-to-world matrix
      InvBindMatrix = "INV_BIND_MATRIX",
      /// Skin influence identifier
      Joint = "JOINT",
      /// Number of piece-wise linear approximation steps to use for the spline segment that
      /// follows this CV
      LinearSteps = "LINEAR_STEPS",
      /// Morph targets for mesh morphing
      MorphTarget = "MORPH_TARGET",
      /// Weights for mesh morphing
      MorphWeight = "MORPH_WEIGHT",
      /// Normal vector
      Normal = "NORMAL",
      /// Sampler output
      Output = "OUTPUT",
      /// Tangent vector for succeeding control point
      OutTangent = "OUT_TANGENT",
      /// Geometric coordinate vector
      Position = "POSITION",
      /// Geometric tangent vector
      Tangent = "TANGENT",
      /// Texture binormal (bitangent) vector
      TexBinormal = "TEXBINORMAL",
      /// Texture coordinate vector
      TexCoord = "TEXCOORD",
      /// Texture coordinate vector
      TexTangent = "TEXTANGENT",
      /// Generic parameter vector
      UV = "UV",
      /// Mesh vertex
      Vertex = "VERTEX",
      /// Skin influence weighting value
      Weight = "WEIGHT",
  }
}
