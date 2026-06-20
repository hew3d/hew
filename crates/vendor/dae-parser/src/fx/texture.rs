use crate::*;

/// Declares the storage for the graphical representation of an object.
#[derive(Clone, Debug)]
pub struct Image {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// A text string value that indicates the image format.
    /// It describes the encoding of the image in [`ImageSource::Data`]
    /// or the format of the image referenced by [`ImageSource::InitFrom`]
    /// if it is in a nonstandard format that cannot be identified by its file extension.
    /// For example, if [`ImageSource::Data`] in a COLLADA document
    /// contains the digital contents of a JPEG file, then set this attribute to "JPG".
    pub format: Option<String>,
    /// An integer value that indicates the height of the image in pixels.
    /// A value of 0 means the value is omitted.
    pub height: u32,
    /// An integer value that indicates the width of the image in pixels.
    /// A value of 0 means the value is omitted.
    pub width: u32,
    /// An integer value that indicates the depth of the image in pixels.
    /// A 2-D image has a depth of 1, which is the default.
    pub depth: u32,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// This specifies either embedded image data or an external image file.
    pub source: ImageSource,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Image {
    /// Construct a new `Image` from the given source.
    pub fn new(id: impl Into<String>, name: Option<String>, source: ImageSource) -> Self {
        Self {
            id: Some(id.into()),
            name,
            format: None,
            height: 0,
            width: 0,
            depth: 1,
            asset: None,
            source,
            extra: vec![],
        }
    }
}

impl XNode for Image {
    const NAME: &'static str = "image";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Image {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            format: element.attr("format").map(Into::into),
            height: parse_attr(element.attr("height"))?.unwrap_or(0),
            width: parse_attr(element.attr("width"))?.unwrap_or(0),
            depth: parse_attr(element.attr("depth"))?.unwrap_or(1),
            asset: Asset::parse_opt_box(&mut it)?,
            source: parse_one_many(&mut it, ImageSource::parse)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Image {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        e.opt_attr("format", &self.format);
        e.def_print_attr("height", self.height, 0);
        e.def_print_attr("width", self.width, 0);
        e.def_print_attr("depth", self.depth, 1);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.source.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// An [`Image`] or [`NewParam`] element.
#[derive(Clone, Debug)]
pub enum ImageParam {
    /// A [`NewParam`] element.
    NewParam(NewParam),
    /// An [`Image`] element.
    Image(Image),
}

impl From<Image> for ImageParam {
    fn from(v: Image) -> Self {
        Self::Image(v)
    }
}

impl ImageParam {
    pub(crate) fn parse_list(it: &mut ElementIter<'_>) -> Result<Vec<Self>> {
        parse_list_many(it, |e| {
            Ok(Some(match e.name() {
                Image::NAME => Self::Image(Image::parse(e)?),
                NewParam::NAME => Self::NewParam(NewParam::parse(e)?),
                _ => return Ok(None),
            }))
        })
    }
}

impl XNodeWrite for ImageParam {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::NewParam(e) => e.write_to(w),
            Self::Image(e) => e.write_to(w),
        }
    }
}

fn parse_hex_array(s: &str) -> Box<[u8]> {
    let mut out = vec![];
    let mut hi = 0;
    let mut odd = false;
    for c in s.bytes() {
        let c = match c {
            b'0'..=b'9' => c - b'0',
            b'a'..=b'f' => c - b'a' + 10,
            b'A'..=b'F' => c - b'A' + 10,
            _ => continue,
        };
        if odd {
            out.push(hi << 4 | c)
        } else {
            hi = c
        }
        odd = !odd
    }
    out.into()
}

/// A specification of the source of data for an image.
#[derive(Clone, Debug)]
pub enum ImageSource {
    /// The data is provided directly as a byte buffer.
    Data(Box<[u8]>),
    /// A URI that specifies an external image file.
    InitFrom(Url),
}

impl ImageSource {
    /// Parse an [`ImageSource`] from an XML element.
    pub fn parse(element: &Element) -> Result<Option<Self>> {
        Ok(Some(match element.name() {
            "data" => {
                let s = get_text(element).ok_or("expected text element")?;
                ImageSource::Data(parse_hex_array(s))
            }
            "init_from" => ImageSource::InitFrom(parse_elem(element)?),
            _ => return Ok(None),
        }))
    }
}

impl XNodeWrite for ImageSource {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Data(data) => {
                let e = ElemBuilder::new("data").start(w)?;
                let mut indent = false;
                for chunk in data.chunks(40) {
                    if indent {
                        w.write_indent()?
                    }
                    indent = true;
                    for &c in chunk {
                        write!(w.get_mut(), "{:x}", c)?
                    }
                }
                e.end(w)
            }
            Self::InitFrom(url) => ElemBuilder::print("init_from", url, w),
        }
    }
}

