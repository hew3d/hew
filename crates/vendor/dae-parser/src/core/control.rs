use crate::*;

/// Categorizes the declaration of generic control information.
#[derive(Clone, Debug)]
pub struct Controller {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// The element that contains control data.
    pub element: ControlElement,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Controller {
    /// Create a new [`Controller`] from a [`ControlElement`].
    pub fn new(element: ControlElement) -> Self {
        Self {
            id: None,
            name: None,
            asset: None,
            element,
            extra: vec![],
        }
    }
}

impl XNode for Controller {
    const NAME: &'static str = "controller";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Controller {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            element: parse_one_many(&mut it, ControlElement::parse)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Controller {
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

/// Extra data associated to [`Instance`]<[`Controller`]>.
#[derive(Clone, Debug, Default)]
pub struct InstanceControllerData {
    /// Indicates where a skin controller is to start to search for the
    /// joint nodes it needs. This element is meaningless for morph controllers.
    pub skeleton: Vec<Url>,
    /// Binds a specific material to a piece of geometry.
    pub bind_material: Option<BindMaterial>,
}

impl XNodeWrite for InstanceControllerData {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        many(&self.skeleton, |e| ElemBuilder::print("skeleton", e, w))?;
        self.bind_material.write_to(w)
    }
}

impl Instantiate for Controller {
    const INSTANCE: &'static str = "instance_controller";
    type Data = InstanceControllerData;
    fn parse_data(_: &Element, it: &mut ElementIter<'_>) -> Result<Self::Data> {
        Ok(InstanceControllerData {
            skeleton: parse_list("skeleton", it, parse_elem)?,
            bind_material: BindMaterial::parse_opt(it)?,
        })
    }
    fn is_empty(data: &Self::Data) -> bool {
        data.skeleton.is_empty() && data.bind_material.is_none()
    }
}

/// The element that contains control data.
#[derive(Clone, Debug)]
pub enum ControlElement {
    /// Control data for blend-weight skinning.
    Skin(Skin),
    /// Control data for blending between sets of static meshes.
    Morph(Morph),
}

impl ControlElement {
    /// Parse a [`ControlElement`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        match e.name() {
            Skin::NAME => Ok(Some(Self::Skin(Skin::parse(e)?))),
            Morph::NAME => Ok(Some(Self::Morph(Morph::parse(e)?))),
            _ => Ok(None),
        }
    }

    /// The `source` URL field on this element.
    pub fn source(&self) -> &Url {
        match self {
            ControlElement::Skin(skin) => &skin.source,
            ControlElement::Morph(morph) => &morph.source,
        }
    }

    /// The `sources` field, which gives the list of [`Source`] elements on this element.
    pub fn sources(&self) -> &[Source] {
        match self {
            ControlElement::Skin(skin) => &skin.sources,
            ControlElement::Morph(morph) => &morph.sources,
        }
    }
}

impl XNodeWrite for ControlElement {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Skin(e) => e.write_to(w),
            Self::Morph(e) => e.write_to(w),
        }
    }
}

/// Declares the association between joint nodes and attribute data.
#[derive(Clone, Debug)]
pub struct Joints {
    /// The interpretation of the [`Source`]s.
    pub inputs: Vec<Input>,
    /// The index into `inputs` for the [`Semantic::Joint`] input (which must exist).
    /// The [`Source`] referenced by this input should contain a [`ArrayElement::Name`]
    /// that contains `sid`s to identify the joint nodes.
    /// `sid`s are used instead of [`IdRef`](ArrayElement::IdRef)s to allow a skin controller
    /// to be instantiated multiple times, where each instance can be animated independently.
    pub joint: usize,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Joints {
    /// Construct a new `Joints` from a list of inputs.
    /// One of the inputs must have [`Semantic::Joint`].
    pub fn new(inputs: Vec<Input>) -> Self {
        Self {
            joint: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Joint)
                .expect("joints: missing JOINT input"),
            inputs,
            extra: vec![],
        }
    }
}

