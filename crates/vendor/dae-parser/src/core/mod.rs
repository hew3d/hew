mod anim;
mod camera;
mod control;
mod data;
mod ext;
mod geom;
mod light;
mod meta;
mod scene;
mod transform;

use crate::*;
pub use {
    anim::*, camera::*, control::*, data::*, ext::*, geom::*, light::*, meta::*, scene::*,
    transform::*,
};

/// An unparsed COLLADA target address.
/// See the "Address Syntax" section in Chapter 3: Schema concepts of the
/// [COLLADA spec](https://www.khronos.org/files/collada_spec_1_4.pdf).
#[derive(Clone, Debug)]
pub struct Address(pub String);

impl Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

/// A trait for nodes that can be placed in a library element.
pub trait ParseLibrary: XNode {
    /// The name of the library element. For example, the [`Geometry`] element has
    /// `LIBRARY = "library_geometries"`,
    /// and the corresponding library type is [`Library`]`<Geometry>`.
    const LIBRARY: &'static str;

    /// Extract the library from a single [`LibraryElement`].
    fn extract_element(e: &LibraryElement) -> Option<&Library<Self>>;

    /// Make a [`LibraryElement`] from a [`Library`].
    fn mk_element(lib: Library<Self>) -> LibraryElement;
}

/// Declares a module of elements of type `T`.
#[derive(Clone, Debug)]
pub struct Library<T> {
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// The individual items in the module.
    pub items: Vec<T>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl<T> Library<T> {
    /// Construct a new `Library` with the given elements.
    pub fn new(items: Vec<T>) -> Self {
        Self {
            asset: None,
            items,
            extra: vec![],
        }
    }

    /// Does this element have no children?
    pub fn is_empty(&self) -> bool {
        self.asset.is_none() && self.items.is_empty() && self.extra.is_empty()
    }
}

impl<T> From<Vec<T>> for Library<T> {
    fn from(items: Vec<T>) -> Self {
        Self::new(items)
    }
}

impl<T: ParseLibrary> XNode for Library<T> {
    const NAME: &'static str = T::LIBRARY;
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Library {
            asset: Asset::parse_opt_box(&mut it)?,
            items: T::parse_list(&mut it)?, // should be 1 or more but blender disagrees
            extra: Extra::parse_many(it)?,
        })
    }
}

impl<T: ParseLibrary> XNodeWrite for Library<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem();
        if self.is_empty() {
            e.end(w)
        } else {
            let e = e.start(w)?;
            self.asset.write_to(w)?;
            self.items.write_to(w)?;
            self.extra.write_to(w)?;
            e.end(w)
        }
    }
}

impl<T: CollectLocalMaps> CollectLocalMaps for Library<T> {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        self.items.collect_local_maps(maps)
    }
}

macro_rules! mk_libraries {
    (@mkdoc $($doc:expr, $name:ident, $arg:ident,)*) => {
        /// A library element, which can be a module of any of the kinds supported by COLLADA.
        #[derive(Clone, Debug)]
        pub enum LibraryElement {
            $(#[doc = $doc] $name(Library<$arg>),)*
        }
    };
    ($($(#[derive(Traversable $(, CollectLocalMaps $($mark:literal)?)?)])?
        $name:ident($arg:ident) = $s:literal,
    )*) => {
        $(
            $(
                impl Traversable for $arg {
                    fn traverse<'a, E>(
                        doc: &'a Document,
                        f: impl FnMut(&'a $arg) -> Result<(), E>,
                    ) -> Result<(), E> {
                        doc.iter().try_for_each(f)
                    }
                }

                $(impl CollectLocalMaps $($mark)? for $arg {
                    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
                        maps.insert(self)
                    }
                })?
            )?

            impl ParseLibrary for $arg {
                const LIBRARY: &'static str = $s;

                fn extract_element(e: &LibraryElement) -> Option<&Library<Self>> {
                    if let LibraryElement::$name(arg) = e {
                        Some(arg)
                    } else {
                        None
                    }
                }

                fn mk_element(lib: Library<Self>) -> LibraryElement {
                    LibraryElement::$name(lib)
                }
            }
        )*

        mk_libraries! {
            @mkdoc $(
                concat!("Declares a module of [`", stringify!($arg), "`] elements."),
                $name, $arg,
            )*
        }

        impl LibraryElement {
            /// Parse a [`LibraryElement`] from an XML element.
            pub fn parse(e: &Element) -> Result<Option<Self>> {
                Ok(Some(match e.name() {
                    $($arg::LIBRARY => Self::$name(Library::parse(e)?),)*
                    _ => return Ok(None),
                }))
            }
        }

        impl XNodeWrite for LibraryElement {
            fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
                match self {
                    $(Self::$name(lib) => lib.write_to(w),)*
                }
            }
        }

        impl CollectLocalMaps for LibraryElement {
            fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
                match self {
                    $(Self::$name(lib) => lib.collect_local_maps(maps),)*
                }
            }
        }
    }
}

