use crate::*;

/// Declares a light source that illuminates a scene.
#[derive(Clone, Debug)]
pub struct Light {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// The kind of light being described.
    pub kind: LightKind,
    /// Declares the information used to process some portion of the content. (optional)
    pub technique: Vec<Technique>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Light {
    /// Construct a new `Light` with the given name and kind.
    pub fn new(id: impl Into<String>, name: Option<String>, kind: impl Into<LightKind>) -> Self {
        Self {
            id: Some(id.into()),
            name,
            asset: None,
            kind: kind.into(),
            technique: vec![],
            extra: vec![],
        }
    }
}

impl XNode for Light {
    const NAME: &'static str = "light";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Light {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            kind: parse_one(Technique::COMMON, &mut it, |e| {
                let mut it = e.children().peekable();
                finish(parse_one_many(&mut it, LightKind::parse)?, it)
            })?,
            technique: Technique::parse_list(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Light {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        let common = ElemBuilder::new(Technique::COMMON).start(w)?;
        self.kind.write_to(w)?;
        common.end(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// The kind of light being described.
#[derive(Clone, Debug)]
pub enum LightKind {
    /// Describes an ambient light source.
    Ambient(AmbientLight),
    /// Describes a directional light source.
    Directional(DirectionalLight),
    /// Describes a point light source.
    Point(Box<PointLight>),
    /// Describes a spot light source.
    Spot(Box<SpotLight>),
}

impl From<SpotLight> for LightKind {
    fn from(v: SpotLight) -> Self {
        Self::Spot(Box::new(v))
    }
}

impl From<PointLight> for LightKind {
    fn from(v: PointLight) -> Self {
        Self::Point(Box::new(v))
    }
}

impl From<DirectionalLight> for LightKind {
    fn from(v: DirectionalLight) -> Self {
        Self::Directional(v)
    }
}

impl From<AmbientLight> for LightKind {
    fn from(v: AmbientLight) -> Self {
        Self::Ambient(v)
    }
}

impl LightKind {
    /// Parse a [`LightKind`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(Some(match e.name() {
            AmbientLight::NAME => Self::Ambient(AmbientLight::parse(e)?),
            DirectionalLight::NAME => Self::Directional(DirectionalLight::parse(e)?),
            PointLight::NAME => Self::Point(PointLight::parse_box(e)?),
            SpotLight::NAME => Self::Spot(SpotLight::parse_box(e)?),
            _ => return Ok(None),
        }))
    }
}

impl XNodeWrite for LightKind {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            LightKind::Ambient(e) => e.write_to(w),
            LightKind::Directional(e) => e.write_to(w),
            LightKind::Point(e) => e.write_to(w),
            LightKind::Spot(e) => e.write_to(w),
        }
    }
}

/// Describes an ambient light source.
#[derive(Clone, Debug)]
pub struct AmbientLight {
    /// Contains three floating-point numbers specifying the color of the light.
    pub color: Box<[f32; 3]>,
}

impl AmbientLight {
    /// Create a new `AmbientLight` with the given color.
    pub fn new(color: [f32; 3]) -> Self {
        Self {
            color: Box::new(color),
        }
    }
}

impl XNode for AmbientLight {
    const NAME: &'static str = "ambient";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let color = parse_one("color", &mut it, parse_array_n)?;
        finish(AmbientLight { color }, it)
    }
}

impl XNodeWrite for AmbientLight {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::print_arr("color", &*self.color, w)?;
        e.end(w)
    }
}

/// Describes a directional light source.
///
/// The light’s default direction vector in local coordinates is [0,0,-1],
/// pointing down the negative z axis.
/// The actual direction of the light is defined by the transform of the node
/// where the light is instantiated.
#[derive(Clone, Debug)]
pub struct DirectionalLight {
    /// Contains three floating-point numbers specifying the color of the light.
    pub color: Box<[f32; 3]>,
}

impl DirectionalLight {
    /// Create a new `DirectionalLight` with the given color.
    pub fn new(color: [f32; 3]) -> Self {
        Self {
            color: Box::new(color),
        }
    }
}

impl XNode for DirectionalLight {
    const NAME: &'static str = "directional";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let color = parse_one("color", &mut it, parse_array_n)?;
        finish(DirectionalLight { color }, it)
    }
}

impl XNodeWrite for DirectionalLight {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::print_arr("color", &*self.color, w)?;
        e.end(w)
    }
}