/// Declares a two-dimensional texture sampler.
#[derive(Clone, Debug)]
pub struct Sampler2D {
    /// A name, which is the `sid` of a [`Surface`].
    /// A `Sampler*` is a definition of how a shader will resolve a
    /// color out of a [`Surface`]. `source` identifies the [`Surface`] to read.
    pub source: NameRef<Surface>,
    /// Wrap mode in the first texture coordinate.
    pub wrap_s: WrapMode,
    /// Wrap mode in the second texture coordinate.
    pub wrap_t: WrapMode,
    /// Texture minimization. Applying a texture to a primitive
    /// implies a mapping from texture image space to framebuffer image space.
    /// In general, this mapping involves a reconstruction of the sampled texture image,
    /// followed by a homogeneous warping implied by the mapping to framebuffer space,
    /// then a filtering, followed finally by a resampling of the filtered, warped,
    /// reconstructed image before applying it to a fragment.
    pub min_filter: SamplerFilter,
    /// Texture magnification. Enumerated type
    /// fx_sampler_filter_common. When gamma indicates
    /// magnification, this value determines how the texture value is
    /// obtained.
    pub mag_filter: SamplerFilter,
    /// MIPmap filter.
    pub mip_filter: SamplerFilter,
    /// When reading past the edge of the texture address space
    /// based on the wrap modes involving clamps, this color takes
    /// over. Type `fx_color_common` (four floating-point numbers in RGBA order).
    pub border_color: Option<Box<[f32; 4]>>,
    /// The maximum number of progressive levels that the sampler will evaluate.
    pub mipmap_max_level: u8,
    /// Biases the gamma (level of detail parameter) that is used by the sampler
    /// to evaluate the MIPmap chain.
    pub mipmap_bias: f32,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Sampler2D {
    /// Construct a new `Sampler2D` from a source.
    pub fn new(source: String) -> Self {
        Self {
            source: Ref::new(source),
            wrap_s: Default::default(),
            wrap_t: Default::default(),
            min_filter: Default::default(),
            mag_filter: Default::default(),
            mip_filter: Default::default(),
            border_color: None,
            mipmap_max_level: 0,
            mipmap_bias: 0.,
            extra: vec![],
        }
    }
}

impl XNode for Sampler2D {
    const NAME: &'static str = "sampler2D";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Sampler2D {
            source: parse_one("source", &mut it, parse_elem)?,
            wrap_s: parse_opt("wrap_s", &mut it, parse_elem)?.unwrap_or_default(),
            wrap_t: parse_opt("wrap_t", &mut it, parse_elem)?.unwrap_or_default(),
            min_filter: parse_opt("minfilter", &mut it, parse_elem)?.unwrap_or_default(),
            mag_filter: parse_opt("magfilter", &mut it, parse_elem)?.unwrap_or_default(),
            mip_filter: parse_opt("mipfilter", &mut it, parse_elem)?.unwrap_or_default(),
            border_color: parse_opt("border_color", &mut it, parse_array_n)?,
            mipmap_max_level: parse_opt("mipmap_maxlevel", &mut it, parse_elem)?.unwrap_or(0),
            mipmap_bias: parse_opt("mipmap_bias", &mut it, parse_elem)?.unwrap_or(0.),
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Sampler2D {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::print("source", &self.source, w)?;
        ElemBuilder::def_print("wrap_s", self.wrap_s, Default::default(), w)?;
        ElemBuilder::def_print("wrap_t", self.wrap_t, Default::default(), w)?;
        ElemBuilder::def_print("minfilter", self.min_filter, Default::default(), w)?;
        ElemBuilder::def_print("magfilter", self.mag_filter, Default::default(), w)?;
        ElemBuilder::def_print("mipfilter", self.mip_filter, Default::default(), w)?;
        opt(&self.border_color, |e| {
            ElemBuilder::print_arr("border_color", &**e, w)
        })?;
        ElemBuilder::def_print("mipmap_maxlevel", self.mipmap_max_level, 0, w)?;
        ElemBuilder::def_print("mipmap_bias", self.mipmap_bias, 0., w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Wrap modes that affect the interpretation of `s`, `t`, and `p` texture coordinates in `Sampler*`
/// elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapMode {
    /// OpenGL symbol `GL_REPEAT`.
    /// Ignores the integer part of texture coordinates, using only the fractional part.
    Wrap,
    /// OpenGL symbol `GL_MIRRORED_REPEAT`.
    /// First mirrors the texture coordinate.
    /// The mirrored coordinate is then clamped as described for [`Clamp`](WrapMode::Clamp).
    Mirror,
    /// OpenGL symbol `GL_CLAMP_TO_EDGE`.
    /// Clamps texture coordinates at all mipmap levels such
    /// that the texture filter never samples a border texel.
    /// *Note*: `GL_CLAMP` takes any texels beyond the
    /// sampling border and substitutes those texels with
    /// the border color. So `CLAMP_TO_EDGE` is more
    /// appropriate. This also works much better with
    /// OpenGL ES where the `GL_CLAMP` symbol was
    /// removed from the OpenGL ES specification.
    Clamp,
    /// OpenGL symbol `GL_CLAMP_TO_BORDER`.
    /// Clamps texture coordinates at all MIPmaps such that
    /// the texture filter always samples border texels for
    /// fragments whose corresponding texture coordinate
    /// is sufficiently far outside the range [0, 1].
    Border,
    /// The defined behavior for `None` is consistent with
    /// decal texturing where the border is black. Mapping
    /// this calculation to `GL_CLAMP_TO_BORDER` is the best approximation of this.
    None,
}

impl Default for WrapMode {
    fn default() -> Self {
        Self::Wrap
    }
}

impl FromStr for WrapMode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "WRAP" => Ok(Self::Wrap),
            "MIRROR" => Ok(Self::Mirror),
            "CLAMP" => Ok(Self::Clamp),
            "BORDER" => Ok(Self::Border),
            "NONE" => Ok(Self::None),
            _ => Err(()),
        }
    }
}

