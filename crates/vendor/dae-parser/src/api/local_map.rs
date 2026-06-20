//! Contains the [`LocalMap`] and [`LocalMaps`] types, used to resolve internal ID references.

use super::*;

impl Document {
    /// Run a function over all elements of type `T` in the document.
    pub fn try_for_each<'a, T: Traversable + ?Sized + 'a, E, F: FnMut(&'a T) -> Result<(), E>>(
        &'a self,
        f: F,
    ) -> Result<(), E> {
        T::traverse(self, f)
    }

    /// Run a function over all elements of type `T` in the document.
    pub fn for_each<'a, T: Traversable + ?Sized + 'a>(&'a self, mut f: impl FnMut(&'a T)) {
        #[allow(clippy::unit_arg)]
        self.try_for_each(|e| Ok(f(e)))
            .unwrap_or_else(|e: std::convert::Infallible| match e {})
    }

    /// Construct an ID -> element mapping for node type `T`.
    /// This can be used to look up ID references.
    pub fn local_map<T: Traversable + HasId + ?Sized>(&self) -> Result<LocalMap<'_, T>> {
        let mut map = LocalMap::default();
        self.try_for_each(|v: &T| map.push(v))?;
        Ok(map)
    }

    /// Convenience function, to return the main [`VisualScene`]
    /// referred to in the `scene` field.
    pub fn get_visual_scene(&self) -> Option<&VisualScene> {
        let scene = self.scene.as_ref()?.instance_visual_scene.as_ref()?;
        self.local_map().ok()?.get(&scene.url)
    }

    /// Construct an ID -> element mapping for every node type `T`.
    /// This can be used to look up ID references.
    ///
    /// This function will initialize *every* type.
    /// See [`LocalMaps::default`] and [`LocalMaps::set`] for a builder API
    /// which allows you to pick which types you are interested in,
    /// or [`LocalMaps::new`] and [`LocalMaps::unset`] to exclude certain types.
    pub fn local_maps(&self) -> LocalMaps<'_> {
        LocalMaps::new().collect(self)
    }
}

/// A generic ID getter function.
pub trait HasId {
    /// Get the ID of the node.
    fn id(&self) -> Option<&str>;

    /// Extract the relevant `LocalMap` field from a `LocalMaps`.
    fn get_local_map<'a, 'b>(maps: &'b LocalMaps<'a>) -> &'b Option<LocalMap<'a, Self>>;

    /// Extract the relevant `LocalMap` field from a `LocalMaps`.
    fn get_local_map_mut<'a, 'b>(maps: &'b mut LocalMaps<'a>)
        -> &'b mut Option<LocalMap<'a, Self>>;
}

/// A map for looking up elements of type `T` in the document by ID.
#[derive(Clone, Debug)]
pub struct LocalMap<'a, T: ?Sized>(pub HashMap<&'a str, &'a T>);

impl<'a, T: ?Sized> Default for LocalMap<'a, T> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<'a, T: ?Sized> LocalMap<'a, T> {
    /// Look up an element by ID.
    pub fn push(&mut self, v: &'a T) -> Result<()>
    where
        T: HasId,
    {
        if let Some(id) = v.id() {
            if self.0.insert(id, v).is_some() {
                return Err(format!("duplicate id {}", id).into());
            }
        }
        Ok(())
    }

    /// Look up an element by ID.
    pub fn get_str(&self, n: &str) -> Option<&'a T> {
        self.0.get(n).copied()
    }

    /// Look up an element by ID, in a `NameRef`.
    pub fn get_name(&self, n: &NameRef<T>) -> Option<&'a T> {
        self.get_str(&n.val)
    }

    /// Look up an element by URL reference.
    ///
    /// This is a local map, meaning that it does not support references to URLs which are not
    /// of the special form `#ref`, referring to an element with ID `ref` in the same document.
    pub fn get_raw(&self, url: &Url) -> Option<&'a T> {
        match url {
            Url::Fragment(n) => self.get_str(n),
            Url::Other(_) => None,
        }
    }

    /// Look up an element by URL reference.
    ///
    /// This is a simple wrapper around [`get_raw`](Self::get_raw),
    /// but it has better type safety, since it ensures that the reference is a reference to a `T`.
    ///
    /// This is a local map, meaning that it does not support references to URLs which are not
    /// of the special form `#ref`, referring to an element with ID `ref` in the same document.
    pub fn get(&self, url: &UrlRef<T>) -> Option<&'a T> {
        self.get_raw(&url.val)
    }
}

