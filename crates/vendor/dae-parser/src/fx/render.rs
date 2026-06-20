use crate::*;

/// Describes one effect pass to evaluate a scene.
#[derive(Clone, Debug)]
pub struct Render {
    /// Refers to a node that contains a camera describing the viewpoint
    /// from which to render this compositing step.
    pub camera_node: UrlRef<Node>,
    /// Specifies which layer or layers to render in this compositing step
    /// while evaluating the scene.
    pub layers: Vec<String>,
    /// Instantiates a COLLADA material resource. See [`InstanceEffectData`]
    /// for the additional instance effect data.
    pub instance_effect: Option<Instance<Effect>>,
}

impl Render {
    /// Construct a new render pass.
    pub fn new(camera_node: Url, layers: Vec<String>, instance_effect: Url) -> Self {
        Self {
            camera_node: Ref::new(camera_node),
            layers,
            instance_effect: Some(Instance::new(instance_effect)),
        }
    }
}

impl XNode for Render {
    const NAME: &'static str = "render";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Render {
            camera_node: parse_attr(element.attr("camera_node"))?
                .ok_or("missing camera_node attr")?,
            layers: parse_list("layer", &mut it, parse_text)?,
            instance_effect: Instance::parse_opt(&mut it)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for Render {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("camera_node", &self.camera_node);
        let e = e.start(w)?;
        many(&self.layers, |e| ElemBuilder::print_str("layer", e, w))?;
        self.instance_effect.write_to(w)?;
        e.end(w)
    }
}

/// A shader element.
#[derive(Clone, Debug)]
pub enum Shader {
    /// Produces a specularly shaded surface with a Blinn BRDF approximation.
    Blinn(Blinn),
    /// Produces a constantly shaded surface that is independent of lighting.
    Constant(ConstantFx),
    /// Produces a diffuse shaded surface that is independent of lighting.
    Lambert(Lambert),
    /// Produces a specularly shaded surface where the specular reflection is shaded
    /// according the Phong BRDF approximation.
    Phong(Phong),
}

impl From<Blinn> for Shader {
    fn from(v: Blinn) -> Self {
        Self::Blinn(v)
    }
}

impl From<ConstantFx> for Shader {
    fn from(v: ConstantFx) -> Self {
        Self::Constant(v)
    }
}

impl From<Lambert> for Shader {
    fn from(v: Lambert) -> Self {
        Self::Lambert(v)
    }
}

impl From<Phong> for Shader {
    fn from(v: Phong) -> Self {
        Self::Phong(v)
    }
}

impl Shader {
    /// Parse a [`Shader`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            Blinn::NAME => Self::Blinn(Blinn::parse(e)?),
            ConstantFx::NAME => Self::Constant(ConstantFx::parse(e)?),
            Lambert::NAME => Self::Lambert(Lambert::parse(e)?),
            Phong::NAME => Self::Phong(Phong::parse(e)?),
            _ => return Ok(None),
        }))
    }

    /// Run the function `f` on all arguments of type [`Texture`] in the parameters to this shader.
    pub fn on_textures<'a, E>(
        &'a self,
        f: &mut impl FnMut(&'a Texture) -> Result<(), E>,
    ) -> Result<(), E> {
        match self {
            Self::Blinn(s) => s.on_textures(f),
            Self::Constant(s) => s.on_textures(f),
            Self::Lambert(s) => s.on_textures(f),
            Self::Phong(s) => s.on_textures(f),
        }
    }
}

impl XNodeWrite for Shader {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Blinn(e) => e.write_to(w),
            Self::Constant(e) => e.write_to(w),
            Self::Lambert(e) => e.write_to(w),
            Self::Phong(e) => e.write_to(w),
        }
    }
}

