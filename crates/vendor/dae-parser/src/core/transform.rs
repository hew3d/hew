use crate::*;

#[cfg(feature = "nalgebra")]
use nalgebra::{Matrix4, Point3, Vector3};

/// A transformation, that can be represented as a matrix
/// (but may be expressed in another way for convenience).
#[derive(Clone, Debug)]
pub enum Transform {
    /// Contains a position and orientation transformation suitable for aiming a camera.
    LookAt(LookAt),
    /// A generic 4x4 matrix.
    Matrix(Matrix),
    /// An axis-angle rotation.
    Rotate(Rotate),
    /// A scale along the three dimentions.
    Scale(Scale),
    /// A skew deformation.
    Skew(Skew),
    /// A translation by a vector.
    Translate(Translate),
}

impl From<LookAt> for Transform {
    fn from(v: LookAt) -> Self {
        Self::LookAt(v)
    }
}

impl From<Matrix> for Transform {
    fn from(v: Matrix) -> Self {
        Self::Matrix(v)
    }
}

impl From<Rotate> for Transform {
    fn from(v: Rotate) -> Self {
        Self::Rotate(v)
    }
}

impl From<Scale> for Transform {
    fn from(v: Scale) -> Self {
        Self::Scale(v)
    }
}

impl From<Skew> for Transform {
    fn from(v: Skew) -> Self {
        Self::Skew(v)
    }
}

impl From<Translate> for Transform {
    fn from(v: Translate) -> Self {
        Self::Translate(v)
    }
}

impl Transform {
    /// Parse a [`Transform`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        match e.name() {
            LookAt::NAME => Ok(Some(Self::LookAt(LookAt::parse(e)?))),
            Matrix::NAME => Ok(Some(Self::Matrix(Matrix::parse(e)?))),
            Rotate::NAME => Ok(Some(Self::Rotate(Rotate::parse(e)?))),
            Scale::NAME => Ok(Some(Self::Scale(Scale::parse(e)?))),
            Skew::NAME => Ok(Some(Self::Skew(Skew::parse(e)?))),
            Translate::NAME => Ok(Some(Self::Translate(Translate::parse(e)?))),
            _ => Ok(None),
        }
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        match self {
            Transform::Translate(tr) => tr.as_matrix(),
            Transform::Rotate(tr) => tr.as_matrix(),
            Transform::LookAt(tr) => tr.as_matrix(),
            Transform::Matrix(tr) => tr.as_matrix(),
            Transform::Scale(tr) => tr.as_matrix(),
            Transform::Skew(tr) => tr.as_matrix(),
        }
    }

    #[cfg(feature = "nalgebra")]
    /// Prepend this transformation to the matrix. Equivalent to `*mat *= self.as_matrix()`.
    pub fn prepend_to_matrix(&self, mat: &mut Matrix4<f32>) {
        match self {
            Transform::Translate(tr) => tr.prepend_to_matrix(mat),
            Transform::Scale(tr) => tr.prepend_to_matrix(mat),
            _ => *mat *= self.as_matrix(),
        }
    }

    #[cfg(feature = "nalgebra")]
    /// Append this transformation to the matrix. Equivalent to `*mat = self.as_matrix() * *mat`.
    pub fn append_to_matrix(&self, mat: &mut Matrix4<f32>) {
        match self {
            Transform::Translate(tr) => tr.append_to_matrix(mat),
            Transform::Scale(tr) => tr.append_to_matrix(mat),
            _ => *mat = self.as_matrix() * *mat,
        }
    }
}

impl XNodeWrite for Transform {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Translate(e) => e.write_to(w),
            Self::Rotate(e) => e.write_to(w),
            Self::LookAt(e) => e.write_to(w),
            Self::Matrix(e) => e.write_to(w),
            Self::Scale(e) => e.write_to(w),
            Self::Skew(e) => e.write_to(w),
        }
    }
}

/// A [`RigidBody`] transform is a subset of the full set of [`Transform`]s
/// which restricts to euclidean transformations (translation and rotation).
#[derive(Clone, Debug)]
pub enum RigidTransform {
    /// A translation.
    Translate(Translate),
    /// A rotation.
    Rotate(Rotate),
}

impl From<Translate> for RigidTransform {
    fn from(v: Translate) -> Self {
        Self::Translate(v)
    }
}

impl From<Rotate> for RigidTransform {
    fn from(v: Rotate) -> Self {
        Self::Rotate(v)
    }
}

impl From<RigidTransform> for Transform {
    fn from(tr: RigidTransform) -> Self {
        match tr {
            RigidTransform::Translate(v) => v.into(),
            RigidTransform::Rotate(v) => v.into(),
        }
    }
}

impl RigidTransform {
    /// Parse a [`RigidTransform`] from an XML element.
    pub fn parse(e: &Element) -> Result<Option<Self>> {
        match e.name() {
            Translate::NAME => Ok(Some(Self::Translate(Translate::parse(e)?))),
            Rotate::NAME => Ok(Some(Self::Rotate(Rotate::parse(e)?))),
            _ => Ok(None),
        }
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        match self {
            RigidTransform::Translate(tr) => tr.as_matrix(),
            RigidTransform::Rotate(tr) => tr.as_matrix(),
        }
    }