mk_libraries! {
    Animations(Animation) = "library_animations",

    #[derive(Traversable, CollectLocalMaps)]
    AnimationClips(AnimationClip) = "library_animation_clips",

    #[derive(Traversable, CollectLocalMaps)]
    Cameras(Camera) = "library_cameras",

    #[derive(Traversable, CollectLocalMaps)]
    Controllers(Controller) = "library_controllers",

    #[derive(Traversable, CollectLocalMaps)]
    Effects(Effect) = "library_effects",

    #[derive(Traversable, CollectLocalMaps)]
    ForceFields(ForceField) = "library_force_fields",

    #[derive(Traversable)]
    Geometries(Geometry) = "library_geometries",

    #[derive(Traversable, CollectLocalMaps)]
    Images(Image) = "library_images",

    #[derive(Traversable, CollectLocalMaps)]
    Lights(Light) = "library_lights",

    #[derive(Traversable, CollectLocalMaps)]
    Materials(Material) = "library_materials",

    Nodes(Node) = "library_nodes",

    #[derive(Traversable, CollectLocalMaps)]
    PhysicsMaterials(PhysicsMaterial) = "library_physics_materials",

    #[derive(Traversable, CollectLocalMaps)]
    PhysicsModels(PhysicsModel) = "library_physics_models",

    #[derive(Traversable, CollectLocalMaps)]
    PhysicsScenes(PhysicsScene) = "library_physics_scenes",

    #[derive(Traversable, CollectLocalMaps)]
    VisualScenes(VisualScene) = "library_visual_scenes",
}

/// Instantiates a COLLADA material resource,
/// possibly applying transformations or effects to the object.
///
/// The `data` field depends on the type of object being instantiated.
/// Most types use `()` for this field but some types have additional data:
/// * `Instance<`[`Geometry`]>: [`InstanceGeometryData`]
/// * `Instance<`[`Controller`]>: [`InstanceControllerData`]
/// * `Instance<`[`Effect`]>: [`InstanceEffectData`]
/// * `Instance<`[`PhysicsModel`]>: [`InstancePhysicsModelData`]
///
/// Additionally, some instance nodes are even more different and have their own types:
/// * [`InstanceMaterial`], not `Instance<`[`Material`]`>`
/// * [`InstanceRigidBody`], not `Instance<`[`RigidBody`]`>`
/// * [`InstanceRigidConstraint`], not `Instance<`[`RigidConstraint`]`>`
#[derive(Clone, Debug)]
pub struct Instance<T: Instantiate> {
    /// A text string containing the scoped identifier of this element.
    /// This value must be unique within the scope of the parent element.
    pub sid: Option<String>,
    /// The URL of the location of the `T` element to instantiate.
    /// Can refer to a local instance or external reference.
    pub url: UrlRef<T>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// The additional data associated with the instantiation, if any.
    pub data: T::Data,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

pub(crate) use private::Instantiate;
pub(crate) mod private {
    use super::*;
    /// The trait for types that can be used in [`Instance<T>`].
    pub trait Instantiate {
        /// The name of the instance node.
        /// For example `Geometry::INSTANCE = "instance_geometry"`.
        const INSTANCE: &'static str;

        /// The type of additional data associated with instantiations, possibly `()`.
        type Data: XNodeWrite + Default;

