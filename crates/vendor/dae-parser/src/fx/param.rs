use crate::*;

/// Instruction to create a new, named `Param` object in the FX Runtime,
/// assign it a type, an initial value, and additional attributes at declaration time.
#[derive(Clone, Debug)]
pub struct NewParam {
    /// Identifier for this parameter (that is, the variable name).
    pub sid: String,
    /// A list of strongly typed annotation remarks.
    pub annotate: Vec<Annotate>,
    /// Meta-information that describes the purpose of the parameter declaration.
    pub semantic: Option<String>,
    /// Additional information about the volatility or linkage.
    pub modifier: Option<Modifier>,
    /// The parameter's type.
    pub ty: ParamType,
}

impl NewParam {
    /// Create a `NewParam` with the given name and type.
    pub fn new(sid: impl Into<String>, ty: impl Into<ParamType>) -> Self {
        Self {
            sid: sid.into(),
            annotate: vec![],
            semantic: None,
            modifier: None,
            ty: ty.into(),
        }
    }
}

impl XNode for NewParam {
    const NAME: &'static str = "newparam";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = NewParam {
            sid: element.attr("sid").ok_or("expecting sid attr")?.into(),
            annotate: Annotate::parse_list(&mut it)?,
            semantic: parse_opt("semantic", &mut it, parse_text)?,
            modifier: parse_opt("modifier", &mut it, parse_elem)?,
            ty: parse_one_many(&mut it, ParamType::parse)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for NewParam {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.attr("sid", &self.sid);
        let e = e.start(w)?;
        self.annotate.write_to(w)?;
        opt(&self.semantic, |e| ElemBuilder::print_str("semantic", e, w))?;
        ElemBuilder::opt_print("modifier", &self.modifier, w)?;
        self.ty.write_to(w)?;
        e.end(w)
    }
}

/// Assigns a new value to a previously defined parameter.
///
/// This type corresponds to the `<setparam>` element in the COLLADA spec
/// in the special case where the parent is an `<instance_effect>`.
#[derive(Clone, Debug)]
pub struct EffectSetParam {
    /// Attempts to reference the predefined parameter that will have its value set.
    pub ref_: String,
    /// The value of the parameter.
    // This is slightly inaccurate; the collada spec allows a larger set of types here
    // than the ones used in <annotation>.
    pub value: AnnotType,
}

impl EffectSetParam {
    /// Construct a new `EffectSetParam` with the given name and type.
    pub fn new(ref_: impl Into<String>, value: impl Into<AnnotType>) -> Self {
        Self {
            ref_: ref_.into(),
            value: value.into(),
        }
    }
}

impl XNode for EffectSetParam {
    const NAME: &'static str = "setparam";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = EffectSetParam {
            ref_: element.attr("ref").ok_or("expected ref attr")?.into(),
            value: parse_one_many(&mut it, AnnotType::parse)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for EffectSetParam {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.attr("ref", &self.ref_);
        let e = e.start(w)?;
        self.value.write_to(w)?;
        e.end(w)
    }
}

/// Adds a strongly typed annotation remark to the parent object.
#[derive(Clone, Debug)]
pub struct Annotate {
    /// The text string name of this element that represents the `SYMBOL` in an object of
    /// the form `SYMBOL = VALUE`.
    pub name: String,
    /// A strongly typed value that represents the VALUE in an object of
    /// the form SYMBOL = VALUE. Consists of a COLLADA type
    /// element that contains a value of that type.
    pub value: AnnotType,
}

impl Annotate {
    /// Construct a new annotation with the given name and typed value.
    pub fn new(name: impl Into<String>, value: impl Into<AnnotType>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

impl XNode for Annotate {
    const NAME: &'static str = "annotate";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Annotate {
            name: element.attr("name").ok_or("expecting name attr")?.into(),
            value: parse_one_many(&mut it, AnnotType::parse)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for Annotate {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.attr("name", &self.name);
        let e = e.start(w)?;
        self.value.write_to(w)?;
        e.end(w)
    }
}

mk_extensible_enum! {
  /// Additional information about the volatility or linkage of a parameter.
  pub enum Modifier {
      /// `CONST` linkage modifier
      Const = "CONST",
      /// `UNIFORM` linkage modifier
      Uniform = "UNIFORM",
      /// `VARYING` linkage modifier
      Varying = "VARYING",
      /// `STATIC` linkage modifier
      Static = "STATIC",
      /// `VOLATILE` linkage modifier
      Volatile = "VOLATILE",
      /// `EXTERN` linkage modifier
      Extern = "EXTERN",
      /// `SHARED` linkage modifier
      Shared = "SHARED",
  }
}

/// A strongly typed value that represents the `VALUE` in an object of the form `SYMBOL = VALUE`.
#[derive(Clone, Debug)]
pub enum AnnotType {
    /// `bool` type
    Bool(bool),
    /// `bool2` type
    Bool2([bool; 2]),
    /// `bool3` type
    Bool3([bool; 3]),
    /// `bool4` type
    Bool4([bool; 4]),
    /// `int` type
    Int(u32),
    /// `int2` type
    Int2([u32; 2]),
    /// `int3` type
    Int3(Box<[u32; 3]>),
    /// `int4` type
    Int4(Box<[u32; 4]>),
    /// `float` type
    Float(f32),
    /// `float2` type
    Float2([f32; 2]),
    /// `float3` type
    Float3(Box<[f32; 3]>),
    /// `float4` type
    Float4(Box<[f32; 4]>),
    /// `float2x2` type (linearized)
    Float2x2(Box<[f32; 2 * 2]>),
    /// `float3x3` type (linearized)
    Float3x3(Box<[f32; 3 * 3]>),
    /// `float4x4` type (linearized)
    Float4x4(Box<[f32; 4 * 4]>),
    /// `string` type
    String(Box<str>),
}

impl From<&str> for AnnotType {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}

impl From<Box<str>> for AnnotType {
    fn from(v: Box<str>) -> Self {
        Self::String(v)
    }
}

impl From<bool> for AnnotType {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<f32> for AnnotType {
    fn from(v: f32) -> Self {
        Self::Float(v)
    }
}

impl From<u32> for AnnotType {
    fn from(v: u32) -> Self {
        Self::Int(v)
    }
}

impl AnnotType {
    /// Parse a [`AnnotType`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            "bool" => Self::Bool(parse_elem(e)?),
            "bool2" => Self::Bool2(*parse_array_n(e)?),
            "bool3" => Self::Bool3(*parse_array_n(e)?),
            "bool4" => Self::Bool4(*parse_array_n(e)?),
            "int" => Self::Int(parse_elem(e)?),
            "int2" => Self::Int2(*parse_array_n(e)?),
            "int3" => Self::Int3(parse_array_n(e)?),
            "int4" => Self::Int4(parse_array_n(e)?),
            "float" => Self::Float(parse_elem(e)?),
            "float2" => Self::Float2(*parse_array_n(e)?),
            "float3" => Self::Float3(parse_array_n(e)?),
            "float4" => Self::Float4(parse_array_n(e)?),
            "float2x2" => Self::Float2x2(parse_array_n(e)?),
            "float3x3" => Self::Float3x3(parse_array_n(e)?),
            "float4x4" => Self::Float4x4(parse_array_n(e)?),
            "string" => Self::String(parse_text(e)?.into()),
            _ => return Ok(None),
        }))
    }
}