impl WrapMode {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Wrap => "WRAP",
            Self::Mirror => "MIRROR",
            Self::Clamp => "CLAMP",
            Self::Border => "BORDER",
            Self::None => "NONE",
        }
    }
}

impl Display for WrapMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// (Undocumented?) Enumerated type `fx_sampler_filter_common` from COLLADA spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SamplerFilter {
    None,
    Nearest,
    Linear,
    NearestMipmapNearest,
    LinearMipmapNearest,
    NearestMipmapLinear,
    LinearMipmapLinear,
}

impl Default for SamplerFilter {
    fn default() -> Self {
        Self::None
    }
}

impl FromStr for SamplerFilter {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "NONE" => Ok(Self::None),
            "NEAREST" => Ok(Self::Nearest),
            "LINEAR" => Ok(Self::Linear),
            "NEAREST_MIPMAP_NEAREST" => Ok(Self::NearestMipmapNearest),
            "LINEAR_MIPMAP_NEAREST" => Ok(Self::LinearMipmapNearest),
            "NEAREST_MIPMAP_LINEAR" => Ok(Self::NearestMipmapLinear),
            "LINEAR_MIPMAP_LINEAR" => Ok(Self::LinearMipmapLinear),
            _ => Err(()),
        }
    }
}

impl SamplerFilter {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::Nearest => "NEAREST",
            Self::Linear => "LINEAR",
            Self::NearestMipmapNearest => "NEAREST_MIPMAP_NEAREST",
            Self::LinearMipmapNearest => "LINEAR_MIPMAP_NEAREST",
            Self::NearestMipmapLinear => "NEAREST_MIPMAP_LINEAR",
            Self::LinearMipmapLinear => "LINEAR_MIPMAP_LINEAR",
        }
    }
}