impl<'a, T: ?Sized> Index<&NameRef<T>> for LocalMap<'a, T> {
    type Output = T;

    fn index(&self, index: &NameRef<T>) -> &Self::Output {
        self.get_name(index).unwrap()
    }
}

impl<'a, T: ?Sized> Index<&UrlRef<T>> for LocalMap<'a, T> {
    type Output = T;

    fn index(&self, index: &UrlRef<T>) -> &Self::Output {
        self.get(index).unwrap()
    }
}

/// A trait for types that can be enumerated in a [`Document`]. This is used to power the
/// [`for_each`](Document::for_each) and [`try_for_each`](Document::try_for_each) functions.
pub trait Traversable {
    /// Run the function `f` on all elements of type `Self` in the document `doc`.
    fn traverse<'a, E>(
        doc: &'a Document,
        f: impl FnMut(&'a Self) -> Result<(), E>,
    ) -> Result<(), E>
    where
        Self: 'a;
}

trait IdField {
    fn try_to_str(&self) -> Option<&str>;
}
impl IdField for Option<String> {
    fn try_to_str(&self) -> Option<&str> {
        self.as_deref()
    }
}
impl IdField for String {
    fn try_to_str(&self) -> Option<&str> {
        Some(self)
    }
}

macro_rules! mk_local_maps {
    ($($name:ident: $ty:ty,)*) => {
        /// A data structure which maintains `LocalMap`s for every applicable type.
        /// If you need to use [`Document::local_map`] for many different types,
        /// it may be more efficient to use this type, which calculates all maps
        /// in a single pass.
        #[derive(Clone, Debug, Default)]
        pub struct LocalMaps<'a> {
            $($name: Option<LocalMap<'a, $ty>>,)*
        }

        impl LocalMaps<'_> {
            /// Constructs a new `LocalMaps` with initializes all its fields to set but empty.
            /// This differs from `default`, which initializes them to `None`.
            /// Any `None` field will not be populated.
            pub fn new() -> Self {
                Self {
                    $($name: Some(LocalMap::default()),)*
                }
            }
        }
        $(
            impl HasId for $ty {
                fn id(&self) -> Option<&str> {
                    self.id.try_to_str()
                }

                fn get_local_map<'a, 'b>(maps: &'b LocalMaps<'a>) -> &'b Option<LocalMap<'a, Self>> {
                    &maps.$name
                }

                fn get_local_map_mut<'a, 'b>(
                    maps: &'b mut LocalMaps<'a>,
                ) -> &'b mut Option<LocalMap<'a, Self>> {
                    &mut maps.$name
                }
            }
        )*
    }
}

mk_local_maps! {
    animation: Animation,
    animation_clip: AnimationClip,
    camera: Camera,
    controller: Controller,
    effect: Effect,
    force_field: ForceField,
    geometry: Geometry,
    image: Image,
    light: Light,
    material: Material,
    node: Node,
    physics_material: PhysicsMaterial,
    physics_model: PhysicsModel,
    physics_scene: PhysicsScene,
    sampler: Sampler,
    source: Source,
    vertices: Vertices,
    visual_scene: VisualScene,
    idref_array: IdRefArray,
    name_array: NameArray,
    bool_array: BoolArray,
    float_array: FloatArray,
    int_array: IntArray,
}

impl<'a> LocalMaps<'a> {
    pub(crate) fn insert<T: HasId>(&mut self, t: &'a T) {
        if let Some(id) = t.id() {
            if let Some(map) = T::get_local_map_mut(self) {
                assert!(map.0.insert(id, t).is_none());
            }
        }
    }

