use crate::*;

/// Declares a view of the visual scene hierarchy or scene graph.
/// The camera contains elements that describe the camera’s optics and imager.
#[derive(Clone, Debug)]
pub struct Camera {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// Describes the field of view and viewing frustum using canonical parameters.
    pub optics: Optics,
    /// Represents the image sensor of a camera (for example, film or CCD).
    pub imager: Option<Imager>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Camera {
    /// Construct a new `Camera` with the given projection type.
    pub fn new(ty: ProjectionType) -> Self {
        Self {
            id: Default::default(),
            name: Default::default(),
            asset: Default::default(),
            optics: Optics::new(ty),
            imager: Default::default(),
            extra: Default::default(),
        }
    }
}

impl XNode for Camera {
    const NAME: &'static str = "camera";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Camera {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            optics: Optics::parse_one(&mut it)?,
            imager: Imager::parse_opt(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Camera {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.optics.write_to(w)?;
        self.imager.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Represents the image sensor of a camera (for example, film or CCD).
#[derive(Clone, Debug)]
pub struct Imager {
    /// Declares the information used to process some portion of the content.
    /// This field is always nonempty, because the spec provides no common data
    /// for `imager` elements.
    pub technique: Vec<Technique>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Imager {
    /// Construct a new `Imager` with the given technique(s).
    pub fn new(technique: Vec<Technique>) -> Self {
        assert!(!technique.is_empty());
        Self {
            technique,
            extra: vec![],
        }
    }
}

impl XNode for Imager {
    const NAME: &'static str = "imager";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Imager {
            technique: Technique::parse_list_n::<1>(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Imager {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Represents the apparatus on a camera that projects the image onto the image sensor.
#[derive(Clone, Debug)]
pub struct Optics {
    /// The projection type.
    pub ty: ProjectionType,
    /// Declares the information used to process some portion of the content. (optional)
    pub technique: Vec<Technique>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Optics {
    /// Construct a new `Optics` from the given projection type.
    pub fn new(ty: ProjectionType) -> Self {
        Self {
            ty,
            technique: Default::default(),
            extra: Default::default(),
        }
    }
}

impl XNode for Optics {
    const NAME: &'static str = "optics";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Optics {
            ty: parse_one(Technique::COMMON, &mut it, |e| {
                let mut it = e.children().peekable();
                finish(parse_one_many(&mut it, ProjectionType::parse)?, it)
            })?,
            technique: Technique::parse_list(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Optics {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        let common = ElemBuilder::new(Technique::COMMON).start(w)?;
        self.ty.write_to(w)?;
        common.end(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// The projection type of the camera.
#[derive(Clone, Debug)]
pub enum ProjectionType {
    /// An orthographic camera
    Orthographic(Orthographic),
    /// A perspective camera
    Perspective(Perspective),
}

impl ProjectionType {
    /// Parse a [`ProjectionType`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        match e.name() {
            Orthographic::NAME => Ok(Some(Self::Orthographic(Orthographic::parse(e)?))),
            Perspective::NAME => Ok(Some(Self::Perspective(Perspective::parse(e)?))),
            _ => Ok(None),
        }
    }
}

impl XNodeWrite for ProjectionType {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Orthographic(e) => e.write_to(w),
            Self::Perspective(e) => e.write_to(w),
        }
    }
}

/// Describes the field of view of an orthographic camera.
#[derive(Clone, Debug)]
pub struct Orthographic {
    /// The horizontal (X) magnification of the view.
    pub xmag: Option<f32>,
    /// The vertical (Y) magnification of the view.
    pub ymag: Option<f32>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
    /// The aspect ratio of the field of view.
    pub aspect_ratio: Option<f32>,
    /// The distance to the near clipping plane.
    pub znear: f32,
    /// The distance to the far clipping plane.
    pub zfar: f32,
}

impl Orthographic {
    /// Construct a new `Orthographic` object.
    pub fn new(xmag: Option<f32>, ymag: Option<f32>, znear: f32, zfar: f32) -> Self {
        Self {
            xmag,
            ymag,
            extra: Default::default(),
            aspect_ratio: Default::default(),
            znear,
            zfar,
        }
    }
}

impl XNode for Orthographic {
    const NAME: &'static str = "orthographic";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Orthographic {
            xmag: parse_opt("xmag", &mut it, parse_elem)?,
            ymag: parse_opt("ymag", &mut it, parse_elem)?,
            extra: Extra::parse_list(&mut it)?,
            aspect_ratio: parse_opt("aspect_ratio", &mut it, parse_elem)?,
            znear: parse_one("znear", &mut it, parse_elem)?,
            zfar: parse_one("zfar", &mut it, parse_elem)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for Orthographic {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::opt_print("xmag", &self.xmag, w)?;
        ElemBuilder::opt_print("ymag", &self.ymag, w)?;
        self.extra.write_to(w)?;
        ElemBuilder::opt_print("aspect_ratio", &self.aspect_ratio, w)?;
        ElemBuilder::print("znear", &self.znear, w)?;
        ElemBuilder::print("zfar", &self.zfar, w)?;
        e.end(w)
    }
}

/// Describes the field of view of a perspective camera.
#[derive(Clone, Copy, Debug)]
pub struct Perspective {
    /// The horizontal field of view in degrees.
    pub xfov: Option<f32>,
    /// The vertical field of view in degrees.
    pub yfov: Option<f32>,
    /// The aspect ratio of the field of view.
    pub aspect_ratio: Option<f32>,
    /// The distance to the near clipping plane.
    pub znear: f32,
    /// The distance to the far clipping plane.
    pub zfar: f32,
}

impl Perspective {
    /// Construct a new `Perspective` object.
    pub fn new(xfov: Option<f32>, yfov: Option<f32>, znear: f32, zfar: f32) -> Self {
        Self {
            xfov,
            yfov,
            aspect_ratio: Default::default(),
            znear,
            zfar,
        }
    }
}

impl XNode for Perspective {
    const NAME: &'static str = "perspective";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Perspective {
            xfov: parse_opt("xfov", &mut it, parse_elem)?,
            yfov: parse_opt("yfov", &mut it, parse_elem)?,
            aspect_ratio: parse_opt("aspect_ratio", &mut it, parse_elem)?,
            znear: parse_one("znear", &mut it, parse_elem)?,
            zfar: parse_one("zfar", &mut it, parse_elem)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for Perspective {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        ElemBuilder::opt_print("xfov", &self.xfov, w)?;
        ElemBuilder::opt_print("yfov", &self.yfov, w)?;
        ElemBuilder::opt_print("aspect_ratio", &self.aspect_ratio, w)?;
        ElemBuilder::print("znear", &self.znear, w)?;
        ElemBuilder::print("zfar", &self.zfar, w)?;
        e.end(w)
    }
}
