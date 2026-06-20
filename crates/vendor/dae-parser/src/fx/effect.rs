use crate::*;

/// Provides a self-contained description of a COLLADA effect.
#[derive(Clone, Debug)]
pub struct Effect {
    /// Global identifier for this object.
    pub id: String,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// A list of strongly typed annotation remarks.
    pub annotate: Vec<Annotate>,
    /// Declares a standard COLLADA image resource.
    pub image: Vec<Image>,
    /// Creates a new parameter from a constrained set of
    /// types recognizable by all platforms, see [`ParamType`].
    pub new_param: Vec<NewParam>,
    /// At least one profile must appear.
    pub profile: Vec<Profile>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for Effect {
    const NAME: &'static str = "effect";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = Effect {
            id: element.attr("id").ok_or("expected id attr")?.into(),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            annotate: Annotate::parse_list(&mut it)?,
            image: Image::parse_list(&mut it)?,
            new_param: NewParam::parse_list(&mut it)?,
            profile: parse_list_many(&mut it, Profile::parse)?,
            extra: Extra::parse_many(it)?,
        };
        if res.profile.is_empty() {
            return Err("expected at least one profile".into());
        }
        Ok(res)
    }
}

impl XNodeWrite for Effect {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.annotate.write_to(w)?;
        self.image.write_to(w)?;
        self.new_param.write_to(w)?;
        self.profile.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

impl Effect {
    /// Construct a new `Effect` from common profile data.
    pub fn new(id: impl Into<String>, technique: TechniqueFx<CommonData>) -> Self {
        Self {
            id: id.into(),
            name: None,
            asset: None,
            annotate: vec![],
            image: vec![],
            new_param: vec![],
            profile: vec![ProfileCommon::new(technique).into()],
            extra: vec![],
        }
    }

    /// Construct a simple `Effect` with one shader.
    pub fn shader(id: impl Into<String>, shader: impl Into<Shader>) -> Self {
        Self::new(id, TechniqueFx::new("common", CommonData::shader(shader)))
    }

    /// Get the first [`ProfileCommon`] in this effect.
    pub fn get_common_profile(&self) -> Option<&ProfileCommon> {
        self.profile.iter().find_map(Profile::as_common)
    }

    /// Get a parameter of the effect by name.
    pub fn get_param(&self, sid: &str) -> Option<&NewParam> {
        self.new_param.iter().rev().find(|p| p.sid == sid)
    }
}

/// Extra data associated to [`Instance`]<[`Effect`]>.
#[derive(Clone, Debug, Default)]
pub struct InstanceEffectData {
    /// [`TechniqueHint`]s indicate the desired or last-used technique
    /// inside an effect profile.
    pub technique_hint: Vec<TechniqueHint>,
    /// [`EffectSetParam`]s assign values to specific effect and profile
    /// parameters that will be unique to the instance.
    pub set_param: Vec<EffectSetParam>,
}

impl XNodeWrite for InstanceEffectData {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        self.technique_hint.write_to(w)?;
        self.set_param.write_to(w)
    }
}

impl Instantiate for Effect {
    const INSTANCE: &'static str = "instance_effect";
    type Data = InstanceEffectData;
    fn parse_data(_: &Element, it: &mut ElementIter<'_>) -> Result<Self::Data> {
        Ok(InstanceEffectData {
            technique_hint: TechniqueHint::parse_list(it)?,
            set_param: EffectSetParam::parse_list(it)?,
        })
    }
    fn is_empty(data: &Self::Data) -> bool {
        data.technique_hint.is_empty() && data.set_param.is_empty()
    }
}

/// Binds geometry vertex inputs to effect vertex inputs upon instantiation.
#[derive(Clone, Debug)]
pub struct BindVertexInput {
    /// Which effect parameter to bind.
    pub semantic: String,
    /// Which input semantic to bind.
    pub input_semantic: String,
    /// Which input set to bind.
    pub input_set: Option<u32>,
}