/// Describes a point light source.
///
/// The position of the light is defined by the transform of the node in which it is instantiated.
///
/// The `constant_attenuation`, `linear_attenuation`, and `quadratic_attenuation` are
/// used to calculate the total attenuation of this light given a distance.
/// The equation used is:
/// ```text
/// A = constant_attenuation + Dist * linear_attenuation + Dist^2 * quadratic_attenuation
/// ```
#[derive(Clone, Debug)]
pub struct PointLight {
    /// Contains three floating-point numbers specifying the color of the light.
    pub color: Box<[f32; 3]>,
    /// The constant term in the attentuation equation, see [`PointLight`].
    pub constant_attenuation: f32,
    /// The linear term in the attentuation equation, see [`PointLight`].
    pub linear_attenuation: f32,
    /// The quadratic term in the attentuation equation, see [`PointLight`].
    pub quadratic_attenuation: f32,
}

impl PointLight {
    /// Create a new `PointLight` with the given color.
    pub fn new(color: [f32; 3]) -> Self {
        Self {
            color: Box::new(color),
            constant_attenuation: 0.,
            linear_attenuation: 0.,
            quadratic_attenuation: 0.,
        }
    }
}

impl XNode for PointLight {
    const NAME: &'static str = "point";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = PointLight {
            color: parse_one("color", &mut it, parse_array_n)?,
            constant_attenuation: parse_opt("constant_attenuation", &mut it, parse_elem)?
                .unwrap_or(0.),
            linear_attenuation: parse_opt("linear_attenuation", &mut it, parse_elem)?.unwrap_or(0.),
            quadratic_attenuation: parse_opt("quadratic_attenuation", &mut it, parse_elem)?
                .unwrap_or(0.),
        };
        finish(res, it)
    }
}

impl XNodeWrite for PointLight {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::print_arr("color", &*self.color, w)?;
        ElemBuilder::def_print("constant_attenuation", self.constant_attenuation, 0., w)?;
        ElemBuilder::def_print("linear_attenuation", self.linear_attenuation, 0., w)?;
        ElemBuilder::def_print("quadratic_attenuation", self.quadratic_attenuation, 0., w)?;
        e.end(w)
    }
}

/// Describes a spot light source.
///
/// The light’s default direction vector in local coordinates is [0,0,-1],
/// pointing down the negative z axis.
/// The actual direction of the light is defined by the transform of the node
/// where the light is instantiated.
///
/// The `constant_attenuation`, `linear_attenuation`, and `quadratic_attenuation` are
/// used to calculate the total attenuation of this light given a distance.
/// The equation used is:
/// ```text
/// A = constant_attenuation + Dist * linear_attenuation + Dist^2 * quadratic_attenuation
/// ```
///
/// The `falloff_angle` and `falloff_exponent` are used to specify
/// the amount of attenuation based on the direction of the light.
#[derive(Clone, Debug)]
pub struct SpotLight {
    /// Contains three floating-point numbers specifying the color of the light.
    pub color: Box<[f32; 3]>,
    /// The constant term in the attentuation equation, see [`SpotLight`].
    pub constant_attenuation: f32,
    /// The linear term in the attentuation equation, see [`SpotLight`].
    pub linear_attenuation: f32,
    /// The quadratic term in the attentuation equation, see [`SpotLight`].
    pub quadratic_attenuation: f32,
    /// The directional attenuation of the light.
    pub falloff_angle: f32,
    /// A term in the directional attenuation equation of the light.
    pub falloff_exponent: f32,
}

impl SpotLight {
    /// Create a new `SpotLight` with the given color.
    pub fn new(color: [f32; 3]) -> Self {
        Self {
            color: Box::new(color),
            constant_attenuation: 0.,
            linear_attenuation: 0.,
            quadratic_attenuation: 0.,
            falloff_angle: 180.,
            falloff_exponent: 0.,
        }
    }
}

impl XNode for SpotLight {
    const NAME: &'static str = "spot";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = SpotLight {
            color: parse_one("color", &mut it, parse_array_n)?,
            constant_attenuation: parse_opt("constant_attenuation", &mut it, parse_elem)?
                .unwrap_or(0.),
            linear_attenuation: parse_opt("linear_attenuation", &mut it, parse_elem)?.unwrap_or(0.),
            quadratic_attenuation: parse_opt("quadratic_attenuation", &mut it, parse_elem)?
                .unwrap_or(0.),
            falloff_angle: parse_opt("falloff_angle", &mut it, parse_elem)?.unwrap_or(180.),
            falloff_exponent: parse_opt("falloff_exponent", &mut it, parse_elem)?.unwrap_or(0.),
        };
        finish(res, it)
    }
}

impl XNodeWrite for SpotLight {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::print_arr("color", &*self.color, w)?;
        ElemBuilder::def_print("constant_attenuation", self.constant_attenuation, 0., w)?;
        ElemBuilder::def_print("linear_attenuation", self.linear_attenuation, 0., w)?;
        ElemBuilder::def_print("quadratic_attenuation", self.quadratic_attenuation, 0., w)?;
        ElemBuilder::def_print("falloff_angle", self.falloff_angle, 180., w)?;
        ElemBuilder::def_print("falloff_exponent", self.falloff_exponent, 0., w)?;
        e.end(w)
    }
}