        /// Parse the [`Self::Data`] given an element iterator,
        /// and a reference to the parent element.
        fn parse_data(e: &Element, it: &mut ElementIter<'_>) -> Result<Self::Data>;

        /// Write attributes from the [`Self::Data`], before the main write.
        fn write_attr(_: &Self::Data, _: &mut ElemBuilder) {}

        /// Returns true if the data field has no elements.
        fn is_empty(_: &Self::Data) -> bool;
    }
}

impl<T: Instantiate> Instance<T> {
    /// Construct a new instance pointing at the given [`Url`] (which should
    /// reference an object of type `T`).
    pub fn new(url: Url) -> Self {
        Self {
            sid: Default::default(),
            url: Ref::new(url),
            name: Default::default(),
            data: Default::default(),
            extra: Default::default(),
        }
    }
}

impl<T: Instantiate> XNode for Instance<T> {
    const NAME: &'static str = T::INSTANCE;
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Instance {
            sid: element.attr("sid").map(Into::into),
            url: parse_attr(element.attr("url"))?.ok_or("missing url attribute")?,
            name: element.attr("name").map(Into::into),
            data: T::parse_data(element, &mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl<T: Instantiate> XNodeWrite for Instance<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("sid", &self.sid);
        e.print_attr("url", &self.url);
        e.opt_attr("name", &self.name);
        T::write_attr(&self.data, &mut e);
        if T::is_empty(&self.data) && self.extra.is_empty() {
            e.end(w)
        } else {
            let e = e.start(w)?;
            self.data.write_to(w)?;
            self.extra.write_to(w)?;
            e.end(w)
        }
    }
}

impl<T: Instantiate> CollectLocalMaps for Instance<T>
where
    T::Data: CollectLocalMaps,
{
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        self.data.collect_local_maps(maps);
    }
}

/// Either an instance of a `T`, or a directly inlined `T` object.
pub enum DefInstance<T: Instantiate> {
    /// A definition of a `T`.
    Def(T),
    /// An instantiation of a `T`.
    Ref(Instance<T>),
}

impl<T: Instantiate> From<Instance<T>> for DefInstance<T> {
    fn from(v: Instance<T>) -> Self {
        Self::Ref(v)
    }
}

impl<T: Instantiate> From<T> for DefInstance<T> {
    fn from(v: T) -> Self {
        Self::Def(v)
    }
}

impl<T: Instantiate + Debug> Debug for DefInstance<T>
where
    T::Data: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Def(t) => f.debug_tuple("Def").field(t).finish(),
            Self::Ref(t) => f.debug_tuple("Ref").field(t).finish(),
        }
    }
}

impl<T: Instantiate + Clone> Clone for DefInstance<T>
where
    T::Data: Clone,
{
    fn clone(&self) -> Self {
        match self {
            Self::Def(t) => Self::Def(t.clone()),
            Self::Ref(t) => Self::Ref(t.clone()),
        }
    }
}

impl<T: Instantiate + XNode> DefInstance<T> {
    pub(crate) fn parse(e: &Element) -> Result<Option<Self>> {
        Ok(if e.name() == T::NAME {
            Some(Self::Def(T::parse(e)?))
        } else if e.name() == T::INSTANCE {
            Some(Self::Ref(Instance::parse(e)?))
        } else {
            None
        })
    }
}

impl<T: Instantiate + XNodeWrite> XNodeWrite for DefInstance<T> {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            DefInstance::Def(e) => e.write_to(w),
            DefInstance::Ref(e) => e.write_to(w),
        }
    }
}

impl<T: Instantiate + CollectLocalMaps> CollectLocalMaps for DefInstance<T> {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        if let DefInstance::Def(t) = self {
            t.collect_local_maps(maps);
        }
    }
}

macro_rules! basic_instance {
    ($($ty:ty => $val:expr;)*) => {
        $(impl Instantiate for $ty {
            const INSTANCE: &'static str = $val;
            type Data = ();
            fn parse_data(_: &Element, _: &mut ElementIter<'_>) -> Result<Self::Data> {
                Ok(())
            }
            fn is_empty(_: &Self::Data) -> bool { true }
        })*
    }
}
basic_instance! {
    Animation => "instance_animation";
    Camera => "instance_camera";
    ForceField => "instance_force_field";
    Light => "instance_light";
    Node => "instance_node";
    PhysicsMaterial => "instance_physics_material";
    PhysicsScene => "instance_physics_scene";
    VisualScene => "instance_visual_scene";
}