impl Display for SamplerFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// Declares a resource that can be used both as the source for
/// texture samples and as the target of a rendering pass.
#[derive(Clone, Debug)]
pub struct Surface {
    /// The type of this surface.
    pub ty: SurfaceType,
    /// An initialization option for this surface.
    pub init: SurfaceInit,
    /// Contains a string representing the texel format for this surface.
    /// If this element is not specified or understood by the application,
    /// then the application will attempt to use `format_hint` if it is provided;
    /// otherwise, it should use a common format linear `R8G8B8A8`.
    pub format: Option<String>,
    /// An application uses `format_hint` if `format` does not exist or
    /// is not understood by the application and `format_hint` exists.
    /// This element describes the important features intended by the author
    /// so that the application can pick a format that best represents what the author wanted.
    pub format_hint: Option<Box<FormatHint>>,
    /// Contains three integer values. If specified, the surface is
    /// sized to these exact dimensions in texels. Surfaces of
    /// type `1D` and `CUBE` use only the first value. Surfaces of
    /// type `2D` and `RECT` use only the first two values,
    /// representing width and then height. Type `3D` uses all
    /// three values, representing width, height, and depth.
    /// Invalid if `viewport_ratio` is used.
    pub size: Option<Box<[u32; 3]>>,
    /// Contains two floating-point values representing width and then height.
    /// If specified, the surface is sized to a dimension
    /// based on these ratios of the viewport's (backbuffer's) dimensions.
    /// For example, `viewport_ratio = Some([0.5, 2])`
    /// scales the surface’s width to half the viewport’s width
    /// and its height to twice the viewport’s height.
    /// This element is valid only for surfaces of type `2D` or `RECT`.
    /// Invalid if `size` is used.
    pub viewport_ratio: Option<Box<[f32; 2]>>,
    /// Contains the number of MIP levels in the surface. A value
    /// of 0 assumes that all MIP levels exist until a dimension
    /// becomes 1 texel. To create a surface that has only one
    /// level of MIP maps (`mip` = 0), set this to 1.
    pub mip_levels: u32,
    /// If false and not all subsurfaces are
    /// initialized because you have not provided MIP-map
    /// levels, the generated surface will have profile- and
    ///
    /// platform-specific behavior. If true, the application is
    /// responsible for initializing the remainder of the
    /// subsurfaces; this is typically done through a graphics API
    /// render state or function that does this automatically, such
    /// as `glGenerateMipmap()`.
    pub mipmap_generate: bool,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Surface {
    /// Construct a new `Surface` from mandatory parameters.
    pub fn new(ty: SurfaceType, init: SurfaceInit) -> Self {
        Self {
            ty,
            init,
            format: None,
            format_hint: None,
            size: None,
            viewport_ratio: None,
            mip_levels: 0,
            mipmap_generate: false,
            extra: vec![],
        }
    }
}

impl XNode for Surface {
    const NAME: &'static str = "surface";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Surface {
            ty: parse_attr(element.attr("type"))?.ok_or("expected 'type' attr")?,
            init: parse_one_many(&mut it, SurfaceInit::parse)?,
            format: parse_opt("format", &mut it, parse_text)?,
            format_hint: FormatHint::parse_opt_box(&mut it)?,
            size: parse_opt("size", &mut it, parse_array_n)?,
            viewport_ratio: parse_opt("viewport_ratio", &mut it, parse_array_n)?,
            mip_levels: parse_opt("mip_levels", &mut it, parse_elem)?.unwrap_or(0),
            mipmap_generate: parse_opt("mipmap_generate", &mut it, parse_elem)?.unwrap_or(false),
            extra: Extra::parse_many(it)?,
        };
        if res.size.is_some() && res.viewport_ratio.is_some() {
            return Err("size and viewport_ratio cannot be used together".into());
        }
        Ok(res)
    }
}