impl XNodeWrite for AnnotType {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            AnnotType::Bool(e) => ElemBuilder::print("bool", e, w),
            AnnotType::Bool2(e) => ElemBuilder::print_arr("bool2", e, w),
            AnnotType::Bool3(e) => ElemBuilder::print_arr("bool3", e, w),
            AnnotType::Bool4(e) => ElemBuilder::print_arr("bool4", e, w),
            AnnotType::Int(e) => ElemBuilder::print("int", e, w),
            AnnotType::Int2(e) => ElemBuilder::print_arr("int2", e, w),
            AnnotType::Int3(e) => ElemBuilder::print_arr("int3", &**e, w),
            AnnotType::Int4(e) => ElemBuilder::print_arr("int4", &**e, w),
            AnnotType::Float(e) => ElemBuilder::print("float", e, w),
            AnnotType::Float2(e) => ElemBuilder::print_arr("float2", e, w),
            AnnotType::Float3(e) => ElemBuilder::print_arr("float3", &**e, w),
            AnnotType::Float4(e) => ElemBuilder::print_arr("float4", &**e, w),
            AnnotType::Float2x2(e) => ElemBuilder::print_arr("float2x2", &**e, w),
            AnnotType::Float3x3(e) => ElemBuilder::print_arr("float3x3", &**e, w),
            AnnotType::Float4x4(e) => ElemBuilder::print_arr("float4x4", &**e, w),
            AnnotType::String(e) => ElemBuilder::print_str("string", e, w),
        }
    }
}
/// A parameter's type. We do not have full support here,
/// but unknown types can be retrieved in the `Other` variant.
#[derive(Clone, Debug)]
pub enum ParamType {
    /// `float` type
    Float(f32),
    /// `float2` type
    Float2([f32; 2]),
    /// `float3` type
    Float3(Box<[f32; 3]>),
    /// `float4` type
    Float4(Box<[f32; 4]>),
    /// `surface` type
    Surface(Box<Surface>),
    /// `sampler2D` type
    Sampler2D(Box<Sampler2D>),
    /// Any other type, stored as a raw XML element.
    Other(Box<Element>),
}

impl From<f32> for ParamType {
    fn from(v: f32) -> Self {
        Self::Float(v)
    }
}

impl ParamType {
    /// Parse a [`ParamType`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            "float" => Self::Float(parse_elem(e)?),
            "float2" => Self::Float2(*parse_array_n(e)?),
            "float3" => Self::Float3(parse_array_n(e)?),
            "float4" => Self::Float4(parse_array_n(e)?),
            Surface::NAME => Self::Surface(Surface::parse_box(e)?),
            Sampler2D::NAME => Self::Sampler2D(Sampler2D::parse_box(e)?),
            _ => Self::Other(Box::new(e.clone())),
        }))
    }

    /// Downcast this element to a [`Surface`].
    pub fn as_surface(&self) -> Option<&Surface> {
        match self {
            ParamType::Surface(s) => Some(s),
            _ => None,
        }
    }

    /// Downcast this element to a [`Sampler2D`].
    pub fn as_sampler2d(&self) -> Option<&Sampler2D> {
        match self {
            ParamType::Sampler2D(s) => Some(s),
            _ => None,
        }
    }
}

impl XNodeWrite for ParamType {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            ParamType::Float(e) => ElemBuilder::print("float", e, w),
            ParamType::Float2(e) => ElemBuilder::print_arr("float2", e, w),
            ParamType::Float3(e) => ElemBuilder::print_arr("float3", &**e, w),
            ParamType::Float4(e) => ElemBuilder::print_arr("float4", &**e, w),
            ParamType::Surface(e) => e.write_to(w),
            ParamType::Sampler2D(e) => e.write_to(w),
            ParamType::Other(e) => XNodeWrite::write_to(e, w),
        }
    }
}