    /// Enable collection for the given type. This is used as part of a builder-style API for
    /// customizing collection before performing the collection pass.
    /// For example, if we only want to collect `Geometry` and `Source` types:
    /// ```
    /// # use dae_parser::*;
    /// # fn foo(doc: Document) {
    /// let maps = LocalMaps::default()
    ///     .set::<Geometry>()
    ///     .set::<Source>()
    ///     .collect(&doc);
    /// # }
    /// ```
    pub fn set<T: HasId + 'a>(mut self) -> Self {
        T::get_local_map_mut(&mut self).get_or_insert_with(Default::default);
        self
    }

    /// Disable collection for the given type. This is used as part of a builder-style API for
    /// customizing collection before performing the collection pass.
    /// For example, if we want to collect everything except the `ForceField` and `Sampler` types:
    /// ```
    /// # use dae_parser::*;
    /// # fn foo(doc: Document) {
    /// let maps = LocalMaps::new()
    ///     .unset::<ForceField>()
    ///     .unset::<Sampler>()
    ///     .collect(&doc);
    /// # }
    /// ```
    pub fn unset<T: HasId + 'a>(mut self) -> Self {
        *T::get_local_map_mut(&mut self) = None;
        self
    }

    /// Run the collection pass, putting all elements in the given IDs based on their types.
    pub fn collect(mut self, t: &'a Document) -> Self {
        t.collect_local_maps(&mut self);
        self
    }

    /// Retrieve a map by type.
    pub fn get_map<T: HasId + ?Sized>(&self) -> Option<&LocalMap<'a, T>> {
        T::get_local_map(self).as_ref()
    }

    /// Look up an element by ID.
    pub fn get_str<T: HasId + ?Sized>(&self, n: &str) -> Option<&'a T> {
        self.get_map()?.get_str(n)
    }

    /// Look up an element by ID, in a `NameRef`.
    pub fn get_name<T: HasId + ?Sized>(&self, n: &NameRef<T>) -> Option<&'a T> {
        self.get_map()?.get_name(n)
    }

    /// Look up an element by URL reference.
    ///
    /// This is a local map, meaning that it does not support references to URLs which are not
    /// of the special form `#ref`, referring to an element with ID `ref` in the same document.
    pub fn get_raw<T: HasId + ?Sized>(&self, url: &Url) -> Option<&'a T> {
        self.get_map()?.get_raw(url)
    }

    /// Look up an element by URL reference.
    ///
    /// This is a simple wrapper around [`get_raw`](Self::get_raw),
    /// but it has better type safety, since it ensures that the reference is a reference to a `T`.
    ///
    /// This is a local map, meaning that it does not support references to URLs which are not
    /// of the special form `#ref`, referring to an element with ID `ref` in the same document.
    pub fn get<T: HasId + ?Sized>(&self, url: &UrlRef<T>) -> Option<&'a T> {
        self.get_map()?.get(url)
    }
}

impl<'a, T: HasId + ?Sized> Index<&NameRef<T>> for LocalMaps<'a> {
    type Output = T;

    fn index(&self, index: &NameRef<T>) -> &Self::Output {
        self.get_name(index).unwrap()
    }
}

impl<'a, T: HasId + ?Sized> Index<&UrlRef<T>> for LocalMaps<'a> {
    type Output = T;

    fn index(&self, index: &UrlRef<T>) -> &Self::Output {
        self.get(index).unwrap()
    }
}

pub(crate) trait CollectLocalMaps {
    /// Insert this node and all its ID-containing children into the `maps` data structure.
    fn collect_local_maps<'a>(&'a self, _: &mut LocalMaps<'a>) {}
}

impl<T: CollectLocalMaps> CollectLocalMaps for Option<T> {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        if let Some(x) = self {
            x.collect_local_maps(maps)
        }
    }
}

impl<T: CollectLocalMaps> CollectLocalMaps for Box<T> {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        (**self).collect_local_maps(maps)
    }
}

impl<T: CollectLocalMaps> CollectLocalMaps for Vec<T> {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        for x in self {
            x.collect_local_maps(maps)
        }
    }
}
