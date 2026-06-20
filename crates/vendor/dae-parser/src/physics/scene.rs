use crate::*;

/// Specifies an environment in which physical objects are instantiated and simulated.
#[derive(Clone, Default, Debug)]
pub struct PhysicsScene {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// Instantiates a [`ForceField`] element to influence this physics scene.
    pub instance_force_field: Vec<Instance<ForceField>>,
    /// Instantiates a [`PhysicsModel`] element and
    /// allows for overriding some or all of its children.
    pub instance_physics_model: Vec<Instance<PhysicsModel>>,
    /// Specifies physics-scene information for the common profile
    /// that all COLLADA implementations must support.
    pub common: PhysicsSceneCommon,
    /// Declares the information used to process some portion of the content. (optional)
    pub technique: Vec<Technique>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for PhysicsScene {
    const NAME: &'static str = "physics_scene";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(PhysicsScene {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            instance_force_field: Instance::parse_list(&mut it)?,
            instance_physics_model: Instance::parse_list(&mut it)?,
            common: parse_one(Technique::COMMON, &mut it, PhysicsSceneCommon::parse)?,
            technique: Technique::parse_list(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for PhysicsScene {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.instance_force_field.write_to(w)?;
        self.instance_physics_model.write_to(w)?;
        let common = ElemBuilder::new(Technique::COMMON).start(w)?;
        self.common.write_to(w)?;
        common.end(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Specifies physics-scene information for the common profile
/// that all COLLADA implementations must support.
#[derive(Clone, Default, Debug)]
pub struct PhysicsSceneCommon {
    /// A vector representation of the scene’s gravity force field.
    /// It is given as a denormalized direction vector of three
    /// floating-point values that indicate both the magnitude
    /// and direction of acceleration caused by the field.
    pub gravity: Option<Box<[f32; 3]>>,
    /// The integration time step, measured in seconds, of the physics scene.
    /// This value is engine-specific. If omitted, the physics engine’s default is used.
    pub time_step: Option<f32>,
}

impl PhysicsSceneCommon {
    /// Parse a [`PhysicsSceneCommon`] from a
    /// `<physics_scene>/<technique_common>` XML element.
    pub fn parse(e: &Element) -> Result<Self> {
        let mut it = e.children().peekable();
        let res = Self {
            gravity: parse_opt("gravity", &mut it, parse_array_n)?,
            time_step: parse_opt("time_step", &mut it, parse_elem)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for PhysicsSceneCommon {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        opt(&self.gravity, |e| {
            ElemBuilder::print_arr("gravity", &**e, w)
        })?;
        ElemBuilder::opt_print("time_step", &self.time_step, w)
    }
}

/// Provides a general container for force fields.
///
/// At the moment, it contains only techniques and extra elements.
#[derive(Clone, Default, Debug)]
pub struct ForceField {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// Declares the information used to process some portion of the content.
    /// This field is always nonempty, because the spec provides no common data
    /// for `force_field` elements.
    pub technique: Vec<Technique>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for ForceField {
    const NAME: &'static str = "force_field";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(ForceField {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            technique: Technique::parse_list_n::<1>(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for ForceField {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Defines an attachment frame, to a rigid body or a node, within a rigid constraint.
#[derive(Clone, Debug)]
pub struct Attachment {
    /// A URI reference to a [`RigidBody`] or [`Node`]. This must refer to a
    /// [`RigidBody`] in `attachment` or in `ref_attachment`; they cannot both be [`Node`]s.
    pub rigid_body: Url,
    /// Changes the position of the attachment point.
    pub transform: Vec<RigidTransform>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Attachment {
    /// Construct an attachment to a [`RigidBody`] or [`Node`].
    pub fn new(rigid_body: Url) -> Self {
        Self {
            rigid_body,
            transform: vec![],
            extra: vec![],
        }
    }

    /// The name of the `<ref_attachment>` element.
    pub const REF: &'static str = "ref_attachment";

    pub(crate) fn write_inner<W: Write>(
        &self,
        mut e: ElemBuilder,
        w: &mut XWriter<W>,
    ) -> Result<()> {
        e.print_attr("rigid_body", &self.rigid_body);
        let e = e.start(w)?;
        self.transform.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl XNode for Attachment {
    const NAME: &'static str = "attachment";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert!(element.name() == Self::NAME || element.name() == Self::REF);
        let mut it = element.children().peekable();
        Ok(Attachment {
            rigid_body: parse_attr(element.attr("rigid_body"))?.ok_or("missing rigid_body attr")?,
            transform: parse_list_many(&mut it, RigidTransform::parse)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Attachment {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.write_inner(Self::elem(), w)
    }
}
