//! Document-level geo-reference: the coordinate reference system a model
//! is placed in (a survey grid, a national projection).
//!
//! A [`GeoReference`] says where the model's origin sits in a map, never
//! where any geometry is: it moves nothing, and nothing in Hew reads it
//! back as a transform. It is data a document carries so that a `.skp`
//! opened and saved again keeps its place on the map, and so a client can
//! read and set it.
//!
//! Names (datum, projection, unit, zone) are kept as the text the file
//! holds, with no registry behind them: Hew does not interpret them. The
//! easting, northing and height are in the CRS's own units
//! ([`GeoReference::map_unit`]); `map_unit_metres` says what one of those
//! is in meters when the file records it.

/// Why a candidate [`GeoReference`] was refused (DEVELOPMENT.md rule 4:
/// nothing is clamped or repaired).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeoReferenceError {
    /// A position, the x axis, the scale or the unit factor is NaN or
    /// infinite.
    NonFinite,
    /// The unit factor (meters per map unit) is zero or negative.
    NonPositiveUnit,
}

impl std::fmt::Display for GeoReferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GeoReferenceError::NonFinite => write!(
                f,
                "geo reference has a non-finite easting, northing, height, axis, scale or unit factor"
            ),
            GeoReferenceError::NonPositiveUnit => {
                write!(f, "geo reference unit factor must be greater than zero")
            }
        }
    }
}

/// The coordinate reference system of a document. The default is the
/// empty record (no name, origin 0/0/0, scale 0); a document with no
/// geo-reference holds `None`, not a default record.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeoReference {
    /// The system's name, as the file gives it.
    pub name: String,
    /// Free-text description.
    pub description: String,
    /// The model origin's easting, in the CRS's own unit.
    pub easting: f64,
    /// The model origin's northing, in the CRS's own unit.
    pub northing: f64,
    /// The model origin's height, in the CRS's own unit.
    pub height: f64,
    /// The model's x axis in the map plane, as a 2D vector (not an angle:
    /// grid north is rotated from it by [`GeoReference::x_axis_angle`]).
    pub x_axis: [f64; 2],
    /// The map scale factor.
    pub scale: f64,
    /// The geodetic datum, as named in the file (empty when none).
    pub geodetic_datum: String,
    /// The vertical datum, as named in the file.
    pub vertical_datum: String,
    /// The map projection, as named in the file.
    pub map_projection: String,
    /// The projection's zone, as named in the file.
    pub map_zone: String,
    /// The CRS's unit, as named in the file (`METRE`, `foot`, ...).
    pub map_unit: String,
    /// What one map unit is in meters, when the file records it.
    pub map_unit_metres: Option<f64>,
    /// The system's registry id, as the file gives it.
    pub tgl_id: String,
    /// The calibration file the system was set from.
    pub calibration_file_name: String,
}

impl GeoReference {
    /// The angle of the x axis in the map plane, in radians
    /// counter-clockwise from map east, or `None` when the file holds a
    /// zero vector (no orientation recorded).
    pub fn x_axis_angle(&self) -> Option<f64> {
        let [x, y] = self.x_axis;
        (x != 0.0 || y != 0.0).then(|| y.atan2(x))
    }

    /// Checks the record's numbers.
    ///
    /// # Errors
    /// [`GeoReferenceError`] naming the first fault.
    pub fn validate(&self) -> Result<(), GeoReferenceError> {
        let numbers = [
            self.easting,
            self.northing,
            self.height,
            self.x_axis[0],
            self.x_axis[1],
            self.scale,
        ];
        if numbers.iter().any(|n| !n.is_finite()) {
            return Err(GeoReferenceError::NonFinite);
        }
        match self.map_unit_metres {
            Some(m) if !m.is_finite() => Err(GeoReferenceError::NonFinite),
            Some(m) if m <= 0.0 => Err(GeoReferenceError::NonPositiveUnit),
            _ => Ok(()),
        }
    }
}