impl XNodeWrite for Surface {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("type", self.ty);
        let e = e.start(w)?;
        self.init.write_to(w)?;
        opt(&self.format, |e| ElemBuilder::print_str("format", e, w))?;
        self.format_hint.write_to(w)?;
        opt(&self.size, |e| ElemBuilder::print_arr("size", &**e, w))?;
        opt(&self.viewport_ratio, |e| {
            ElemBuilder::print_arr("viewport_ratio", &**e, w)
        })?;
        ElemBuilder::def_print("mip_levels", self.mip_levels, 0, w)?;
        ElemBuilder::def_print("mipmap_generate", self.mipmap_generate, false, w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Specifies a surface on a cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceType {
    /// When a surface’s type attribute is set to `Untyped`,
    /// its type is initially unknown and established later by the context in which
    /// it is used, such as by a texture sampler that references it.
    /// A surface of any other type may be changed into an `Untyped` surface at
    /// run-time, as if it were created by [`NewParam`], using [`EffectSetParam`].
    /// If there is a type mismatch between a [`EffectSetParam`] operation and
    /// what the run-time decides the type should be, the result is profile- and
    /// platform-specific behavior.
    Untyped,
    /// A one dimensional texture.
    _1D,
    /// A two dimensional texture.
    _2D,
    /// A three dimensional texture.
    _3D,
    /// A RECT texture, see <http://www.opengl.org/registry/specs/ARB/texture_rectangle.txt>.
    Rect,
    /// A cube map.
    Cube,
    /// A depth map.
    Depth,
}

impl Default for SurfaceType {
    fn default() -> Self {
        Self::Untyped
    }
}

impl FromStr for SurfaceType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "UNTYPED" => Ok(Self::Untyped),
            "1D" => Ok(Self::_1D),
            "2D" => Ok(Self::_2D),
            "3D" => Ok(Self::_3D),
            "RECT" => Ok(Self::Rect),
            "CUBE" => Ok(Self::Cube),
            "DEPTH" => Ok(Self::Depth),
            _ => Err(()),
        }
    }
}

impl SurfaceType {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Untyped => "UNTYPED",
            Self::_1D => "1D",
            Self::_2D => "2D",
            Self::_3D => "3D",
            Self::Rect => "RECT",
            Self::Cube => "CUBE",
            Self::Depth => "DEPTH",
        }
    }
}

impl Display for SurfaceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// This element describes the important features intended by the author so that the
/// application can pick a format that best represents what the author wanted.
#[derive(Clone, Debug)]
pub struct FormatHint {
    /// The per-texel layout of the format.
    pub channels: SurfaceChannels,
    /// The range of texel channel values.
    pub range: SurfaceRange,
    /// The precision of the texel channel value.
    pub precision: SurfacePrecision,
    /// Additional hints about data relationships and other
    /// things to help an application pick the best format.
    pub options: Vec<SurfaceOption>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl FormatHint {
    /// Construct a new `FormatHint`.
    pub fn new(
        channels: SurfaceChannels,
        range: SurfaceRange,
        precision: SurfacePrecision,
    ) -> Self {
        Self {
            channels,
            range,
            precision,
            options: vec![],
            extra: vec![],
        }
    }
}

impl XNode for FormatHint {
    const NAME: &'static str = "format_hint";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(FormatHint {
            channels: parse_one("channels", &mut it, parse_elem)?,
            range: parse_one("range", &mut it, parse_elem)?,
            precision: parse_one("precision", &mut it, parse_elem)?,
            options: parse_list("option", &mut it, parse_elem)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for FormatHint {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::print("channels", &self.channels, w)?;
        ElemBuilder::print("range", &self.range, w)?;
        ElemBuilder::print("precision", &self.precision, w)?;
        many(&self.options, |e| ElemBuilder::print("option", e, w))?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// A [`Surface`] initialization option, which specifies
/// whether to initialize the surface and how to do so.
#[derive(Clone, Debug)]
pub enum SurfaceInit {
    /// This surface is intended to be initialized later externally by a [`EffectSetParam`] element.
    /// If it is used before being initialized, there is profile- and platform-specific behavior.
    /// Most elements on the [`Surface`] element that contains this will be ignored,
    /// including [`mip_levels`](Surface::mip_levels), [`mipmap_generate`](Surface::mipmap_generate),
    /// [`size`](Surface::size), [`viewport_ratio`](Surface::viewport_ratio),
    /// and [`format`](Surface::format).
    Null,
    /// Initializes this surface as a target for depth, stencil, or color. It does not need image
    /// data. If this element is used, [`mipmap_generate`](Surface::mipmap_generate) is ignored.
    Target,
    // Cube(InitCube),
    // Volume(InitVolume),
    // Planar(InitPlanar),
    /// Contains a reference to a 1D or 2D image. Initializes the surface one subsurface at
    /// a time by specifying combinations of `mip`, `face`, and `slice` that make sense for a
    /// particular surface type. Each subsurface is initialized by a common 1-D or 2-D
    /// image, not a complex compound image such as DDS. If not all subsurfaces are
    /// initialized, the surface is invalid and will result in profile- and platform-specific
    /// behavior unless [`mipmap_generate`](Surface::mipmap_generate) is responsible for
    /// initializing the remaining subsurfaces.
    From {
        /// The MIP level.
        mip: u32,
        /// Which 2D layer within a volume to initialize.
        /// There are anywhere from 0 to `n` slices in a volume,
        /// where `n` is the volume’s depth slice.
        /// This attribute is used in combination with `mip` because a volume might have MIPmaps.
        slice: u32,
        /// Which surface of a cube to initialize from the specified image.
        /// This attribute is used in combination with `mip` because a cubemap might
        /// have MIPmaps.
        face: SurfaceFace,
        /// Each subsurface is initialized by a common 1-D or 2-D
        /// image, not a complex compound image such as DDS.
        image: NameRef<Image>,
    },
}

impl SurfaceInit {
    /// Construct a `SurfaceInit::From` variant with default arguments.
    pub fn init_from(image: impl Into<String>) -> Self {
        Self::From {
            mip: 0,
            slice: 0,
            face: Default::default(),
            image: Ref::new(image.into()),
        }
    }
    /// Parse a [`SurfaceInit`] from an XML element.
    pub fn parse(element: &Element) -> Result<Option<Self>> {
        Ok(Some(match element.name() {
            "init_as_null" => Self::Null,
            "init_as_target" => Self::Target,
            "init_cube" | "init_volume" | "init_planar" => unimplemented!(),
            "init_from" => Self::From {
                mip: parse_attr(element.attr("mip"))?.unwrap_or(0),
                slice: parse_attr(element.attr("slice"))?.unwrap_or(0),
                face: parse_attr(element.attr("face"))?.unwrap_or_default(),
                image: parse_elem(element)?,
            },
            _ => return Ok(None),
        }))
    }
}

impl XNodeWrite for SurfaceInit {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Null => ElemBuilder::new("init_as_null").end(w),
            Self::Target => ElemBuilder::new("init_as_target").end(w),
            &Self::From {
                mip,
                slice,
                face,
                ref image,
            } => {
                let mut e = ElemBuilder::new("init_from");
                e.def_print_attr("mip", mip, 0);
                e.def_print_attr("slice", slice, 0);
                e.def_print_attr("face", face, Default::default());
                let e = e.start(w)?;
                print_elem(image, w)?;
                e.end(w)
            }
        }
    }
}

/// Specifies a surface on a cube.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceFace {
    /// The `+x` face
    PosX,
    /// The `-x` face
    NegX,
    /// The `+y` face
    PosY,
    /// The `-y` face
    NegY,
    /// The `+z` face
    PosZ,
    /// The `-z` face
    NegZ,
}

