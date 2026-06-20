use crate::*;

#[cfg(feature = "nalgebra")]
use nalgebra::Matrix4;

/// Embodies the entire set of information that can be visualized
/// from the contents of a COLLADA resource.
#[derive(Clone, Default, Debug)]
pub struct Scene {
    /// The instantiated [`PhysicsScene`] elements describe
    /// any physics being applied to the scene.
    pub instance_physics_scene: Vec<Instance<PhysicsScene>>,
    /// The scene graph is built from the [`VisualScene`] elements instantiated under [`Scene`].
    pub instance_visual_scene: Option<Instance<VisualScene>>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl Scene {
    /// Construct a new `Scene` from a [`VisualScene`] instance.
    pub fn new(instance_visual_scene: Instance<VisualScene>) -> Self {
        Self {
            instance_physics_scene: vec![],
            instance_visual_scene: Some(instance_visual_scene),
            extra: vec![],
        }
    }
}

impl XNode for Scene {
    const NAME: &'static str = "scene";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(Scene {
            instance_physics_scene: Instance::parse_list(&mut it)?,
            instance_visual_scene: Instance::parse_opt(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for Scene {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let e = Self::elem().start(w)?;
        self.instance_physics_scene.write_to(w)?;
        self.instance_visual_scene.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Embodies the entire set of information that can be visualized
/// from the contents of a COLLADA resource.
#[derive(Clone, Debug)]
pub struct VisualScene {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// The scene graph subtrees.
    pub nodes: Vec<Node>,
    /// The [`EvaluateScene`] element declares information
    /// specifying how to evaluate this [`VisualScene`].
    pub evaluate_scene: Vec<EvaluateScene>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl VisualScene {
    /// Create a new empty `VisualScene`.
    pub fn new(id: impl Into<String>, name: Option<String>) -> Self {
        Self {
            id: Some(id.into()),
            name,
            asset: None,
            nodes: vec![],
            evaluate_scene: vec![],
            extra: vec![],
        }
    }
}

impl XNode for VisualScene {
    const NAME: &'static str = "visual_scene";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        Ok(VisualScene {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            asset: Asset::parse_opt_box(&mut it)?,
            nodes: Node::parse_list(&mut it)?,
            evaluate_scene: EvaluateScene::parse_list(&mut it)?,
            extra: Extra::parse_many(it)?,
        })
    }
}

impl XNodeWrite for VisualScene {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.asset.write_to(w)?;
        self.nodes.write_to(w)?;
        self.evaluate_scene.write_to(w)?;
        self.extra.write_to(w)?;
        e.end(w)
    }
}

/// Embodies the hierarchical relationship of elements in a scene.
///
/// The [`Node`] element declares a point of interest in a scene.
/// A node denotes one point on a branch of the scene graph.
/// The [`Node`] element is essentially the root of a subgraph of the entire scene graph.
#[derive(Clone, Debug)]
pub struct Node {
    /// A text string containing the unique identifier of the element.
    pub id: Option<String>,
    /// The text string name of this element.
    pub name: Option<String>,
    /// A text string value containing the subidentifier of this element.
    /// This value must be unique within the scope of the parent element.
    pub sid: Option<String>,
    /// The type of the [`Node`] element.
    pub ty: NodeType,
    /// The layers to which this node belongs.
    pub layer: Vec<String>,
    /// Asset management information about this element.
    pub asset: Option<Box<Asset>>,
    /// Any combination of geometric transforms.
    pub transforms: Vec<Transform>,
    /// Allows the node to instantiate a camera object.
    pub instance_camera: Vec<Instance<Camera>>,
    /// Allows the node to instantiate a controller object.
    pub instance_controller: Vec<Instance<Controller>>,
    /// Allows the node to instantiate a geometry object.
    pub instance_geometry: Vec<Instance<Geometry>>,
    /// Allows the node to instantiate a light object.
    pub instance_light: Vec<Instance<Light>>,
    /// Allows the node to instantiate a hierarchy of other nodes.
    pub instance_node: Vec<Instance<Node>>,
    /// Allows the node to recursively define hierarchy.
    pub children: Vec<Node>,
    /// Provides arbitrary additional information about this element.
    pub extra: Vec<Extra>,
}

impl XNode for Node {
    const NAME: &'static str = "node";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let extra;
        Ok(Node {
            id: element.attr("id").map(Into::into),
            name: element.attr("name").map(Into::into),
            sid: element.attr("sid").map(Into::into),
            ty: parse_attr(element.attr("type"))?.unwrap_or_default(),
            layer: element.attr("layer").map_or_else(Vec::new, |s| {
                s.split_ascii_whitespace().map(|s| s.to_owned()).collect()
            }),
            asset: Asset::parse_opt_box(&mut it)?,
            transforms: parse_list_many(&mut it, Transform::parse)?,
            instance_camera: Instance::parse_list(&mut it)?,
            instance_controller: Instance::parse_list(&mut it)?,
            instance_geometry: Instance::parse_list(&mut it)?,
            instance_light: Instance::parse_list(&mut it)?,
            instance_node: Instance::parse_list(&mut it)?,
            children: {
                // Note: this is nonconforming, COLLADA spec says `extra` should come
                // after `children`.
                // However FBX Collada exporter has been witnessed producing such files
                extra = Extra::parse_list(&mut it)?;
                Node::parse_list(&mut it)?
            },
            extra: Extra::parse_append_many(extra, it)?,
        })
    }
}