/// Produces a specularly shaded surface with a Blinn BRDF approximation.
#[derive(Clone, Default, Debug)]
pub struct Blinn {
    /// Declares the amount of light emitted from the surface of this object.
    pub emission: Option<WithSid<ColorParam>>,
    /// Declares the amount of ambient light emitted from the surface of this object.
    pub ambient: Option<WithSid<ColorParam>>,
    /// Declares the amount of light diffusely reflected from the surface of this object.
    pub diffuse: Option<WithSid<ColorParam>>,
    /// Declares the color of light specularly reflected from the surface of this object.
    pub specular: Option<WithSid<ColorParam>>,
    /// Declares the specularity or roughness of the specular reflection lobe.
    pub shininess: Option<WithSid<FloatParam>>,
    /// Declares the color of a perfect mirror reflection.
    pub reflective: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfect mirror reflection to be added
    /// to the reflected light as a value between 0.0 and 1.0.
    pub reflectivity: Option<WithSid<FloatParam>>,
    /// Declares the color of perfectly refracted light.
    pub transparent: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfectly refracted light added
    /// to the reflected color as a scalar value between 0.0 and 1.0.
    pub transparency: Option<WithSid<FloatParam>>,
    /// Declares the index of refraction for perfectly refracted light
    /// as a single scalar index.
    pub index_of_refraction: Option<WithSid<FloatParam>>,
}

impl XNode for Blinn {
    const NAME: &'static str = "blinn";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Blinn {
            emission: parse_opt("emission", &mut it, WithSid::parse)?,
            ambient: parse_opt("ambient", &mut it, WithSid::parse)?,
            diffuse: parse_opt("diffuse", &mut it, WithSid::parse)?,
            specular: parse_opt("specular", &mut it, WithSid::parse)?,
            shininess: parse_opt("shininess", &mut it, WithSid::parse)?,
            reflective: parse_opt("reflective", &mut it, WithSid::parse)?,
            reflectivity: parse_opt("reflectivity", &mut it, WithSid::parse)?,
            transparent: parse_opt("transparent", &mut it, WithSid::parse)?,
            transparency: parse_opt("transparency", &mut it, WithSid::parse)?,
            index_of_refraction: parse_opt("index_of_refraction", &mut it, WithSid::parse)?,
        })
    }
}

impl XNodeWrite for Blinn {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        WithSid::write_opt(&self.emission, "emission", w)?;
        WithSid::write_opt(&self.ambient, "ambient", w)?;
        WithSid::write_opt(&self.diffuse, "diffuse", w)?;
        WithSid::write_opt(&self.specular, "specular", w)?;
        WithSid::write_opt(&self.shininess, "shininess", w)?;
        WithSid::write_opt(&self.reflective, "reflective", w)?;
        WithSid::write_opt(&self.reflectivity, "reflectivity", w)?;
        WithSid::write_opt(&self.transparent, "transparent", w)?;
        WithSid::write_opt(&self.transparency, "transparency", w)?;
        WithSid::write_opt(&self.index_of_refraction, "index_of_refraction", w)?;
        e.end(w)
    }
}

impl Blinn {
    /// Run the function `f` on all arguments of type [`Texture`] in the parameters to this shader.
    pub fn on_textures<'a, E>(
        &'a self,
        f: &mut impl FnMut(&'a Texture) -> Result<(), E>,
    ) -> Result<(), E> {
        on_color_as_texture(&self.emission, f)?;
        on_color_as_texture(&self.ambient, f)?;
        on_color_as_texture(&self.diffuse, f)?;
        on_color_as_texture(&self.specular, f)?;
        on_color_as_texture(&self.reflective, f)?;
        on_color_as_texture(&self.transparent, f)
    }
}

/// Produces a constantly shaded surface that is independent of lighting.
#[derive(Clone, Default, Debug)]
pub struct ConstantFx {
    /// Declares the amount of light emitted from the surface of this object.
    pub emission: Option<WithSid<ColorParam>>,
    /// Declares the color of a perfect mirror reflection.
    pub reflective: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfect mirror reflection to be added
    /// to the reflected light as a value between 0.0 and 1.0.
    pub reflectivity: Option<WithSid<FloatParam>>,
    /// Declares the color of perfectly refracted light.
    pub transparent: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfectly refracted light added
    /// to the reflected color as a scalar value between 0.0 and 1.0.
    pub transparency: Option<WithSid<FloatParam>>,
    /// Declares the index of refraction for perfectly refracted light
    /// as a single scalar index.
    pub index_of_refraction: Option<WithSid<FloatParam>>,
}

