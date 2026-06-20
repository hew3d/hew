use crate::*;

/// A `<profile_*>` element.
#[derive(Clone, Debug)]
pub enum Profile {
    /// Opens a block of platform-independent declarations for the common,
    /// fixed-function shader.
    Common(ProfileCommon),
    /// Declares platform-specific data types and [`Technique`](TechniqueFx)s
    /// for the Cg language.
    CG(ProfileCG),
    /// Declares platform-specific data types and [`Technique`](TechniqueFx)s
    /// for OpenGL ES.
    GLES(ProfileGLES),
    /// Declares platform-specific data types and [`Technique`](TechniqueFx)s
    /// for OpenGL Shading Language.
    GLSL(ProfileGLSL),
}

impl From<ProfileCommon> for Profile {
    fn from(v: ProfileCommon) -> Self {
        Self::Common(v)
    }
}

impl From<ProfileCG> for Profile {
    fn from(v: ProfileCG) -> Self {
        Self::CG(v)
    }
}

impl From<ProfileGLES> for Profile {
    fn from(v: ProfileGLES) -> Self {
        Self::GLES(v)
    }
}

impl From<ProfileGLSL> for Profile {
    fn from(v: ProfileGLSL) -> Self {
        Self::GLSL(v)
    }
}

impl Profile {
    /// Parse a [`Profile`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            ProfileCommon::NAME => Self::Common(ProfileCommon::parse(e)?),
            ProfileCG::NAME => Self::CG(ProfileCG::parse(e)?),
            ProfileGLES::NAME => Self::GLES(ProfileGLES::parse(e)?),
            ProfileGLSL::NAME => Self::GLSL(ProfileGLSL::parse(e)?),
            _ => return Ok(None),
        }))
    }

    /// Variant extractor for [`ProfileCommon`].
    pub fn as_common(&self) -> Option<&ProfileCommon> {
        match self {
            Profile::Common(p) => Some(p),
            _ => None,
        }
    }
}

impl XNodeWrite for Profile {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Common(e) => e.write_to(w),
            Self::CG(e) => e.write_to(w),
            Self::GLES(e) => e.write_to(w),
            Self::GLSL(e) => e.write_to(w),
        }
    }
}

/// Opens a block of platform-independent declarations for the common, fixed-function shader.
#[derive(Clone, Debug)]
pub struct ProfileCommon {
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// Declares a standard COLLADA image resource.
    pub image: Vec<Image>,
    /// Creates a new parameter from a constrained set of
    /// types recognizable by all platforms, see [`ParamType`].
    pub new_param: Vec<NewParam>,
    /// Declares the only technique for this effect.
    pub technique: TechniqueFx<CommonData>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for ProfileCommon {
    const NAME: &'static str = "profile_COMMON";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let asset = Asset::parse_opt_box(&mut it)?;
        let image_param = ImageParam::parse_list(&mut it)?;
        let mut image = vec![];
        let mut new_param = vec![];
        for ip in image_param {
            match ip {
                ImageParam::Image(e) => image.push(e),
                ImageParam::NewParam(e) => new_param.push(e),
            }
        }
        Ok(ProfileCommon {
            asset,
            image,
            new_param,
            technique: TechniqueFx::parse_one(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for ProfileCommon {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.asset.write_to(w)?;
        self.image.write_to(w)?;
        self.new_param.write_to(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl ProfileCommon {
    /// Construct a new `ProfileCommon` from the `TechniqueFx` data.
    pub fn new(technique: TechniqueFx<CommonData>) -> Self {
        Self {
            asset: None,
            image: vec![],
            new_param: vec![],
            technique,
            extra: vec![],
        }
    }

    /// Get a parameter by name, looking in the parameters to the technique,
    /// the parameters to the profile, and finally the parameters to the parent effect.
    pub fn get_param<'a>(&'a self, parent: &'a Effect, sid: &str) -> Option<&'a NewParam> {
        for p in self.technique.data.image_param.iter().rev() {
            if let ImageParam::NewParam(p) = p {
                if p.sid == sid {
                    return Some(p);
                }
            }
        }
        for p in self.new_param.iter().rev() {
            if p.sid == sid {
                return Some(p);
            }
        }
        parent.get_param(sid)
    }
}

/// The extra data in a [`TechniqueFx`] as the child of [`ProfileCommon`].
#[derive(Clone, Default, Debug)]
pub struct CommonData {
    /// A list of [`Image`] and [`NewParam`] elements in no particular order.
    pub image_param: Vec<ImageParam>,
    /// A list of shader elements.
    pub shaders: Vec<Shader>,
}

impl ProfileData for CommonData {
    fn parse(it: &mut ElementIter<'_>) -> Result<Self> {
        Ok(CommonData {
            image_param: ImageParam::parse_list(it)?,
            shaders: parse_list_many(it, Shader::parse)?,
        })
    }
}

impl XNodeWrite for CommonData {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.image_param.write_to(w)?;
        self.shaders.write_to(w)
    }
}

impl CommonData {
    /// Construct a simple `CommonData` with one shader.
    pub fn shader(shader: impl Into<Shader>) -> Self {
        Self {
            image_param: vec![],
            shaders: vec![shader.into()],
        }
    }

    /// Run the function `f` on all arguments of type [`Texture`] in the profile.
    pub fn on_textures<'a, E>(
        &'a self,
        mut f: impl FnMut(&'a Texture) -> Result<(), E>,
    ) -> Result<(), E> {
        for s in &self.shaders {
            s.on_textures(&mut f)?
        }
        Ok(())
    }
}

/// The `<profile_CG>` element and its contents are unsupported
/// and represented here as a raw XML element.
#[derive(Clone, Debug)]
pub struct ProfileCG(pub Element); // TODO

impl XNode for ProfileCG {
    const NAME: &'static str = "profile_CG";
    fn parse(element: &Element) -> Result<Self> {
        Ok(ProfileCG(element.clone()))
    }
}

impl XNodeWrite for ProfileCG {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        XNodeWrite::write_to(&self.0, w)
    }
}

/// The `<profile_GLES>` element and its contents are unsupported
/// and represented here as a raw XML element.
#[derive(Clone, Debug)]
pub struct ProfileGLES(pub Element); // TODO

impl XNode for ProfileGLES {
    const NAME: &'static str = "profile_GLES";
    fn parse(element: &Element) -> Result<Self> {
        Ok(ProfileGLES(element.clone()))
    }
}

impl XNodeWrite for ProfileGLES {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        XNodeWrite::write_to(&self.0, w)
    }
}

/// The `<profile_GLSL>` element and its contents are unsupported
/// and represented here as a raw XML element.
#[derive(Clone, Debug)]
pub struct ProfileGLSL(pub Element); // TODO

impl XNode for ProfileGLSL {
    const NAME: &'static str = "profile_GLSL";
    fn parse(element: &Element) -> Result<Self> {
        Ok(ProfileGLSL(element.clone()))
    }
}

impl XNodeWrite for ProfileGLSL {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        XNodeWrite::write_to(&self.0, w)
    }
}