    #[cfg(feature = "nalgebra")]
    /// Prepend this transformation to the matrix. Equivalent to `*mat *= self.as_matrix()`.
    pub fn prepend_to_matrix(&self, mat: &mut Matrix4<f32>) {
        match self {
            RigidTransform::Translate(tr) => tr.prepend_to_matrix(mat),
            _ => *mat *= self.as_matrix(),
        }
    }

    #[cfg(feature = "nalgebra")]
    /// Append this transformation to the matrix. Equivalent to `*mat = self.as_matrix() * *mat`.
    pub fn append_to_matrix(&self, mat: &mut Matrix4<f32>) {
        match self {
            RigidTransform::Translate(tr) => tr.append_to_matrix(mat),
            _ => *mat = self.as_matrix() * *mat,
        }
    }
}

impl XNodeWrite for RigidTransform {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        match self {
            Self::Translate(e) => e.write_to(w),
            Self::Rotate(e) => e.write_to(w),
        }
    }
}

/// Contains a position and orientation transformation suitable for aiming a camera.
#[derive(Clone, Debug)]
pub struct LookAt(
    /// A list of 9 floating-point values.
    /// These values are organized into three vectors as follows:
    /// 1.  Eye position is given as Px, Py, Pz.
    /// 2.  Interest point is given as Ix, Iy, Iz.
    /// 3.  Up-axis direction is given as UPx, UPy, UPz.
    pub Box<[f32; 9]>,
);

impl XNode for LookAt {
    const NAME: &'static str = "lookat";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(LookAt(parse_array_n(element)?))
    }
}

impl XNodeWrite for LookAt {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_arr(Self::NAME, &*self.0, w)
    }
}

impl LookAt {
    /// Construct a look-at transform from the given eye position,
    /// target position, and up direction.
    pub fn new(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Self {
        Self(Box::new([
            eye[0], eye[1], eye[2], target[0], target[1], target[2], up[0], up[1], up[2],
        ]))
    }

    /// The eye position for this look-at orientation.
    #[inline]
    pub fn eye(&self) -> &[f32; 3] {
        self.0[0..3].try_into().unwrap()
    }

    /// The target position (interest point) for this look-at orientation.
    #[inline]
    pub fn target(&self) -> &[f32; 3] {
        self.0[3..6].try_into().unwrap()
    }

    /// The up direction for this look-at orientation.
    #[inline]
    pub fn up(&self) -> &[f32; 3] {
        self.0[6..9].try_into().unwrap()
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        let eye = Point3::from_slice(self.eye());
        let target = Point3::from_slice(self.target());
        let up = Vector3::from_column_slice(self.up());
        Matrix4::look_at_rh(&eye, &target, &up)
    }
}

/// Describes transformations that embody mathematical changes to points
/// within a coordinate system or the coordinate system itself.
#[derive(Clone, Debug)]
pub struct Matrix(
    /// A list of 16 floating-point values.
    /// These values are organized into a 4-by-4
    /// column-order matrix suitable for matrix composition.
    pub Box<[f32; 16]>,
);

impl XNode for Matrix {
    const NAME: &'static str = "matrix";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Matrix(parse_array_n(element)?))
    }
}

impl XNodeWrite for Matrix {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_arr(Self::NAME, &*self.0, w)
    }
}

impl Matrix {
    /// Construct a matrix transform from a column order 4x4 matrix.
    pub fn new(data: [f32; 16]) -> Self {
        Self(Box::new(data))
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        // We have to transpose the matrix to match nalgebra conventions
        // on projective transformations
        Matrix4::from_row_slice(&*self.0)
    }
}

#[cfg(feature = "nalgebra")]
impl From<Matrix4<f32>> for Matrix {
    fn from(mut mat: Matrix4<f32>) -> Self {
        // We have to transpose the matrix to match nalgebra conventions
        // on projective transformations
        mat.transpose_mut();
        Self(Box::new(mat.as_slice().try_into().expect("impossible")))
    }
}

#[cfg(feature = "nalgebra")]
impl From<Matrix4<f32>> for Transform {
    fn from(mat: Matrix4<f32>) -> Self {
        Self::Matrix(mat.into())
    }
}

/// Specifies how to rotate an object around an axis.
#[derive(Clone, Debug)]
pub struct Rotate(
    /// A list of four floating-point values.
    /// These values are organized into a column vector `[X, Y, Z]`
    /// specifying the axis of rotation, followed by an angle in degrees.
    pub Box<[f32; 4]>,
);

impl XNode for Rotate {
    const NAME: &'static str = "rotate";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Rotate(parse_array_n(element)?))
    }
}

impl XNodeWrite for Rotate {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_arr(Self::NAME, &*self.0, w)
    }
}

impl Rotate {
    /// Construct a new axis-angle transformation.
    pub fn new(axis: [f32; 3], angle: f32) -> Self {
        Self(Box::new([axis[0], axis[1], axis[2], angle]))
    }