impl XNode for ConstantFx {
    const NAME: &'static str = "constant";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(ConstantFx {
            emission: parse_opt("emission", &mut it, WithSid::parse)?,
            reflective: parse_opt("reflective", &mut it, WithSid::parse)?,
            reflectivity: parse_opt("reflectivity", &mut it, WithSid::parse)?,
            transparent: parse_opt("transparent", &mut it, WithSid::parse)?,
            transparency: parse_opt("transparency", &mut it, WithSid::parse)?,
            index_of_refraction: parse_opt("index_of_refraction", &mut it, WithSid::parse)?,
        })
    }
}

impl XNodeWrite for ConstantFx {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        WithSid::write_opt(&self.emission, "emission", w)?;
        WithSid::write_opt(&self.reflective, "reflective", w)?;
        WithSid::write_opt(&self.reflectivity, "reflectivity", w)?;
        WithSid::write_opt(&self.transparent, "transparent", w)?;
        WithSid::write_opt(&self.transparency, "transparency", w)?;
        WithSid::write_opt(&self.index_of_refraction, "index_of_refraction", w)?;
        e.end(w)
    }
}

impl ConstantFx {
    /// Run the function `f` on all arguments of type [`Texture`] in the parameters to this shader.
    pub fn on_textures<'a, E>(
        &'a self,
        f: &mut impl FnMut(&'a Texture) -> Result<(), E>,
    ) -> Result<(), E> {
        on_color_as_texture(&self.emission, f)?;
        on_color_as_texture(&self.reflective, f)?;
        on_color_as_texture(&self.transparent, f)
    }
}

/// Produces a diffuse shaded surface that is independent of lighting.
#[derive(Clone, Default, Debug)]
pub struct Lambert {
    /// Declares the amount of light emitted from the surface of this object.
    pub emission: Option<WithSid<ColorParam>>,
    /// Declares the amount of ambient light emitted from the surface of this object.
    pub ambient: Option<WithSid<ColorParam>>,
    /// Declares the amount of light diffusely reflected from the surface of this object.
    pub diffuse: Option<WithSid<ColorParam>>,
    /// Declares the color of a perfect mirror reflection.
    pub reflective: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfect mirror reflection to be added
    /// to the reflected light as a value between 0.0 and 1.0.
    pub reflectivity: Option<WithSid<FloatParam>>,
    /// Declares the color of perfectly refracted light.
    pub transparent: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfectly refracted light added
    /// to the reflected color as a scalar value between 0.0 and 1.0.
    pub transparency: Option<WithSid<FloatParam>>,
    /// Declares the index of refraction for perfectly refracted light
    /// as a single scalar index.
    pub index_of_refraction: Option<WithSid<FloatParam>>,
}

impl XNode for Lambert {
    const NAME: &'static str = "lambert";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Lambert {
            emission: parse_opt("emission", &mut it, WithSid::parse)?,
            ambient: parse_opt("ambient", &mut it, WithSid::parse)?,
            diffuse: parse_opt("diffuse", &mut it, WithSid::parse)?,
            reflective: parse_opt("reflective", &mut it, WithSid::parse)?,
            reflectivity: parse_opt("reflectivity", &mut it, WithSid::parse)?,
            transparent: parse_opt("transparent", &mut it, WithSid::parse)?,
            transparency: parse_opt("transparency", &mut it, WithSid::parse)?,
            index_of_refraction: parse_opt("index_of_refraction", &mut it, WithSid::parse)?,
        })
    }
}

impl XNodeWrite for Lambert {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        WithSid::write_opt(&self.emission, "emission", w)?;
        WithSid::write_opt(&self.ambient, "ambient", w)?;
        WithSid::write_opt(&self.diffuse, "diffuse", w)?;
        WithSid::write_opt(&self.reflective, "reflective", w)?;
        WithSid::write_opt(&self.reflectivity, "reflectivity", w)?;
        WithSid::write_opt(&self.transparent, "transparent", w)?;
        WithSid::write_opt(&self.transparency, "transparency", w)?;
        WithSid::write_opt(&self.index_of_refraction, "index_of_refraction", w)?;
        e.end(w)
    }
}