impl Default for SurfaceFace {
    fn default() -> Self {
        Self::PosX
    }
}

impl FromStr for SurfaceFace {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "POSITIVE_X" => Ok(Self::PosX),
            "NEGATIVE_X" => Ok(Self::NegX),
            "POSITIVE_Y" => Ok(Self::PosY),
            "NEGATIVE_Y" => Ok(Self::NegY),
            "POSITIVE_Z" => Ok(Self::PosZ),
            "NEGATIVE_Z" => Ok(Self::NegZ),
            _ => Err(()),
        }
    }
}

impl SurfaceFace {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::PosX => "POSITIVE_X",
            Self::NegX => "NEGATIVE_X",
            Self::PosY => "POSITIVE_Y",
            Self::NegY => "NEGATIVE_Y",
            Self::PosZ => "POSITIVE_Z",
            Self::NegZ => "NEGATIVE_Z",
        }
    }
}

impl Display for SurfaceFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// The per-texel layout of the format.
/// The length of the enumeration string indicates how many channels there are
/// and each letter represents the name of a channel. There are typically 1 to 4 channels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceChannels {
    /// Red/Green/Blue color map.
    RGB,
    /// Red/Green/Blue/Alpha map, often used for color and transparency
    /// or other things packed into channel A, such as specular power.
    RGBA,
    /// Luminance map, often used for light mapping.
    L,
    /// Luminance/Alpha map, often used for light mapping.
    LA,
    /// Depth map, often used for displacement, parallax, relief, or shadow mapping.
    D,
    /// Typically used for normal maps or three-component displacement maps.
    XYZ,
    /// Typically used for normal maps, where `W` is the depth for relief or parallax mapping.
    XYZW,
}