    /// The rotation axis of this rotation transform.
    pub fn axis(&self) -> &[f32; 3] {
        self.0[0..3].try_into().unwrap()
    }

    /// The magnitude of this rotation transform in degrees.
    pub fn angle(&self) -> f32 {
        self.0[3]
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        let axis = Vector3::from_column_slice(self.axis()).normalize();
        Matrix4::from_axis_angle(
            &nalgebra::Unit::new_normalize(axis),
            self.angle().to_radians(),
        )
    }
}

/// Specifies how to change an object’s size.
#[derive(Clone, Debug)]
pub struct Scale(
    /// A list of three floating-point values.
    /// These values are organized into a column vector suitable for matrix composition.
    pub Box<[f32; 3]>,
);

impl XNode for Scale {
    const NAME: &'static str = "scale";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Scale(parse_array_n(element)?))
    }
}

impl XNodeWrite for Scale {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_arr(Self::NAME, &*self.0, w)
    }
}

impl Scale {
    /// Construct a new nonuniform scale transformation.
    pub fn new(val: [f32; 3]) -> Self {
        Self(Box::new(val))
    }

    /// Construct a new uniform scale transformation.
    pub fn uniform(val: f32) -> Self {
        Self::new([val, val, val])
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        Matrix4::new_nonuniform_scaling(&Vector3::from_row_slice(&*self.0))
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn prepend_to_matrix(&self, mat: &mut Matrix4<f32>) {
        mat.prepend_nonuniform_scaling_mut(&Vector3::from_row_slice(&*self.0))
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn append_to_matrix(&self, mat: &mut Matrix4<f32>) {
        mat.append_nonuniform_scaling_mut(&Vector3::from_row_slice(&*self.0))
    }
}

/// Specifies how to deform an object along one axis.
#[derive(Clone, Debug)]
pub struct Skew(
    /// A list of seven floating-point values.
    /// These values are organized into an angle in degrees
    /// followed by two column vectors specifying the axes of rotation and translation.
    pub Box<[f32; 7]>,
);

impl XNode for Skew {
    const NAME: &'static str = "skew";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Skew(parse_array_n(element)?))
    }
}

impl XNodeWrite for Skew {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_arr(Self::NAME, &*self.0, w)
    }
}

impl Skew {
    /// Construct a new skew transformation from the given parameters.
    pub fn new(angle: f32, axis: [f32; 3], trans: [f32; 3]) -> Self {
        Self(Box::new([
            angle, axis[0], axis[1], axis[2], trans[0], trans[1], trans[2],
        ]))
    }

    /// The angle of this skew transformation.
    #[inline]
    pub fn angle(&self) -> f32 {
        self.0[0]
    }

    /// The axis of rotation for this skew transformation.
    #[inline]
    pub fn axis(&self) -> &[f32; 3] {
        self.0[1..4].try_into().unwrap()
    }

    /// The axis of translation for this skew transformation.
    #[inline]
    pub fn trans(&self) -> &[f32; 3] {
        self.0[4..7].try_into().unwrap()
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        // From the RenderMan spec (on which the COLLADA operation is based):
        //
        // This operation shifts all points along lines
        // parallel to the axis vector (dx2, dy2, dz2).
        // Points along the axis vector (dx1, dy1, dz1)
        // are mapped onto the vector (x, y, z),
        // where angle specifies the angle (in degrees)
        // between the vectors (dx1, dy1, dz1) and (x, y, z),
        // The two axes are not required to be perpendicular,
        // however it is an error to specify an angle that is greater than
        // or equal to the angle between them.
        // A negative angle can be specified,
        // but it must be greater than 180 degrees minus the angle between the two axes.
        unimplemented!("<skew> transforms are not supported")
    }
}

/// Changes the position of an object in a local coordinate system.
#[derive(Clone, Debug)]
pub struct Translate(
    /// A list of three floating-point values.
    /// These values are organized into a column vector suitable for a matrix composition.
    pub Box<[f32; 3]>,
);

impl XNode for Translate {
    const NAME: &'static str = "translate";
    fn parse(element: &Element) -> Result<Self> {
        debug_assert_eq!(element.name(), Self::NAME);
        Ok(Translate(parse_array_n(element)?))
    }
}

impl XNodeWrite for Translate {
    fn write_to<W: Write>(&self, w: &mut XWriter<W>) -> Result<()> {
        ElemBuilder::print_arr(Self::NAME, &*self.0, w)
    }
}

impl Translate {
    /// Construct a new translation from the given parameters.
    pub fn new(val: [f32; 3]) -> Self {
        Self(Box::new(val))
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn as_matrix(&self) -> Matrix4<f32> {
        Matrix4::new_translation(&Vector3::from_row_slice(&*self.0))
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn prepend_to_matrix(&self, mat: &mut Matrix4<f32>) {
        mat.prepend_translation_mut(&Vector3::from_row_slice(&*self.0))
    }

    #[cfg(feature = "nalgebra")]
    /// Convert this transformation to a [`nalgebra::Matrix4`].
    pub fn append_to_matrix(&self, mat: &mut Matrix4<f32>) {
        mat.append_translation_mut(&Vector3::from_row_slice(&*self.0))
    }
}