impl Lambert {
    /// Run the function `f` on all arguments of type [`Texture`] in the parameters to this shader.
    pub fn on_textures<'a, E>(
        &'a self,
        f: &mut impl FnMut(&'a Texture) -> Result<(), E>,
    ) -> Result<(), E> {
        on_color_as_texture(&self.emission, f)?;
        on_color_as_texture(&self.ambient, f)?;
        on_color_as_texture(&self.diffuse, f)?;
        on_color_as_texture(&self.reflective, f)?;
        on_color_as_texture(&self.transparent, f)
    }
}

/// Produces a specularly shaded surface where the specular reflection is shaded
/// according the Phong BRDF approximation.
#[derive(Clone, Default, Debug)]
pub struct Phong {
    /// Declares the amount of light emitted from the surface of this object.
    pub emission: Option<WithSid<ColorParam>>,
    /// Declares the amount of ambient light emitted from the surface of this object.
    pub ambient: Option<WithSid<ColorParam>>,
    /// Declares the amount of light diffusely reflected from the surface of this object.
    pub diffuse: Option<WithSid<ColorParam>>,
    /// the surface of this object.  the surface of this object.
    pub specular: Option<WithSid<ColorParam>>,
    /// reflection lobe.reflection lobe.
    pub shininess: Option<WithSid<FloatParam>>,
    /// Declares the color of a perfect mirror reflection.
    pub reflective: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfect mirror reflection to be added
    /// to the reflected light as a value between 0.0 and 1.0.
    pub reflectivity: Option<WithSid<FloatParam>>,
    /// Declares the color of perfectly refracted light.
    pub transparent: Option<WithSid<ColorParam>>,
    /// Declares the amount of perfectly refracted light added
    /// to the reflected color as a scalar value between 0.0 and 1.0.
    pub transparency: Option<WithSid<FloatParam>>,
    /// Declares the index of refraction for perfectly refracted light
    /// as a single scalar index.
    pub index_of_refraction: Option<WithSid<FloatParam>>,
}

impl XNode for Phong {
    const NAME: &'static str = "phong";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Phong {
            emission: parse_opt("emission", &mut it, WithSid::parse)?,
            ambient: parse_opt("ambient", &mut it, WithSid::parse)?,
            diffuse: parse_opt("diffuse", &mut it, WithSid::parse)?,
            specular: parse_opt("specular", &mut it, WithSid::parse)?,
            shininess: parse_opt("shininess", &mut it, WithSid::parse)?,
            reflective: parse_opt("reflective", &mut it, WithSid::parse)?,
            reflectivity: parse_opt("reflectivity", &mut it, WithSid::parse)?,
            transparent: parse_opt("transparent", &mut it, WithSid::parse)?,
            transparency: parse_opt("transparency", &mut it, WithSid::parse)?,
            index_of_refraction: parse_opt("index_of_refraction", &mut it, WithSid::parse)?,
        })
    }
}

impl XNodeWrite for Phong {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        WithSid::write_opt(&self.emission, "emission", w)?;
        WithSid::write_opt(&self.ambient, "ambient", w)?;
        WithSid::write_opt(&self.diffuse, "diffuse", w)?;
        WithSid::write_opt(&self.specular, "specular", w)?;
        WithSid::write_opt(&self.shininess, "shininess", w)?;
        WithSid::write_opt(&self.reflective, "reflective", w)?;
        WithSid::write_opt(&self.reflectivity, "reflectivity", w)?;
        WithSid::write_opt(&self.transparent, "transparent", w)?;
        WithSid::write_opt(&self.transparency, "transparency", w)?;
        WithSid::write_opt(&self.index_of_refraction, "index_of_refraction", w)?;
        e.end(w)
    }
}

impl Phong {
    /// Run the function `f` on all arguments of type [`Texture`] in the parameters to this shader.
    pub fn on_textures<'a, E>(
        &'a self,
        f: &mut impl FnMut(&'a Texture) -> Result<(), E>,
    ) -> Result<(), E> {
        on_color_as_texture(&self.emission, f)?;
        on_color_as_texture(&self.ambient, f)?;
        on_color_as_texture(&self.diffuse, f)?;
        on_color_as_texture(&self.specular, f)?;
        on_color_as_texture(&self.reflective, f)?;
        on_color_as_texture(&self.transparent, f)
    }
}