impl BindVertexInput {
    /// Construct a `BindVertexInput` from the input data.
    pub fn new(
        semantic: impl Into<String>,
        input_semantic: impl Into<String>,
        input_set: Option<u32>,
    ) -> Self {
        Self {
            semantic: semantic.into(),
            input_semantic: input_semantic.into(),
            input_set,
        }
    }
}

impl XNode for BindVertexInput {
    const NAME: &'static str = "bind_vertex_input";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let semantic = element.attr("semantic");
        let input_semantic = element.attr("input_semantic");
        Ok(BindVertexInput {
            semantic: semantic.ok_or("missing semantic attribute")?.into(),
            input_semantic: input_semantic.ok_or("missing input semantic")?.into(),
            input_set: parse_attr(element.attr("input_set"))?,
        })
    }
}

impl XNodeWrite for BindVertexInput {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.attr("semantic", &self.semantic);
        e.attr("input_semantic", &self.input_semantic);
        e.opt_print_attr("input_set", &self.input_set);
        e.end(w)
    }
}

/// A trait for the types that are legal to go in a [`TechniqueFx<T>`].
pub trait ProfileData: XNodeWrite + Sized {
    /// Parse the embedded data from a subsequence of children in the `<technique>` node.
    fn parse(it: &mut ElementIter<'_>) -> Result<Self>;
}

/// Holds a description of the textures, samplers, shaders, parameters,
/// and passes necessary for rendering this effect using one method.
///
/// It is parameterized on additional data determined by the parent of this element.
#[derive(Clone, Debug)]
pub struct TechniqueFx<T> {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// A text string value containing the subidentifier of this element.
    /// This value must be unique within the scope of the parent element.
    pub sid: String,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// The profile-specific child data.
    pub data: T,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl<T> TechniqueFx<T> {
    /// Construct a new `TechniqueFx` given the profile-specific data.
    pub fn new(sid: impl Into<String>, data: T) -> Self {
        Self {
            id: None,
            sid: sid.into(),
            asset: None,
            data,
            extra: vec![],
        }
    }

    /// Construct a new `TechniqueFx` with default data.
    pub fn default(sid: impl Into<String>) -> Self
    where
        T: Default,
    {
        Self::new(sid, T::default())
    }
}

impl<T: ProfileData> XNode for TechniqueFx<T> {
    const NAME: &'static str = "technique";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(TechniqueFx {
            id: element.attr("id").map(Into::into),
            sid: element.attr("sid").ok_or("expecting sid attr")?.into(),
            asset: Asset::parse_opt_box(&mut it)?,
            data: T::parse(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl<T: ProfileData> XNodeWrite for TechniqueFx<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.attr("sid", &self.sid);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.data.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Adds a hint for a platform of which technique to use in this effect.
#[derive(Clone, Debug)]
pub struct TechniqueHint {
    /// Defines a string that specifies for which platform this hint is intended.
    pub platform: Option<String>,
    /// A reference to the name of the platform.
    pub ref_: String,
    /// A string that specifies for which API profile this hint is intended.
    /// It is the name of the profile within the effect that contains the technique.
    /// Profiles are constructed by appending this attribute’s value to "Profile".
    /// For example, to select [`ProfileCG`], specify `profile="CG"`.
    pub profile: Option<String>,
}

impl TechniqueHint {
    /// Construct a new `TechniqueHint`.
    pub fn new(
        platform: impl Into<String>,
        ref_: impl Into<String>,
        profile: impl Into<String>,
    ) -> Self {
        Self {
            platform: Some(platform.into()),
            ref_: ref_.into(),
            profile: Some(profile.into()),
        }
    }
}

impl XNode for TechniqueHint {
    const NAME: &'static str = "technique_hint";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(TechniqueHint {
            platform: element.attr("platform").map(Into::into),
            ref_: element.attr("ref").ok_or("expected 'ref' attr")?.into(),
            profile: element.attr("profile").map(Into::into),
        })
    }
}

impl XNodeWrite for TechniqueHint {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("platform", &self.platform);
        e.attr("ref", &self.ref_);
        e.opt_attr("profile", &self.profile);
        e.end(w)
    }
}