impl XNodeWrite for Node {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("id", &self.id);
        e.opt_attr("name", &self.name);
        e.opt_attr("sid", &self.sid);
        e.def_print_attr("type", self.ty, Default::default());
        if let Some(s) = arr_to_string(&self.layer) {
            e.attr("layer", &s)
        }
        if self.is_empty() {
            e.end(w)
        } else {
            let e = e.start(w)?;
            self.asset.write_to(w)?;
            self.transforms.write_to(w)?;
            self.instance_camera.write_to(w)?;
            self.instance_controller.write_to(w)?;
            self.instance_geometry.write_to(w)?;
            self.instance_light.write_to(w)?;
            self.instance_node.write_to(w)?;
            self.children.write_to(w)?;
            self.extra.write_to(w)?;
            e.end(w)
        }
    }
}

impl CollectLocalMaps for Node {
    fn collect_local_maps<'a>(&'a self, maps: &mut LocalMaps<'a>) {
        maps.insert(self);
        self.children.collect_local_maps(maps);
    }
}

impl Node {
    /// Construct a new default node with the given `id` and `name`.
    pub fn new(id: impl Into<String>, name: Option<String>) -> Self {
        Self {
            id: Some(id.into()),
            name,
            sid: Default::default(),
            ty: Default::default(),
            layer: Default::default(),
            asset: Default::default(),
            transforms: Default::default(),
            instance_camera: Default::default(),
            instance_controller: Default::default(),
            instance_geometry: Default::default(),
            instance_light: Default::default(),
            instance_node: Default::default(),
            children: Default::default(),
            extra: Default::default(),
        }
    }

    /// Add a transform to this node's transformation stack.
    pub fn push_transform(&mut self, transform: impl Into<Transform>) {
        self.transforms.push(transform.into())
    }

    fn on_children<'a, E>(
        &'a self,
        f: &mut impl FnMut(&'a Self) -> Result<(), E>,
    ) -> Result<(), E> {
        f(self)?;
        for child in &self.children {
            child.on_children(f)?
        }
        Ok(())
    }

    /// Returns true if this node has no sub-elements.
    pub fn is_empty(&self) -> bool {
        self.asset.is_none()
            && self.transforms.is_empty()
            && self.instance_camera.is_empty()
            && self.instance_controller.is_empty()
            && self.instance_geometry.is_empty()
            && self.instance_light.is_empty()
            && self.instance_node.is_empty()
            && self.children.is_empty()
            && self.extra.is_empty()
    }
}

impl Traversable for Node {
    fn traverse<'a, E>(
        doc: &'a Document,
        mut f: impl FnMut(&'a Node) -> Result<(), E>,
    ) -> Result<(), E> {
        doc.library.iter().try_for_each(|elem| match elem {
            LibraryElement::Nodes(lib) => lib.items.iter().try_for_each(|e| e.on_children(&mut f)),
            LibraryElement::VisualScenes(lib) => lib
                .items
                .iter()
                .try_for_each(|e| e.nodes.iter().try_for_each(|e| e.on_children(&mut f))),
            _ => Ok(()),
        })
    }
}

#[cfg(feature = "nalgebra")]
impl Node {
    /// Apply the transformation stack on this node to a [`Matrix4`].
    /// If `mat` beforehand is the transformation applying at the parent of the
    /// current node, then `mat` after this call is the transformation that
    /// applies to everything in this node and in the children.
    pub fn prepend_transforms(&self, mat: &mut Matrix4<f32>) {
        for t in &self.transforms {
            t.prepend_to_matrix(mat)
        }
    }

    /// Apply the transformation stack on this node to a [`Matrix4`].
    /// If `mat` before this call is a transformation applying to a child of this node,
    /// then `mat` afterward is the transformation situating the child in the parent frame.
    pub fn append_transforms(&self, mat: &mut Matrix4<f32>) {
        for t in self.transforms.iter().rev() {
            t.append_to_matrix(mat)
        }
    }

    /// Convert this node's transformation stack to a [`Matrix4`].
    pub fn transform_as_matrix(&self) -> Matrix4<f32> {
        let mut mat = Matrix4::identity();
        self.prepend_transforms(&mut mat);
        mat
    }
}

/// The type of a [`Node`] element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeType {
    /// A regular node.
    Node,
    /// A joint. See [`Joints`].
    Joint,
}

impl Default for NodeType {
    fn default() -> Self {
        Self::Node
    }
}

impl FromStr for NodeType {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "NODE" => Ok(Self::Node),
            "JOINT" => Ok(Self::Joint),
            _ => Err(()),
        }
    }
}

impl NodeType {
    /// The XML name of a value in this enumeration.
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Node => "NODE",
            Self::Joint => "JOINT",
        }
    }
}

impl Display for NodeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.to_str(), f)
    }
}

/// Declares information specifying how to evaluate a [`VisualScene`].
#[derive(Clone, Debug)]
pub struct EvaluateScene {
    /// The text string name of this element.
    pub name: Option<String>,
    /// Describes the effect passes to evaluate a scene.
    pub render: Vec<Render>,
}

impl EvaluateScene {
    /// Construct a new `EvaluateScene` with the given effect passes.
    pub fn new(render: Vec<Render>) -> Self {
        assert!(!render.is_empty());
        Self { name: None, render }
    }
}

impl XNode for EvaluateScene {
    const NAME: &'static str = "evaluate_scene";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        let mut it = element.children().peekable();
        let res = EvaluateScene {
            name: element.attr("name").map(Into::into),
            render: Render::parse_list_n::<1>(&mut it)?,
        };
        finish(res, it)
    }
}

impl XNodeWrite for EvaluateScene {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        let mut e = Self::elem();
        e.opt_attr("name", &self.name);
        let e = e.start(w)?;
        self.render.write_to(w)?;
        e.end(w)
    }
}