/// A struct that attaches an optional SID to a shader parameter.
#[derive(Clone, Default, Debug)]
pub struct WithSid<T> {
    sid: Option<String>,
    data: T,
}

impl<T> Deref for WithSid<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

pub(crate) use private::CanWithSid;
pub(crate) mod private {
    use super::*;
    pub trait CanWithSid: XNodeWrite + Sized {
        fn parse(element: &Element) -> Result<Option<Self>>;

        fn write_with_sid<W: Write>(&self, sid: &Option<String>, w: &mut XWriter<W>) -> Result<()>;
    }
}

impl<T> From<T> for WithSid<T> {
    fn from(data: T) -> Self {
        Self::new(data)
    }
}

impl<T> WithSid<T> {
    /// Construct a new `WithSid` with no sid.
    pub fn new(data: T) -> Self {
        Self { sid: None, data }
    }

    /// Construct a new `WithSid` with a sid.
    #[allow(clippy::self_named_constructors)]
    pub fn with_sid(sid: impl Into<String>, data: T) -> Self {
        Self {
            sid: Some(sid.into()),
            data,
        }
    }
}

impl<T: CanWithSid> WithSid<T> {
    /// Parse a [`WithSid<T>`] from an XML element.
    pub fn parse(element: &Element) -> Result<Self> {
        let mut it = element.children().peekable();
        parse_one_many(&mut it, |e| {
            Ok(T::parse(e)?.map(|data| Self {
                sid: e.attr("sid").map(Into::into),
                data,
            }))
        })
    }

    fn write_opt(this: &Option<Self>, name: &str, w: &mut XWriter<impl Write>) -> Result<()> {
        opt(this, |this| {
            let elem = ElemBuilder::new(name).start(w)?;
            this.write_to(w)?;
            elem.end(w)
        })
    }
}

impl<T: CanWithSid> XNodeWrite for WithSid<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.data.write_with_sid(&self.sid, w)
    }
}

/// A type that describes color attributes of fixed-function shader elements inside
/// [`ProfileCommon`] effects.
#[derive(Clone, Debug)]
pub enum ColorParam {
    /// The value is a literal color, specified by four floating-point numbers in RGBA order.
    Color(Box<[f32; 4]>),
    /// The value is specified by a reference to a previously defined parameter
    /// in the current scope that can be cast directly to a `float4`.
    Param(Box<str>),
    /// The value is specified by a reference to a previously defined `sampler2D` object.
    Texture(Box<Texture>),
}

impl From<[f32; 4]> for ColorParam {
    fn from(rgba: [f32; 4]) -> Self {
        Self::color(rgba)
    }
}

impl From<[f32; 4]> for WithSid<ColorParam> {
    fn from(rgba: [f32; 4]) -> Self {
        WithSid::new(rgba.into())
    }
}

impl From<Texture> for ColorParam {
    fn from(tex: Texture) -> Self {
        Self::Texture(Box::new(tex))
    }
}

impl From<Texture> for WithSid<ColorParam> {
    fn from(tex: Texture) -> Self {
        WithSid::new(tex.into())
    }
}

impl ColorParam {
    /// Construct a new `ColorParam` from a color.
    pub fn color(rgba: [f32; 4]) -> Self {
        Self::Color(Box::new(rgba))
    }
}

impl CanWithSid for ColorParam {
    fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            "color" => Self::Color(parse_array_n(e)?),
            Param::NAME => Self::Param(e.attr("ref").ok_or("expected ref attr")?.into()),
            Texture::NAME => Self::Texture(Texture::parse_box(e)?),
            _ => return Ok(None),
        }))
    }

    fn write_with_sid<W: Write>(&self, sid: &Option<String>, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Color(arr) => {
                let mut e = ElemBuilder::new("color");
                e.opt_attr("sid", sid);
                let e = e.start(w)?;
                print_arr(&**arr, w)?;
                e.end(w)
            }
            Self::Param(ref_) => {
                let mut e = ElemBuilder::new(Param::NAME);
                e.opt_attr("sid", sid);
                e.attr("ref", ref_);
                e.end(w)
            }
            Self::Texture(e) => e.write_to(w),
        }
    }
}