impl XNode for Joints {
    const NAME: &'static str = "joints";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let inputs = Input::parse_list_n::<2>(&mut it)?;
        Ok(Joints {
            joint: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Joint)
                .ok_or("joints: missing JOINT input")?,
            inputs,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Joints {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.inputs.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl Joints {
    /// The input with [`Semantic::Joint`].
    pub fn joint_input(&self) -> &Input {
        &self.inputs[self.joint]
    }
}

/// Describes the data required to blend between sets of static meshes.
#[derive(Clone, Debug)]
pub struct Morph {
    /// Refers to the [`Geometry`] that describes the base mesh.
    pub source: UrlRef<Geometry>,
    /// Which blending technique to use.
    pub method: MorphMethod,
    /// Data for morph weights and for morph targets.
    pub sources: Vec<Source>,
    /// Input meshes (morph targets) to be blended.
    pub targets: Targets,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Morph {
    /// Construct a new `Morph` of  from a list of sources and targets.
    /// * The `source` should reference a `Geometry`.
    /// * There should be at least two `sources`.
    /// * One of the `targets` must have [`Semantic::MorphTarget`].
    /// * One of the `targets` must have [`Semantic::MorphWeight`].
    pub fn new(source: Url, sources: Vec<Source>, targets: Vec<Input>) -> Self {
        assert!(sources.len() >= 2);
        Self {
            source: Ref::new(source),
            method: Default::default(),
            sources,
            targets: Targets::new(targets),
            extra: vec![],
        }
    }
}

impl XNode for Morph {
    const NAME: &'static str = "morph";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Morph {
            source: parse_attr(element.attr("source"))?.ok_or("missing source attr")?,
            method: parse_attr(element.attr("method"))?.unwrap_or_default(),
            sources: Source::parse_list_n::<2>(&mut it)?,
            targets: Targets::parse_one(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Morph {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("source", &self.source);
        e.print_attr("method", self.method);
        let e = e.start(w)?;
        self.sources.write_to(w)?;
        self.targets.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Which blending technique to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MorphMethod {
    /// ```text
    /// (Target1, Target2, ...)*(w1, w2, ...) =
    ///     (1-w1-w2-...)*BaseMesh + w1*Target1 + w2*Target2 + ...
    /// ```
    Normalized,
    /// ```text
    /// (Target1, Target2, ...) + (w1, w2, ...) =
    ///     BaseMesh + w1*Target1 + w2*Target2 + ...
    /// ```
    Relative,
}

impl Default for MorphMethod {
    fn default() -> Self {
        Self::Normalized
    }
}

impl FromStr for MorphMethod {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "NORMALIZED" => Ok(Self::Normalized),
            "RELATIVE" => Ok(Self::Relative),
            _ => Err(()),
        }
    }
}

impl MorphMethod {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Normalized => "NORMALIZED",
            Self::Relative => "RELATIVE",
        }
    }
}

impl Display for MorphMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// Contains vertex and primitive information sufficient to describe blend-weight skinning.
#[derive(Clone, Debug)]
pub struct Skin {
    /// A URI reference to the base mesh (a static mesh or a morphed mesh).
    /// This also provides the bind-shape of the skinned mesh.
    pub source: UrlRef<Mesh>,
    /// Provides extra information about the position and
    /// orientation of the base mesh before binding. Contains
    /// sixteen floating-point numbers representing a four-by-
    /// four matrix in column-major order. If `bind_shape_matrix` is not specified
    /// then an identity matrix may be used as the `bind_shape_matrix`.
    pub bind_shape_matrix: Option<Box<[f32; 16]>>,
    /// Provides most of the data required for skinning the given base mesh.
    pub sources: Vec<Source>,
    /// Aggregates the per-joint information needed for this skin.
    pub joints: Joints,
    /// Describes a per-vertex combination of joints and
    /// weights used in this skin. An index of `–1` into the array of
    /// joints refers to the bind shape. Weights should be
    /// normalized before use.
    pub weights: VertexWeights,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Skin {
    /// Construct a `Skin` from the mandatory data.
    /// * The source should reference a `Mesh`.
    /// * There should be at least 3 `sources`.
    /// * One of the `joints` must have [`Semantic::Joint`].
    pub fn new(
        source: Url,
        sources: Vec<Source>,
        joints: Vec<Input>,
        weights: VertexWeights,
    ) -> Self {
        assert!(sources.len() >= 3);
        Self {
            source: Ref::new(source),
            bind_shape_matrix: None,
            sources,
            joints: Joints::new(joints),
            weights,
            extra: vec![],
        }
    }
}

impl XNode for Skin {
    const NAME: &'static str = "skin";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Skin {
            source: parse_attr(element.attr("source"))?.ok_or("missing source attr")?,
            bind_shape_matrix: parse_opt("bind_shape_matrix", &mut it, parse_array_n)?,
            sources: Source::parse_list_n::<3>(&mut it)?,
            joints: Joints::parse_one(&mut it)?,
            weights: VertexWeights::parse_one(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Skin {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("source", &self.source);
        let e = e.start(w)?;
        opt(&self.bind_shape_matrix, |e| {
            ElemBuilder::print_arr("bind_shape_matrix", &**e, w)
        })?;
        self.sources.write_to(w)?;
        self.joints.write_to(w)?;
        self.weights.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Declares morph targets, their weights, and any user-defined attributes associated with them.
#[derive(Clone, Debug)]
pub struct Targets {
    /// The interpretation of the [`Source`]s.
    pub inputs: Vec<Input>,
    /// The index into `inputs` for the [`Semantic::MorphTarget`] input (which must exist).
    pub morph_target: usize,
    /// The index into `inputs` for the [`Semantic::MorphWeight`] input (which must exist).
    pub morph_weight: usize,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Targets {
    /// Construct a new `Targets` from a list of inputs.
    /// One of the inputs must have [`Semantic::MorphTarget`], and
    /// one of the inputs must have [`Semantic::MorphWeight`].
    pub fn new(inputs: Vec<Input>) -> Self {
        Self {
            morph_target: inputs
                .iter()
                .position(|i| i.semantic == Semantic::MorphTarget)
                .expect("targets: missing MORPH_TARGET input"),
            morph_weight: inputs
                .iter()
                .position(|i| i.semantic == Semantic::MorphWeight)
                .expect("targets: missing MORPH_WEIGHT input"),
            inputs,
            extra: vec![],
        }
    }
}

impl XNode for Targets {
    const NAME: &'static str = "targets";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let inputs = Input::parse_list(&mut it)?;
        Ok(Targets {
            morph_target: inputs
                .iter()
                .position(|i| i.semantic == Semantic::MorphTarget)
                .ok_or("targets: missing MORPH_TARGET input")?,
            morph_weight: inputs
                .iter()
                .position(|i| i.semantic == Semantic::MorphWeight)
                .ok_or("targets: missing MORPH_WEIGHT input")?,
            inputs,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Targets {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.inputs.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl Targets {
    /// The input with [`Semantic::MorphTarget`].
    pub fn morph_target_input(&self) -> &Input {
        &self.inputs[self.morph_target]
    }

    /// The input with [`Semantic::MorphWeight`].
    pub fn morph_weight_input(&self) -> &Input {
        &self.inputs[self.morph_weight]
    }
}

/// Describes the combination of joints and weights used by a skin.
#[derive(Clone, Debug)]
pub struct VertexWeights {
    /// The number of vertices in the base mesh.
    pub count: usize,
    /// The [`InputS`] elements describe the joints and the attributes to be associated with them.
    pub inputs: InputList,
    /// The index into `inputs` for the [`Semantic::Joint`] input (which must exist).
    pub joint: usize,
    /// Contains a list of integers, each specifying the number of
    /// bones associated with one of the influences defined by [`VertexWeights`].
    pub vcount: Box<[u32]>,
    /// Contains a list of indices that describe which bones and
    /// attributes are associated with each vertex. An index of `-1`
    /// into the array of joints refers to the bind shape. Weights
    /// should be normalized before use.
    pub prim: Box<[i32]>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for VertexWeights {
    const NAME: &'static str = "vertex_weights";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let inputs = InputList::parse::<2>(&mut it)?;
        let res = VertexWeights {
            count: parse_attr(element.attr("count"))?.ok_or("expected 'count' attr")?,
            joint: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Joint)
                .ok_or("vertex_weights: missing JOINT input")?,
            inputs,
            vcount: parse_opt("vcount", &mut it, parse_array)?.unwrap_or_default(),
            prim: parse_opt("v", &mut it, parse_array)?.unwrap_or_default(),
            extra: Extra::parse_many(it)?,
        };
        validate_vcount(res.count, res.inputs.stride, &res.vcount, &res.prim)?;
        Ok(res)
    }
}

impl XNodeWrite for VertexWeights {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.print_attr("count", self.count);
        let e = e.start(w)?;
        self.inputs.write_to(w)?;
        ElemBuilder::print_arr("vcount", &self.vcount, w)?;
        ElemBuilder::print_arr("v", &self.prim, w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl VertexWeights {
    /// Construct a new `VertexWeights` from a list of inputs.
    /// One of the `inputs` must have [`Semantic::Joint`].
    pub fn new(inputs: Vec<InputS>, vcount: Box<[u32]>, prim: Box<[i32]>) -> Self {
        let inputs = InputList::new(inputs);
        assert!(
            inputs.stride * vcount.iter().sum::<u32>() as usize == prim.len(),
            "vcount does not match prim"
        );
        Self {
            count: vcount.len(),
            joint: inputs
                .iter()
                .position(|i| i.semantic == Semantic::Joint)
                .expect("vertex_weights: missing JOINT input"),
            inputs,
            vcount,
            prim,
            extra: vec![],
        }
    }

    /// The input with [`Semantic::Joint`].
    pub fn joint_input(&self) -> &Input {
        &self.inputs[self.joint]
    }
}
