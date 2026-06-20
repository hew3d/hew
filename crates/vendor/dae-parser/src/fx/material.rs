use crate::*;

/// Describes the visual appearance of a geometric object.
#[derive(Clone, Debug)]
pub struct Material {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// Instantiates a COLLADA material resource. See [`InstanceEffectData`]
    /// for the additional instance effect data.
    pub instance_effect: Instance<Effect>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Material {
    /// Construct a new `Material` from an instance effect.
    pub fn new(id: impl Into<String>, name: impl Into<String>, instance_effect: Url) -> Self {
        Self {
            id: Some(id.into()),
            name: Some(name.into()),
            asset: None,
            instance_effect: Instance::new(instance_effect),
            extra: vec![],
        }
    }
}

impl XNode for Material {
    const NAME: &'static str = "material";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Material {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            instance_effect: Instance::parse_one(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Material {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.instance_effect.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Instantiates a COLLADA material resource.
#[derive(Clone, Debug)]
pub struct InstanceMaterial {
    /// A text string value containing the subidentifier of this element.
    /// This value must be unique within the scope of the parent element.
    pub sid: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Which symbol defined from within the geometry this material binds to.
    pub symbol: String,
    /// The URI of the location of the [`Material`] element to instantiate.
    /// Can refer to a local instance or external reference.
    /// For a local instance, this is a relative URI fragment identifier
    /// that begins with the `"#"` character.
    /// The fragment identifier is an XPointer shorthand pointer that
    /// consists of the ID of the element to instantiate.
    /// For an external reference, this is an absolute or relative URL.
    pub target: UrlRef<Material>,
    /// Connects a parameter in the material’s effect by semantic
    /// to a target in the scene.
    pub bind: Vec<BindM>,
    /// Binds vertex inputs to effect parameters upon instantiation.
    pub bind_vertex_input: Vec<BindVertexInput>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl InstanceMaterial {
    /// Construct a new `InstanceMaterial` with the given bindings.
    pub fn new(
        symbol: impl Into<String>,
        target: Url,
        bind_vertex_input: Vec<BindVertexInput>,
    ) -> Self {
        Self {
            sid: None,
            name: None,
            symbol: symbol.into(),
            target: Ref::new(target),
            bind: vec![],
            bind_vertex_input,
            extra: vec![],
        }
    }
}

impl XNode for InstanceMaterial {
    const NAME: &'static str = "instance_material";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let symbol = element.attr("symbol").ok_or("expecting symbol attr")?;
        let mut it = element.children().peekable();
        Ok(InstanceMaterial {
            sid: element.attr("sid").map(Into::into),
            name: element.attr("name").map(Into::into),
            symbol: symbol.into(),
            target: parse_attr(element.attr("target"))?.ok_or("missing target attribute")?,
            bind: BindM::parse_list(&mut it)?,
            bind_vertex_input: BindVertexInput::parse_list(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for InstanceMaterial {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("sid", &self.sid);
        e.opt_attr("name", &self.name);
        e.attr("symbol", &self.symbol);
        e.print_attr("target", &self.target);
        let e = e.start(w)?;
        self.bind.write_to(w)?;
        self.bind_vertex_input.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Binds a specific material to a piece of geometry,
/// binding varying and uniform parameters at the same time.
#[derive(Clone, Debug)]
pub struct BindMaterial {
    /// In [`BindMaterial`] these are added to be targets for animation.
    /// These objects can then be bound to input parameters in the normal manner
    /// without requiring the animation targeting system to parse the internal
    /// layout of an [`Effect`].
    pub param: Vec<Param>,
    /// The common profile data is list of [`InstanceMaterial`]s.
    pub instance_material: Vec<InstanceMaterial>,
    /// Declares the information used to process some portion of the content. (optional)
    pub technique: Vec<Technique>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl BindMaterial {
    /// Construct a `BindMaterial` with the given instances.
    pub fn new(instance_material: Vec<InstanceMaterial>) -> Self {
        assert!(!instance_material.is_empty());
        Self {
            param: vec![],
            instance_material,
            technique: vec![],
            extra: vec![],
        }
    }
}

impl XNode for BindMaterial {
    const NAME: &'static str = "bind_material";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(BindMaterial {
            param: Param::parse_list(&mut it)?,
            instance_material: parse_one(Technique::COMMON, &mut it, |e| {
                let mut it = e.children().peekable();
                finish(InstanceMaterial::parse_list_n::<1>(&mut it)?, it)
            })?,
            technique: Technique::parse_list(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for BindMaterial {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.param.write_to(w)?;
        let common = ElemBuilder::new(Technique::COMMON).start(w)?;
        self.instance_material.write_to(w)?;
        common.end(w)?;
        self.technique.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Binds values to uniform inputs of a shader or binds values to effect
/// parameters upon instantiation.
/// In the COLLADA spec, this element is called "`<bind>` (material)".
#[derive(Clone, Debug)]
pub struct BindM {
    /// Which effect parameter to bind.
    pub semantic: Option<String>,
    /// The location of the value to bind to the specified semantic.
    pub target: Address,
}

impl BindM {
    /// Construct a `BindM` with the given semantic and target.
    pub fn new(semantic: impl Into<String>, target: impl Into<String>) -> Self {
        Self {
            semantic: Some(semantic.into()),
            target: Address(target.into()),
        }
    }
}

impl XNode for BindM {
    const NAME: &'static str = "bind";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let target = element.attr("target").ok_or("missing target attribute")?;
        Ok(BindM {
            semantic: element.attr("semantic").map(Into::into),
            target: Address(target.into()),
        })
    }
}

impl XNodeWrite for BindM {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("semantic", &self.semantic);
        e.print_attr("target", &self.target);
        e.end(w)
    }
}