impl XNodeWrite for ColorParam {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.write_with_sid(&None, w)
    }
}

impl ColorParam {
    /// Convert this parameter to a texture reference, if it is one.
    pub fn as_texture(&self) -> Option<&Texture> {
        match self {
            ColorParam::Texture(tex) => Some(tex),
            _ => None,
        }
    }

    /// Get the color literal of this parameter, if it is a literal.
    pub fn as_color(&self) -> Option<&[f32; 4]> {
        match self {
            ColorParam::Color(c) => Some(c),
            _ => None,
        }
    }
}

/// A type that describes the scalar attributes of fixed-function shader elements inside
/// [`ProfileCommon`] effects.
#[derive(Clone, Debug)]
pub enum FloatParam {
    /// The value is represented by a literal floating-point scalar.
    Float(f32),
    /// The value is represented by a reference to a previously
    /// defined parameter that can be directly cast to a floating-point scalar.
    Param(Box<str>),
}

impl From<f32> for FloatParam {
    fn from(val: f32) -> Self {
        Self::Float(val)
    }
}

impl From<f32> for WithSid<FloatParam> {
    fn from(val: f32) -> Self {
        WithSid::new(val.into())
    }
}

impl CanWithSid for FloatParam {
    fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            "float" => Self::Float(parse_elem(e)?),
            Param::NAME => Self::Param(e.attr("ref").ok_or("expected ref attr")?.into()),
            _ => return Ok(None),
        }))
    }

    fn write_with_sid<W: Write>(&self, sid: &Option<String>, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Float(val) => {
                let mut e = ElemBuilder::new("float");
                e.opt_attr("sid", sid);
                let e = e.start(w)?;
                print_elem(val, w)?;
                e.end(w)
            }
            Self::Param(ref_) => {
                let mut e = ElemBuilder::new(Param::NAME);
                e.opt_attr("sid", sid);
                e.attr("ref", ref_);
                e.end(w)
            }
        }
    }
}

impl XNodeWrite for FloatParam {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.write_with_sid(&None, w)
    }
}

/// A color parameter referencing a texture.
#[derive(Clone, Debug)]
pub struct Texture {
    /// The texture to reference.
    pub texture: String,
    /// A semantic token, which will be referenced within
    /// [`BindMaterial`] to bind an array of texcoords from a
    /// [`Geometry`] instance to the `TextureUnit`.
    pub texcoord: String,
    /// Provides arbitrary additional information about this element.
    pub extra: Option<Box<Extra>>,
}

impl Texture {
    /// Construct a new `Texture` from the mandatory data.
    pub fn new(texture: impl Into<String>, texcoord: impl Into<String>) -> Self {
        Self {
            texture: texture.into(),
            texcoord: texcoord.into(),
            extra: None,
        }
    }

    fn write_with_sid<W: Write>(&self, sid: &Option<String>, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("sid", sid);
        e.attr("texture", &self.texture);
        e.attr("texcoord", &self.texcoord);
        if let Some(extra) = &self.extra {
            let e = e.start(w)?;
            extra.write_to(w)?;
            e.end(w)
        } else {
            e.end(w)
        }
    }
}

impl XNode for Texture {
    const NAME: &'static str = "texture";
    fn parse(e: &Element) -> Result<Self> {
        let mut it = e.children().peekable();
        let res = Texture {
            texture: e.attr("texture").ok_or("expected texture attr")?.into(),
            texcoord: e.attr("texcoord").ok_or("expected texcoord attr")?.into(),
            extra: Extra::parse_opt_box(&mut it)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for Texture {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.write_with_sid(&None, w)
    }
}

fn on_color_as_texture<'a, E>(
    opt: &'a Option<WithSid<ColorParam>>,
    f: &mut impl FnMut(&'a Texture) -> Result<(), E>,
) -> Result<(), E> {
    if let Some(WithSid {
        data: ColorParam::Texture(tex),
        ..
    }) = opt
    {
        f(tex)?
    }
    Ok(())
}