impl FromStr for SurfaceChannels {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "RGB" => Ok(Self::RGB),
            "RGBA" => Ok(Self::RGBA),
            "L" => Ok(Self::L),
            "LA" => Ok(Self::LA),
            "D" => Ok(Self::D),
            "XYZ" => Ok(Self::XYZ),
            "XYZW" => Ok(Self::XYZW),
            _ => Err(()),
        }
    }
}

impl SurfaceChannels {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::RGB => "RGB",
            Self::RGBA => "RGBA",
            Self::L => "L",
            Self::LA => "LA",
            Self::D => "D",
            Self::XYZ => "XYZ",
            Self::XYZW => "XYZW",
        }
    }
}

impl Display for SurfaceChannels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// The range of texel channel values. Each channel represents a range of values.
/// Some example ranges are signed or unsigned integers, or
/// are within a clamped range such as 0.0f to 1.0f, or are a
/// high dynamic range via floating point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRange {
    /// Format represents a decimal value that remains within the -1 to 1 range.
    /// Implementation could be integer, fixed-point, or float.
    SNorm,
    /// Format represents a decimal value that remains within the 0 to 1 range.
    /// Implementation could be integer, fixed-point, or float.
    UNorm,
    /// Format represents signed integer numbers;
    /// for example, 8 bits can represent -128 to 127.
    SInt,
    /// Format represents unsigned integer numbers.
    /// For example, 8 bits can represent 0 to 255.
    UInt,
    /// Format should support full floating-point ranges typically used for high dynamic range.
    Float,
}

impl FromStr for SurfaceRange {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SNORM" => Ok(Self::SNorm),
            "UNORM" => Ok(Self::UNorm),
            "SINT" => Ok(Self::SInt),
            "UINT" => Ok(Self::UInt),
            "FLOAT" => Ok(Self::Float),
            _ => Err(()),
        }
    }
}

impl SurfaceRange {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::SNorm => "SNORM",
            Self::UNorm => "UNORM",
            Self::SInt => "SINT",
            Self::UInt => "UINT",
            Self::Float => "FLOAT",
        }
    }
}

impl Display for SurfaceRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// The precision of the texel channel value.
///
/// Each channel of the texel has a precision. Typically, channels have the same precision. An
/// exact format may lower the precision of an individual channel
/// but applying a higher precision by linking the channels may still convey the same information.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfacePrecision {
    /// For integers, this typically represents 8 bits.
    /// For floats, typically 16 bits.
    Low,
    /// For integers, this typically represents 8 to 24
    /// bits. For floats, typically 16 to 32 bits.
    Mid,
    /// For integers, this typically represents 16 to
    /// 32 bits. For floats, typically 24 to 32 bits.
    High,
}

impl FromStr for SurfacePrecision {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "LOW" => Ok(Self::Low),
            "MID" => Ok(Self::Mid),
            "HIGH" => Ok(Self::High),
            _ => Err(()),
        }
    }
}

impl SurfacePrecision {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Low => "LOW",
            Self::Mid => "MID",
            Self::High => "HIGH",
        }
    }
}

impl Display for SurfacePrecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// Contains additional hints about data relationships and other
/// things to help an application pick the best format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceOption {
    /// Colors are stored with respect
    /// to the sRGB 2.2 gamma curve rather than linear.
    SrgbGamma,
    /// The texel’s XYZ/RGB should be
    /// normalized such as in a normal map.
    Normalized3,
    /// The texel’s XYZW/RGBA should
    /// be normalized such as in a normal map.
    Normalized4,
    /// The surface may use run-time compression.
    /// Consider the best compression based on desired [`SurfaceChannels`],
    /// [`SurfaceRange`], [`SurfacePrecision`], and [`SurfaceOption`]s.
    Compressible,
}

impl FromStr for SurfaceOption {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "SRGB_GAMMA" => Ok(Self::SrgbGamma),
            "NORMALIZED3" => Ok(Self::Normalized3),
            "NORMALIZED4" => Ok(Self::Normalized4),
            "COMPRESSABLE" => Ok(Self::Compressible),
            _ => Err(()),
        }
    }
}

impl SurfaceOption {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::SrgbGamma => "SRGB_GAMMA",
            Self::Normalized3 => "NORMALIZED3",
            Self::Normalized4 => "NORMALIZED4",
            Self::Compressible => "COMPRESSABLE",
        }
    }
}

impl Display for SurfaceOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}
